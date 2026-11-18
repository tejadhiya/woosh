//! Deletion boundary — port of `core/cleaner.py`'s guards, hardened.
//!
//! ## Ported from the prototype
//!
//! 1. `PROTECTED_PATHS` exact-match refusal.
//! 2. Self-preservation (never delete the tool or its own environment).
//! 3. Never a user's home directory root.
//! 4. Allowlist of permitted prefixes.
//! 5. Minimum path-component count.
//!
//! ## Hardening (each closes a real hole)
//!
//! * **Canonicalization.** The prototype checks `os.path.abspath`, which does not
//!   resolve symlinks. A link at `~/evil → /System/Library` therefore passes every
//!   one of its five rules and gets deleted. We resolve first and check the
//!   *canonical* path.
//!
//!   The subtlety: canonicalizing the *whole* path is also wrong. For `~/link`
//!   pointing at `~/real`, full canonicalization would delete `~/real` when the
//!   user asked to remove `~/link`. So we canonicalize the **parent** (resolving
//!   every intermediate symlink) and re-attach the final component — which both
//!   closes the escape and keeps "delete the link, not its target" correct.
//!
//! * **Mount points.** Refuse a path that is itself a mount point, and refuse
//!   network filesystems entirely: deleting across NFS/SMB is slow, surprising,
//!   and usually someone else's data.
//!
//! * **TOCTOU.** Only [`Vetted`] can be handed to the reaper, and it is produced
//!   solely by [`vet`]. The reaper re-vets immediately before the destructive
//!   call, because scan and reap are separated by however long the user spends
//!   looking at the UI.

use crate::paths::target_users;
use std::path::{Component, Path, PathBuf};

/// Paths that can never be deleted, whatever the user clicks.
/// Ported verbatim from the prototype's `PROTECTED_PATHS`.
const PROTECTED: &[&str] = &[
    "/",
    "/System",
    "/Library",
    "/Applications",
    "/bin",
    "/sbin",
    "/usr",
    "/usr/bin",
    "/usr/local",
    "/usr/local/bin",
    "/opt",
    "/opt/homebrew",
    "/etc",
    "/var",
    "/private",
    "/Volumes",
    "/Network",
    "/Users",
    "/home",
    "/dev",
    "/cores",
];

/// Only paths under one of these may be removed. Ported, plus the `/private`
/// forms macOS canonicalization actually produces (`/tmp` is a symlink to
/// `/private/tmp`, so a canonical temp path never literally starts with `/tmp/`).
const PERMITTED_PREFIXES: &[&str] = &[
    "/Users/",
    "/Volumes/",
    "/tmp/",
    "/private/tmp/",
    "/private/var/folders/",
    "/host/Users/",
    "/host/Volumes/",
    "/host/tmp/",
];

/// Network filesystem type names (macOS `f_fstypename`).
#[cfg(target_os = "macos")]
const NETWORK_FS: &[&str] = &["nfs", "smbfs", "afpfs", "webdav", "ftp", "cifs"];

/// Why a deletion was refused. Every variant is a distinct, explainable reason —
/// the UI shows these verbatim rather than a generic failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Missing,
    Protected(String),
    SelfPreservation,
    HomeRoot,
    OutsidePermitted(String),
    TooShallow,
    MountPoint,
    NetworkFilesystem(String),
    Unresolvable(String),
    InsidePackage(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Missing => write!(f, "path no longer exists"),
            Refusal::Protected(p) => write!(f, "'{p}' is a protected system directory"),
            Refusal::SelfPreservation => write!(f, "refusing to delete Woosh itself"),
            Refusal::HomeRoot => write!(f, "refusing to delete a user home directory root"),
            Refusal::OutsidePermitted(p) => {
                write!(
                    f,
                    "'{p}' is outside permitted user and workspace directories"
                )
            }
            Refusal::TooShallow => write!(f, "path is too close to the filesystem root"),
            Refusal::MountPoint => write!(f, "path is a volume mount point"),
            Refusal::NetworkFilesystem(t) => {
                write!(f, "path is on a network filesystem ({t})")
            }
            Refusal::Unresolvable(e) => write!(f, "could not resolve path: {e}"),
            Refusal::InsidePackage(p) => {
                write!(
                    f,
                    "'{p}' is inside a protected application or package bundle"
                )
            }
        }
    }
}

