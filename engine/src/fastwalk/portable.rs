//! Portable directory reader — `std::fs` + `st_blocks`.
//!
//! Exists so the engine compiles and its whole test suite runs inside Linux
//! Docker (plan §0.1), where `getattrlistbulk` does not exist. It is deliberately
//! the *same shape* as [`super::macos::read_dir`]: one directory, no recursion, no
//! symlink following. All parallelism lives in `crate::sweep`, shared by both
//! backends — so the host differential (`make verify-macos`) isolates exactly one
//! variable, the syscall layer.
//!
//! `st_blocks` is in 512-byte units on both Linux and macOS, matching
//! `ATTR_FILE_ALLOCSIZE` semantics: allocated, not apparent.

use super::{Entry, Kind, Listing, NameMode, WalkError};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub fn read_dir_with(path: &Path, names: NameMode) -> Result<Listing, WalkError> {
    let rd = std::fs::read_dir(path).map_err(map_io)?;
    let mut entries = Vec::with_capacity(64);

    for item in rd {
        let item = match item {
            Ok(i) => i,
            // A single bad entry must not sink the whole directory, matching the
            // per-entry ATTR_CMN_ERROR handling in the macOS backend.
            Err(_) => continue,
        };
        let name = item.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }

        // symlink_metadata == FSOPT_NOFOLLOW: report the link, never its target.
        let md = match std::fs::symlink_metadata(item.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let ft = md.file_type();
        let kind = if ft.is_symlink() {
            Kind::Symlink
        } else if ft.is_dir() {
            Kind::Dir
        } else if ft.is_file() {
            Kind::File
        } else {
            Kind::Other
        };

        let alloc = match kind {
            Kind::Dir | Kind::File => md.blocks().saturating_mul(512),
            _ => 0,
        };

        entries.push(Entry {
            name: if names == NameMode::All || kind == Kind::Dir {
                name.into()
            } else {
                Box::from("")
            },
            kind,
            alloc,
            ino: md.ino(),
            dev: md.dev() as i64,
            mtime: md.mtime(),
        });
    }

    Ok(Listing { entries })
}

fn map_io(e: std::io::Error) -> WalkError {
    WalkError::from_errno(e.raw_os_error().unwrap_or(0))
}
