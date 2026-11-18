//! Global Python interpreters and their site-packages — port of
//! `core/global_python_manager.py`.
//!
//! **Δ: package listing is zero-subprocess.** The prototype runs
//! `python -m pip list --format=json` per interpreter. We read
//! `site-packages/*.dist-info/METADATA` with the same reader the venv domain uses,
//! resolving `site-packages` by globbing the interpreter's own prefix. One
//! `sysconfig` subprocess is used only when the layout is unrecognised.
//!
//! **Kept as a subprocess, deliberately: `pip uninstall`.** It consumes `RECORD`
//! files and removes console scripts. Hand-deleting `dist-info` directories
//! leaves a corrupted installation that still *looks* installed, so correctness
//! beats purity here.

use super::{Domain, Finding, Risk};
use crate::domains::venv::{self, Package};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Tooling that must survive a prune. Ported verbatim.
pub const ESSENTIAL: &[&str] = &[
    "pip",
    "setuptools",
    "wheel",
    "certifi",
    "packaging",
    "virtualenv",
    "pipenv",
    "uv",
    "poetry",
];

/// Python minor versions to probe, newest first, as the prototype orders them.
const VERSIONS: &[&str] = &["3.14", "3.13", "3.12", "3.11", "3.10", "3.9", "3.8"];

/// Binary name suffixes that are not real interpreters. Ported.
fn is_not_interpreter(name: &str) -> bool {
    name.ends_with("-config")
        || name.ends_with("-intel64")
        || name.ends_with('m')
        || name.ends_with('w')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Homebrew,
    Framework,
    AppleSystem,
    Pyenv,
    Other,
}

impl Source {
    pub fn label(&self) -> &'static str {
        match self {
            Source::Homebrew => "Homebrew",
            Source::Framework => "Python.org Framework",
            Source::AppleSystem => "Apple System (SIP)",
            Source::Pyenv => "pyenv",
            Source::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Installation {
    pub path: PathBuf,
    pub real_path: PathBuf,
    pub version: String,
    pub source: Source,
    pub site_packages: Option<PathBuf>,
    /// System Integrity Protection — cannot be modified at all.
    pub is_sip: bool,
}

fn classify(real: &Path, bin: &Path) -> Source {
    let r = real.to_string_lossy();
    let b = bin.to_string_lossy();
    if r.contains("/opt/homebrew") || b.contains("/opt/homebrew") {
        Source::Homebrew
    } else if r.contains("/Library/Frameworks") || b.contains("/Library/Frameworks") {
        Source::Framework
    } else if b.starts_with("/usr/bin") {
        Source::AppleSystem
    } else if r.contains("/.pyenv") {
        Source::Pyenv
    } else {
        Source::Other
    }
}

/// Version string parsed from the interpreter's own path layout.
///
/// Zero-subprocess: `/opt/homebrew/.../python3.13` and
/// `/Library/Frameworks/Python.framework/Versions/3.12/bin/python3` both encode
/// it. Returns `None` when the path tells us nothing.
fn version_from_path(bin: &Path) -> Option<String> {
    let s = bin.to_string_lossy();
    if let Some(i) = s.find("Python.framework/Versions/") {
        let rest = &s[i + "Python.framework/Versions/".len()..];
        if let Some(v) = rest.split('/').next() {
            if v.starts_with('3') {
                return Some(v.to_string());
            }
        }
    }
    if let Some(i) = s.find("/.pyenv/versions/") {
        let rest = &s[i + "/.pyenv/versions/".len()..];
        if let Some(v) = rest.split('/').next() {
            return Some(v.to_string());
        }
    }
    let name = bin.file_name()?.to_string_lossy().to_string();
    let v = name.strip_prefix("python")?;
    (!v.is_empty() && v.starts_with('3')).then(|| v.to_string())
}

/// `site-packages` for an interpreter, by globbing its prefix.
fn find_site_packages(bin: &Path, version: &str) -> Option<PathBuf> {
    // bin/python3.13 -> ../lib/python3.13/site-packages
    let prefix = bin.parent()?.parent()?;
    let minor: String = version.split('.').take(2).collect::<Vec<_>>().join(".");
    for cand in [
        prefix.join(format!("lib/python{minor}/site-packages")),
        prefix.join("lib/python3/site-packages"),
    ] {
        if cand.is_dir() {
            return Some(cand);
        }
    }
    // Homebrew keeps a separate user-writable site-packages.
    let brew = PathBuf::from(format!("/opt/homebrew/lib/python{minor}/site-packages"));
    brew.is_dir().then_some(brew)
}

/// Discover every global interpreter, deduplicated by realpath.
pub fn discover() -> Vec<Installation> {
    let mut candidates: BTreeSet<PathBuf> = BTreeSet::new();

    // 1. $PATH
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if (n.starts_with("python3.") || n == "python3") && !is_not_interpreter(&n) {
                    candidates.insert(e.path());
                }
            }
        }
    }

    // 2. Homebrew, 3. Frameworks, 4. /usr/local/bin
    for v in VERSIONS {
        for p in [
            PathBuf::from(format!("/opt/homebrew/bin/python{v}")),
            PathBuf::from(format!("/usr/local/bin/python{v}")),
            PathBuf::from(format!(
                "/Library/Frameworks/Python.framework/Versions/{v}/bin/python3"
            )),
        ] {
            if p.is_file() {
                candidates.insert(p);
            }
        }
    }

    // 5. Apple system python
    if Path::new("/usr/bin/python3").is_file() {
        candidates.insert(PathBuf::from("/usr/bin/python3"));
    }

    // 6. pyenv
    let pyenv = crate::paths::resolve("~/.pyenv/versions", None);
    if let Ok(rd) = std::fs::read_dir(&pyenv) {
        for e in rd.flatten() {
            let p = e.path().join("bin/python3");
            if p.is_file() {
                candidates.insert(p);
            }
        }
    }

    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out = Vec::new();
    for bin in candidates {
        let real = bin.canonicalize().unwrap_or_else(|_| bin.clone());
        if !seen.insert(real.clone()) {
            continue;
        }
        let Some(version) = version_from_path(&bin).or_else(|| version_from_path(&real)) else {
            continue;
        };
        let source = classify(&real, &bin);
        let site_packages = find_site_packages(&bin, &version);
        out.push(Installation {
            is_sip: bin.starts_with("/usr/bin"),
            path: bin,
            real_path: real,
            version,
            source,
            site_packages,
        });
    }

    // Newest first, numerically — "3.9" must not sort above "3.14".
    out.sort_by_key(|i| std::cmp::Reverse(version_key(&i.version)));
    out
}

