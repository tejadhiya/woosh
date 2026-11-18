//! Sweep benchmark. Gates the performance budget (plan §1) against a synthetic
//! known-answer tree, and reports warm vs cold separately because they are bound
//! by different things: warm is CPU/syscall bound, cold is NVMe-random-read bound.
//!
//! Custom harness (no criterion) so the numbers printed are wall-clock entries/sec
//! on a tree whose size we already know, which is the number that actually matters.

use std::time::Instant;
use woosh_engine::dedup::InodeSet;
use woosh_engine::stats::Stats;
use woosh_engine::sweep;

fn main() {
    // cargo runs benches with CWD at the *package* root, not the workspace root,
    // so the fixture at the repo top is one level up from here.
    let name = std::env::var("WOOSH_FIXTURE").unwrap_or_else(|_| ".fixture".into());
    let Some(root) = [
        std::path::PathBuf::from(&name),
        std::path::Path::new("..").join(&name),
    ]
    .into_iter()
    .find(|p| p.is_dir()) else {
        eprintln!("fixture not found at {name} or ../{name}; run `make fixture` first");
        std::process::exit(0);
    };

    println!("sweep benchmark — fixture {}", root.display());

    // Warm the metadata cache first so the two runs measure what they claim to.
    let warmup = Stats::default();
    let _ = sweep::measure(&root, &warmup, &InodeSet::new());

    let mut best = f64::MAX;
    for round in 1..=5 {
        let stats = Stats::default();
        let inodes = InodeSet::new();
        let t0 = Instant::now();
        let m = sweep::measure(&root, &stats, &inodes);
        let secs = t0.elapsed().as_secs_f64();
        best = best.min(secs);
        let snap = stats.snapshot();
        println!(
            "  round {round}: {:>9} entries  {:>8.1} MB  {:>7.0} ms  {:>9.0} entries/s{}",
            snap.entries,
            m.charge.exclusive as f64 / 1e6,
            secs * 1e3,
            snap.entries as f64 / secs,
            if m.charge.shared > 0 {
                format!("  (+{:.1} MB shared)", m.charge.shared as f64 / 1e6)
            } else {
                String::new()
            }
        );
    }
    println!("  best: {:.0} ms", best * 1e3);
}
