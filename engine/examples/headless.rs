//! Headless sweep — runs the real engine against this machine and prints what it
//! finds. Exists so the full pipeline (fast walk, detectors, dedup, Docker socket,
//! Ollama API, interpreter audit) can be exercised and timed without the GUI.
//!
//!   cargo run --release --example headless
//!   cargo run --release --example headless -- --deep
//!   cargo run --release --example headless -- --root ~/dev

use std::path::PathBuf;
use woosh_engine::{telemetry::human_bytes, Config};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let deep = args.iter().any(|a| a == "--deep");
    let quiet = args.iter().any(|a| a == "--quiet");
    let roots: Vec<PathBuf> = args
        .windows(2)
        .filter(|w| w[0] == "--root")
        .map(|w| PathBuf::from(shellexpand(&w[1])))
        .collect();

    let cfg = Config {
        roots: (!roots.is_empty()).then_some(roots),
        max_depth: 5,
        deep,
        ..Config::default()
    };

    // Track phase transitions from the telemetry stream itself -- which also
    // exercises the coalescer under real load.
    let phases = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<(String, u64, u64)>::new()));
    let ph = phases.clone();

    let t0 = std::time::Instant::now();
    let r = woosh_engine::scan(&cfg, move |f| {
        {
            let mut v = ph.lock();
            let name = format!("{:?}", f.phase);
            match v.last_mut() {
                Some(last) if last.0 == name => {
                    last.2 = f.elapsed_us;
                }
                _ => v.push((name, f.elapsed_us, f.elapsed_us)),
            }
        }
        if !quiet {
            for l in &f.lines {
                println!(
                    "  [{:>9.4}s] {} {}",
                    f.elapsed_us as f64 / 1e6,
                    l.glyph,
                    l.text
                );
            }
        }
    });

    println!("\n{}", "─".repeat(78));
    println!(
        "  {:>10}  entries   {:>10}  wall   {:>12}  entries/sec",
        r.entries_scanned,
        format!("{:.0}ms", t0.elapsed().as_secs_f64() * 1000.0),
        format!(
            "{:.0}",
            r.entries_scanned as f64 / t0.elapsed().as_secs_f64()
        ),
    );
    println!(
        "  disk {} free of {}  ·  {} reclaimable across {} items",
        human_bytes(r.disk.free),
        human_bytes(r.disk.total),
        human_bytes(r.totals.reclaimable),
        r.totals.items
    );
    if r.totals.shared > 0 {
        println!(
            "  {} additionally shared via hardlinks/clones (not double-counted)",
            human_bytes(r.totals.shared)
        );
    }
    if !r.denied.is_empty() {
        println!("  {} paths unreadable (Full Disk Access)", r.denied.len());
    }
    println!("  phases:");
    for (name, start, end) in phases.lock().iter() {
        println!(
            "    {:<10} {:>7.0}ms",
            name.to_lowercase(),
            (*end as f64 - *start as f64) / 1000.0
        );
    }
    println!("{}", "─".repeat(78));

    for (label, bytes, n) in &r.totals.by_domain {
        println!(
            "  {:<24} {:>12}  {:>4} items",
            label,
            human_bytes(*bytes),
            n
        );
    }
    println!("{}", "─".repeat(78));

    println!("\n  top findings");
    for f in r.findings.iter().take(20) {
        println!(
            "  {:>11}  {:<40} {}",
            human_bytes(f.bytes),
            truncate(&f.name, 40),
            truncate(&f.detail, 48)
        );
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

fn shellexpand(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(h) = std::env::var("HOME") {
            return format!("{h}/{rest}");
        }
    }
    s.to_string()
}
