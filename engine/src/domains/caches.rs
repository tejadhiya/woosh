//! Toolchain, IDE and platform caches — port of `core/cache_manager.py`
//! plus the expansion targets from the research pass (plan §4).
//!
//! Table-driven: every entry is a path spec, a human explanation, a risk level and
//! how to get it back. `~` resolves through [`crate::paths::resolve`], so the whole
//! table works unchanged against a container-mounted host filesystem.

use super::{Domain, Finding, Risk};
use crate::paths;
use std::path::{Path, PathBuf};

/// Minimum size worth showing, ported from the prototype.
pub const MIN_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    PackageManagers,
    AppleMobile,
    Ides,
    MlCaches,
    System,
}

impl Group {
    pub fn label(&self) -> &'static str {
        match self {
            Group::PackageManagers => "Package managers",
            Group::AppleMobile => "Apple & mobile",
            Group::Ides => "IDEs & logs",
            Group::MlCaches => "ML toolchains",
            Group::System => "System",
        }
    }
}

pub struct CacheDef {
    pub id: &'static str,
    pub group: Group,
    pub name: &'static str,
    pub detail: &'static str,
    /// Path spec; `~` expands to the discovered host home.
    pub spec: &'static str,
    /// Env var that relocates this cache, checked before `spec`.
    pub env_override: Option<&'static str>,
    pub risk: Risk,
    /// Canonical command, shown as a hint. Woosh deletes the path directly; this
    /// tells the user what the tool itself would have done.
    pub cleanup_hint: Option<&'static str>,
}

