//! Domain detectors and the findings they produce.
//!
//! Detection is **structural, never name-based**, matching the prototype's design
//! intent: a virtual environment is recognised by `pyvenv.cfg` or `bin/activate`,
//! not by being called `.venv`, so `mynenv` and `custom-env` are found too.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod artifacts;
pub mod caches;
pub mod docker;
pub mod global_python;
pub mod large_files;
pub mod models;
pub mod node;
pub mod simulators;
pub mod toolchains;
pub mod venv;

/// Top-level grouping shown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Venv,
    NodeModules,
    GlobalPython,
    AiModel,
    Docker,
    Cache,
    Artifact,
    LargeFile,
}

impl Domain {
    pub fn label(&self) -> &'static str {
        match self {
            Domain::Venv => "Virtual environments",
            Domain::NodeModules => "node_modules",
            Domain::GlobalPython => "Global Python",
            Domain::AiModel => "AI & ML models",
            Domain::Docker => "Docker",
            Domain::Cache => "Toolchain caches",
            Domain::Artifact => "Build artifacts",
            Domain::LargeFile => "Large files",
        }
    }
}

/// How much care a finding needs before removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Regenerated automatically on next use. Safe to select by default.
    Safe,
    /// Recoverable, but costs real time or network to rebuild.
    Caution,
    /// Never deleted by Woosh — reported so the user can act with the right tool.
    Advisory,
}

/// One reclaimable thing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub domain: Domain,
    pub name: String,
    /// Absent for action-based findings (Docker prunes, Ollama model deletes).
    pub path: Option<PathBuf>,
    pub detail: String,
    /// Bytes actually returned by deleting this (hardlink-deduplicated, plan §1).
    pub bytes: u64,
    /// Bytes shared with something else already counted. Freed only when the last
    /// referrer goes — shown separately so the total never over-promises.
    pub shared_bytes: u64,
    pub files: u64,
    pub mtime: i64,
    pub days_idle: i64,
    pub risk: Risk,
    /// How to get it back. The whole safety story for `Caution` findings.
    pub restore: Option<String>,
    /// Domain-specific extras: package lists, package manager, snapshot counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl Finding {
    pub fn new(domain: Domain, id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            domain,
            name: name.into(),
            path: None,
            detail: String::new(),
            bytes: 0,
            shared_bytes: 0,
            files: 0,
            mtime: 0,
            days_idle: 0,
            risk: Risk::Safe,
            restore: None,
            meta: None,
        }
    }

    pub fn at(mut self, p: impl Into<PathBuf>) -> Self {
        self.path = Some(p.into());
        self
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }
    pub fn risk(mut self, r: Risk) -> Self {
        self.risk = r;
        self
    }
    pub fn restore(mut self, c: impl Into<String>) -> Self {
        self.restore = Some(c.into());
        self
    }
    pub fn meta(mut self, m: serde_json::Value) -> Self {
        self.meta = Some(m);
        self
    }
    pub fn sized(mut self, bytes: u64, shared: u64, files: u64) -> Self {
        self.bytes = bytes;
        self.shared_bytes = shared;
        self.files = files;
        self
    }
    pub fn aged(mut self, mtime: i64) -> Self {
        self.mtime = mtime;
        self.days_idle = days_since(mtime);
        self
    }
}

/// Whole days between `mtime` and now. Negative clock skew clamps to 0.
pub fn days_since(mtime: i64) -> i64 {
    if mtime <= 0 {
        return 0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    ((now - mtime) / 86_400).max(0)
}

/// Read the timestamp of the last git commit/checkout from `.git/logs/HEAD`.
/// Zero subprocesses: reads filesystem metadata directly.
pub fn git_last_touched(project_dir: &Path) -> Option<i64> {
    let git_log = project_dir.join(".git/logs/HEAD");
    if let Ok(meta) = std::fs::metadata(&git_log) {
        if let Ok(mtime) = meta.modified() {
            if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                return Some(dur.as_secs() as i64);
            }
        }
    }
    let git_index = project_dir.join(".git/index");
    if let Ok(meta) = std::fs::metadata(&git_index) {
        if let Ok(mtime) = meta.modified() {
            if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                return Some(dur.as_secs() as i64);
            }
        }
    }
    None
}

/// Directory names pruned from traversal entirely.
///
/// Ported from the prototype's walk filters. `dist` and `build` are pruned but
/// deliberately **not** reported: unlike the caches in [`artifacts`], they are
/// frequently a project's real, wanted output, and a cleaner that offers to
/// delete someone's build output has lost the user's trust.
pub const PRUNE_DIRS: &[&str] = &[
    ".git",
    "Library",
    ".Trash",
    "Pods",
    "DerivedData",
    "dist",
    "build",
    ".venv-backup",
];

/// Path fragments that abort a traversal branch, ported from the prototype's
/// `skip_substrings`.
pub const SKIP_FRAGMENTS: &[&str] = &[
    "/Library/",
    "/.Trash",
    "/Applications/",
    "/.git/",
    "/.cargo/",
    "/.npm/",
    "/.cache/",
    "/.rustup/",
    "/.antigravity",
    "/.gemini",
    "/node_modules/",
];

/// True when this directory should not be entered at all.
pub fn is_noise(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if SKIP_FRAGMENTS.iter().any(|f| s.contains(f)) {
        return true;
    }
    path.file_name()
        .map(|n| PRUNE_DIRS.contains(&n.to_string_lossy().as_ref()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_output_dirs_are_pruned_but_never_reported() {
        // Guards the deliberate choice above: pruning `dist`/`build` keeps the
        // scan fast; reporting them would offer to delete real build output.
        assert!(PRUNE_DIRS.contains(&"dist"));
        assert!(PRUNE_DIRS.contains(&"build"));
        assert!(is_noise(Path::new("/Users/a/proj/dist")));
    }

    #[test]
    fn library_and_trash_branches_are_skipped() {
        assert!(is_noise(Path::new("/Users/a/Library/Caches/x")));
        assert!(is_noise(Path::new("/Users/a/.Trash")));
        assert!(!is_noise(Path::new("/Users/a/dev/kraken")));
    }

    #[test]
    fn days_since_clamps_future_timestamps() {
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 999_999;
        assert_eq!(days_since(future), 0);
        assert_eq!(days_since(0), 0);
    }

    #[test]
    fn git_last_touched_reads_logs_head_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let proj = temp.path();
        let logs_head = proj.join(".git/logs/HEAD");
        std::fs::create_dir_all(logs_head.parent().unwrap()).unwrap();
        std::fs::write(&logs_head, "commit ref").unwrap();

        let touched = git_last_touched(proj);
        assert!(touched.is_some());
        assert!(touched.unwrap() > 0);

        // Without .git directory
        let other = temp.path().join("no-git");
        std::fs::create_dir_all(&other).unwrap();
        assert!(git_last_touched(&other).is_none());
    }
}
