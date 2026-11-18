//! Python virtual environments — port of `core/venv_manager.py`.
//!
//! Name-agnostic by design: `.venv`, `venv`, `myenv`, `mynenv`, `custom-env` and
//! anything else are all found, because detection is structural.
//!
//! **Zero subprocess.** Packages come from reading
//! `site-packages/*.dist-info/METADATA` directly. The prototype's `pip freeze`
//! fallback is dropped (Δ): it spawns an interpreter, and the folder-name parse
//! below already covers the case it existed for. Likewise no `python --version`
//! fallback — an unparseable `pyvenv.cfg` reports `Unknown`, which is honest and free.

use super::{Domain, Finding, Risk};
use crate::fastwalk::{Entry, Kind};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// Structural detection, in the prototype's priority order.
///
/// The common case (`pyvenv.cfg` present in the listing) costs **zero extra
/// syscalls** — the walker already handed us the directory contents. Only the
/// `bin/`-based layouts need a stat, and only for directories that contain a
/// `bin` or `Scripts` child at all.
pub fn detect(dir: &Path, entries: &[Entry]) -> Option<&'static str> {
    let has = |n: &str, k: Kind| entries.iter().any(|e| &*e.name == n && e.kind == k);

    // 1. venv / virtualenv / uv
    if has("pyvenv.cfg", Kind::File) {
        return Some("venv (pyvenv.cfg)");
    }
    // 4. conda / mamba  (checked early: also free, it's a directory entry)
    if has("conda-meta", Kind::Dir) {
        return Some("conda");
    }
    // 2. Unix layout without a cfg
    if has("bin", Kind::Dir) {
        let bin = dir.join("bin");
        if bin.join("activate").is_file()
            && (bin.join("python").exists() || bin.join("python3").exists())
        {
            return Some("venv (bin/activate)");
        }
    }
    // 3. Windows layout, for shared or mounted volumes
    if has("Scripts", Kind::Dir) {
        let s = dir.join("Scripts");
        if s.join("activate").is_file() && s.join("python.exe").exists() {
            return Some("venv (Scripts)");
        }
    }
    None
}

/// Python version from `pyvenv.cfg`, accepting both `version` and `version_info`.
pub fn python_version(venv: &Path) -> String {
    let cfg = venv.join("pyvenv.cfg");
    if let Ok(f) = std::fs::File::open(&cfg) {
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            let line = line.trim();
            for key in ["version_info", "version"] {
                if let Some(rest) = line.strip_prefix(key) {
                    let rest = rest.trim_start();
                    if let Some(v) = rest.strip_prefix('=') {
                        return v.trim().to_string();
                    }
                }
            }
        }
    }
    "Unknown".into()
}

/// All `site-packages` directories inside a venv.
fn site_packages(venv: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let lib = venv.join("lib");
    if let Ok(rd) = std::fs::read_dir(&lib) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("python") {
                let sp = e.path().join("site-packages");
                if sp.is_dir() {
                    out.push(sp);
                }
            }
        }
    }
    // virtualenv on some layouts, and the Windows layout
    for alt in [lib.join("site-packages"), venv.join("Lib/site-packages")] {
        if alt.is_dir() {
            out.push(alt);
        }
    }
    out
}

/// Parse `Name:` / `Version:` out of a `METADATA` file.
///
/// Stops at the first blank line (end of the RFC822 header) or as soon as both
/// fields are found, so a package with a 40 KB long-description costs the same
/// as one without.
fn parse_metadata(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(f) = std::fs::File::open(path) else {
        return (None, None);
    };
    let mut name = None;
    let mut version = None;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        if line.is_empty() {
            break;
        }
        if name.is_none() {
            if let Some(v) = line.strip_prefix("Name: ") {
                name = Some(v.trim().to_string());
            }
        }
        if version.is_none() {
            if let Some(v) = line.strip_prefix("Version: ") {
                version = Some(v.trim().to_string());
            }
        }
        if name.is_some() && version.is_some() {
            break;
        }
    }
    (name, version)
}

/// Split `foo_bar-1.2.3.dist-info` into `("foo_bar", "1.2.3")`.
fn from_dist_info_name(folder: &str) -> Option<(String, String)> {
    let stem = folder.strip_suffix(".dist-info")?;
    match stem.rsplit_once('-') {
        Some((n, v)) if !n.is_empty() => Some((n.to_string(), v.to_string())),
        _ => Some((stem.to_string(), "unknown".to_string())),
    }
}

