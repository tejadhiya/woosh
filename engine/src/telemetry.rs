//! Live telemetry: a 60 Hz frame coalescer and the log voice.
//!
//! ## Why a coalescer
//!
//! The walker sustains hundreds of thousands of entries per second. One event per
//! entry would mean ~800k IPC messages/sec and a scan paced by the renderer. So:
//!
//! ```text
//! rayon workers ──► relaxed atomics (entries, bytes, dirs)      ─┐
//!              └──► bounded channel (discoveries — rare)        ─┼─► 60 Hz tick ──► one frame
//!              └──► sampling ring (live paths, never queued)    ─┘
//! ```
//!
//! Counters are *read* by the coalescer, never pushed. Discoveries are rare enough
//! (hundreds, not millions) to ride a channel. Live paths are sampled. Nothing in
//! the hot loop can ever block on the UI.

use crate::stats::Stats;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 60 Hz. Fast enough to feel live, slow enough that the renderer keeps up.
pub const TICK: Duration = Duration::from_micros(16_666);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Boot,
    Discover,
    Measure,
    Probe,
    Reap,
    Done,
}

/// Log line class. The UI colour-codes on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Boot,
    Scan,
    Found,
    Woosh,
    Warn,
    Done,
}

impl LineKind {
    pub fn glyph(&self) -> &'static str {
        match self {
            LineKind::Boot => "⟫",
            LineKind::Scan => "⟳",
            LineKind::Found => "◈",
            LineKind::Woosh => "✦",
            LineKind::Warn => "⚠",
            LineKind::Done => "✓",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub t_us: u64,
    pub kind: LineKind,
    pub glyph: &'static str,
    pub text: String,
}

impl LogLine {
    pub fn new(t_us: u64, kind: LineKind, text: impl Into<String>) -> Self {
        Self {
            t_us,
            kind,
            glyph: kind.glyph(),
            text: text.into(),
        }
    }
}

/// One coalesced frame. Emitted on the `"telemetry"` channel.
#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    pub seq: u64,
    pub elapsed_us: u64,
    pub phase: Phase,
    pub entries: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Entries per second over the last tick — instantaneous, not cumulative.
    pub rate: u64,
    pub denied: u64,
    pub findings: u64,
    pub paths: Vec<String>,
    pub lines: Vec<LogLine>,
}

/// Human byte formatting, matching the prototype's `format_bytes`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n == 0 {
        return "0 B".into();
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i >= 2 {
        format!("{v:.2} {}", UNITS[i])
    } else {
        format!("{v:.0} {}", UNITS[i])
    }
}

/// Shorten a path for display: `~` for home, elide the middle when long.
pub fn pretty_path(p: &std::path::Path, home: &std::path::Path) -> String {
    let s = p
        .strip_prefix(home)
        .map(|r| format!("~/{}", r.display()))
        .unwrap_or_else(|_| p.display().to_string());
    if s.len() <= 58 {
        return s;
    }
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() > 4 {
        format!(
            "{}/{}/…/{}/{}",
            parts[0],
            parts[1],
            parts[parts.len() - 2],
            parts[parts.len() - 1]
        )
    } else {
        s
    }
}

/// Scan-progress narration. Rotates deterministically — no RNG, so a replayed
/// scan reads identically and the wit never costs information: every variant
/// still carries the live counters.
pub fn scan_line(
    t_us: u64,
    entries: u64,
    bytes: u64,
    rate: u64,
    path: Option<&str>,
    seq: u64,
) -> LogLine {
    let where_at = path.unwrap_or("the deep stacks");
    let verb = ["descending", "sweeping", "combing", "tracing", "sounding"][(seq % 5) as usize];
    LogLine::new(
        t_us,
        LineKind::Scan,
        format!(
            "{entries} entries · {} · {}/s · {verb} {where_at}",
            human_bytes(bytes),
            thousands(rate)
        ),
    )
}

/// Separate thousands with thin separators for legibility in a monospace feed.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let b = s.as_bytes();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*c as char);
    }
    out
}

/// Narration for a discovery.
pub fn found_line(t_us: u64, name: &str, bytes: u64, note: &str) -> LogLine {
    LogLine::new(
        t_us,
        LineKind::Found,
        format!(
            "FOUND  {name}   {}{}",
            human_bytes(bytes),
            if note.is_empty() {
                String::new()
            } else {
                format!("   {note}")
            }
        ),
    )
}

/// Narration for a completed deletion.
pub fn woosh_line(t_us: u64, name: &str, bytes: u64, restorable: bool) -> LogLine {
    LogLine::new(
        t_us,
        LineKind::Woosh,
        format!(
            "WOOSH  {name} → {}   {}",
            if restorable { "Trash" } else { "gone" },
            human_bytes(bytes)
        ),
    )
}

/// Running handle for a coalescer thread.
pub struct Handle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// Stop ticking and wait for the final frame to be emitted.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Current phase, shared so the engine can advance it as work progresses.
pub type PhaseCell = Arc<parking_lot::Mutex<Phase>>;

