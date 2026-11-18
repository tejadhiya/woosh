//! Woosh engine — developer storage discovery and reclamation for macOS.
//!
//! Pure Rust with **no Tauri dependency**: that is what lets the entire suite
//! compile and run inside Linux Docker while the shipped `.app` is bundled on the
//! host (`docs/IMPLEMENTATION_PLAN.md` §0.1).
//!
//! ```text
//!   paths  ──►  sweep::discover  ──►  sweep::measure  ──►  Finding
//!               (depth-bounded,       (pure sum, inode      │
//!                detectors claim)      de-duplicated)       │
//!                                                           ▼
//!   probes (caches · models · docker · global python)  ──►  ScanResult
//!                                                           │
//!                              safety::vet  ──►  reaper  ──►  Receipt
//! ```

pub mod dedup;
pub mod domains;
pub mod fastwalk;
pub mod http;
pub mod index;
pub mod paths;
pub mod reaper;
pub mod safety;
pub mod search;
pub mod stats;
pub mod sweep;
pub mod sysinfo;
pub mod telemetry;

use dedup::InodeSet;
use domains::{artifacts, caches, docker, global_python, models, node, simulators, venv, Finding};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use stats::Stats;
use std::path::PathBuf;
use std::sync::Arc;
use sweep::Verdict;
use telemetry::{LineKind, LogLine, Phase, PhaseCell};

/// Default minimum allocated bytes for capturing a large file (50 MB).
pub const DEFAULT_LARGE_FILE_THRESHOLD: u64 = 50 * 1024 * 1024;

/// Scan configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Override the discovered scan roots.
    pub roots: Option<Vec<PathBuf>>,
    /// Depth limit for discovery. The prototype uses 4–5; 5 is the default here.
    pub max_depth: usize,
    /// Deep mode: raise the depth limit and scan the whole home (plan §1).
    pub deep: bool,
    /// Threshold in allocated bytes for capturing individual large files (default 50 MB).
    pub large_file_threshold: u64,
}