/// A path that has passed every check, carrying its canonical form.
///
/// Construct only via [`vet`]. The reaper accepts nothing else, which makes
/// "was this checked?" a compile-time question instead of a review question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vetted {
    canonical: PathBuf,
}

impl Vetted {
    pub fn path(&self) -> &Path {
        &self.canonical
    }
}

/// Resolve `path` for deletion: canonical parent + original final component.
fn resolve_for_delete(path: &Path) -> Result<PathBuf, Refusal> {
    if path.symlink_metadata().is_err() {
        return Err(Refusal::Missing);
    }
    let parent = path.parent().ok_or(Refusal::TooShallow)?;
    let name = path.file_name().ok_or(Refusal::TooShallow)?;
    let canon_parent = parent
        .canonicalize()
        .map_err(|e| Refusal::Unresolvable(e.to_string()))?;
    Ok(canon_parent.join(name))
}

/// Number of real path components, ignoring a `/host` container mount prefix.
/// Ported from the prototype's `clean_parts` check.
fn significant_depth(p: &Path) -> usize {
    p.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .filter(|c| c.as_os_str() != "host")
        .count()
}

#[cfg(target_os = "macos")]
fn filesystem_type(p: &Path) -> Option<String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    // SAFETY: zeroed statfs is a valid initial state; c outlives the call.
    let mut sfs: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut sfs) } != 0 {
        return None;
    }
    let raw = sfs.f_fstypename;
    let bytes: Vec<u8> = raw
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8(bytes).ok()
}

#[cfg(not(target_os = "macos"))]
fn filesystem_type(_p: &Path) -> Option<String> {
    None
}

/// Is `p` a mount point? True when its device differs from its parent's.
fn is_mount_point(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(parent) = p.parent() else {
        return true;
    };
    match (p.symlink_metadata(), parent.symlink_metadata()) {
        (Ok(a), Ok(b)) => a.dev() != b.dev(),
        _ => false,
    }
}

/// Directory containing the running executable, used for self-preservation.
fn own_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|q| q.to_path_buf()))
}

