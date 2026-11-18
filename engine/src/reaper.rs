//! Deletion engine — port of `core/cleaner.py`'s execution half.
//!
//! ## The fallback the prototype has, and this does not (Δ)
//!
//! `move_to_macos_trash()` falls through to `_permanent_delete()` when the Trash
//! move fails. That means a transient failure silently escalates a recoverable
//! action into an irreversible one — the user asked for "move to Trash" and got
//! `rmtree`. Here a Trash failure is simply a failed receipt. Permanent deletion
//! happens **only** when the user explicitly chose that mode.
//!
//! ## Other guarantees
//!
//! * Nothing is deleted without a [`Vetted`] path, and every path is **re-vetted
//!   immediately before** the destructive call — scan and reap are separated by
//!   however long the user spends in the UI (TOCTOU).
//! * A virtual environment is never removed until `req-inplace.txt` is written.
//!   That manifest is the entire reason deleting a venv is acceptable, so a failed
//!   write aborts the deletion rather than proceeding.
//! * Real Finder Trash via the `trash` crate (`NSFileManager trashItemAtURL:`), so
//!   Put Back works and no AppleScript permission prompt is ever triggered.

use crate::domains::{venv, Domain, Finding};
use crate::safety::{self, Vetted};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Move to Finder Trash. Recoverable via Put Back.
    Trash,
    /// Irreversible. Only ever used when explicitly selected.
    Permanent,
    /// Produce every receipt the real run would, touch nothing.
    DryRun,
}

impl Mode {
    pub fn is_dry(&self) -> bool {
        matches!(self, Mode::DryRun)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub domain: Domain,
    pub bytes: u64,
    pub success: bool,
    /// `trash` | `permanent` | `dry-run` | `docker-prune` | `ollama-api` | `pip-uninstall` | `refused`
    pub method: String,
    pub error: Option<String>,
    /// The restore manifest written before a venv was removed.
    pub manifest: Option<venv::ReqInplace>,
    /// Exact path where the item was placed in Trash (if method == "trash").
    #[serde(default)]
    pub trashed_path: Option<PathBuf>,
    /// Inode of the item when trashed, used to verify identity during Put Back.
    #[serde(default)]
    pub trashed_inode: Option<u64>,
    pub timestamp: String,
    pub micros: u64,
}

impl Receipt {
    fn fail(f: &Finding, method: &'static str, err: impl Into<String>, micros: u64) -> Self {
        Self {
            id: f.id.clone(),
            name: f.name.clone(),
            path: f.path.clone(),
            domain: f.domain,
            bytes: 0,
            success: false,
            method: method.to_string(),
            error: Some(err.into()),
            manifest: None,
            trashed_path: None,
            trashed_inode: None,
            timestamp: now_string(),
            micros,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub timestamp: String,
    pub mode: Mode,
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub reclaimed: u64,
    pub receipts: Vec<Receipt>,
}

fn now_string() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Default audit log location.
pub fn default_audit_path() -> PathBuf {
    crate::paths::resolve("~/Library/Application Support/Woosh/audit.jsonl", None)
}

/// Append one summary to the audit log.
///
/// **Δ: JSONL, not a rewritten JSON array.** The prototype reads the whole
/// history, prepends, truncates to 50 and rewrites — a crash mid-write loses the
/// entire history. An append cannot.
pub fn append_audit(path: &Path, summary: &Summary) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(summary).unwrap_or_default();
    writeln!(f, "{line}")
}

/// Read the audit log back, newest first.
pub fn read_audit(path: &Path, limit: usize) -> Vec<Summary> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out: Vec<Summary> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    out.reverse();
    out.truncate(limit);
    out
}

struct Removal {
    method: &'static str,
    trashed_path: Option<PathBuf>,
    trashed_inode: Option<u64>,
}

#[cfg(target_os = "macos")]
fn move_to_trash(p: &Path) -> Result<(PathBuf, u64), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    use std::os::unix::fs::MetadataExt;

    let path_str = p
        .to_str()
        .ok_or_else(|| "Invalid UTF-8 in path".to_string())?;
    let ns_path = NSString::from_str(path_str);
    let url = NSURL::fileURLWithPath(&ns_path);
    let fm = NSFileManager::defaultManager();

    let mut resulting_url = None;
    let res = fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting_url));
    if let Err(err) = res {
        return Err(format!("trashItemAtURL failed: {err}"));
    }

    let trashed_nsurl =
        resulting_url.ok_or_else(|| "trashItemAtURL returned no resulting URL".to_string())?;
    let trashed_nsstr = trashed_nsurl
        .path()
        .ok_or_else(|| "Resulting URL has no path".to_string())?;
    let trashed_path = PathBuf::from(trashed_nsstr.to_string());

    let md = std::fs::symlink_metadata(&trashed_path).map_err(|e| {
        format!(
            "Cannot stat trashed item at {}: {e}",
            trashed_path.display()
        )
    })?;
    let inode = md.ino();

    Ok((trashed_path, inode))
}

