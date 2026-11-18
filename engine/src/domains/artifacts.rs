//! Project-local build artifacts and regenerable caches.
//!
//! Extends the prototype's `scan_project_artifacts`, which covered `node_modules`
//! and Rust `target/` only.
//!
//! The `target/` gate is ported exactly and matters: a directory is only claimed
//! when a sibling `Cargo.toml` exists. Without it, every unrelated folder called
//! `target` on the disk becomes a deletion candidate.

use super::{Domain, Finding, Risk};
use std::path::Path;

pub const MIN_BYTES: u64 = 5 * 1024 * 1024;

/// Caches that are *unambiguously* regenerated on next build.
///
/// Deliberately excludes `dist` and `build` (see `PRUNE_DIRS`): those are
/// frequently a project's real wanted output, and offering to delete someone's
/// build output is how a cleaner loses trust.
pub const REGENERABLE: &[(&str, &str, &str)] = &[
    (
        "__pycache__",
        "Python bytecode cache",
        "regenerated on next import",
    ),
    (
        ".pytest_cache",
        "pytest cache",
        "regenerated on next test run",
    ),
    (
        ".mypy_cache",
        "mypy type cache",
        "regenerated on next type check",
    ),
    (".ruff_cache", "Ruff lint cache", "regenerated on next lint"),
    (".next", "Next.js build cache", "next build"),
    (
        ".turbo",
        "TurboRepo task cache",
        "regenerated on next turbo run",
    ),
    (".parcel-cache", "Parcel cache", "regenerated on next build"),
    (
        ".svelte-kit",
        "SvelteKit generated files",
        "svelte-kit sync",
    ),
    (".nuxt", "Nuxt build output", "nuxt build"),
    (
        ".angular",
        "Angular build cache",
        "regenerated on next build",
    ),
    (
        ".vite",
        "Vite dependency cache",
        "regenerated on next dev server start",
    ),
    (
        ".gradle",
        "Project Gradle cache",
        "regenerated on next gradle build",
    ),
    (".tox", "tox environments", "tox -r"),
    (".nx", "Nx computation cache", "regenerated on next nx run"),
    ("target", "Rust build output", "cargo build"),
];

/// Which artifact kind this directory is, if any.
///
/// `target` additionally requires the sibling `Cargo.toml` gate. That stat is
/// only ever paid for directories literally named `target`.
pub fn classify(dir: &Path) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let name = dir.file_name()?;
    let hit = REGENERABLE.iter().find(|(n, _, _)| *n == name)?;

    if name == "target" {
        let parent = dir.parent()?;
        if !parent.join("Cargo.toml").is_file() {
            return None;
        }
    }
    Some(hit)
}

pub fn finding(path: &Path, bytes: u64, shared: u64, files: u64, mtime: i64) -> Finding {
    let (kind, label, restore) = match classify(path) {
        Some(c) => *c,
        None => ("artifact", "Build artifact", "regenerated on next build"),
    };
    let project_dir = path.parent().unwrap_or(path);
    let project = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let git_time = super::git_last_touched(project_dir);
    let git_idle = git_time.map(super::days_since);

    let restore_cmd = if kind == "target" {
        format!("cd {:?} && cargo clean", project_dir)
    } else {
        restore.to_string()
    };

    Finding::new(
        Domain::Artifact,
        format!("artifact:{}", path.display()),
        format!("{project}/{kind}"),
    )
    .at(path)
    .detail(label.to_string())
    .sized(bytes, shared, files)
    .aged(mtime)
    .risk(Risk::Safe)
    .restore(restore_cmd)
    .meta(serde_json::json!({
        "kind": kind,
        "project_name": project,
        "project_dir": project_dir,
        "git_last_touched": git_time,
        "git_idle_days": git_idle,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_target_requires_a_sibling_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().join("target");
        std::fs::create_dir_all(&t).unwrap();

        // The gate: without Cargo.toml this is somebody else's `target` folder.
        assert!(classify(&t).is_none());

        std::fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(classify(&t).map(|c| c.0), Some("target"));
    }

    #[test]
    fn regenerable_caches_need_no_gate() {
        let tmp = tempfile::tempdir().unwrap();
        for n in ["__pycache__", ".mypy_cache", ".next", ".turbo"] {
            let d = tmp.path().join(n);
            std::fs::create_dir_all(&d).unwrap();
            assert_eq!(classify(&d).map(|c| c.0), Some(n), "{n}");
        }
    }

    #[test]
    fn build_output_is_not_classified_as_an_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        for n in ["dist", "build", "src", "out"] {
            let d = tmp.path().join(n);
            std::fs::create_dir_all(&d).unwrap();
            assert!(classify(&d).is_none(), "{n} must not be claimed");
        }
    }
}
