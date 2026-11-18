//! `node_modules` — port of `core/node_modules_manager.py`.

use super::{Domain, Finding, Risk};
use std::path::Path;

/// Minimum size worth reporting, ported from the prototype.
pub const MIN_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Pnpm,
    Bun,
    Yarn,
    Npm,
}

impl Manager {
    pub fn install_cmd(&self) -> &'static str {
        match self {
            Manager::Pnpm => "pnpm install",
            Manager::Bun => "bun install",
            Manager::Yarn => "yarn install",
            Manager::Npm => "npm install",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Manager::Pnpm => "pnpm",
            Manager::Bun => "bun",
            Manager::Yarn => "yarn",
            Manager::Npm => "npm",
        }
    }

    /// Whether reinstalling reads from a local store or hits the network.
    ///
    /// This changes the user's decision, so it belongs in the UI. pnpm restores by
    /// hardlinking from `~/Library/pnpm/store` — effectively instant. npm and yarn
    /// re-download.
    pub fn restores_locally(&self) -> bool {
        matches!(self, Manager::Pnpm | Manager::Bun)
    }
}

/// Lockfile precedence, exactly as the prototype orders it.
pub fn detect_manager(project: &Path) -> Manager {
    if project.join("pnpm-lock.yaml").is_file() {
        return Manager::Pnpm;
    }
    if project.join("bun.lockb").is_file() || project.join("bun.lock").is_file() {
        return Manager::Bun;
    }
    if project.join("yarn.lock").is_file() {
        return Manager::Yarn;
    }
    if project.join("package-lock.json").is_file() {
        return Manager::Npm;
    }
    Manager::Npm
}

/// True when the project uses Yarn Plug'n'Play, which has no real `node_modules`.
///
/// **Δ from the prototype:** under PnP there is nothing here worth deleting, and
/// reporting it would be a false positive. Detected by `nodeLinker: pnp` (or the
/// absence of a `node-modules` linker) in `.yarnrc.yml`.
pub fn is_yarn_pnp(project: &Path) -> bool {
    let Ok(body) = std::fs::read_to_string(project.join(".yarnrc.yml")) else {
        return false;
    };
    body.lines().any(|l| {
        let l = l.trim();
        l.starts_with("nodeLinker:") && (l.contains("pnp") || l.contains("pnpm"))
    })
}

pub fn finding(path: &Path, bytes: u64, shared: u64, files: u64, mtime: i64) -> Finding {
    let project_dir = path.parent().unwrap_or(path);
    let project = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mgr = detect_manager(project_dir);

    let share_note = if shared > bytes && bytes > 0 {
        " · mostly hardlinked to the local store"
    } else if mgr.restores_locally() {
        " · restores from local store"
    } else {
        " · reinstall re-downloads"
    };

    let git_time = super::git_last_touched(project_dir);
    let git_idle = git_time.map(super::days_since);

    Finding::new(
        Domain::NodeModules,
        format!("node_modules:{}", path.display()),
        format!("{project}/node_modules"),
    )
    .at(path)
    .detail(format!("{}{share_note}", mgr.name()))
    .sized(bytes, shared, files)
    .aged(mtime)
    .risk(Risk::Caution)
    .restore(format!("cd {project_dir:?} && {}", mgr.install_cmd()))
    .meta(serde_json::json!({
        "package_manager": mgr.name(),
        "restores_locally": mgr.restores_locally(),
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
    fn lockfile_precedence_matches_prototype() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        // No lockfile at all -> npm, as the prototype defaults.
        assert_eq!(detect_manager(p), Manager::Npm);

        std::fs::write(p.join("package-lock.json"), "{}").unwrap();
        assert_eq!(detect_manager(p), Manager::Npm);

        std::fs::write(p.join("yarn.lock"), "").unwrap();
        assert_eq!(detect_manager(p), Manager::Yarn);

        std::fs::write(p.join("bun.lockb"), "").unwrap();
        assert_eq!(detect_manager(p), Manager::Bun);

        // pnpm wins over everything.
        std::fs::write(p.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_manager(p), Manager::Pnpm);
    }

    #[test]
    fn bun_lock_text_variant_is_recognised() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bun.lock"), "").unwrap();
        assert_eq!(detect_manager(tmp.path()), Manager::Bun);
    }

    #[test]
    fn yarn_pnp_projects_are_identified() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_yarn_pnp(tmp.path()));
        std::fs::write(tmp.path().join(".yarnrc.yml"), "nodeLinker: pnp\n").unwrap();
        assert!(is_yarn_pnp(tmp.path()));
    }

    #[test]
    fn yarn_with_node_modules_linker_is_not_pnp() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".yarnrc.yml"), "nodeLinker: node-modules\n").unwrap();
        assert!(!is_yarn_pnp(tmp.path()));
    }

    #[test]
    fn local_store_managers_are_flagged_for_the_ui() {
        assert!(Manager::Pnpm.restores_locally());
        assert!(Manager::Bun.restores_locally());
        assert!(!Manager::Npm.restores_locally());
        assert!(!Manager::Yarn.restores_locally());
    }
}
