//! Integration tests against the known-answer fixture (`scripts/make-fixture.sh`).
//!
//! These are the behaviours that are easy to get silently wrong and expensive to
//! get wrong: name-agnostic detection, hardlink de-duplication, sparse files, the
//! `target/` gate, and every safety refusal. Each test states what breaks if the
//! assertion fails.
//!
//! The same assertions run against **both** fastwalk backends — `getattrlistbulk`
//! on macOS, `std::fs` in the Linux verification container — which is what makes
//! the host differential meaningful.

use std::path::{Path, PathBuf};
use woosh_engine::dedup::InodeSet;
use woosh_engine::domains::Domain;
use woosh_engine::safety;
use woosh_engine::stats::Stats;
use woosh_engine::sweep;

fn fixture() -> Option<PathBuf> {
    let root = std::env::var("WOOSH_FIXTURE").unwrap_or_else(|_| ".fixture".into());
    // Tests run with CWD = the crate dir; the fixture lives at the workspace root.
    for cand in [PathBuf::from(&root), PathBuf::from("..").join(&root)] {
        if cand.is_dir() {
            return Some(cand.canonicalize().unwrap_or(cand));
        }
    }
    None
}

macro_rules! fixture_or_skip {
    () => {
        match fixture() {
            Some(f) => f,
            None => {
                eprintln!("fixture missing — run `make fixture`; skipping");
                return;
            }
        }
    };
}

fn scan(root: &Path) -> Vec<woosh_engine::domains::Finding> {
    let stats = Stats::default();
    let inodes = InodeSet::new();
    woosh_engine::scan_paths(&[root.to_path_buf()], 8, &stats, &inodes)
}

#[test]
fn venv_detection_is_name_agnostic() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    let venvs: Vec<_> = found.iter().filter(|x| x.domain == Domain::Venv).collect();

    // The prototype's core design intent: structural detection. If this regresses,
    // every environment not literally named `.venv` becomes invisible.
    for name in [".venv", "venv", "myenv", "mynenv", "custom-env"] {
        assert!(
            venvs
                .iter()
                .any(|v| v.path.as_ref().unwrap().ends_with(name)),
            "venv named {name:?} was not detected; found: {:?}",
            venvs.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
    }
    // conda-meta is a venv marker too.
    assert!(
        venvs
            .iter()
            .any(|v| v.path.as_ref().unwrap().ends_with("env") && v.detail.contains("conda")),
        "conda environment not detected"
    );
}

#[test]
fn venv_packages_are_read_without_subprocesses() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    let v = found
        .iter()
        .find(|x| x.domain == Domain::Venv && x.path.as_ref().unwrap().ends_with("mynenv"))
        .expect("mynenv not found");

    let meta = v.meta.as_ref().unwrap();
    assert_eq!(meta["python_version"], "3.13.1");
    let pkgs = meta["packages"].as_array().unwrap();
    let names: Vec<&str> = pkgs.iter().map(|p| p["name"].as_str().unwrap()).collect();
    for want in ["requests", "numpy", "flask"] {
        assert!(names.contains(&want), "missing {want} in {names:?}");
    }
}

#[test]
fn dist_info_folder_name_fallback_covers_missing_metadata() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    let v = found
        .iter()
        .find(|x| x.domain == Domain::Venv && x.path.as_ref().unwrap().ends_with(".venv"))
        .expect(".venv not found");
    let pkgs = v.meta.as_ref().unwrap()["packages"].as_array().unwrap();
    // `orphan-9.9.9.dist-info` has no METADATA at all. Dropping it would silently
    // produce an incomplete req-inplace.txt, which is a data-loss bug.
    assert!(
        pkgs.iter()
            .any(|p| p["name"] == "orphan" && p["version"] == "9.9.9"),
        "folder-name fallback failed: {pkgs:?}"
    );
}

#[test]
fn hardlinked_bytes_are_charged_exactly_once() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    let linked: Vec<_> = found
        .iter()
        .filter(|x| x.name.starts_with("linked-"))
        .collect();
    assert_eq!(linked.len(), 2, "expected both linked projects");

    let exclusive: u64 = linked.iter().map(|x| x.bytes).sum();
    let shared: u64 = linked.iter().map(|x| x.shared_bytes).sum();

    // One 4 MB blob hardlinked into two projects. Deleting both returns 4 MB, not 8.
    // `du -sk` per directory -- what the prototype does -- reports 8 MB here.
    assert!(
        (3_500_000..5_000_000).contains(&exclusive),
        "exclusive bytes should be ~4MB once, got {exclusive}"
    );
    assert!(
        shared > 3_000_000,
        "second sighting should be shared, got {shared}"
    );
}

#[test]
fn sparse_files_report_allocated_not_apparent_size() {
    let f = fixture_or_skip!();
    let sparse = f.join("sparse");
    if !sparse.join("thin.img").exists() {
        return;
    }
    let stats = Stats::default();
    let m = sweep::measure(&sparse, &stats, &InodeSet::new());

    // The file is 64 MB apparent, ~0 allocated. Using apparent length would make
    // Docker.raw and every sparse image wildly over-report.
    let apparent = std::fs::metadata(sparse.join("thin.img")).unwrap().len();
    if apparent > 1_000_000 {
        assert!(
            m.charge.exclusive < 1_000_000,
            "sparse file reported {} bytes for a {apparent}-byte apparent file",
            m.charge.exclusive
        );
    }
}