/// Every known cache. Prototype entries first, expansion second — all of them
/// carry the prototype's `safe_to_clean` semantics in the `risk` field
/// (`~/.android` stays `Caution`, exactly as the prototype refused to auto-clean it).
pub const CACHES: &[CacheDef] = &[
    // ---------------- package managers (ported) ----------------
    CacheDef {
        id: "cache:npm",
        group: Group::PackageManagers,
        name: "npm cache",
        detail: "Cached tarballs and git repos from npm install.",
        spec: "~/.npm",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("npm cache clean --force"),
    },
    CacheDef {
        id: "cache:pip",
        group: Group::PackageManagers,
        name: "pip cache",
        detail: "Cached wheels and source distributions.",
        spec: "~/Library/Caches/pip",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("pip cache purge"),
    },
    CacheDef {
        id: "cache:homebrew",
        group: Group::PackageManagers,
        name: "Homebrew cache",
        detail: "Downloaded bottle archives and git clones.",
        spec: "~/Library/Caches/Homebrew",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("brew cleanup -s"),
    },
    CacheDef {
        id: "cache:gradle",
        group: Group::PackageManagers,
        name: "Gradle caches",
        detail: "Cached jars, distributions and build cache.",
        spec: "~/.gradle/caches",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:cargo_registry",
        group: Group::PackageManagers,
        name: "Cargo registry cache",
        detail: "Downloaded .crate archives.",
        spec: "~/.cargo/registry/cache",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:yarn",
        group: Group::PackageManagers,
        name: "Yarn (classic) cache",
        detail: "Global tarball store for Yarn 1.x.",
        spec: "~/Library/Caches/Yarn",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("yarn cache clean"),
    },
    CacheDef {
        id: "cache:pnpm",
        group: Group::PackageManagers,
        name: "pnpm store",
        detail: "Content-addressable store. Deleting it forces every pnpm project to re-download.",
        spec: "~/Library/pnpm/store",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: Some("pnpm store prune"),
    },
    CacheDef {
        id: "cache:bun",
        group: Group::PackageManagers,
        name: "Bun install cache",
        detail: "Decompressed npm tarballs; commonly several GB after a year.",
        spec: "~/.bun/install/cache",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("bun pm cache rm"),
    },
    CacheDef {
        id: "cache:go_build",
        group: Group::PackageManagers,
        name: "Go build cache",
        detail: "Compiled package artifacts from go build.",
        spec: "~/Library/Caches/go-build",
        env_override: Some("GOCACHE"),
        risk: Risk::Safe,
        cleanup_hint: Some("go clean -cache"),
    },
    CacheDef {
        id: "cache:go_mod",
        group: Group::PackageManagers,
        name: "Go module cache",
        detail: "Downloaded read-only module dependencies.",
        spec: "~/go/pkg/mod",
        env_override: Some("GOMODCACHE"),
        risk: Risk::Caution,
        cleanup_hint: Some("go clean -modcache"),
    },
    CacheDef {
        id: "cache:cocoapods",
        group: Group::PackageManagers,
        name: "CocoaPods cache",
        detail: "Downloaded pod archives.",
        spec: "~/Library/Caches/CocoaPods",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("pod cache clean --all"),
    },
    // ---------------- package managers (expansion) ----------------
    CacheDef {
        id: "cache:cargo_git",
        group: Group::PackageManagers,
        name: "Cargo git checkouts",
        detail: "Full git clones of git-dependency crates.",
        spec: "~/.cargo/git",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:yarn_berry",
        group: Group::PackageManagers,
        name: "Yarn Berry cache",
        detail: "Zip-per-package store for Yarn 2+.",
        spec: "~/.yarn/berry/cache",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("yarn cache clean"),
    },
    CacheDef {
        id: "cache:deno",
        group: Group::PackageManagers,
        name: "Deno cache",
        detail: "Remote modules and compiled artifacts.",
        spec: "~/Library/Caches/deno",
        env_override: Some("DENO_DIR"),
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:turbo",
        group: Group::PackageManagers,
        name: "TurboRepo cache",
        detail: "Global task output cache.",
        spec: "~/.turbo",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:maven",
        group: Group::PackageManagers,
        name: "Maven repository",
        detail: "Local .m2 artifact repository.",
        spec: "~/.m2/repository",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:gradle_daemon",
        group: Group::PackageManagers,
        name: "Gradle daemon logs",
        detail: "Daemon logs; individual files can reach several GB.",
        spec: "~/.gradle/daemon",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:uv",
        group: Group::PackageManagers,
        name: "uv cache",
        detail: "Wheel and source cache for uv.",
        spec: "~/Library/Caches/uv",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("uv cache clean"),
    },
    CacheDef {
        id: "cache:conda_pkgs",
        group: Group::PackageManagers,
        name: "Conda package cache",
        detail: "Downloaded conda packages and tarballs.",
        spec: "~/.conda/pkgs",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("conda clean --all"),
    },
    CacheDef {
        id: "cache:bazel",
        group: Group::PackageManagers,
        name: "Bazel cache",
        detail: "Action cache and output base; can reach tens of GB.",
        spec: "~/.cache/bazel",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("bazel clean --expunge"),
    },
    CacheDef {
        id: "cache:node_gyp",
        group: Group::PackageManagers,
        name: "node-gyp headers",
        detail: "Downloaded Node headers for native module builds.",
        spec: "~/.node-gyp",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:electron",
        group: Group::PackageManagers,
        name: "Electron downloads",
        detail: "Cached Electron binaries, ~200 MB per version.",
        spec: "~/Library/Caches/electron",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:electron_builder",
        group: Group::PackageManagers,
        name: "electron-builder cache",
        detail: "Cached build toolchains and app images.",
        spec: "~/Library/Caches/electron-builder",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:playwright",
        group: Group::PackageManagers,
        name: "Playwright browsers",
        detail: "Full Chromium/Firefox/WebKit builds, ~500 MB each.",
        spec: "~/Library/Caches/ms-playwright",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("playwright uninstall --all"),
    },
    CacheDef {
        id: "cache:puppeteer",
        group: Group::PackageManagers,
        name: "Puppeteer browsers",
        detail: "Downloaded Chromium revisions.",
        spec: "~/Library/Caches/puppeteer",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    // ---------------- Apple & mobile ----------------
    CacheDef {
        id: "cache:xcode_deriveddata",
        group: Group::AppleMobile,
        name: "Xcode DerivedData",
        detail: "Intermediate builds, module caches and indexes. Rebuilt on demand.",
        spec: "~/Library/Developer/Xcode/DerivedData",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:xcode_archives",
        group: Group::AppleMobile,
        name: "Xcode Archives",
        detail: "Past release builds and dSYMs. Needed to symbolicate old crash reports.",
        spec: "~/Library/Developer/Xcode/Archives",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:ios_devicesupport",
        group: Group::AppleMobile,
        name: "iOS DeviceSupport",
        detail: "Symbols from physical devices you have connected, per iOS version.",
        spec: "~/Library/Developer/Xcode/iOS DeviceSupport",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:watchos_devicesupport",
        group: Group::AppleMobile,
        name: "watchOS DeviceSupport",
        detail: "Symbols from connected watches.",
        spec: "~/Library/Developer/Xcode/watchOS DeviceSupport",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:simulator_caches",
        group: Group::AppleMobile,
        name: "CoreSimulator caches",
        detail: "Transient simulator runtime caches.",
        spec: "~/Library/Developer/CoreSimulator/Caches",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: Some("xcrun simctl delete unavailable"),
    },
    CacheDef {
        id: "cache:simulator_runtimes",
        group: Group::AppleMobile,
        name: "Simulator runtimes",
        detail: "Downloaded iOS/watchOS/tvOS runtimes, 3-8 GB each.",
        spec: "~/Library/Developer/CoreSimulator/Profiles/Runtimes",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:swiftpm",
        group: Group::AppleMobile,
        name: "Swift Package Manager cache",
        detail: "Cloned and resolved Swift packages.",
        spec: "~/Library/Caches/org.swift.swiftpm",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:cocoapods_repos",
        group: Group::AppleMobile,
        name: "CocoaPods spec repos",
        detail: "Full git clones of the podspec trunk.",
        spec: "~/.cocoapods/repos",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:android",
        group: Group::AppleMobile,
        name: "Android SDK & AVDs",
        detail: "Virtual device images, snapshots and emulator caches.",
        spec: "~/.android",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:ios_backups",
        group: Group::AppleMobile,
        name: "iOS device backups",
        detail: "Local iPhone & iPad backups made in Finder or iTunes. Requires Full Disk Access.",
        spec: "~/Library/Application Support/MobileSync/Backup",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:ios_firmware",
        group: Group::AppleMobile,
        name: "iOS firmware updates",
        detail: "Downloaded .ipsw restore images for iPhone.",
        spec: "~/Library/iTunes/iPhone Software Updates",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:ipad_firmware",
        group: Group::AppleMobile,
        name: "iPadOS software updates",
        detail: "Downloaded iPad restore images (.ipsw).",
        spec: "~/Library/iTunes/iPad Software Updates",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:xcode_previews",
        group: Group::AppleMobile,
        name: "Xcode SwiftUI Previews",
        detail: "Cached SwiftUI preview builds and execution state.",
        spec: "~/Library/Developer/Xcode/UserData/Previews",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:xcode_doc_cache",
        group: Group::AppleMobile,
        name: "Xcode Documentation Cache",
        detail: "Downloaded developer documentation sets and indexes.",
        spec: "~/Library/Caches/com.apple.dt.Xcode",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    // ---------------- IDEs & logs ----------------
    CacheDef {
        id: "cache:vscode_workspace",
        group: Group::Ides,
        name: "VS Code workspace storage",
        detail: "Per-project state and indexes, including for deleted projects.",
        spec: "~/Library/Application Support/Code/User/workspaceStorage",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:vscode_caches",
        group: Group::Ides,
        name: "VS Code app cache",
        detail: "Electron GPU and HTTP caches.",
        spec: "~/Library/Caches/com.microsoft.VSCode",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:cursor_workspace",
        group: Group::Ides,
        name: "Cursor workspace storage",
        detail: "Per-project state and indexes for Cursor.",
        spec: "~/Library/Application Support/Cursor/User/workspaceStorage",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:jetbrains",
        group: Group::Ides,
        name: "JetBrains caches",
        detail: "Indexes and local history for IntelliJ, PyCharm, WebStorm, GoLand.",
        spec: "~/Library/Caches/JetBrains",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:diagnostic_reports",
        group: Group::Ides,
        name: "Crash & diagnostic reports",
        detail: "Accumulated crash dumps and spin dumps from local testing.",
        spec: "~/Library/Logs/DiagnosticReports",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    // ---------------- ML toolchains ----------------
    CacheDef {
        id: "cache:triton",
        group: Group::MlCaches,
        name: "Triton kernel cache",
        detail: "Compiled GPU kernels from PyTorch inductor.",
        spec: "~/.triton/cache",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    CacheDef {
        id: "cache:torch_inductor",
        group: Group::MlCaches,
        name: "TorchInductor cache",
        detail: "Compiled graph artifacts from torch.compile.",
        spec: "~/.cache/torch_inductor",
        env_override: None,
        risk: Risk::Safe,
        cleanup_hint: None,
    },
    // ---------------- system ----------------
    CacheDef {
        id: "cache:trash",
        group: Group::System,
        name: "Trash",
        detail: "Files already in the Trash, waiting to be emptied.",
        spec: "~/.Trash",
        env_override: None,
        risk: Risk::Caution,
        cleanup_hint: None,
    },
];

/// Resolve a definition to a concrete existing path, honouring any env override.
pub fn resolve_def(def: &CacheDef) -> Option<PathBuf> {
    if let Some(var) = def.env_override {
        if let Ok(v) = std::env::var(var) {
            let p = PathBuf::from(v);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let p = paths::resolve(def.spec, None);
    p.is_dir().then_some(p)
}

pub fn finding(
    def: &CacheDef,
    path: &Path,
    bytes: u64,
    shared: u64,
    files: u64,
    mtime: i64,
) -> Finding {
    let mut f = Finding::new(Domain::Cache, def.id, def.name)
        .at(path.to_path_buf())
        .detail(def.detail.to_string())
        .sized(bytes, shared, files)
        .aged(mtime)
        .risk(def.risk)
        .meta(serde_json::json!({
            "group": def.group,
            "group_label": def.group.label(),
            "cleanup_hint": def.cleanup_hint,
        }));
    if let Some(h) = def.cleanup_hint {
        f = f.restore(h.to_string());
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn cache_ids_are_unique() {
        let mut seen = HashSet::new();
        for d in CACHES {
            assert!(seen.insert(d.id), "duplicate cache id {}", d.id);
        }
    }

    #[test]
    fn every_spec_is_home_relative_or_absolute() {
        for d in CACHES {
            assert!(
                d.spec.starts_with('~') || d.spec.starts_with('/'),
                "{} has a relative spec: {}",
                d.id,
                d.spec
            );
        }
    }

    #[test]
    fn stores_that_cost_a_redownload_are_not_marked_safe() {
        // Deleting these is recoverable but expensive, so they must never be
        // pre-selected for the user. pnpm's store in particular backs every
        // pnpm project on the machine.
        for id in [
            "cache:pnpm",
            "cache:go_mod",
            "cache:maven",
            "cache:trash",
            "cache:android",
            "cache:ios_backups",
            "cache:xcode_archives",
            "cache:simulator_runtimes",
        ] {
            let d = CACHES.iter().find(|d| d.id == id).expect(id);
            assert_eq!(d.risk, Risk::Caution, "{id} must be Caution");
        }
    }

    #[test]
    fn prototype_entries_are_all_present() {
        // Parity gate: nothing the Python version found may be silently dropped.
        for id in [
            "cache:npm",
            "cache:pip",
            "cache:homebrew",
            "cache:gradle",
            "cache:cargo_registry",
            "cache:yarn",
            "cache:pnpm",
            "cache:bun",
            "cache:go_build",
            "cache:go_mod",
            "cache:cocoapods",
            "cache:xcode_deriveddata",
            "cache:xcode_archives",
            "cache:ios_devicesupport",
            "cache:simulator_caches",
            "cache:android",
            "cache:vscode_workspace",
            "cache:vscode_caches",
            "cache:cursor_workspace",
            "cache:jetbrains",
            "cache:diagnostic_reports",
            "cache:trash",
        ] {
            assert!(
                CACHES.iter().any(|d| d.id == id),
                "missing prototype entry {id}"
            );
        }
    }

    #[test]
    fn env_overrides_are_only_set_where_the_tool_supports_one() {
        for d in CACHES {
            if let Some(v) = d.env_override {
                assert!(
                    ["GOCACHE", "GOMODCACHE", "DENO_DIR"].contains(&v),
                    "{} declares unknown override {v}",
                    d.id
                );
            }
        }
    }
}