/// Start emitting frames at [`TICK`] until the handle is stopped.
///
/// `rx` carries discoveries. Each tick drains whatever is pending, reads the
/// atomics, samples the path ring, and emits exactly one frame.
pub fn spawn<F>(
    stats: Arc<Stats>,
    phase: PhaseCell,
    rx: crossbeam_channel::Receiver<LogLine>,
    emit: F,
) -> Handle
where
    F: Fn(Frame) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = stop.clone();

    let thread = std::thread::Builder::new()
        .name("woosh-telemetry".into())
        .spawn(move || {
            let t0 = Instant::now();
            let mut seq = 0u64;
            let mut last_entries = 0u64;
            let mut last_t = t0;

            loop {
                let stopping = stop_t.load(Ordering::Relaxed);
                std::thread::sleep(TICK);

                let snap = stats.snapshot();
                let now = Instant::now();
                let dt = now.duration_since(last_t).as_secs_f64().max(1e-6);
                let rate = ((snap.entries.saturating_sub(last_entries)) as f64 / dt) as u64;
                last_entries = snap.entries;
                last_t = now;

                let elapsed_us = t0.elapsed().as_micros() as u64;
                let mut lines: Vec<LogLine> = rx.try_iter().collect();

                // Narrate progress only while there is progress to narrate, and
                // only when the discovery feed is quiet — findings outrank status.
                let current_phase = *phase.lock();
                if lines.is_empty()
                    && rate > 0
                    && matches!(current_phase, Phase::Discover | Phase::Measure)
                {
                    let paths = stats.paths.snapshot();
                    lines.push(scan_line(
                        elapsed_us,
                        snap.entries,
                        snap.bytes,
                        rate,
                        paths.first().map(|s| s.as_str()),
                        seq,
                    ));
                }

                emit(Frame {
                    seq,
                    elapsed_us,
                    phase: current_phase,
                    entries: snap.entries,
                    dirs: snap.dirs,
                    bytes: snap.bytes,
                    rate,
                    denied: snap.denied,
                    findings: snap.findings,
                    paths: stats.paths.snapshot(),
                    lines,
                });
                seq += 1;

                // One extra tick after the stop flag, so the final counters and any
                // last queued lines always reach the UI.
                if stopping {
                    break;
                }
            }
        })
        .expect("spawn telemetry thread");

    Handle {
        stop,
        thread: Some(thread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn human_bytes_matches_prototype_formatting() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.00 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
    }

    #[test]
    fn thousands_separates_correctly() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(772_431), "772,431");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn pretty_path_uses_tilde_and_elides_long_paths() {
        let home = Path::new("/Users/dev");
        assert_eq!(
            pretty_path(Path::new("/Users/dev/code/app"), home),
            "~/code/app"
        );
        let long = Path::new(
            "/Users/dev/code/really/deeply/nested/project/tree/with/many/segments/node_modules",
        );
        let out = pretty_path(long, home);
        assert!(out.contains('…'), "expected elision, got {out}");
        assert!(out.ends_with("node_modules"), "tail must survive: {out}");
    }

    #[test]
    fn scan_lines_always_carry_the_live_counters() {
        // The wit must never replace the data. Every rotation variant keeps both.
        for seq in 0..10 {
            let l = scan_line(1, 318_442, 1_500_000_000, 772_431, Some("~/x"), seq);
            assert!(l.text.contains("318442"), "{}", l.text);
            assert!(l.text.contains("772,431"), "{}", l.text);
        }
    }

    #[test]
    fn coalescer_emits_frames_and_stops_cleanly() {
        let stats = Arc::new(Stats::default());
        let phase: PhaseCell = Arc::new(parking_lot::Mutex::new(Phase::Discover));
        let (tx, rx) = crossbeam_channel::unbounded();
        let count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let c2 = count.clone();

        let h = spawn(stats.clone(), phase, rx, move |_f| {
            c2.fetch_add(1, Ordering::Relaxed);
        });

        stats.add_entries(1000);
        tx.send(LogLine::new(0, LineKind::Found, "test")).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        h.stop();

        assert!(
            count.load(Ordering::Relaxed) >= 2,
            "expected several frames, got {}",
            count.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn discoveries_are_never_dropped_in_favour_of_status_lines() {
        // Findings outrank narration: when a discovery is pending, the frame
        // carries it rather than a progress line.
        let stats = Arc::new(Stats::default());
        let phase: PhaseCell = Arc::new(parking_lot::Mutex::new(Phase::Discover));
        let (tx, rx) = crossbeam_channel::unbounded();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::<LineKind>::new()));
        let s2 = seen.clone();

        let h = spawn(stats.clone(), phase, rx, move |f| {
            s2.lock().extend(f.lines.iter().map(|l| l.kind));
        });
        for i in 0..5 {
            tx.send(LogLine::new(i, LineKind::Found, format!("f{i}")))
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(120));
        h.stop();

        let kinds = seen.lock();
        assert_eq!(
            kinds.iter().filter(|k| **k == LineKind::Found).count(),
            5,
            "all discoveries must survive: {kinds:?}"
        );
    }
}
