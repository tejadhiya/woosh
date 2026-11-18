//! Tauri shell. Deliberately thin: command surface, event bridge, window chrome.
//!
//! All real work lives in `woosh-engine`, which carries no Tauri dependency so it
//! stays compilable and testable inside Linux Docker (plan §0.1).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use woosh_engine::domains::Finding;
use woosh_engine::reaper::{self, Mode};
use woosh_engine::{paths, search, sysinfo, telemetry, Config, ScanResult};

/// Last scan's findings, so `reap` can resolve the ids the UI sends back.
///
/// The UI never hands us a path to delete — only an id from a scan this process
/// performed. Combined with the engine re-vetting every path at delete time, a
/// compromised or buggy frontend cannot name an arbitrary target.
struct AppState {
    findings: Mutex<Vec<Finding>>,
    scanning: Arc<Mutex<bool>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            findings: Mutex::new(Vec::new()),
            scanning: Arc::new(Mutex::new(false)),
        }
    }
}

/// RAII drop guard that resets `scanning` to `false` when dropped.
struct ScanGuard(Arc<Mutex<bool>>);

impl Drop for ScanGuard {
    fn drop(&mut self) {
        *self.0.lock() = false;
    }
}

#[derive(Debug, Serialize)]
struct Permissions {
    /// Whether Full Disk Access appears to be granted.
    full_disk_access: bool,
    home: PathBuf,
}

/// Probe for Full Disk Access.
///
/// There is no API to *request* it (plan §8.3), so we detect it by attempting a
/// read of a TCC-protected path. `TCC.db` is the canonical probe: readable only
/// with Full Disk Access, and harmless to attempt without it.
fn probe_full_disk_access() -> bool {
    let tcc = paths::resolve("~/Library/Application Support/com.apple.TCC/TCC.db", None);
    std::fs::File::open(&tcc).is_ok()
}

#[tauri::command]
fn permissions() -> Permissions {
    Permissions {
        full_disk_access: probe_full_disk_access(),
        home: paths::primary_home(),
    }
}

#[tauri::command]
fn disk() -> sysinfo::DiskStats {
    sysinfo::disk_stats(std::path::Path::new("/"))
}

#[tauri::command]
fn scan_roots() -> Vec<PathBuf> {
    paths::default_scan_roots()
}