/// Every installed package in a venv, deduplicated case-insensitively and sorted.
pub fn packages(venv: &Path) -> Vec<Package> {
    let mut out: Vec<Package> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for sp in site_packages(venv) {
        collect_dist_info(&sp, &mut out, &mut seen);
    }
    sort_packages(&mut out);
    out
}

/// Every installed package in a single `site-packages` directory.
///
/// Shared with the global-interpreter audit, which has a bare `site-packages`
/// path rather than a venv root — same parser, no duplicated logic.
pub fn packages_in(site_packages_dir: &Path) -> Vec<Package> {
    let mut out: Vec<Package> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_dist_info(site_packages_dir, &mut out, &mut seen);
    sort_packages(&mut out);
    out
}

fn sort_packages(v: &mut [Package]) {
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
}

fn collect_dist_info(
    sp: &Path,
    out: &mut Vec<Package>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(rd) = std::fs::read_dir(sp) else {
        return;
    };
    for e in rd.flatten() {
        let folder = e.file_name().to_string_lossy().to_string();
        if !folder.ends_with(".dist-info") {
            continue;
        }
        let (mut name, mut version) = parse_metadata(&e.path().join("METADATA"));
        if name.is_none() {
            // Fallback: the directory name itself encodes both.
            if let Some((n, v)) = from_dist_info_name(&folder) {
                name = Some(n);
                version = version.or(Some(v));
            }
        }
        if let Some(n) = name {
            if seen.insert(n.to_lowercase()) {
                out.push(Package {
                    name: n,
                    version: version.unwrap_or_else(|| "unknown".into()),
                });
            }
        }
    }
}

/// Result of writing a restore manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReqInplace {
    pub file_path: PathBuf,
    pub package_count: usize,
    pub python_version: String,
}