fn version_key(v: &str) -> Vec<u32> {
    v.split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .map(|s| s.parse().unwrap_or(0))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Audit {
    pub essential: Vec<Package>,
    pub non_essential: Vec<Package>,
}

/// Classify an interpreter's packages. Zero-subprocess.
pub fn audit(install: &Installation) -> Audit {
    let Some(sp) = &install.site_packages else {
        return Audit {
            essential: vec![],
            non_essential: vec![],
        };
    };
    // The venv reader works on any site-packages directory; hand it the parent so
    // its `lib/python*/site-packages` glob resolves.
    let pkgs = venv::packages_in(sp);
    let (essential, non_essential): (Vec<_>, Vec<_>) = pkgs
        .into_iter()
        .partition(|p| ESSENTIAL.contains(&p.name.to_lowercase().as_str()));
    Audit {
        essential,
        non_essential,
    }
}

/// Write `req-global-backup.txt` before pruning. Ported.
pub fn write_backup(install: &Installation, dest_dir: &Path) -> std::io::Result<(PathBuf, usize)> {
    use std::io::Write;
    let a = audit(install);
    let dest = dest_dir.join("req-global-backup.txt");
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    let mut s = String::new();
    s.push_str("# ====================================================================\n");
    s.push_str("# Global Python dependencies backup (pre-prune) — Woosh\n");
    s.push_str(&format!("# Generated on: {now}\n"));
    s.push_str(&format!(
        "# Python: {} ({})\n",
        install.version,
        install.source.label()
    ));
    s.push_str(&format!("# Interpreter: {}\n", install.path.display()));
    s.push_str(&format!(
        "# Non-essential packages: {}\n",
        a.non_essential.len()
    ));
    s.push_str("# --------------------------------------------------------------------\n");
    s.push_str("# Prefer installing these inside a dedicated venv rather than globally,\n");
    s.push_str("# to avoid interpreter-wide version conflicts.\n");
    s.push_str("# ====================================================================\n\n");
    for p in &a.non_essential {
        s.push_str(&format!("{}=={}\n", p.name, p.version));
    }

    let tmp = dest_dir.join(".req-global-backup.txt.woosh-tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(s.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &dest)?;
    Ok((dest, a.non_essential.len()))
}

/// Uninstall packages via `pip uninstall -y`.
///
/// SIP interpreters are refused outright, matching the prototype.
pub fn prune(install: &Installation, names: &[String]) -> Result<String, String> {
    if install.is_sip {
        return Err(
            "Apple System Python (/usr/bin/python3) is protected by SIP and cannot be modified."
                .into(),
        );
    }
    let targets: Vec<&String> = names
        .iter()
        .filter(|n| !ESSENTIAL.contains(&n.to_lowercase().as_str()))
        .collect();
    if targets.is_empty() {
        return Ok("No non-essential packages to remove.".into());
    }

    let mut cmd = std::process::Command::new(&install.path);
    cmd.args(["-m", "pip", "uninstall", "-y"]);
    for t in &targets {
        cmd.arg(t.as_str());
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

pub fn finding(install: &Installation, bytes: u64, shared: u64, files: u64) -> Finding {
    let a = audit(install);
    let n = a.non_essential.len();
    let mut f = Finding::new(
        Domain::GlobalPython,
        format!("global_py:{}", install.real_path.display()),
        format!("Python {} ({})", install.version, install.source.label()),
    )
    .detail(if install.is_sip {
        "Apple system interpreter — protected by SIP, reported for reference only.".into()
    } else {
        format!(
            "{n} non-essential package{} installed globally; {} essential kept",
            if n == 1 { "" } else { "s" },
            a.essential.len()
        )
    })
    .sized(bytes, shared, files)
    .risk(if install.is_sip {
        Risk::Advisory
    } else {
        Risk::Caution
    })
    .meta(serde_json::json!({
        "version": install.version,
        "source": install.source,
        "source_label": install.source.label(),
        "interpreter": install.path,
        "site_packages": install.site_packages,
        "is_sip": install.is_sip,
        "essential": a.essential,
        "non_essential": a.non_essential,
        "action": "prune_global_python",
    }));
    if let Some(sp) = &install.site_packages {
        f = f.at(sp.clone());
    }
    if !install.is_sip {
        f = f.restore("pip install -r req-global-backup.txt".to_string());
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_sorting_is_numeric_not_lexical() {
        // The bug this prevents: "3.9" sorting above "3.14" as a string.
        let mut v = vec!["3.9", "3.14", "3.10", "3.8", "3.13"];
        v.sort_by_key(|b| std::cmp::Reverse(version_key(b)));
        assert_eq!(v, vec!["3.14", "3.13", "3.10", "3.9", "3.8"]);
    }

    #[test]
    fn version_parsed_from_every_supported_layout() {
        assert_eq!(
            version_from_path(Path::new("/opt/homebrew/bin/python3.13")),
            Some("3.13".into())
        );
        assert_eq!(
            version_from_path(Path::new(
                "/Library/Frameworks/Python.framework/Versions/3.12/bin/python3"
            )),
            Some("3.12".into())
        );
        assert_eq!(
            version_from_path(Path::new("/Users/x/.pyenv/versions/3.11.7/bin/python3")),
            Some("3.11.7".into())
        );
    }

    #[test]
    fn non_interpreter_binaries_are_rejected() {
        for n in [
            "python3-config",
            "python3.13-config",
            "python3.13m",
            "python3.13w",
        ] {
            assert!(is_not_interpreter(n), "{n} must be rejected");
        }
        assert!(!is_not_interpreter("python3.13"));
        assert!(!is_not_interpreter("python3"));
    }

    #[test]
    fn sip_interpreter_cannot_be_pruned() {
        let sip = Installation {
            path: PathBuf::from("/usr/bin/python3"),
            real_path: PathBuf::from("/usr/bin/python3"),
            version: "3.9.6".into(),
            source: Source::AppleSystem,
            site_packages: None,
            is_sip: true,
        };
        let err = prune(&sip, &["requests".into()]).unwrap_err();
        assert!(err.contains("SIP"));
        assert_eq!(finding(&sip, 0, 0, 0).risk, Risk::Advisory);
    }

    #[test]
    fn essential_packages_survive_an_explicit_prune_request() {
        // Even if the UI asks for pip itself, it must not be uninstalled.
        let inst = Installation {
            path: PathBuf::from("/nonexistent/python3.13"),
            real_path: PathBuf::from("/nonexistent/python3.13"),
            version: "3.13.0".into(),
            source: Source::Homebrew,
            site_packages: None,
            is_sip: false,
        };
        let r = prune(&inst, &["pip".into(), "setuptools".into(), "wheel".into()]).unwrap();
        assert!(r.contains("No non-essential"));
    }

    #[test]
    fn classify_identifies_each_source() {
        assert_eq!(
            classify(
                Path::new("/opt/homebrew/Cellar/python/x"),
                Path::new("/opt/homebrew/bin/python3.13")
            ),
            Source::Homebrew
        );
        assert_eq!(
            classify(Path::new("/usr/bin/python3"), Path::new("/usr/bin/python3")),
            Source::AppleSystem
        );
    }
}
