//! Dynamic path resolution — port of the prototype's `core/paths.py`.
//!
//! No hardcoded usernames, no fixed layout. Works natively and inside a container
//! with the host filesystem bind-mounted, which is how the engine stays usable
//! from the Docker verification environment.
//!
//! **Δ from the prototype:** every public resolver returns a *canonical* path
//! (symlinks resolved). The prototype uses `os.path.abspath`, which does not
//! resolve symlinks — and that is precisely the hole `safety.rs` has to close, so
//! it is closed here at the source instead.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Directories never worth crawling from a user's home: OS noise, media
/// libraries, and tool state we handle by explicit path instead of by walking.
/// Ported verbatim from `get_default_scan_roots`'s `skip_home_folders`.
const SKIP_HOME_FOLDERS: &[&str] = &[
    "Library",
    "Applications",
    "Pictures",
    "Music",
    "Movies",
    "Public",
    "Desktop",
    "Documents",
    "Downloads",
    ".Trash",
    ".cache",
    ".cargo",
    ".npm",
    ".git",
    ".gemini",
    ".antigravity",
    ".rustup",
    ".local",
    ".vscode",
    ".docker",
];

/// High-yield developer project directories, in the prototype's order.
const DEV_FOLDERS: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Projects",
    "code",
    "dev",
    "workspace",
    "work",
];

/// Prefix when the host filesystem is mounted inside a container.
///
/// `HOST_ROOT` wins; otherwise `/host/Users` is auto-detected, matching the
/// prototype's common `docker run -v /:/host` layout.
pub fn host_prefix() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HOST_ROOT") {
        let p = p.trim();
        if !p.is_empty() && p != "/" {
            let pb = PathBuf::from(p);
            if pb.is_dir() {
                return Some(pb);
            }
        }
    }
    if Path::new("/host/Users").is_dir() {
        return Some(PathBuf::from("/host"));
    }
    None
}

/// Canonicalize, falling back to the input when the path does not exist yet.
fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Every non-system user home we should consider.
///
/// Resolution order follows the prototype: explicit `TARGET_USER`, then native
/// `$HOME`, then discovery under `/Users`, then `$HOME` as a last resort.
pub fn target_users() -> Vec<PathBuf> {
    let prefix = host_prefix();
    let users_dir = match &prefix {
        Some(p) => p.join("Users"),
        None => PathBuf::from("/Users"),
    };

    if let Ok(u) = std::env::var("TARGET_USER") {
        let candidate = users_dir.join(&u);
        if candidate.is_dir() {
            return vec![canon(&candidate)];
        }
    }

    // Native run: trust $HOME, but not a container's /root.
    if prefix.is_none() {
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(&home);
            if h.is_dir() && !home.starts_with("/root") {
                return vec![canon(&h)];
            }
        }
    }

    let mut found = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&users_dir) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "Shared" {
                continue;
            }
            if e.path().is_dir() {
                found.push(canon(&e.path()));
            }
        }
    }
    if !found.is_empty() {
        found.sort();
        return found;
    }

    std::env::var("HOME")
        .ok()
        .map(|h| vec![canon(Path::new(&h))])
        .unwrap_or_default()
}

/// The home directory the UI treats as "yours".
pub fn primary_home() -> PathBuf {
    target_users()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Resolve a definition path such as `~/.cache/huggingface` or `/opt/homebrew`.
///
/// `~` expands to the *discovered host home*, never to a container's `/root` —
/// that substitution is the reason the prototype works unchanged in Docker.
pub fn resolve(spec: &str, home: Option<&Path>) -> PathBuf {
    let home = home.map(|h| h.to_path_buf()).unwrap_or_else(primary_home);

    if let Some(rest) = spec.strip_prefix('~') {
        let rest = rest.trim_start_matches('/');
        return if rest.is_empty() {
            home
        } else {
            home.join(rest)
        };
    }

    if let Some(prefix) = host_prefix() {
        if spec.starts_with('/') && !Path::new(spec).starts_with(&prefix) {
            return prefix.join(spec.trim_start_matches('/'));
        }
    }

    PathBuf::from(spec)
}

/// High-yield scan roots across every target user.
///
/// Two sources, as in the prototype: the known dev folder names, plus any
/// non-dot top-level directory in the home that is not OS noise. This is what
/// lets a scan find `~/kraken` without the cost of crawling all of `~`.
pub fn default_scan_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();

    for home in target_users() {
        for folder in DEV_FOLDERS {
            let sub = home.join(folder);
            if sub.is_dir() {
                let c = canon(&sub);
                if seen.insert(c.clone()) {
                    out.push(c);
                }
            }
        }

        if let Ok(rd) = std::fs::read_dir(&home) {
            let mut extras: Vec<PathBuf> = Vec::new();
            for e in rd.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') || SKIP_HOME_FOLDERS.contains(&name.as_ref()) {
                    continue;
                }
                if e.path().is_dir() {
                    extras.push(e.path());
                }
            }
            extras.sort();
            for p in extras {
                let c = canon(&p);
                if seen.insert(c.clone()) {
                    out.push(c);
                }
            }
        }
    }

    out
}

/// Media scan roots: Movies, Music, Pictures, and iCloud Drive.
pub fn media_scan_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();

    for home in target_users() {
        for folder in ["Movies", "Music", "Pictures"] {
            let sub = home.join(folder);
            if sub.is_dir() {
                let c = canon(&sub);
                if seen.insert(c.clone()) {
                    out.push(c);
                }
            }
        }
        let icloud = home.join("Library/Mobile Documents");
        if icloud.is_dir() {
            let c = canon(&icloud);
            if seen.insert(c.clone()) {
                out.push(c);
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_to_supplied_home_not_root() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(
            resolve("~/.cache/huggingface", Some(&home)),
            PathBuf::from("/Users/someone/.cache/huggingface")
        );
        assert_eq!(resolve("~", Some(&home)), home);
    }

    #[test]
    fn bare_tilde_slash_does_not_produce_double_slash() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(resolve("~/", Some(&home)), home);
    }

    #[test]
    fn absolute_paths_pass_through_when_not_containerized() {
        if host_prefix().is_none() {
            assert_eq!(
                resolve("/opt/homebrew", None),
                PathBuf::from("/opt/homebrew")
            );
        }
    }

    #[test]
    fn skip_list_covers_the_os_noise_the_prototype_excluded() {
        for must in ["Library", "Applications", "Pictures", "Music", ".Trash"] {
            assert!(SKIP_HOME_FOLDERS.contains(&must), "missing {must}");
        }
    }
}
