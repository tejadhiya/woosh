//! Toolchain version managers (Rustup, Gradle, NVM, Pyenv).
//!
//! Developers accumulate multiple GBs of older, inactive compiler toolchains
//! and runtimes. Woosh identifies non-default versions and reports them with
//! safe restore and uninstallation hints.

use super::{Domain, Finding, Risk};
use crate::dedup::InodeSet;
use crate::stats::Stats;
use std::fs;
use std::path::Path;

/// Probe toolchain version managers for inactive versions.
pub fn probe_toolchains(home: &Path, stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let mut findings = Vec::new();

    findings.extend(probe_rustup(home, stats, inodes));
    findings.extend(probe_gradle_dists(home, stats, inodes));
    findings.extend(probe_nvm(home, stats, inodes));
    findings.extend(probe_pyenv(home, stats, inodes));

    findings
}

/// Discover inactive Rustup toolchains (e.g. older nightlies when stable is default).
fn probe_rustup(home: &Path, stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let rustup_home = home.join(".rustup");
    let toolchains_dir = rustup_home.join("toolchains");
    if !toolchains_dir.is_dir() {
        return Vec::new();
    }

    // Read default toolchain from ~/.rustup/settings.toml if present
    let settings_file = rustup_home.join("settings.toml");
    let default_toolchain = fs::read_to_string(&settings_file)
        .ok()
        .and_then(|content| {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("default_toolchain") {
                    let parts: Vec<&str> = trimmed.split('=').collect();
                    if parts.len() == 2 {
                        return Some(parts[1].trim().trim_matches('"').to_string());
                    }
                }
            }
            None
        })
        .unwrap_or_default();

    let entries = match fs::read_dir(&toolchains_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut findings = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }

        let name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Don't flag the active default toolchain
        if !default_toolchain.is_empty() && name == default_toolchain {
            continue;
        }

        let m = crate::sweep::measure(&p, stats, inodes);
        if m.charge.exclusive == 0 && m.charge.shared == 0 {
            continue;
        }

        let is_nightly = name.starts_with("nightly");
        let risk = if is_nightly {
            Risk::Safe
        } else {
            Risk::Caution
        };

        let mut f = Finding::new(
            Domain::Cache,
            format!("toolchain:rustup:{name}"),
            format!("Rust toolchain {name}"),
        );
        f.path = Some(p.clone());
        f.detail = format!(
            "Inactive Rust toolchain (default is {})",
            if default_toolchain.is_empty() {
                "active"
            } else {
                &default_toolchain
            }
        );
        f.bytes = m.charge.exclusive;
        f.shared_bytes = m.charge.shared;
        f.files = m.files;
        f.risk = risk;
        f.restore = Some(format!("rustup toolchain install {name}"));
        findings.push(f);
    }

    findings
}

/// Discover older Gradle wrapper distributions (~/.gradle/wrapper/dists).
fn probe_gradle_dists(home: &Path, stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let dists_dir = home.join(".gradle/wrapper/dists");
    if !dists_dir.is_dir() {
        return Vec::new();
    }

    let entries = match fs::read_dir(&dists_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut dir_entries = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                dir_entries.push((name.to_string(), p));
            }
        }
    }

    if dir_entries.len() <= 1 {
        return Vec::new(); // Keep the single version
    }

    // Sort to keep newest version (e.g. gradle-8.11.1 > gradle-8.3 > gradle-7.5)
    dir_entries.sort_by(|a, b| {
        let ver_a = parse_version_tuple(&a.0);
        let ver_b = parse_version_tuple(&b.0);
        ver_b.cmp(&ver_a)
    });

    let newest_name = dir_entries[0].0.clone();
    let mut findings = Vec::new();

    // All versions except the newest are candidate for reclamation
    for (name, path) in dir_entries.into_iter().skip(1) {
        let m = crate::sweep::measure(&path, stats, inodes);
        if m.charge.exclusive == 0 && m.charge.shared == 0 {
            continue;
        }

        let mut f = Finding::new(
            Domain::Cache,
            format!("toolchain:gradle:{name}"),
            format!("Gradle wrapper {name}"),
        );
        f.path = Some(path);
        f.detail = format!("Old Gradle distribution (newest {newest_name} kept)");
        f.bytes = m.charge.exclusive;
        f.shared_bytes = m.charge.shared;
        f.files = m.files;
        f.risk = Risk::Safe;
        f.restore = Some("Downloaded automatically by gradlew when needed".into());
        findings.push(f);
    }

    findings
}