#[test]
fn cargo_target_requires_the_sibling_manifest() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    let targets: Vec<_> = found
        .iter()
        .filter(|x| x.path.as_ref().is_some_and(|p| p.ends_with("target")))
        .collect();

    assert!(
        targets.iter().any(|t| t
            .path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains("rs-real")),
        "a real Cargo project's target/ must be found"
    );
    // Without the gate, every unrelated folder called `target` becomes a
    // deletion candidate.
    assert!(
        !targets.iter().any(|t| t
            .path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains("not-rust")),
        "target/ without Cargo.toml must NOT be claimed"
    );
}

#[test]
fn build_output_is_pruned_and_never_reported() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    for bad in ["output/dist", "output/build"] {
        assert!(
            !found.iter().any(|x| x
                .path
                .as_ref()
                .is_some_and(|p| p.to_string_lossy().contains(bad))),
            "{bad} must never be offered for deletion — it is often real build output"
        );
    }
}

#[test]
fn regenerable_caches_are_claimed() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    for c in ["__pycache__", ".mypy_cache", ".next", ".turbo"] {
        assert!(
            found
                .iter()
                .any(|x| x.path.as_ref().is_some_and(|p| p.ends_with(c))),
            "{c} should be claimed as a safe artifact"
        );
    }
}

#[test]
fn node_modules_package_manager_is_detected_per_project() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    for (proj, mgr) in [
        ("js-npm", "npm"),
        ("js-pnpm", "pnpm"),
        ("js-yarn", "yarn"),
        ("js-bun", "bun"),
    ] {
        let nm = found
            .iter()
            .find(|x| {
                x.domain == Domain::NodeModules
                    && x.path.as_ref().unwrap().to_string_lossy().contains(proj)
            })
            .unwrap_or_else(|| panic!("{proj}/node_modules not found"));
        assert_eq!(
            nm.meta.as_ref().unwrap()["package_manager"],
            mgr,
            "{proj} should be detected as {mgr}"
        );
    }
}

#[test]
fn yarn_pnp_projects_are_not_reported() {
    let f = fixture_or_skip!();
    let found = scan(&f);
    // Under PnP there is no real node_modules to reclaim; reporting one is a
    // false positive that costs the user trust.
    //
    // Matched on an exact path component: "js-pnp" is a substring of "js-pnpm",
    // so a `contains` check here passes for the wrong reason.
    assert!(
        !found.iter().any(|x| x
            .path
            .as_ref()
            .is_some_and(|p| p.components().any(|c| c.as_os_str() == "js-pnp"))),
        "Yarn PnP project must not yield a node_modules finding"
    );
}

#[test]
fn symlink_traps_never_escape_the_tree() {
    let f = fixture_or_skip!();
    let traps = f.join("traps");
    if !traps.exists() {
        return;
    }

    // A path *through* a symlink pointing at a system directory must be refused.
    let through = traps.join("escape-absolute").join("CoreServices");
    assert!(
        safety::vet(&through).is_err(),
        "path through an escaping symlink must be refused"
    );

    // And the walk must not follow links at all: a symlink loop would hang forever.
    let stats = Stats::default();
    let m = sweep::measure(&traps, &stats, &InodeSet::new());
    assert!(
        m.dirs < 50,
        "walker appears to have followed symlinks: {} dirs",
        m.dirs
    );
}

#[test]
fn every_finding_carries_a_restore_path_or_is_advisory() {
    let f = fixture_or_skip!();
    for finding in scan(&f) {
        assert!(
            finding.restore.is_some() || finding.risk == woosh_engine::domains::Risk::Advisory,
            "{} offers deletion with no way back",
            finding.id
        );
    }
}

#[test]
fn large_files_and_packages_are_claimed() {
    let f = fixture_or_skip!();
    std::env::set_var("WOOSH_LARGE_FILE_THRESHOLD", "5242880");
    let found = scan(&f);

    // 1. Photos library package reported as ONE finding in Domain::LargeFile, Risk::Advisory
    let pkg = found.iter().find(|x| {
        x.path
            .as_ref()
            .is_some_and(|p| p.to_string_lossy().ends_with("MyPhotos.photoslibrary"))
    });
    assert!(pkg.is_some(), "MyPhotos.photoslibrary must be found");
    let pkg = pkg.unwrap();
    assert_eq!(pkg.domain, Domain::LargeFile);
    assert_eq!(pkg.risk, woosh_engine::domains::Risk::Advisory);
    // Ensure the walker did not descend inside the package to report its internal files
    assert!(
        !found.iter().any(|x| x
            .path
            .as_ref()
            .is_some_and(|p| p.to_string_lossy().contains("database.sqlite"))),
        "files inside package must not be individual findings"
    );

    // 2. Large files: .dmg and .mp4
    let dmg = found.iter().find(|x| x.name == "sample_installer.dmg");
    assert!(dmg.is_some(), "sample_installer.dmg must be found");
    assert_eq!(dmg.unwrap().domain, Domain::LargeFile);
    assert_eq!(dmg.unwrap().risk, woosh_engine::domains::Risk::Caution);

    let mp4 = found.iter().find(|x| x.name == "vacation.mp4");
    assert!(mp4.is_some(), "vacation.mp4 must be found");
    assert_eq!(mp4.unwrap().domain, Domain::LargeFile);
    assert_eq!(mp4.unwrap().risk, woosh_engine::domains::Risk::Caution);

    // 3. Sparse large file (1GB apparent, ~0 allocated) must NOT be claimed because threshold is by allocated size
    let sparse = found.iter().find(|x| x.name == "huge_sparse.vmdk");
    assert!(
        sparse.is_none(),
        "sparse file with ~0 allocated bytes must NOT meet the allocated-bytes threshold"
    );
}

