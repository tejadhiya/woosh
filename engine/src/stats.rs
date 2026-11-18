//! Live scan counters.
//!
//! ## The throughput problem
//!
//! The walker sustains hundreds of thousands of entries per second. Emitting one
//! IPC event per entry would mean ~800k messages/sec across the Tauri bridge,
//! which would destroy both the bridge and the renderer — and the scan would end
//! up pacing itself against the UI.
//!
//! So **event volume is decoupled from scan volume**: workers only ever touch
//! relaxed atomics and a small sampling ring. Nothing blocks, nothing queues, and
//! backpressure is impossible by construction. A coalescer
//! ([`crate::telemetry`]) reads these at 60 Hz and emits one frame per tick.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many "currently descending" paths to keep visible.
const RING: usize = 6;

/// Sampling ring of in-flight paths.
///
/// Deliberately *sampled, not queued*: workers overwrite slots and the coalescer
/// reads whatever happens to be current. A queue here would add backpressure to
/// the hot loop to show the user strings they can't read at 60 Hz anyway.
pub struct PathRing {
    slots: [Mutex<String>; RING],
    cursor: AtomicU64,
}

impl Default for PathRing {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| Mutex::new(String::new())),
            cursor: AtomicU64::new(0),
        }
    }
}

impl PathRing {
    #[inline]
    pub fn offer(&self, path: &str) {
        let i = (self.cursor.fetch_add(1, Ordering::Relaxed) as usize) % RING;
        // try_lock: if the coalescer is mid-read we simply skip this sample.
        // Never make the walker wait on the UI.
        if let Some(mut slot) = self.slots[i].try_lock() {
            slot.clear();
            slot.push_str(path);
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.slots
            .iter()
            .filter_map(|s| s.try_lock().map(|g| g.clone()))
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Monotonic scan counters. All relaxed — these drive a progress display, so
/// exact ordering between them has no observable meaning.
#[derive(Default)]
pub struct Stats {
    pub entries: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    /// Directories we were refused. Drives the Full Disk Access hint (plan §8.3).
    pub denied: AtomicU64,
    pub findings: AtomicU64,
    pub paths: PathRing,
}

impl Stats {
    #[inline]
    pub fn add_entries(&self, n: u64) {
        self.entries.fetch_add(n, Ordering::Relaxed);
    }
    #[inline]
    pub fn add_dir(&self) {
        self.dirs.fetch_add(1, Ordering::Relaxed);
    }
    #[inline]
    pub fn add_bytes(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }
    #[inline]
    pub fn add_denied(&self) {
        self.denied.fetch_add(1, Ordering::Relaxed);
    }
    #[inline]
    pub fn add_finding(&self) {
        self.findings.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            entries: self.entries.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            denied: self.denied.load(Ordering::Relaxed),
            findings: self.findings.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct StatsSnapshot {
    pub entries: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub denied: u64,
    pub findings: u64,
}
