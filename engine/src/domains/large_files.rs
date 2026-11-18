//! Large and forgotten files: big videos, installers, archives, VM disks, and packages.
//!
//! Unlike development caches and virtualenvs, these are user files:
//! - **Never Safe and never pre-selected**: user must explicitly review and check them.
//! - **Trash only**: reaper refuses permanent deletion.
//! - **Never descend into packages**: `.photoslibrary`, `.app`, `.band`, `.utm`, etc.
//!   are reported as one Advisory unit.

use super::{Domain, Finding, Risk};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Package extensions that must never be descended into and are reported
/// as one Advisory unit ("manage in Photos / Finder / etc.").
pub const PACKAGES: &[(&str, &str, &str)] = &[
    ("photoslibrary", "Photos Library", "Photos"),
    ("fcpbundle", "Final Cut Pro Library", "Final Cut Pro"),
    ("logicx", "Logic Pro Project", "Logic Pro"),
    ("band", "GarageBand Project", "GarageBand"),
    ("app", "Application Bundle", "Finder"),
    ("musiclibrary", "Music Library", "Music"),
    ("tvlibrary", "TV Library", "TV"),
    ("imovielibrary", "iMovie Library", "iMovie"),
    ("utm", "UTM Virtual Machine", "UTM"),
];

/// Returns `Some((ext, label, managing_app))` if `path` is a known package bundle.
pub fn is_package(path: &Path) -> Option<(&'static str, &'static str, &'static str)> {
    let ext = path.extension()?.to_str()?;
    PACKAGES
        .iter()
        .find(|(e, _, _)| e.eq_ignore_ascii_case(ext))
        .copied()
}

/// Is `path` inside any known package bundle? Used to refuse deleting files inside packages.
pub fn is_inside_package(path: &Path) -> bool {
    let mut current = path.parent();
    while let Some(parent) = current {
        if is_package(parent).is_some() {
            return true;
        }
        current = parent.parent();
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCategory {
    Video,
    DiskImage,
    Installer,
    Archive,
    VmDisk,
    Audio,
    Other,
}

/// Classify a file by extension into high-level categories.
pub fn classify_extension(ext: &str) -> (FileCategory, &'static str) {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "mov" | "mkv" | "avi" | "m4v" | "webm" => (FileCategory::Video, "Video"),
        "dmg" | "iso" | "img" | "sparseimage" => (FileCategory::DiskImage, "Disk image"),
        "pkg" | "xip" | "ipsw" => (FileCategory::Installer, "Installer"),
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" | "xz" | "zst" => {
            (FileCategory::Archive, "Archive")
        }
        "vmdk" | "qcow2" | "vdi" | "raw" => (FileCategory::VmDisk, "VM disk"),
        "wav" | "aif" | "aiff" | "flac" => (FileCategory::Audio, "Audio"),
        _ => (FileCategory::Other, "File"),
    }
}

/// Check if a matching `.app` exists in `/Applications` or `/System/Applications`.
fn matches_installed_app(name: &str) -> bool {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    let base = stem.split(['_', '-', '.']).next().unwrap_or(stem);
    if base.is_empty() {
        return false;
    }
    for app_dir in ["/Applications", "/System/Applications"] {
        let direct = PathBuf::from(app_dir).join(format!("{base}.app"));
        if direct.exists() {
            return true;
        }
        if let Ok(rd) = std::fs::read_dir(app_dir) {
            for e in rd.flatten() {
                let n = e.file_name();
                let n_str = n.to_string_lossy();
                if n_str
                    .to_ascii_lowercase()
                    .starts_with(&base.to_ascii_lowercase())
                    && n_str.ends_with(".app")
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Construct a finding for a package bundle (e.g. `.photoslibrary`).
pub fn package_finding(path: &Path, bytes: u64, shared: u64, files: u64, mtime: i64) -> Finding {
    let (_, label, app) = is_package(path).unwrap_or(("package", "Package Bundle", "Finder"));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| label.to_string());
    let mut f = Finding::new(
        Domain::LargeFile,
        format!("pkg:{}", path.display()),
        format!("{stem} ({label})"),
    );
    f.path = Some(path.to_path_buf());
    f.detail = format!("{label} — manage it in {app}");
    f.bytes = bytes;
    f.shared_bytes = shared;
    f.files = files;
    f.mtime = mtime;
    f.days_idle = super::days_since(mtime);
    f.risk = Risk::Advisory;
    f.restore = None;
    f.meta = Some(serde_json::json!({
        "package": true,
        "app": app,
    }));
    f
}

/// Construct a finding for a regular large file.
pub fn file_finding(path: &Path, bytes: u64, shared: u64, mtime: i64) -> Finding {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let (cat, cat_label) = classify_extension(ext);
    let days_idle = super::days_since(mtime);

    let is_download = path.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("downloads")
    });

    let is_installer_like = matches!(cat, FileCategory::Installer | FileCategory::DiskImage)
        || ext.eq_ignore_ascii_case("ipsw");

    let detail = if is_download {
        if is_installer_like && matches_installed_app(&name) {
            format!("{cat_label} in Downloads — app is already installed in /Applications")
        } else if is_installer_like {
            format!("{cat_label} in Downloads — installer can be removed if already installed")
        } else if cat == FileCategory::Archive {
            "Archive in Downloads — safe to remove if already extracted".to_string()
        } else {
            format!("Downloaded {cat_label} — Woosh moves it to the Trash; Put Back restores it.")
        }
    } else {
        "Your file. Woosh moves it to the Trash; Put Back restores it.".to_string()
    };

    let mut f = Finding::new(Domain::LargeFile, format!("file:{}", path.display()), name);
    f.path = Some(path.to_path_buf());
    f.detail = detail;
    f.bytes = bytes;
    f.shared_bytes = shared;
    f.files = 1;
    f.mtime = mtime;
    f.days_idle = days_idle;
    f.risk = Risk::Caution;
    f.restore = Some("Put Back from Trash restores it to its original location".to_string());
    f.meta = Some(serde_json::json!({
        "category": cat,
        "extension": ext,
        "is_download": is_download,
    }));
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_extensions_recognized() {
        for ext in ["photoslibrary", "band", "logicx", "fcpbundle", "app", "utm"] {
            let p = Path::new("test").with_extension(ext);
            assert!(is_package(&p).is_some(), "expected {ext} to be a package");
        }
        assert!(is_package(Path::new("normal_dir")).is_none());
        assert!(is_package(Path::new("video.mp4")).is_none());
    }

    #[test]
    fn inside_package_detected() {
        let inside =
            Path::new("/Users/me/Pictures/Photos Library.photoslibrary/resources/model.bin");
        assert!(is_inside_package(inside));

        let outside = Path::new("/Users/me/Movies/vacation.mp4");
        assert!(!is_inside_package(outside));
    }

    #[test]
    fn extensions_categorized() {
        assert_eq!(classify_extension("mp4").0, FileCategory::Video);
        assert_eq!(classify_extension("MOV").0, FileCategory::Video);
        assert_eq!(classify_extension("dmg").0, FileCategory::DiskImage);
        assert_eq!(classify_extension("xip").0, FileCategory::Installer);
        assert_eq!(classify_extension("tar.gz").0, FileCategory::Other); // file_name ext is gz
        assert_eq!(classify_extension("gz").0, FileCategory::Archive);
        assert_eq!(classify_extension("vmdk").0, FileCategory::VmDisk);
        assert_eq!(classify_extension("flac").0, FileCategory::Audio);
    }
}
