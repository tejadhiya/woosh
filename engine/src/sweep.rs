//! Parallel traversal built on top of a [`fastwalk`] backend.
//!
//! Two passes, both rayon work-stealing, and shared by **both** backends so the
//! host differential isolates exactly one variable (the syscall layer):
//!
//! * [`discover`] — depth-bounded walk of the dev roots. A caller-supplied
//!   detector claims directories it recognises; claimed subtrees are *not*
//!   classified further, which is what keeps the entry count down.
//! * [`measure`]  — sums a claimed subtree's allocated bytes with inode
//!   de-duplication. Pure accumulation, zero classification logic: the cheapest
//!   traversal that exists.
//!
//! The prototype does traversal **plus** one `du -sk` subprocess per candidate
//! (~5–15 ms of process spawn each, before any actual work). Here discovery and
//! sizing come out of one fused pass, because the walker already has the sizes.

use crate::dedup::{Charge, InodeSet};
use crate::fastwalk::{join, read_dir, read_dir_with, Entry, Kind, NameMode, WalkError};
use crate::stats::Stats;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What the detector decided about a directory.
pub enum Verdict {
    /// Not interesting in itself — keep classifying its children.
    Descend,
    /// Recognised. Measure the subtree as one finding and stop classifying inside.
    ///
    /// `kind` carries the *specific* variant the detector matched (which venv
    /// marker, which artifact cache). Without it the enrichment pass would have to
    /// re-detect from a path alone, which it cannot do — the markers live in the
    /// directory listing the walker has already consumed.
    Claim {
        tag: &'static str,
        kind: &'static str,
    },
    /// Ignore entirely (OS noise, another tool's territory, our own app dir).
    Skip,
}

/// A directory a detector recognised.
#[derive(Debug, Clone)]
pub struct Claim {
    pub path: PathBuf,
    pub tag: &'static str,
    /// The specific variant matched, e.g. `"venv (pyvenv.cfg)"` or `"conda"`.
    pub kind: &'static str,
    pub depth: usize,
    /// mtime of the claimed directory itself, for staleness ranking.
    pub mtime: i64,
}

/// An individual large file captured during discovery without extra syscalls.
#[derive(Debug, Clone)]
pub struct FileCandidate {
    pub path: PathBuf,
    pub alloc: u64,
    pub mtime: i64,
    pub dev: i64,
    pub ino: u64,
}

#[derive(Debug, Default)]
pub struct Discovered {
    pub claims: Vec<Claim>,
    pub files: Vec<FileCandidate>,
    /// Roots we were refused. Surfaced to the user, never swallowed (plan §8.3).
    pub denied: Vec<PathBuf>,
}

struct Cx<'a, F> {
    detect: F,
    max_depth: usize,
    file_threshold: u64,
    stats: &'a Stats,
    claims: Mutex<Vec<Claim>>,
    files: Mutex<Vec<FileCandidate>>,
    denied: Mutex<Vec<PathBuf>>,
}

/// Depth-bounded parallel discovery across `roots`.
///
/// `detect` is called once per directory with its listing already in hand, so a
/// detector can test for marker files (`pyvenv.cfg`, `Cargo.toml`, lockfiles)
/// without issuing a single extra syscall.
///
/// Any file with `alloc >= file_threshold` is captured into [`Discovered::files`]
/// during the same traversal at zero extra syscall cost.
pub fn discover<F>(
    roots: &[PathBuf],
    max_depth: usize,
    file_threshold: u64,
    stats: &Stats,
    detect: F,
) -> Discovered
where
    F: Fn(&Path, &[Entry], usize) -> Verdict + Send + Sync,
{
    let cx = Cx {
        detect,
        max_depth,
        file_threshold,
        stats,
        claims: Mutex::new(Vec::new()),
        files: Mutex::new(Vec::new()),
        denied: Mutex::new(Vec::new()),
    };

    rayon::scope(|s| {
        for root in roots {
            let cx = &cx;
            let root = root.clone();
            s.spawn(move |s| walk(s, root, 0, cx));
        }
    });

    Discovered {
        claims: cx.claims.into_inner(),
        files: cx.files.into_inner(),
        denied: cx.denied.into_inner(),
    }
}