#[test]
fn package_safety_refusal_at_vet_time() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("deep/enough/Photos.photoslibrary");
    let inner_db = pkg.join("database.sqlite");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(&inner_db, b"db").unwrap();

    // Deleting inside a package bundle must be refused by safety::vet with InsidePackage
    let res = safety::vet(&inner_db);
    assert!(
        matches!(res, Err(safety::Refusal::InsidePackage(_))),
        "safety::vet must refuse deleting files inside a package bundle, got {res:?}"
    );

    // Regular file outside packages in a permitted directory is vetted
    let video = tmp.path().join("deep/enough/vacation.mp4");
    std::fs::write(&video, b"video").unwrap();
    assert!(
        safety::vet(&video).is_ok(),
        "regular file outside packages must be vetted"
    );
}

#[test]
fn fast_walker_agrees_with_std_on_counts_and_bytes() {
    let f = fixture_or_skip!();
    let stats = Stats::default();
    let m = sweep::measure(&f, &stats, &InodeSet::new());
    let (files, bytes, raw) = std_walk(&f);

    // Counts catch a truncated directory listing.
    assert_eq!(
        m.files, files,
        "backend saw {} files, filesystem has {files}",
        m.files
    );

    // Bytes catch a mis-parsed attribute buffer. This is the gate that matters:
    // the macOS backend once read allocsize 8 bytes late -- landing on the name
    // string -- and parsed filenames as sizes, reporting exabytes. File counts
    // were perfect throughout, so only a byte-level differential catches it.
    //
    // Both halves of the charge are checked, because they mean different things:
    //   exclusive          == what deleting actually returns (de-duplicated)
    //   exclusive + shared == every sighting, de-duplication switched off
    // Comparing only the sum would pass even if de-duplication were broken.
    assert_eq!(
        m.charge.exclusive,
        bytes,
        "reclaimable bytes: backend {} vs std {bytes} (delta {})",
        m.charge.exclusive,
        m.charge.exclusive as i128 - bytes as i128
    );
    assert_eq!(
        m.charge.total(),
        raw,
        "raw bytes: backend {} vs std {raw} (delta {})",
        m.charge.total(),
        m.charge.total() as i128 - raw as i128
    );
    // The fixture hardlinks one 4 MiB blob three ways, so the two must differ --
    // otherwise this test would pass with de-duplication silently disabled.
    assert!(
        m.charge.shared > 0 && raw > bytes,
        "fixture should contain hardlinked bytes; shared={} raw={raw} dedup={bytes}",
        m.charge.shared
    );
}

#[test]
fn allocated_size_is_reported_not_apparent_size() {
    // Its own tempdir, never inside the fixture: cargo runs tests in parallel, and
    // writing a probe file into the shared tree races the differential's file count.
    let tmp = tempfile::tempdir().unwrap();
    let probe = tmp.path();
    std::fs::write(probe.join("one-byte.txt"), b"x").unwrap();

    let stats = Stats::default();
    let m = sweep::measure(probe, &stats, &InodeSet::new());

    // A 1-byte file occupies a whole filesystem block. Reporting 1 byte would mean
    // we are reading apparent length, which under-reports every small file and
    // wildly over-reports every sparse one.
    assert!(
        m.charge.total() >= 512,
        "1-byte file reported {} bytes; expected a full block",
        m.charge.total()
    );
}

/// Independent walk via std, used to cross-check the fast walker.
///
/// Returns `(files, deduplicated_bytes, raw_bytes)`. `st_blocks * 512` matches
/// `ATTR_FILE_ALLOCSIZE` semantics: allocated, not apparent.
fn std_walk(root: &Path) -> (u64, u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let (mut files, mut bytes, mut raw) = (0u64, 0u64, 0u64);
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(md) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            let ft = md.file_type();
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                files += 1;
                let b = md.blocks().saturating_mul(512);
                raw += b;
                // Mirror the engine's hardlink de-duplication.
                if seen.insert((md.dev(), md.ino())) {
                    bytes += b;
                }
            }
        }
    }
    (files, bytes, raw)
}