#[cfg(not(target_os = "macos"))]
fn move_to_trash(p: &Path) -> Result<(PathBuf, u64), String> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::symlink_metadata(p)
        .map_err(|e| format!("Cannot stat before trashing {}: {e}", p.display()))?;
    let inode = md.ino();
    trash::delete(p).map_err(|e| e.to_string())?;

    let file_name = p.file_name().ok_or_else(|| "No file name".to_string())?;
    let home = crate::paths::primary_home();
    let trash_candidates = [
        home.join(".Trash").join(file_name),
        home.join(".local/share/Trash/files").join(file_name),
    ];
    let mut trashed_path = trash_candidates[0].clone();
    for cand in &trash_candidates {
        if cand.exists() {
            trashed_path = cand.clone();
            break;
        }
    }

    Ok((trashed_path, inode))
}

fn is_trash_dir(p: &Path) -> bool {
    p.file_name().is_some_and(|name| name == ".Trash")
}

#[cfg(unix)]
fn ensure_writable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = p.symlink_metadata() {
        let mut perms = meta.permissions();
        let mode = perms.mode();
        if mode & 0o200 == 0 {
            perms.set_mode(mode | 0o700);
            let _ = std::fs::set_permissions(p, perms);
        }
    }
}

#[cfg(not(unix))]
fn ensure_writable(_p: &Path) {}

#[cfg(target_os = "macos")]
fn clear_flags(p: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    if let Ok(c_path) = CString::new(p.as_os_str().as_bytes()) {
        unsafe {
            libc::chflags(c_path.as_ptr(), 0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn clear_flags(_p: &Path) {}

fn remove_file_robust(p: &Path) -> Result<(), String> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            clear_flags(p);
            ensure_writable(p);
            if let Some(parent) = p.parent() {
                clear_flags(parent);
                ensure_writable(parent);
            }
            match std::fs::remove_file(p) {
                Ok(()) => Ok(()),
                Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e2) => Err(format!(
                    "cannot remove file {}: {e2} (initial: {e})",
                    p.display()
                )),
            }
        }
    }
}

fn remove_dir_robust(p: &Path) -> Result<(), String> {
    match std::fs::remove_dir(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            clear_flags(p);
            ensure_writable(p);
            if let Some(parent) = p.parent() {
                clear_flags(parent);
                ensure_writable(parent);
            }
            match std::fs::remove_dir(p) {
                Ok(()) => Ok(()),
                Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e2) => Err(format!(
                    "cannot remove dir {}: {e2} (initial: {e})",
                    p.display()
                )),
            }
        }
    }
}

/// Recursively delete a directory tree with permission elevation and Rayon parallelization.
fn fast_remove_dir_all(dir: &Path) -> Result<(), String> {
    ensure_writable(dir);

    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            clear_flags(dir);
            ensure_writable(dir);
            std::fs::read_dir(dir)
                .map_err(|e2| format!("cannot read dir {}: {e2} (initial: {e})", dir.display()))?
        }
    };

    let mut subdirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            subdirs.push(entry.path());
        } else {
            files.push(entry.path());
        }
    }

    // Parallel recursive removal across subtrees
    subdirs
        .par_iter()
        .try_for_each(|sub| fast_remove_dir_all(sub))?;

    if files.len() > 32 {
        files
            .par_iter()
            .try_for_each(|file| remove_file_robust(file))?;
    } else {
        for file in &files {
            remove_file_robust(file)?;
        }
    }

    remove_dir_robust(dir)
}

/// Perform permanent deletion on a path, handling files, directories, and special folders like `.Trash`.
fn fast_permanent_remove(p: &Path) -> Result<(), String> {
    let md = p
        .symlink_metadata()
        .map_err(|e| format!("cannot stat {}: {e}", p.display()))?;

    if md.is_dir() {
        if is_trash_dir(p) {
            // macOS kernel protects ~/.Trash itself with an ACL (group:everyone deny delete).
            // Empty all child items inside ~/.Trash without attempting to rmdir the root.
            let entries = match std::fs::read_dir(p) {
                Ok(rd) => rd,
                Err(e) => return Err(format!("cannot read trash {}: {e}", p.display())),
            };
            let items: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            items
                .par_iter()
                .try_for_each(|item| fast_permanent_remove(item))?;
            Ok(())
        } else {
            fast_remove_dir_all(p)
        }
    } else {
        remove_file_robust(p)
    }
}