/// Write `req-inplace.txt` into the **project root** (the venv's parent).
///
/// This is the prototype's core safety contract and it is not optional: the
/// reaper refuses to delete a venv if this write fails, because the whole reason
/// deleting a venv is acceptable is that it can be reconstructed afterwards.
pub fn write_req_inplace(venv: &Path, filename: &str) -> std::io::Result<ReqInplace> {
    use std::io::Write;

    let parent = venv.parent().unwrap_or(venv);
    let venv_name = venv
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ".venv".into());
    let pkgs = packages(venv);
    let pyver = python_version(venv);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let dest = parent.join(filename);

    let mut s = String::with_capacity(pkgs.len() * 24 + 1024);
    s.push_str("# ====================================================================\n");
    s.push_str("# Automatically generated by Woosh (req-inplace)\n");
    s.push_str(&format!("# Generated on: {now}\n"));
    s.push_str(&format!("# Source Environment: {}\n", venv.display()));
    s.push_str(&format!("# Environment Folder Name: {venv_name}\n"));
    s.push_str(&format!("# Python Version: {pyver}\n"));
    s.push_str(&format!("# Total Packages Recorded: {}\n", pkgs.len()));
    s.push_str("# --------------------------------------------------------------------\n");
    s.push_str("# To recreate this virtual environment, execute in this project root:\n");
    s.push_str(&format!("#   python3 -m venv {venv_name}\n"));
    s.push_str(&format!("#   source {venv_name}/bin/activate\n"));
    s.push_str(&format!("#   pip install -r {filename}\n"));
    s.push_str("# ====================================================================\n\n");
    for p in &pkgs {
        s.push_str(&format!("{}=={}\n", p.name, p.version));
    }

    // Write to a temp sibling then rename, so an interrupted write can never
    // leave a truncated manifest that looks complete.
    let tmp = parent.join(format!(".{filename}.woosh-tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(s.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &dest)?;

    Ok(ReqInplace {
        file_path: dest,
        package_count: pkgs.len(),
        python_version: pyver,
    })
}

/// Build the finding for a claimed venv.
pub fn finding(
    path: &Path,
    kind: &str,
    bytes: u64,
    shared: u64,
    files: u64,
    mtime: i64,
) -> Finding {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let project = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let pkgs = packages(path);
    let pyver = python_version(path);
    let has_manifest = path
        .parent()
        .map(|p| p.join("req-inplace.txt").is_file())
        .unwrap_or(false);

    let project_dir = path.parent().unwrap_or(path);
    let git_time = super::git_last_touched(project_dir);
    let git_idle = git_time.map(super::days_since);

    Finding::new(Domain::Venv, format!("venv:{}", path.display()), format!("{project}/{name}"))
        .at(path)
        .detail(format!(
            "{kind} · Python {pyver} · {} packages",
            pkgs.len()
        ))
        .sized(bytes, shared, files)
        .aged(mtime)
        .risk(Risk::Caution)
        .restore(format!(
            "cd {:?} && python3 -m venv {venv} && source {venv}/bin/activate && pip install -r req-inplace.txt",
            project_dir,
            venv = name
        ))
        .meta(serde_json::json!({
            "venv_type": kind,
            "python_version": pyver,
            "package_count": pkgs.len(),
            "packages": pkgs,
            "project_name": project,
            "project_dir": project_dir,
            "req_inplace_exists": has_manifest,
            "git_last_touched": git_time,
            "git_idle_days": git_idle,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fastwalk::Entry;

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.into(),
            kind,
            alloc: 0,
            ino: 1,
            dev: 1,
            mtime: 0,
        }
    }

    #[test]
    fn detects_pyvenv_cfg_without_touching_disk() {
        let e = vec![entry("pyvenv.cfg", Kind::File), entry("bin", Kind::Dir)];
        assert_eq!(
            detect(Path::new("/nonexistent"), &e),
            Some("venv (pyvenv.cfg)")
        );
    }

    #[test]
    fn detects_conda_environments() {
        let e = vec![entry("conda-meta", Kind::Dir)];
        assert_eq!(detect(Path::new("/nonexistent"), &e), Some("conda"));
    }

    #[test]
    fn name_agnostic_detection() {
        // The whole point: folder name is irrelevant.
        let e = vec![entry("pyvenv.cfg", Kind::File)];
        for n in ["/p/.venv", "/p/mynenv", "/p/custom-env", "/p/whatever"] {
            assert!(detect(Path::new(n), &e).is_some(), "{n}");
        }
    }

    #[test]
    fn ignores_ordinary_directories() {
        let e = vec![entry("src", Kind::Dir), entry("README.md", Kind::File)];
        assert_eq!(detect(Path::new("/p/proj"), &e), None);
    }

    #[test]
    fn dist_info_folder_name_fallback() {
        assert_eq!(
            from_dist_info_name("requests-2.31.0.dist-info"),
            Some(("requests".into(), "2.31.0".into()))
        );
        assert_eq!(
            from_dist_info_name("typing_extensions-4.9.0.dist-info"),
            Some(("typing_extensions".into(), "4.9.0".into()))
        );
        assert_eq!(
            from_dist_info_name("weird.dist-info"),
            Some(("weird".into(), "unknown".into()))
        );
    }

    #[test]
    fn metadata_parse_and_req_inplace_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let venv = tmp.path().join("mynenv");
        let sp = venv.join("lib/python3.13/site-packages");
        std::fs::create_dir_all(&sp).unwrap();
        std::fs::write(
            venv.join("pyvenv.cfg"),
            "home = /opt/homebrew/bin\nversion = 3.13.1\n",
        )
        .unwrap();

        let di = sp.join("requests-2.31.0.dist-info");
        std::fs::create_dir_all(&di).unwrap();
        std::fs::write(
            di.join("METADATA"),
            "Metadata-Version: 2.1\nName: requests\nVersion: 2.31.0\n\nLong description here\n",
        )
        .unwrap();

        // A package whose METADATA is missing must still be recorded, via folder name.
        std::fs::create_dir_all(sp.join("orphan-9.9.9.dist-info")).unwrap();

        assert_eq!(python_version(&venv), "3.13.1");
        let pkgs = packages(&venv);
        assert_eq!(pkgs.len(), 2);
        assert!(pkgs
            .iter()
            .any(|p| p.name == "requests" && p.version == "2.31.0"));
        assert!(pkgs
            .iter()
            .any(|p| p.name == "orphan" && p.version == "9.9.9"));

        let r = write_req_inplace(&venv, "req-inplace.txt").unwrap();
        assert_eq!(r.package_count, 2);
        // Written to the PROJECT root, not inside the venv about to be deleted.
        assert_eq!(r.file_path, tmp.path().join("req-inplace.txt"));
        let body = std::fs::read_to_string(&r.file_path).unwrap();
        assert!(body.contains("requests==2.31.0"));
        assert!(body.contains("python3 -m venv mynenv"));
        assert!(body.contains("Python Version: 3.13.1"));
    }
}