/// Run a sweep. Telemetry frames stream on `"telemetry"` while it runs.
///
/// Blocking work is moved off the main thread so the webview keeps painting the
/// live feed; without this the 60 Hz frames would queue behind the scan.
#[tauri::command(async)]
async fn scan(
    app: AppHandle,
    state: State<'_, AppState>,
    deep: bool,
) -> Result<ScanResult, String> {
    let _guard = {
        let mut guard = state.scanning.lock();
        if *guard {
            return Err("a scan is already running".into());
        }
        *guard = true;
        ScanGuard(state.scanning.clone())
    };

    let cfg = Config {
        roots: None,
        max_depth: 5,
        deep,
        ..Config::default()
    };

    let emitter = app.clone();
    let finding_emitter = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        woosh_engine::scan_streaming(
            &cfg,
            move |frame: telemetry::Frame| {
                let _ = emitter.emit("telemetry", frame);
            },
            move |finding: &Finding| {
                let _ = finding_emitter.emit("finding", finding);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?;

    *state.findings.lock() = result.findings.clone();

    // Persist to index cache for instant startup (< 3 ms)
    let cache_path = woosh_engine::index::default_cache_path();
    let event_id = woosh_engine::index::current_fsevent_id();
    let _ = woosh_engine::index::save_cache(&cache_path, &result, event_id);

    Ok(result)
}

/// Load the most recent scan result from disk cache for instant startup.
#[tauri::command]
fn load_cached_scan(state: State<'_, AppState>) -> Option<woosh_engine::index::CachedScan> {
    let cache_path = woosh_engine::index::default_cache_path();
    if let Ok(Some(cached)) = woosh_engine::index::load_cache(&cache_path) {
        *state.findings.lock() = cached.scan_result.findings.clone();
        Some(cached)
    } else {
        None
    }
}

/// Restore a previously trashed item back to its original location.
#[tauri::command]
fn put_back(receipt: reaper::Receipt) -> Result<(), String> {
    reaper::put_back(&receipt)
}

/// Validate that a path requested by the webview is part of known scan findings
/// or installed applications before opening or previewing it (Bug B6).
fn validate_allowed_path(state: &AppState, target: &std::path::Path) -> Result<PathBuf, String> {
    let canonical = target
        .canonicalize()
        .map_err(|e| format!("Invalid or non-existent path {}: {e}", target.display()))?;

    // Check 1: Is this path (or an ancestor) in current scan findings?
    {
        let findings = state.findings.lock();
        for f in findings.iter() {
            if let Some(ref fp) = f.path {
                if let Ok(c_fp) = fp.canonicalize() {
                    if canonical == c_fp || canonical.starts_with(&c_fp) {
                        return Ok(canonical);
                    }
                }
            }
        }
    }

    // Check 2: Is it an installed application from discover_apps?
    let home = paths::primary_home();
    let apps = search::discover_apps(Some(&home));
    for app in apps {
        let app_path = std::path::Path::new(&app.path);
        if let Ok(c_app) = app_path.canonicalize() {
            if canonical == c_app || canonical.starts_with(&c_app) {
                return Ok(canonical);
            }
        }
    }

    Err(format!(
        "Access denied: path '{}' is not part of known scan findings",
        target.display()
    ))
}

#[tauri::command]
fn quick_look(state: State<'_, AppState>, path: PathBuf) -> Result<(), String> {
    let allowed = validate_allowed_path(&state, &path)?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("qlmanage")
            .arg("-p")
            .arg(&allowed)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = allowed;
        Ok(())
    }
}

#[tauri::command]
fn open_path(state: State<'_, AppState>, path: PathBuf) -> Result<(), String> {
    let allowed = validate_allowed_path(&state, &path)?;
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let path_str = allowed
            .to_str()
            .ok_or_else(|| "Invalid UTF-8 in path".to_string())?;
        let ns_path = NSString::from_str(path_str);
        let url = NSURL::fileURLWithPath(&ns_path);
        let ws = NSWorkspace::sharedWorkspace();
        if ws.openURL(&url) {
            Ok(())
        } else {
            Err(format!("NSWorkspace failed to open {}", allowed.display()))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = allowed;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ReapRequest {
    ids: Vec<String>,
    /// `trash` | `permanent` | `dry_run`
    mode: String,
}

fn parse_mode(s: &str) -> Result<Mode, String> {
    match s {
        "trash" => Ok(Mode::Trash),
        "permanent" => Ok(Mode::Permanent),
        "dry_run" | "dry-run" => Ok(Mode::DryRun),
        other => Err(format!("unknown mode '{other}'")),
    }
}

/// Delete the selected findings. Receipts stream on `"reap"` as they complete.
#[tauri::command(async)]
async fn reap(
    app: AppHandle,
    state: State<'_, AppState>,
    req: ReapRequest,
) -> Result<reaper::Summary, String> {
    let mode = parse_mode(&req.mode)?;

    // Resolve ids against the last scan. Unknown ids are dropped rather than
    // guessed at -- the UI cannot name a target the engine never found.
    let selected: Vec<Finding> = {
        let all = state.findings.lock();
        let map: HashMap<&str, &Finding> = all.iter().map(|f| (f.id.as_str(), f)).collect();
        req.ids
            .iter()
            .filter_map(|id| map.get(id.as_str()).map(|&f| f.clone()))
            .collect()
    };
    if selected.is_empty() {
        return Err("nothing selected, or the scan is stale — run a new sweep".into());
    }

    let audit = reaper::default_audit_path();
    let emitter = app.clone();
    let summary = tauri::async_runtime::spawn_blocking(move || {
        reaper::reap(&selected, mode, Some(&audit), |r| {
            let _ = emitter.emit("reap", r);
        })
    })
    .await
    .map_err(|e| e.to_string())?;

    // Successfully reaped findings are no longer valid targets.
    {
        let mut all = state.findings.lock();
        let gone: HashSet<&str> = summary
            .receipts
            .iter()
            .filter(|r| r.success)
            .map(|r| r.id.as_str())
            .collect();
        all.retain(|f| !gone.contains(f.id.as_str()));
    }

    Ok(summary)
}

#[tauri::command]
fn history(limit: Option<usize>) -> Vec<reaper::Summary> {
    reaper::read_audit(&reaper::default_audit_path(), limit.unwrap_or(25))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            permissions,
            disk,
            scan_roots,
            scan,
            load_cached_scan,
            reap,
            put_back,
            history,
            quick_look,
            open_path
        ])
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("Woosh");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Woosh");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_guard_resets_scanning_flag_on_drop() {
        let scanning = Arc::new(Mutex::new(false));
        {
            let mut guard = scanning.lock();
            assert!(!*guard);
            *guard = true;
            let _scan_guard = ScanGuard(scanning.clone());
            drop(guard);
            assert!(*scanning.lock());
            // Simulated error / early return causes _scan_guard to drop here
        }
        assert!(
            !*scanning.lock(),
            "scanning flag must be reset to false so next scan works"
        );
    }

    #[test]
    fn validate_allowed_path_rejects_arbitrary_unindexed_paths() {
        let state = AppState::default();
        let res = validate_allowed_path(&state, std::path::Path::new("/bin/sh"));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Access denied"));
    }

    #[test]
    fn validate_allowed_path_accepts_known_findings() {
        let state = AppState::default();
        let tmp = tempfile::tempdir().unwrap();
        let dummy_finding_path = tmp.path().join("proj/node_modules");
        std::fs::create_dir_all(&dummy_finding_path).unwrap();

        // 1. Add finding
        {
            let mut findings = state.findings.lock();
            findings.push(
                Finding::new(
                    woosh_engine::domains::Domain::NodeModules,
                    "nm:1",
                    "node_modules",
                )
                .at(&dummy_finding_path),
            );
        }

        // Finding path must be accepted
        assert!(validate_allowed_path(&state, &dummy_finding_path).is_ok());

        // File inside finding folder should also be accepted
        let child_file = dummy_finding_path.join("package.json");
        std::fs::write(&child_file, b"{}").unwrap();
        assert!(validate_allowed_path(&state, &child_file).is_ok());

        // Arbitrary outside path must be rejected
        let outside_file = tmp.path().join("secret.txt");
        std::fs::write(&outside_file, b"secret").unwrap();
        assert!(validate_allowed_path(&state, &outside_file).is_err());
    }
}