impl Default for Config {
    fn default() -> Self {
        let threshold = std::env::var("WOOSH_LARGE_FILE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_LARGE_FILE_THRESHOLD);
        Self {
            roots: None,
            max_depth: 5,
            deep: false,
            large_file_threshold: threshold,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Totals {
    pub reclaimable: u64,
    pub shared: u64,
    pub items: usize,
    pub by_domain: Vec<(String, u64, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub disk: sysinfo::DiskStats,
    pub findings: Vec<Finding>,
    pub totals: Totals,
    /// Roots that need Full Disk Access. Surfaced, never swallowed (plan §8.3).
    pub denied: Vec<PathBuf>,
    pub duration_us: u64,
    pub entries_scanned: u64,
    pub roots: Vec<PathBuf>,
}

/// The detector used during discovery.
///
/// One closure recognises every path-based domain, and it runs with the directory
/// listing **already in hand** — so testing for `pyvenv.cfg`, a lockfile or
/// `Cargo.toml` costs no extra syscall in the common case.
pub fn detect(path: &std::path::Path, entries: &[fastwalk::Entry], _depth: usize) -> Verdict {
    if domains::is_noise(path) {
        return Verdict::Skip;
    }
    // Never descend into package bundles (.photoslibrary, .app, .band, etc.)
    if let Some((ext, _, _)) = domains::large_files::is_package(path) {
        return Verdict::Claim {
            tag: "package",
            kind: ext,
        };
    }
    let Some(name) = path.file_name() else {
        return Verdict::Descend;
    };

    if name == "node_modules" {
        // Yarn PnP projects have no real node_modules worth reporting.
        if let Some(parent) = path.parent() {
            if node::is_yarn_pnp(parent) {
                return Verdict::Skip;
            }
        }
        return Verdict::Claim {
            tag: "node_modules",
            kind: "node_modules",
        };
    }
    if let Some(kind) = venv::detect(path, entries) {
        return Verdict::Claim { tag: "venv", kind };
    }
    if let Some(c) = artifacts::classify(path) {
        return Verdict::Claim {
            tag: "artifact",
            kind: c.0,
        };
    }
    Verdict::Descend
}

/// Measure a slice of claims with deduplication and map them to [`Finding`]s.
pub fn measure_claims<F>(
    claims: &[sweep::Claim],
    stats: &Stats,
    inodes: &InodeSet,
    on_found: F,
) -> Vec<Finding>
where
    F: Fn(&Finding, &sweep::Claim) + Sync + Send,
{
    claims
        .par_iter()
        .filter_map(|c| {
            let m = sweep::measure(&c.path, stats, inodes);
            let mtime = if m.newest_mtime > 0 {
                m.newest_mtime
            } else {
                c.mtime
            };
            let f = match c.tag {
                "venv" => venv::finding(
                    &c.path,
                    c.kind,
                    m.charge.exclusive,
                    m.charge.shared,
                    m.files,
                    mtime,
                ),
                "node_modules" => {
                    if m.charge.total() < node::MIN_BYTES {
                        return None;
                    }
                    node::finding(&c.path, m.charge.exclusive, m.charge.shared, m.files, mtime)
                }
                "artifact" => {
                    if m.charge.total() < artifacts::MIN_BYTES {
                        return None;
                    }
                    artifacts::finding(&c.path, m.charge.exclusive, m.charge.shared, m.files, mtime)
                }
                "package" => {
                    // Filter empty packages (must be >= 1 MB)
                    if m.charge.total() < 1024 * 1024 {
                        return None;
                    }
                    domains::large_files::package_finding(
                        &c.path,
                        m.charge.exclusive,
                        m.charge.shared,
                        m.files,
                        mtime,
                    )
                }
                _ => return None,
            };
            on_found(&f, c);
            Some(f)
        })
        .collect()
}

/// Discover and measure every **path-based** finding under `roots`.
///
/// Split out from [`scan`] so the integration suite can exercise the walk, the
/// detectors and the de-duplication against a fixture tree without the probes
/// reaching into the real machine's caches, Docker daemon and interpreters.
pub fn scan_paths(
    roots: &[PathBuf],
    max_depth: usize,
    stats: &Stats,
    inodes: &InodeSet,
) -> Vec<Finding> {
    let threshold = std::env::var("WOOSH_LARGE_FILE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LARGE_FILE_THRESHOLD);

    let discovered = sweep::discover(roots, max_depth, threshold, stats, detect);
    let mut findings = measure_claims(&discovered.claims, stats, inodes, |_, _| {});
    let file_findings: Vec<Finding> = discovered
        .files
        .par_iter()
        .filter_map(|file| {
            let charge = inodes.charge(file.dev, file.ino, file.alloc);
            let f = domains::large_files::file_finding(
                &file.path,
                charge.exclusive,
                charge.shared,
                file.mtime,
            );
            Some(f)
        })
        .collect();
    findings.extend(file_findings);
    findings
}

/// Run a full scan.
pub fn scan<F>(cfg: &Config, emit: F) -> ScanResult
where
    F: Fn(telemetry::Frame) + Send + 'static,
{
    scan_streaming(cfg, emit, |_| {})
}

/// Run a full scan, streaming telemetry frames at 60 Hz and individual findings
/// as they are measured.
pub fn scan_streaming<F, G>(cfg: &Config, emit: F, on_finding: G) -> ScanResult
where
    F: Fn(telemetry::Frame) + Send + 'static,
    G: Fn(&Finding) + Sync + Send + 'static,
{
    let t0 = std::time::Instant::now();
    let stats = Arc::new(Stats::default());
    let inodes = InodeSet::new();
    let phase: PhaseCell = Arc::new(parking_lot::Mutex::new(Phase::Boot));
    let (tx, rx) = crossbeam_channel::unbounded::<LogLine>();
    let tele = telemetry::spawn(stats.clone(), phase.clone(), rx, emit);

    let home = paths::primary_home();
    let roots = cfg.roots.clone().unwrap_or_else(paths::default_scan_roots);
    let max_depth = if cfg.deep { 12 } else { cfg.max_depth };
    let threshold = cfg.large_file_threshold;
    let us = || t0.elapsed().as_micros() as u64;
    let say = |kind: LineKind, text: String| {
        let _ = tx.send(LogLine::new(us(), kind, text));
    };

    say(
        LineKind::Boot,
        format!(
            "hangar doors open · {} root{} · {} threads warm{}",
            roots.len(),
            if roots.len() == 1 { "" } else { "s" },
            rayon::current_num_threads(),
            if cfg.deep { " · deep sweep" } else { "" }
        ),
    );

    // ---------------- overlapped discovery + measuring + probes (P1 + P2) ----------------
    *phase.lock() = Phase::Discover;
    let on_finding = Arc::new(on_finding);

    let (path_leg_result, probe_findings) = rayon::join(
        || {
            // Path-based sweep: dev roots + media roots (Phase 1)
            let (discovered_dev, discovered_media) = rayon::join(
                || sweep::discover(&roots, max_depth, threshold, &stats, detect),
                || {
                    if cfg.roots.is_none() {
                        let media_roots = paths::media_scan_roots();
                        sweep::discover(&media_roots, 24, threshold, &stats, detect)
                    } else {
                        sweep::Discovered::default()
                    }
                },
            );

            let mut claims = discovered_dev.claims;
            claims.extend(discovered_media.claims);
            let mut files = discovered_dev.files;
            files.extend(discovered_media.files);
            let mut denied = discovered_dev.denied;
            denied.extend(discovered_media.denied);

            *phase.lock() = Phase::Measure;
            let on_finding_ref = on_finding.clone();
            let tx_found = tx.clone();
            let home_found = home.clone();
            let measured = measure_claims(&claims, &stats, &inodes, move |f, c| {
                let note = if f.shared_bytes > f.bytes && f.bytes > 0 {
                    "mostly hardlinked".to_string()
                } else {
                    String::new()
                };
                let _ = tx_found.send(telemetry::found_line(
                    us(),
                    &telemetry::pretty_path(&c.path, &home_found),
                    f.bytes,
                    &note,
                ));
                on_finding_ref(f);
            });

            let on_finding_file = on_finding.clone();
            let tx_file = tx.clone();
            let home_file = home.clone();
            let file_findings: Vec<Finding> = files
                .par_iter()
                .filter_map(|file| {
                    let charge = inodes.charge(file.dev, file.ino, file.alloc);
                    let f = domains::large_files::file_finding(
                        &file.path,
                        charge.exclusive,
                        charge.shared,
                        file.mtime,
                    );
                    let _ = tx_file.send(telemetry::found_line(
                        us(),
                        &telemetry::pretty_path(&file.path, &home_file),
                        f.bytes,
                        "",
                    ));
                    on_finding_file(&f);
                    Some(f)
                })
                .collect();

            let mut all_path = measured;
            all_path.extend(file_findings);
            (all_path, denied)
        },
        || {
            // Independent probes run concurrently with the walk (P1)
            let on_f1 = on_finding.clone();
            let on_f2 = on_finding.clone();
            let on_f3 = on_finding.clone();
            let on_f4 = on_finding.clone();
            let on_f5 = on_finding.clone();
            let on_f6 = on_finding.clone();

            let (cache_f, (model_f, (docker_f, (gp_f, (sim_f, toolchain_f))))) = rayon::join(
                || {
                    let fs = probe_caches(&stats, &inodes);
                    for f in &fs {
                        on_f1(f);
                    }
                    fs
                },
                || {
                    rayon::join(
                        || {
                            let fs = probe_models(&stats, &inodes);
                            for f in &fs {
                                on_f2(f);
                            }
                            fs
                        },
                        || {
                            rayon::join(
                                || {
                                    let fs = probe_docker();
                                    for f in &fs {
                                        on_f3(f);
                                    }
                                    fs
                                },
                                || {
                                    rayon::join(
                                        || {
                                            let fs = probe_global_python(&stats, &inodes);
                                            for f in &fs {
                                                on_f4(f);
                                            }
                                            fs
                                        },
                                        || {
                                            rayon::join(
                                                || {
                                                    let fs = simulators::probe_simulators(
                                                        &home, &stats, &inodes,
                                                    );
                                                    for f in &fs {
                                                        on_f5(f);
                                                    }
                                                    fs
                                                },
                                                || {
                                                    let fs = domains::toolchains::probe_toolchains(
                                                        &home, &stats, &inodes,
                                                    );
                                                    for f in &fs {
                                                        on_f6(f);
                                                    }
                                                    fs
                                                },
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                },
            );

            let mut probes = cache_f;
            probes.extend(model_f);
            probes.extend(docker_f);
            probes.extend(gp_f);
            probes.extend(sim_f);
            probes.extend(toolchain_f);
            probes
        },
    );

    let (mut all, denied) = path_leg_result;
    all.extend(probe_findings);
    all.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    // ---------------- totals ----------------
    let mut by: std::collections::BTreeMap<String, (u64, usize)> = Default::default();
    for f in &all {
        let e = by.entry(f.domain.label().to_string()).or_insert((0, 0));
        e.0 += f.bytes;
        e.1 += 1;
    }
    let totals = Totals {
        reclaimable: all.iter().map(|f| f.bytes).sum(),
        shared: all.iter().map(|f| f.shared_bytes).sum(),
        items: all.len(),
        by_domain: by.into_iter().map(|(k, (b, n))| (k, b, n)).collect(),
    };

    let snap = stats.snapshot();
    say(
        LineKind::Done,
        format!(
            "sweep complete · {} entries · {} reclaimable · {:.0}ms",
            telemetry::thousands(snap.entries),
            telemetry::human_bytes(totals.reclaimable),
            t0.elapsed().as_secs_f64() * 1000.0
        ),
    );
    if !denied.is_empty() {
        say(
            LineKind::Warn,
            format!(
                "{} path{} unreadable — grant Full Disk Access to include them",
                denied.len(),
                if denied.len() == 1 { "" } else { "s" }
            ),
        );
    }

    *phase.lock() = Phase::Done;
    tele.stop();

    ScanResult {
        disk: sysinfo::disk_stats(std::path::Path::new("/")),
        findings: all,
        totals,
        denied,
        duration_us: t0.elapsed().as_micros() as u64,
        entries_scanned: snap.entries,
        roots,
    }
}

fn probe_caches(stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    caches::CACHES
        .par_iter()
        .filter_map(|def| {
            let path = caches::resolve_def(def)?;
            let m = sweep::measure(&path, stats, inodes);
            if m.charge.total() < caches::MIN_BYTES {
                return None;
            }
            stats.add_finding();
            Some(caches::finding(
                def,
                &path,
                m.charge.exclusive,
                m.charge.shared,
                m.files,
                m.newest_mtime,
            ))
        })
        .collect()
}

fn probe_models(stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let mut out: Vec<Finding> = models::list_hf_models()
        .par_iter()
        .map(|m| {
            let s = sweep::measure(&m.path, stats, inodes);
            stats.add_finding();
            models::hf_finding(
                m,
                s.charge.exclusive,
                s.charge.shared,
                s.files,
                s.newest_mtime,
            )
        })
        .collect();

    // Ollama: exact sizes from the API; fall back to sizing the blob store.
    match models::list_ollama_models() {
        Some(list) if !list.is_empty() => {
            for m in &list {
                stats.add_finding();
                out.push(models::ollama_finding(m));
            }
        }
        _ => {
            let blobs = models::ollama_blobs();
            if blobs.is_dir() {
                let m = sweep::measure(&blobs, stats, inodes);
                if m.charge.total() > caches::MIN_BYTES {
                    stats.add_finding();
                    out.push(
                        Finding::new(
                            domains::Domain::AiModel,
                            "ollama:blobs",
                            "Ollama model blobs",
                        )
                        .at(blobs)
                        .detail(
                            "Ollama daemon is not running, so individual models cannot be listed."
                                .to_string(),
                        )
                        .sized(m.charge.exclusive, m.charge.shared, m.files)
                        .aged(m.newest_mtime)
                        .risk(domains::Risk::Caution)
                        .restore("ollama pull <model>"),
                    );
                }
            }
        }
    }

    for (name, path) in models::lmstudio_models() {
        let m = sweep::measure(&path, stats, inodes);
        if m.charge.total() < caches::MIN_BYTES {
            continue;
        }
        stats.add_finding();
        out.push(
            Finding::new(
                domains::Domain::AiModel,
                format!("lmstudio:{name}"),
                name.clone(),
            )
            .at(path)
            .detail("LM Studio".to_string())
            .sized(m.charge.exclusive, m.charge.shared, m.files)
            .aged(m.newest_mtime)
            .risk(domains::Risk::Caution),
        );
    }

    for (src, label, spec) in models::ML_CACHES {
        let p = paths::resolve(spec, None);
        if !p.is_dir() {
            continue;
        }
        let m = sweep::measure(&p, stats, inodes);
        if m.charge.total() < 1024 * 1024 {
            continue;
        }
        stats.add_finding();
        out.push(
            Finding::new(domains::Domain::AiModel, format!("ml_cache:{src}"), *label)
                .at(p)
                .detail("Downloaded model weights and datasets".to_string())
                .sized(m.charge.exclusive, m.charge.shared, m.files)
                .aged(m.newest_mtime)
                .risk(domains::Risk::Caution),
        );
    }

    out
}

fn probe_docker() -> Vec<Finding> {
    docker::scan().1
}

fn probe_global_python(stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    global_python::discover()
        .par_iter()
        .filter_map(|inst| {
            let a = global_python::audit(inst);
            // Nothing to prune and nothing to warn about.
            if a.non_essential.is_empty() && !inst.is_sip {
                return None;
            }
            let (bytes, shared, files) = match &inst.site_packages {
                Some(sp) => {
                    let m = sweep::measure(sp, stats, inodes);
                    (m.charge.exclusive, m.charge.shared, m.files)
                }
                None => (0, 0, 0),
            };
            if inst.is_sip && a.non_essential.is_empty() {
                return None;
            }
            stats.add_finding();
            Some(global_python::finding(inst, bytes, shared, files))
        })
        .collect()
}