/// Discover unused NVM Node.js versions (~/.nvm/versions/node).
fn probe_nvm(home: &Path, stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let nvm_home = home.join(".nvm");
    let node_dir = nvm_home.join("versions/node");
    if !node_dir.is_dir() {
        return Vec::new();
    }

    let default_alias = fs::read_to_string(nvm_home.join("alias/default"))
        .unwrap_or_default()
        .trim()
        .to_string();

    let entries = match fs::read_dir(&node_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut versions = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                versions.push((name.to_string(), p));
            }
        }
    }

    if versions.len() <= 1 {
        return Vec::new();
    }

    let mut findings = Vec::new();
    for (name, path) in versions {
        // If default alias is e.g. "18" and version is "v18.20.8", don't remove default
        let is_default = !default_alias.is_empty()
            && (name == default_alias
                || name == format!("v{default_alias}")
                || name.starts_with(&format!("v{default_alias}.")));

        if is_default {
            continue;
        }

        let m = crate::sweep::measure(&path, stats, inodes);
        if m.charge.exclusive == 0 && m.charge.shared == 0 {
            continue;
        }

        let mut f = Finding::new(
            Domain::Cache,
            format!("toolchain:nvm:{name}"),
            format!("Node.js {name}"),
        );
        f.path = Some(path);
        f.detail = format!(
            "Inactive Node.js version (default alias is {})",
            if default_alias.is_empty() {
                "active"
            } else {
                &default_alias
            }
        );
        f.bytes = m.charge.exclusive;
        f.shared_bytes = m.charge.shared;
        f.files = m.files;
        f.risk = Risk::Caution;
        f.restore = Some(format!("nvm install {name}"));
        findings.push(f);
    }

    findings
}

/// Discover inactive Pyenv Python versions (~/.pyenv/versions).
fn probe_pyenv(home: &Path, stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let pyenv_home = home.join(".pyenv");
    let versions_dir = pyenv_home.join("versions");
    if !versions_dir.is_dir() {
        return Vec::new();
    }

    let global_version = fs::read_to_string(pyenv_home.join("version"))
        .unwrap_or_default()
        .trim()
        .to_string();

    let entries = match fs::read_dir(&versions_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut versions = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                versions.push((name.to_string(), p));
            }
        }
    }

    if versions.len() <= 1 {
        return Vec::new();
    }

    let mut findings = Vec::new();
    for (name, path) in versions {
        if !global_version.is_empty() && name == global_version {
            continue;
        }

        let m = crate::sweep::measure(&path, stats, inodes);
        if m.charge.exclusive == 0 && m.charge.shared == 0 {
            continue;
        }

        let mut f = Finding::new(
            Domain::Cache,
            format!("toolchain:pyenv:{name}"),
            format!("Python {name} (pyenv)"),
        );
        f.path = Some(path);
        f.detail = format!(
            "Inactive Pyenv Python version (global is {})",
            if global_version.is_empty() {
                "active"
            } else {
                &global_version
            }
        );
        f.bytes = m.charge.exclusive;
        f.shared_bytes = m.charge.shared;
        f.files = m.files;
        f.risk = Risk::Caution;
        f.restore = Some(format!("pyenv install {name}"));
        findings.push(f);
    }

    findings
}

/// Parse a version string into numeric parts for semantic comparison (e.g. "8.11.1" -> [8, 11, 1]).
fn parse_version_tuple(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_gradle_dists_keeps_newest() {
        let dir = tempdir().unwrap();
        let dists = dir.path().join(".gradle/wrapper/dists");
        fs::create_dir_all(dists.join("gradle-7.5-bin")).unwrap();
        fs::create_dir_all(dists.join("gradle-8.3-bin")).unwrap();
        fs::create_dir_all(dists.join("gradle-8.11.1-all")).unwrap();

        fs::write(dists.join("gradle-7.5-bin/dummy.jar"), vec![0u8; 1024]).unwrap();
        fs::write(dists.join("gradle-8.3-bin/dummy.jar"), vec![0u8; 1024]).unwrap();
        fs::write(dists.join("gradle-8.11.1-all/dummy.jar"), vec![0u8; 1024]).unwrap();

        let stats = Stats::default();
        let inodes = InodeSet::new();

        let findings = probe_gradle_dists(dir.path(), &stats, &inodes);

        assert_eq!(findings.len(), 2);
        assert!(findings
            .iter()
            .any(|f| f.name == "Gradle wrapper gradle-7.5-bin"));
        assert!(findings
            .iter()
            .any(|f| f.name == "Gradle wrapper gradle-8.3-bin"));
        assert!(!findings
            .iter()
            .any(|f| f.name == "Gradle wrapper gradle-8.11.1-all"));
    }
}