fn is_installer_ext(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".dmg")
        || lower.ends_with(".pkg")
        || lower.ends_with(".iso")
        || lower.ends_with(".xip")
}

fn walk<'s, F>(scope: &rayon::Scope<'s>, dir: PathBuf, depth: usize, cx: &'s Cx<'s, F>)
where
    F: Fn(&Path, &[Entry], usize) -> Verdict + Send + Sync,
{
    cx.stats.add_dir();
    if let Some(s) = dir.to_str() {
        cx.stats.paths.offer(s);
    }

    let listing = match read_dir(&dir) {
        Ok(l) => l,
        Err(e) => {
            if e.is_denied() {
                cx.stats.add_denied();
                cx.denied.lock().push(dir);
            }
            return;
        }
    };

    cx.stats.add_entries(listing.entries.len() as u64);

    match (cx.detect)(&dir, &listing.entries, depth) {
        Verdict::Skip => return,
        Verdict::Claim { tag, kind } => {
            cx.stats.add_finding();
            let mtime = listing.entries.iter().map(|e| e.mtime).max().unwrap_or(0);
            cx.claims.lock().push(Claim {
                path: dir,
                tag,
                kind,
                depth,
                mtime,
            });
            return;
        }
        Verdict::Descend => {}
    }

    if cx.file_threshold > 0 {
        let is_download_dir = dir.components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("downloads")
        });

        for entry in &listing.entries {
            if entry.kind == Kind::File {
                if entry.name.starts_with('.') && entry.name.ends_with(".icloud") {
                    continue;
                }
                let is_qualifying = entry.alloc >= cx.file_threshold
                    || (is_download_dir
                        && is_installer_ext(&entry.name)
                        && entry.alloc >= 1_000_000);

                if is_qualifying {
                    cx.stats.add_finding();
                    let path = join(&dir, &entry.name);
                    cx.files.lock().push(FileCandidate {
                        path,
                        alloc: entry.alloc,
                        mtime: entry.mtime,
                        dev: entry.dev,
                        ino: entry.ino,
                    });
                }
            }
        }
    }

    if depth >= cx.max_depth {
        return;
    }

    for entry in &listing.entries {
        // Never follow symlinks: a link out of the tree would both skew sizes and
        // put us somewhere the safety layer never approved (plan §5.1).
        if entry.kind != Kind::Dir {
            continue;
        }
        let child = join(&dir, &entry.name);
        scope.spawn(move |s| walk(s, child, depth + 1, cx));
    }
}

/// Result of summing one subtree.
#[derive(Debug, Default, Clone, Copy)]
pub struct Measured {
    pub charge: Charge,
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    /// Most recent mtime anywhere inside — real "last used", unlike the directory's
    /// own mtime, which only tracks its immediate children.
    pub newest_mtime: i64,
}

struct MCx<'a> {
    exclusive: AtomicU64,
    shared: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    denied: AtomicU64,
    newest: AtomicU64,
    inodes: &'a InodeSet,
    stats: &'a Stats,
}

/// Sum a subtree's allocated bytes, de-duplicating hardlinks and APFS clones.
pub fn measure(root: &Path, stats: &Stats, inodes: &InodeSet) -> Measured {
    let cx = MCx {
        exclusive: AtomicU64::new(0),
        shared: AtomicU64::new(0),
        files: AtomicU64::new(0),
        dirs: AtomicU64::new(0),
        denied: AtomicU64::new(0),
        newest: AtomicU64::new(0),
        inodes,
        stats,
    };

    rayon::scope(|s| {
        let cx = &cx;
        let root = root.to_path_buf();
        s.spawn(move |s| sum(s, root, cx, 0));
    });

    Measured {
        charge: Charge {
            exclusive: cx.exclusive.load(Ordering::Relaxed),
            shared: cx.shared.load(Ordering::Relaxed),
        },
        files: cx.files.load(Ordering::Relaxed),
        dirs: cx.dirs.load(Ordering::Relaxed),
        denied: cx.denied.load(Ordering::Relaxed),
        newest_mtime: cx.newest.load(Ordering::Relaxed) as i64,
    }
}