/// Perform the actual filesystem removal of an already-vetted path.
fn remove(v: &Vetted, mode: Mode) -> Result<Removal, String> {
    let p = v.path();
    match mode {
        Mode::DryRun => Ok(Removal {
            method: "dry-run",
            trashed_path: None,
            trashed_inode: None,
        }),
        Mode::Trash => {
            if is_trash_dir(p) {
                return Ok(Removal {
                    method: "trash",
                    trashed_path: Some(p.to_path_buf()),
                    trashed_inode: None,
                });
            }
            // No fallback to permanent deletion on failure -- see module docs.
            let (trashed_path, inode) = move_to_trash(p)?;
            Ok(Removal {
                method: "trash",
                trashed_path: Some(trashed_path),
                trashed_inode: Some(inode),
            })
        }
        Mode::Permanent => {
            fast_permanent_remove(p)?;
            Ok(Removal {
                method: "permanent",
                trashed_path: None,
                trashed_inode: None,
            })
        }
    }
}

/// Reap one finding.
pub fn reap_one(f: &Finding, mode: Mode) -> Receipt {
    let t0 = Instant::now();
    let us = |t: Instant| t.elapsed().as_micros() as u64;

    // --- action-based findings (no path to delete) ---
    if let Some(action) = f
        .meta
        .as_ref()
        .and_then(|m| m.get("action"))
        .and_then(|a| a.as_str())
    {
        return reap_action(f, action, mode, t0);
    }

    let Some(path) = f.path.clone() else {
        return Receipt::fail(
            f,
            "refused",
            "finding has neither a path nor an action",
            us(t0),
        );
    };

    // Advisory findings are reported only, never deleted.
    if f.risk == crate::domains::Risk::Advisory {
        return Receipt::fail(
            f,
            "refused",
            "advisory items are reported only and cannot be deleted",
            us(t0),
        );
    }

    // Large user files: Trash only. Permanent deletion is refused.
    if f.domain == Domain::LargeFile && mode == Mode::Permanent {
        return Receipt::fail(
            f,
            "refused",
            "large user files can only be moved to Trash, never permanently deleted",
            us(t0),
        );
    }

    // Docker's VM disk is sized and reported, but deleting it directly destroys
    // every image, container and volume at once. Prune through the API instead.
    if crate::domains::docker::is_vm_disk(&path) {
        return Receipt::fail(
            f,
            "refused",
            "Docker.raw is the whole Docker VM disk — prune images, containers, \
             build cache and volumes instead of deleting it",
            us(t0),
        );
    }

    // --- TOCTOU: re-vet immediately before acting ---
    let vetted = match safety::vet(&path) {
        Ok(v) => v,
        Err(e) => return Receipt::fail(f, "refused", e.to_string(), us(t0)),
    };

    // --- the venv safety contract ---
    let mut manifest = None;
    if f.domain == Domain::Venv {
        match venv::write_req_inplace(vetted.path(), "req-inplace.txt") {
            Ok(m) => manifest = Some(m),
            Err(e) => {
                // Abort, do not proceed. Without the manifest this deletion is
                // not recoverable, which is the only thing that made it safe.
                return Receipt::fail(
                    f,
                    "refused",
                    format!("refusing to delete a venv without a restore manifest: {e}"),
                    us(t0),
                );
            }
        }
    }

    // Simulator devices:
    // When deleting permanently, invoke `xcrun simctl delete <udid>` first.
    // CoreSimulator cleanly tears down daemons, unregisters the device
    // from device_set.plist, releases volume locks, and deletes the simulator's storage.
    // If xcrun is unavailable or returns an error, we fall back to fast filesystem removal.
    if mode == Mode::Permanent && f.id.starts_with("sim:") {
        let udid = f
            .meta
            .as_ref()
            .and_then(|m| m.get("udid"))
            .and_then(|u| u.as_str())
            .or_else(|| f.id.strip_prefix("sim:"));

        if let Some(udid) = udid {
            if let Ok(output) = std::process::Command::new("xcrun")
                .args(["simctl", "delete", udid])
                .output()
            {
                if output.status.success() && !vetted.path().exists() {
                    return Receipt {
                        id: f.id.clone(),
                        name: f.name.clone(),
                        path: Some(vetted.path().to_path_buf()),
                        domain: f.domain,
                        bytes: f.bytes,
                        success: true,
                        method: "permanent".to_string(),
                        error: None,
                        manifest,
                        trashed_path: None,
                        trashed_inode: None,
                        timestamp: now_string(),
                        micros: us(t0),
                    };
                }
            }
        }
    }

    match remove(&vetted, mode) {
        Ok(rem) => Receipt {
            id: f.id.clone(),
            name: f.name.clone(),
            path: Some(vetted.path().to_path_buf()),
            domain: f.domain,
            bytes: f.bytes,
            success: true,
            method: rem.method.to_string(),
            error: None,
            manifest,
            trashed_path: rem.trashed_path,
            trashed_inode: rem.trashed_inode,
            timestamp: now_string(),
            micros: us(t0),
        },
        Err(e) => {
            let mut r = Receipt::fail(f, "refused", e, us(t0));
            r.manifest = manifest;
            r
        }
    }
}