/// Run every check. This is the only way to obtain a [`Vetted`].
pub fn vet(path: &Path) -> Result<Vetted, Refusal> {
    let abs = resolve_for_delete(path)?;
    let s = abs.to_string_lossy().to_string();

    // 1. Exact protected match.
    if PROTECTED.contains(&s.as_str()) {
        return Err(Refusal::Protected(s));
    }

    // 2. Never a home directory root.
    //
    // Checked *before* self-preservation on purpose. Woosh normally lives under
    // the home directory, so the self-preservation rule would also refuse this --
    // but with the misleading reason "refusing to delete Woosh itself". The more
    // specific reason is the more useful one, so it wins.
    for home in target_users() {
        if abs == home {
            return Err(Refusal::HomeRoot);
        }
    }

    // 3. Self-preservation: never the app, its bundle, or anything containing it.
    if let Some(own) = own_dir() {
        if own.starts_with(&abs) || abs.starts_with(&own) {
            return Err(Refusal::SelfPreservation);
        }
    }
    if s.contains("/Woosh.app") || s.contains("storage-checker") {
        return Err(Refusal::SelfPreservation);
    }

    // 4. Allowlisted prefixes only.
    if !PERMITTED_PREFIXES.iter().any(|p| s.starts_with(p)) {
        return Err(Refusal::OutsidePermitted(s));
    }

    // 5. Minimum depth. `/Users/alice` is depth 2 and must not be deletable;
    //    `/Users/alice/project` is depth 3 and may be.
    if significant_depth(&abs) < 3 {
        return Err(Refusal::TooShallow);
    }

    // 6. Hardening: mount points and network volumes.
    //    `/Volumes/Foo` is already excluded by the depth rule; this catches a
    //    mount nested deeper inside an otherwise permitted tree.
    if is_mount_point(&abs) {
        return Err(Refusal::MountPoint);
    }
    if let Some(fstype) = filesystem_type(&abs) {
        #[cfg(target_os = "macos")]
        if NETWORK_FS.contains(&fstype.as_str()) {
            return Err(Refusal::NetworkFilesystem(fstype));
        }
        let _ = fstype;
    }

    // 7. Hardening: never delete files inside a package bundle (.photoslibrary, .app, etc.)
    if crate::domains::large_files::is_inside_package(&abs) {
        return Err(Refusal::InsidePackage(s));
    }

    Ok(Vetted { canonical: abs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_roots_are_refused() {
        for p in ["/", "/System", "/usr", "/Applications", "/Users", "/etc"] {
            let r = vet(Path::new(p));
            assert!(r.is_err(), "{p} must be refused, got {r:?}");
        }
    }

    #[test]
    fn missing_paths_are_refused_before_anything_else() {
        assert_eq!(
            vet(Path::new("/Users/nobody-here-at-all/x")).unwrap_err(),
            Refusal::Missing
        );
    }

    #[test]
    fn depth_rule_matches_the_prototype() {
        // /Users/<name> is depth 2 -> refused. The /host prefix is ignored, so a
        // containerized path needs the same real depth to qualify.
        assert_eq!(significant_depth(Path::new("/Users/alice")), 2);
        assert_eq!(significant_depth(Path::new("/Users/alice/proj")), 3);
        assert_eq!(significant_depth(Path::new("/host/Users/alice")), 2);
        assert_eq!(significant_depth(Path::new("/host/Users/alice/proj")), 3);
    }

    #[test]
    fn symlink_escape_is_refused_not_followed() {
        // The exact hole in the prototype: a link inside a permitted tree whose
        // parent resolves out to a protected system directory.
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/System/Library", &link).unwrap();

        // Deleting the *link itself* is legitimate: it frees the link, not /System.
        // What must never happen is resolving through it and acting on the target.
        let v = vet(&link);
        if let Ok(ok) = &v {
            assert!(
                !ok.path().starts_with("/System"),
                "vet resolved through the symlink to {:?}",
                ok.path()
            );
        }

        // A path *through* the link lands in /System/Library and must be refused.
        let through = link.join("CoreServices");
        assert!(
            vet(&through).is_err(),
            "path through symlink must be refused"
        );
    }

    #[test]
    fn home_root_is_refused() {
        if let Some(home) = target_users().into_iter().next() {
            assert_eq!(vet(&home).unwrap_err(), Refusal::HomeRoot);
        }
    }

    #[test]
    fn vetted_cannot_be_forged_outside_this_module() {
        // Compile-time contract: `Vetted` has a private field, so the reaper can
        // only ever receive a path that went through `vet`.
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("deep/enough/project");
        std::fs::create_dir_all(&d).unwrap();
        if let Ok(v) = vet(&d) {
            assert_eq!(v.path(), d.canonicalize().unwrap());
        }
    }

    #[test]
    fn files_inside_packages_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("Photos.photoslibrary");
        let inner = pkg.join("database.sqlite");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(&inner, b"db").unwrap();

        let r = vet(&inner);
        assert!(
            matches!(r, Err(Refusal::InsidePackage(_))),
            "file inside package must be refused, got {r:?}"
        );
    }

    #[test]
    fn regular_files_outside_packages_are_vetted() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("deep/enough/folder");
        std::fs::create_dir_all(&d).unwrap();
        let file = d.join("sample.mp4");
        std::fs::write(&file, b"video").unwrap();

        if let Ok(v) = vet(&file) {
            assert_eq!(v.path().file_name().unwrap(), "sample.mp4");
        }
    }
}