/// How deep inline recursion may go before switching back to spawning.
///
/// Inline recursion keeps the stack, so an adversarially deep tree could overflow
/// it. Real trees are nowhere near this; spawning past it costs nothing because
/// trees that deep are rare.
const INLINE_DEPTH_LIMIT: usize = 48;

fn sum<'s>(scope: &rayon::Scope<'s>, dir: PathBuf, cx: &'s MCx<'s>, depth: usize) {
    cx.dirs.fetch_add(1, Ordering::Relaxed);
    cx.stats.add_dir();

    // Pure accumulation: file names are never read here, so do not allocate them.
    let listing = match read_dir_with(&dir, NameMode::DirsOnly) {
        Ok(l) => l,
        Err(e) => {
            if e.is_denied() {
                cx.denied.fetch_add(1, Ordering::Relaxed);
                cx.stats.add_denied();
            }
            return;
        }
    };

    cx.stats.add_entries(listing.entries.len() as u64);
    let mut local = Charge::default();
    let mut local_files = 0u64;
    let mut newest = 0i64;
    let mut subdirs: Vec<PathBuf> = Vec::new();

    for entry in &listing.entries {
        newest = newest.max(entry.mtime);
        match entry.kind {
            Kind::Dir => subdirs.push(join(&dir, &entry.name)),
            Kind::File => {
                local_files += 1;
                local += cx.inodes.charge(entry.dev, entry.ino, entry.alloc);
            }
            // Symlinks and devices hold no reclaimable bytes of their own, and
            // following them would double-count or escape the subtree.
            Kind::Symlink | Kind::Other => {}
        }
    }

    if local.exclusive > 0 {
        cx.exclusive.fetch_add(local.exclusive, Ordering::Relaxed);
        cx.stats.add_bytes(local.exclusive);
    }
    if local.shared > 0 {
        cx.shared.fetch_add(local.shared, Ordering::Relaxed);
    }
    if local_files > 0 {
        cx.files.fetch_add(local_files, Ordering::Relaxed);
    }
    cx.newest.fetch_max(newest.max(0) as u64, Ordering::Relaxed);

    // Spawn every child but one, and recurse into the last inline.
    //
    // Dependency trees are overwhelmingly single-child chains
    // (`node_modules/@scope/pkg/dist/esm/...`), so this removes one spawn per level
    // of every such chain. Measured effect on wall clock: none -- the walk is
    // syscall-bound, not scheduler-bound. It is kept because it bounds how many
    // queued tasks and `PathBuf`s a deep tree can pile up at once, not as a
    // speedup.
    let inline = if depth < INLINE_DEPTH_LIMIT {
        subdirs.pop()
    } else {
        None
    };
    for child in subdirs {
        scope.spawn(move |s| sum(s, child, cx, 0));
    }
    if let Some(child) = inline {
        sum(scope, child, cx, depth + 1);
    }
}

/// Size a single directory's immediate contents only (no recursion).
///
/// For the many cache roots whose size lives in one flat level, this avoids
/// spinning up a rayon scope for a job that is one syscall.
pub fn measure_shallow(dir: &Path, inodes: &InodeSet) -> Result<Measured, WalkError> {
    let listing = read_dir_with(dir, NameMode::DirsOnly)?;
    let mut m = Measured::default();
    for entry in &listing.entries {
        m.newest_mtime = m.newest_mtime.max(entry.mtime);
        if entry.kind == Kind::File {
            m.files += 1;
            m.charge += inodes.charge(entry.dev, entry.ino, entry.alloc);
        } else if entry.kind == Kind::Dir {
            m.dirs += 1;
        }
    }
    Ok(m)
}