fn reap_action(f: &Finding, action: &str, mode: Mode, t0: Instant) -> Receipt {
    let us = |t: Instant| t.elapsed().as_micros() as u64;
    let ok = |method: &'static str, bytes: u64| Receipt {
        id: f.id.clone(),
        name: f.name.clone(),
        path: f.path.clone(),
        domain: f.domain,
        bytes,
        success: true,
        method: method.to_string(),
        error: None,
        manifest: None,
        trashed_path: None,
        trashed_inode: None,
        timestamp: now_string(),
        micros: us(t0),
    };

    if mode.is_dry() {
        return ok("dry-run", f.bytes);
    }

    match action {
        "prune_images" | "prune_containers" | "prune_volumes" | "prune_build_cache" => {
            let id = match action {
                "prune_images" => "docker:images",
                "prune_containers" => "docker:containers",
                "prune_volumes" => "docker:volumes",
                _ => "docker:build_cache",
            };
            match crate::domains::docker::prune(id) {
                // The daemon reports exactly what it freed; trust it over our estimate.
                Ok(freed) => ok("docker-prune", freed),
                Err(e) => Receipt::fail(f, "refused", e, us(t0)),
            }
        }
        "delete_image" => {
            let target = f
                .meta
                .as_ref()
                .and_then(|m| m.get("target").or_else(|| m.get("image_id")))
                .and_then(|s| s.as_str())
                .unwrap_or(&f.id);
            match crate::domains::docker::delete_image(target) {
                Ok(_) => ok("docker-rmi", f.bytes),
                Err(e) => Receipt::fail(f, "refused", e, us(t0)),
            }
        }
        "delete_container" => {
            let target = f
                .meta
                .as_ref()
                .and_then(|m| m.get("target").or_else(|| m.get("container_id")))
                .and_then(|s| s.as_str())
                .unwrap_or(&f.id);
            match crate::domains::docker::delete_container(target) {
                Ok(()) => ok("docker-rm", f.bytes),
                Err(e) => Receipt::fail(f, "refused", e, us(t0)),
            }
        }
        "delete_volume" => {
            let target = f
                .meta
                .as_ref()
                .and_then(|m| m.get("target").or_else(|| m.get("volume_name")))
                .and_then(|s| s.as_str())
                .unwrap_or(&f.id);
            match crate::domains::docker::delete_volume(target) {
                Ok(()) => ok("docker-vol-rm", f.bytes),
                Err(e) => Receipt::fail(f, "refused", e, us(t0)),
            }
        }
        "delete_ollama" => {
            let model = f
                .meta
                .as_ref()
                .and_then(|m| m.get("model"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if model.is_empty() {
                return Receipt::fail(f, "refused", "no ollama model name in finding", us(t0));
            }
            match crate::domains::models::delete_ollama_model(model) {
                Ok(()) => ok("ollama-api", f.bytes),
                Err(e) => Receipt::fail(f, "refused", e, us(t0)),
            }
        }
        "prune_global_python" => reap_global_python(f, t0),
        other => Receipt::fail(f, "refused", format!("unknown action '{other}'"), us(t0)),
    }
}

fn reap_global_python(f: &Finding, t0: Instant) -> Receipt {
    use crate::domains::global_python as gp;
    let us = |t: Instant| t.elapsed().as_micros() as u64;

    let Some(meta) = f.meta.as_ref() else {
        return Receipt::fail(f, "refused", "missing interpreter metadata", us(t0));
    };
    let Some(interp) = meta.get("interpreter").and_then(|p| p.as_str()) else {
        return Receipt::fail(f, "refused", "missing interpreter path", us(t0));
    };

    let install = gp::discover()
        .into_iter()
        .find(|i| i.path.to_string_lossy() == interp);
    let Some(install) = install else {
        return Receipt::fail(f, "refused", "interpreter no longer present", us(t0));
    };
    if install.is_sip {
        return Receipt::fail(
            f,
            "refused",
            "Apple System Python is protected by SIP and cannot be modified",
            us(t0),
        );
    }

    // Backup before pruning, as the prototype does. A failed backup aborts.
    let home = crate::paths::primary_home();
    let backup = match gp::write_backup(&install, &home) {
        Ok((p, n)) => Some((p, n)),
        Err(e) => {
            return Receipt::fail(
                f,
                "refused",
                format!("refusing to prune without a backup manifest: {e}"),
                us(t0),
            )
        }
    };

    let names: Vec<String> = meta
        .get("non_essential")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    match gp::prune(&install, &names) {
        Ok(_out) => Receipt {
            id: f.id.clone(),
            name: f.name.clone(),
            path: f.path.clone(),
            domain: f.domain,
            bytes: f.bytes,
            success: true,
            method: "pip-uninstall".to_string(),
            error: None,
            manifest: backup.map(|(p, n)| venv::ReqInplace {
                file_path: p,
                package_count: n,
                python_version: install.version.clone(),
            }),
            trashed_path: None,
            trashed_inode: None,
            timestamp: now_string(),
            micros: us(t0),
        },
        Err(e) => Receipt::fail(f, "refused", e, us(t0)),
    }
}

/// Reap a batch, in order, and append one audit entry for the whole run.
///
/// Sequential on purpose: these are destructive, and a readable receipt stream
/// the user can follow matters more than shaving milliseconds off a delete.
pub fn reap<F>(findings: &[Finding], mode: Mode, audit: Option<&Path>, mut on_receipt: F) -> Summary
where
    F: FnMut(&Receipt),
{
    let mut receipts = Vec::with_capacity(findings.len());
    let mut reclaimed = 0u64;
    let mut succeeded = 0usize;

    for f in findings {
        let r = reap_one(f, mode);
        if r.success {
            succeeded += 1;
            reclaimed += r.bytes;
        }
        on_receipt(&r);
        receipts.push(r);
    }

    let summary = Summary {
        timestamp: now_string(),
        mode,
        total: findings.len(),
        succeeded,
        failed: findings.len() - succeeded,
        reclaimed,
        receipts,
    };

    // Dry runs are not history: nothing happened.
    if !mode.is_dry() {
        if let Some(p) = audit {
            let _ = append_audit(p, &summary);
        }
    }
    summary
}

/// Restore a trashed item back to its original location ("Put Back").
///
/// Invariants:
/// - Only applicable to receipts with `method == "trash"` and a known `path`.
/// - Destination path must not already exist (refuse to overwrite).
/// - Must have exact `trashed_path` and verifies `trashed_inode` if recorded.
/// - Never guesses by prefix or restores the wrong item.
pub fn put_back(receipt: &Receipt) -> Result<(), String> {
    if receipt.method != "trash" {
        return Err("Only trashed items can be put back".to_string());
    }

    let target_path = match &receipt.path {
        Some(p) => p,
        None => return Err("Receipt does not contain a file path".to_string()),
    };

    if target_path.exists() {
        return Err(format!(
            "Destination already exists: {}",
            target_path.display()
        ));
    }

    let trashed_path = match &receipt.trashed_path {
        Some(p) => p,
        None => {
            return Err("Receipt does not record an exact trashed path".to_string());
        }
    };

    if !trashed_path.exists() {
        return Err(format!(
            "Item not found at trashed path: {}",
            trashed_path.display()
        ));
    }

    // Inode verification: refuse if inode doesn't match recorded receipt inode
    if let Some(expected_inode) = receipt.trashed_inode {
        use std::os::unix::fs::MetadataExt;
        let md = std::fs::symlink_metadata(trashed_path).map_err(|e| {
            format!(
                "Cannot stat trashed item at {}: {e}",
                trashed_path.display()
            )
        })?;
        let actual_inode = md.ino();
        if actual_inode != expected_inode {
            return Err(format!(
                "Refusing to restore: inode mismatch for {} (expected {}, found {})",
                trashed_path.display(),
                expected_inode,
                actual_inode
            ));
        }
    }

    if let Some(parent) = target_path.parent() {
        if !parent.is_dir() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }

    std::fs::rename(trashed_path, target_path).map_err(|e| {
        format!(
            "Failed to restore {} to {}: {}",
            trashed_path.display(),
            target_path.display(),
            e
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::{Finding, Risk};

    fn finding_at(p: &Path, domain: Domain) -> Finding {
        Finding::new(domain, "t:1", "test").at(p).risk(Risk::Safe)
    }

    #[test]
    fn dry_run_touches_nothing_but_reports_success() {
        let tmp = tempfile::tempdir().unwrap();
        let victim = tmp.path().join("deep/enough/junk");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("f.bin"), b"data").unwrap();

        let f = finding_at(&victim, Domain::Artifact);
        let r = reap_one(&f, Mode::DryRun);

        assert!(r.success, "{:?}", r.error);
        assert_eq!(r.method, "dry-run");
        assert!(victim.exists(), "dry run must not delete anything");
    }

    #[test]
    fn protected_paths_are_refused_at_reap_time() {
        // Every protected root is refused. Existence is asserted only for the ones
        // present on both macOS and the Linux verification container -- otherwise
        // the assertion tests the container rather than the guard.
        for p in [
            "/",
            "/usr",
            "/etc",
            "/System",
            "/Applications",
            "/opt/homebrew",
        ] {
            let f = finding_at(Path::new(p), Domain::Cache);
            let r = reap_one(&f, Mode::Permanent);
            assert!(!r.success, "{p} must be refused");
            assert_eq!(r.method, "refused", "{p}");
        }
        for p in ["/", "/usr", "/etc"] {
            assert!(Path::new(p).exists(), "{p} must still be there");
        }
    }

    #[test]
    fn venv_deletion_is_refused_when_the_manifest_cannot_be_written() {
        // The contract: no restore manifest, no deletion. Here the venv's parent
        // is read-only, so the manifest write must fail and abort the delete.
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("blocked/project");
        let venv_dir = project.join(".venv");
        std::fs::create_dir_all(venv_dir.join("lib")).unwrap();
        std::fs::write(venv_dir.join("pyvenv.cfg"), "version = 3.13.0\n").unwrap();

        // Block the manifest with a non-empty *directory* on its path. chmod
        // would not do: the verification container runs as uid 0, which ignores
        // permission bits, so a mode-based test would pass on macOS and silently
        // no-op in CI. A rename onto a directory fails for root too.
        let blocker = project.join("req-inplace.txt");
        std::fs::create_dir_all(blocker.join("occupied")).unwrap();

        let f = finding_at(&venv_dir, Domain::Venv);
        let r = reap_one(&f, Mode::Permanent);

        assert!(!r.success, "must refuse without a manifest");
        assert!(
            r.error
                .as_deref()
                .unwrap_or("")
                .contains("restore manifest"),
            "unexpected error: {:?}",
            r.error
        );
        assert!(venv_dir.exists(), "venv must survive a refused deletion");
    }

    #[test]
    fn venv_deletion_writes_the_manifest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj/inner");
        let venv_dir = project.join("mynenv");
        let sp = venv_dir.join("lib/python3.13/site-packages/flask-3.0.0.dist-info");
        std::fs::create_dir_all(&sp).unwrap();
        std::fs::write(sp.join("METADATA"), "Name: flask\nVersion: 3.0.0\n\n").unwrap();
        std::fs::write(venv_dir.join("pyvenv.cfg"), "version = 3.13.0\n").unwrap();

        let f = finding_at(&venv_dir, Domain::Venv);
        let r = reap_one(&f, Mode::Permanent);

        assert!(r.success, "{:?}", r.error);
        let m = r.manifest.expect("manifest must be recorded");
        assert_eq!(m.package_count, 1);
        // Manifest survives in the project root; the venv is gone.
        assert!(m.file_path.is_file());
        assert!(!venv_dir.exists());
        assert!(std::fs::read_to_string(&m.file_path)
            .unwrap()
            .contains("flask==3.0.0"));
    }

    #[test]
    fn unknown_actions_are_refused_not_guessed() {
        let f = Finding::new(Domain::Docker, "d:1", "test")
            .meta(serde_json::json!({ "action": "prune_everything" }));
        let r = reap_one(&f, Mode::Permanent);
        assert!(!r.success);
        assert!(r.error.unwrap().contains("unknown action"));
    }

    #[test]
    fn docker_vm_disk_is_never_deleted_directly() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("vms/0/data");
        std::fs::create_dir_all(&d).unwrap();
        let raw = d.join("Docker.raw");
        std::fs::write(&raw, b"x").unwrap();

        let f = finding_at(&raw, Domain::Docker);
        let r = reap_one(&f, Mode::Permanent);
        assert!(!r.success);
        assert!(raw.exists(), "Docker.raw must survive");
    }

    #[test]
    fn audit_log_appends_and_reads_back_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("audit.jsonl");

        for i in 0..3 {
            let s = Summary {
                timestamp: format!("2026-01-0{i}"),
                mode: Mode::Trash,
                total: 1,
                succeeded: 1,
                failed: 0,
                reclaimed: (i as u64 + 1) * 1000,
                receipts: vec![],
            };
            append_audit(&log, &s).unwrap();
        }

        let back = read_audit(&log, 10);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].reclaimed, 3000, "newest first");
    }

    #[test]
    fn dry_runs_are_not_written_to_history() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("audit.jsonl");
        let victim = tmp.path().join("deep/enough/thing");
        std::fs::create_dir_all(&victim).unwrap();

        let f = finding_at(&victim, Domain::Artifact);
        reap(&[f], Mode::DryRun, Some(&log), |_| {});
        assert!(!log.exists(), "a dry run is not history");
    }

    #[test]
    fn large_files_refuse_permanent_deletion() {
        let tmp = tempfile::tempdir().unwrap();
        let victim = tmp.path().join("deep/enough/movie.mp4");
        std::fs::create_dir_all(victim.parent().unwrap()).unwrap();
        std::fs::write(&victim, b"video data").unwrap();

        let mut f = finding_at(&victim, Domain::LargeFile);
        f.risk = crate::domains::Risk::Caution;
        let r = reap_one(&f, Mode::Permanent);
        assert!(!r.success);
        assert_eq!(r.method, "refused");
        assert!(
            r.error
                .as_deref()
                .unwrap_or("")
                .contains("never permanently deleted"),
            "expected permanent deletion refusal error, got: {:?}",
            r.error
        );
        assert!(
            victim.exists(),
            "file must not be deleted on permanent refusal"
        );

        // Dry run is allowed
        let r_dry = reap_one(&f, Mode::DryRun);
        assert!(r_dry.success);
        assert_eq!(r_dry.method, "dry-run");
    }

    #[test]
    fn advisory_findings_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let victim = tmp.path().join("deep/enough/Photos.photoslibrary");
        std::fs::create_dir_all(&victim).unwrap();

        let mut f = finding_at(&victim, Domain::LargeFile);
        f.risk = crate::domains::Risk::Advisory;
        let r = reap_one(&f, Mode::Trash);
        assert!(!r.success);
        assert_eq!(r.method, "refused");
        assert!(
            r.error.as_deref().unwrap_or("").contains("advisory items"),
            "expected advisory refusal error, got: {:?}",
            r.error
        );
    }

    #[test]
    fn docker_individual_actions_dry_run_supported() {
        let mut f_img = Finding::new(Domain::Docker, "docker:image:123456", "my-img:v1");
        f_img.bytes = 1000;
        f_img.risk = crate::domains::Risk::Caution;
        f_img.meta = Some(serde_json::json!({
            "action": "delete_image",
            "target": "sha256:1234567890",
        }));
        let r = reap_one(&f_img, Mode::DryRun);
        assert!(r.success);
        assert_eq!(r.method, "dry-run");
        assert_eq!(r.bytes, 1000);

        let mut f_cont = Finding::new(Domain::Docker, "docker:container:abc", "Stopped: my-cont");
        f_cont.bytes = 500;
        f_cont.risk = crate::domains::Risk::Caution;
        f_cont.meta = Some(serde_json::json!({
            "action": "delete_container",
            "target": "c_id_123",
        }));
        let r = reap_one(&f_cont, Mode::DryRun);
        assert!(r.success);
        assert_eq!(r.method, "dry-run");

        let mut f_vol = Finding::new(Domain::Docker, "docker:volume:my-vol", "Volume: my-vol");
        f_vol.bytes = 2000;
        f_vol.risk = crate::domains::Risk::Caution;
        f_vol.meta = Some(serde_json::json!({
            "action": "delete_volume",
            "target": "my-vol",
        }));
        let r = reap_one(&f_vol, Mode::DryRun);
        assert!(r.success);
        assert_eq!(r.method, "dry-run");
    }

    #[test]
    fn put_back_refuses_non_trash_and_missing_files() {
        let receipt = Receipt {
            id: "test:1".into(),
            name: "test_item".into(),
            path: Some(PathBuf::from("/Users/test/deep/path/item")),
            domain: Domain::Cache,
            bytes: 100,
            success: true,
            method: "permanent".into(),
            error: None,
            manifest: None,
            trashed_path: None,
            trashed_inode: None,
            timestamp: now_string(),
            micros: 10,
        };

        // Permanent deletions cannot be put back
        assert!(put_back(&receipt).is_err());

        let mut trash_receipt = receipt;
        trash_receipt.method = "trash".into();

        // Missing trashed_path returns error
        assert!(put_back(&trash_receipt).is_err());

        // Non-existent trash item returns error
        trash_receipt.trashed_path = Some(PathBuf::from("/tmp/nonexistent_trash_file_xyz"));
        assert!(put_back(&trash_receipt).is_err());
    }

    #[test]
    fn put_back_with_same_named_trash_items_never_restores_wrong_item() {
        use std::os::unix::fs::MetadataExt;

        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = tmp.path().join("fake_trash");
        std::fs::create_dir_all(&trash_dir).unwrap();

        let proj_a_dest = tmp.path().join("proj_a/node_modules");
        let proj_b_dest = tmp.path().join("proj_b/node_modules");

        // Create two items in the fake trash with the same base name but distinct paths & inodes
        let trash_item_a = trash_dir.join("node_modules");
        let trash_item_b = trash_dir.join("node_modules 2");

        std::fs::create_dir_all(&trash_item_a).unwrap();
        std::fs::write(trash_item_a.join("a.txt"), b"project a").unwrap();
        let ino_a = std::fs::symlink_metadata(&trash_item_a).unwrap().ino();

        std::fs::create_dir_all(&trash_item_b).unwrap();
        std::fs::write(trash_item_b.join("b.txt"), b"project b").unwrap();
        let ino_b = std::fs::symlink_metadata(&trash_item_b).unwrap().ino();

        assert_ne!(ino_a, ino_b);

        // Receipt A points to item_a with ino_a
        let receipt_a = Receipt {
            id: "a:1".into(),
            name: "node_modules".into(),
            path: Some(proj_a_dest.clone()),
            domain: Domain::NodeModules,
            bytes: 100,
            success: true,
            method: "trash".into(),
            error: None,
            manifest: None,
            trashed_path: Some(trash_item_a.clone()),
            trashed_inode: Some(ino_a),
            timestamp: now_string(),
            micros: 10,
        };

        // Tampered receipt that attempts to put back B's trash into A's destination with wrong inode or path
        let mut tampered_receipt = receipt_a.clone();
        tampered_receipt.trashed_path = Some(trash_item_b.clone()); // points to B's trash path but expects ino_a
        let err = put_back(&tampered_receipt).unwrap_err();
        assert!(
            err.contains("inode mismatch"),
            "Error should be inode mismatch: {err}"
        );

        // Normal put_back for A succeeds and moves ONLY item_a
        put_back(&receipt_a).expect("put back A must succeed");
        assert!(proj_a_dest.join("a.txt").exists());
        assert!(!trash_item_a.exists());
        // item_b is STILL in the trash untouched!
        assert!(trash_item_b.exists());
        assert!(trash_item_b.join("b.txt").exists());

        // Receipt B restores item_b to proj_b
        let receipt_b = Receipt {
            id: "b:1".into(),
            name: "node_modules".into(),
            path: Some(proj_b_dest.clone()),
            domain: Domain::NodeModules,
            bytes: 100,
            success: true,
            method: "trash".into(),
            error: None,
            manifest: None,
            trashed_path: Some(trash_item_b.clone()),
            trashed_inode: Some(ino_b),
            timestamp: now_string(),
            micros: 10,
        };
        put_back(&receipt_b).expect("put back B must succeed");
        assert!(proj_b_dest.join("b.txt").exists());
        assert!(!trash_item_b.exists());
    }

    #[test]
    fn permanent_deletion_cleans_readonly_directories_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested/deep/readonly_target");
        let sub = target.join("sub_ro");
        std::fs::create_dir_all(&sub).unwrap();

        let file = sub.join("locked.txt");
        std::fs::write(&file, b"content").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Set file to read-only
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
            // Set sub directory to read-only (0o555: r-xr-xr-x, no write bit)
            std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
        }

        let f = finding_at(&target, Domain::Cache);
        let r = reap_one(&f, Mode::Permanent);

        assert!(
            r.success,
            "Reap should succeed even with read-only subdirectories: {:?}",
            r.error
        );
        assert!(!target.exists(), "Target should be completely removed");
    }

    #[test]
    fn permanent_deletion_empties_trash_without_removing_trash_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = tmp.path().join("user/home/.Trash");
        std::fs::create_dir_all(&trash_dir).unwrap();

        let item1 = trash_dir.join("old_cache");
        std::fs::create_dir_all(&item1).unwrap();
        std::fs::write(item1.join("data.bin"), b"junk").unwrap();

        let item2 = trash_dir.join("file.pkg");
        std::fs::write(&item2, b"junk").unwrap();

        let f = finding_at(&trash_dir, Domain::Cache);
        let r = reap_one(&f, Mode::Permanent);

        assert!(r.success, "Emptying trash must succeed: {:?}", r.error);
        assert!(
            trash_dir.exists(),
            ".Trash directory itself must remain intact"
        );
        assert!(!item1.exists(), "item1 inside .Trash must be deleted");
        assert!(!item2.exists(), "item2 inside .Trash must be deleted");
    }

    #[test]
    fn simulator_finding_reaps_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let sim_dir = tmp
            .path()
            .join("Library/Developer/CoreSimulator/Devices/TEST-UDID-1234");
        let data_dir = sim_dir.join("data/Containers");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("cache.db"), b"fake simulator data").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }

        let mut f = finding_at(&sim_dir, Domain::Cache);
        f.id = "sim:TEST-UDID-1234".to_string();
        f.meta = Some(serde_json::json!({
            "udid": "TEST-UDID-1234",
            "device_name": "iPhone 14 Pro Max",
        }));

        let r = reap_one(&f, Mode::Permanent);
        assert!(r.success, "Simulator reap must succeed: {:?}", r.error);
        assert!(!sim_dir.exists(), "Simulator folder must be gone");
    }
}
