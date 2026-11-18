//! Filesystem traversal layer.
//!
//! Two interchangeable backends behind one trait:
//!
//! * [`macos`] — `getattrlistbulk(2)`. Name + type + **allocated size** + inode +
//!   mtime for a whole batch of entries in a single syscall. This is what makes the
//!   sweep budget reachable; see `docs/IMPLEMENTATION_PLAN.md` §0.2.
//! * [`portable`] — `std::fs` + `st_blocks` fallback. Exists so the engine compiles
//!   and the whole suite runs inside Linux Docker (plan §0.1).
//!
//! Both are driven against the same fixtures with the same asserted answers; a
//! host-side differential (`make verify-macos`) proves they agree.

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(not(target_os = "macos"))]
pub mod portable;

#[cfg(target_os = "macos")]
pub use macos::read_dir_with;
#[cfg(not(target_os = "macos"))]
pub use portable::read_dir_with;

/// Which entry names the caller actually needs.
///
/// The measure pass reads `alloc`/`ino`/`dev` and never looks at a file's name --
/// only directory names, to build child paths. Allocating a `String` per file
/// anyway costs one heap allocation per entry, and at ~1.7M entries that is the
/// single largest cost in a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMode {
    /// Names for every entry. Required for detection.
    All,
    /// Names for directories only; files get an empty name.
    DirsOnly,
}

/// Read a directory, with names for every entry.
#[inline]
pub fn read_dir(path: &Path) -> Result<Listing, WalkError> {
    read_dir_with(path, NameMode::All)
}

/// What a directory entry is. Mirrors `enum vtype` from `<sys/vnode.h>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    Other,
}

/// One directory entry, with everything the engine needs to decide and to size.
///
/// `alloc` is **allocated** bytes (`ATTR_FILE_ALLOCSIZE` / `st_blocks * 512`), not
/// apparent length. That is what actually comes back when the file is removed, and
/// it makes sparse files (`Docker.raw`: ~64 GB apparent, ~12 GB real) correct for
/// free, with no special case.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: Box<str>,
    pub kind: Kind,
    pub alloc: u64,
    /// Inode, for hardlink/clone de-duplication (plan §1). `0` when unavailable.
    pub ino: u64,
    /// Device id. Paired with `ino` it forms a globally unique file identity, so a
    /// multi-volume scan cannot false-collide in the dedup set.
    pub dev: i64,
    /// Modification time, unix seconds.
    pub mtime: i64,
}

impl Entry {
    #[inline]
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }
}

/// Why a directory could not be read.
///
/// `Denied` is deliberately its own variant and never collapsed into "error":
/// it is the signal that drives the Full Disk Access prompt (plan §8.3). The
/// Python prototype's bare `except Exception: pass` is what makes a 40 GB
/// simulator folder silently vanish from the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkError {
    /// `EACCES` / `EPERM` — needs Full Disk Access or belongs to another user.
    Denied,
    /// `ENOENT` — vanished mid-scan. Normal; caches churn while we look at them.
    Gone,
    /// `ENOTDIR`, `ELOOP`, I/O failures, …
    Other(i32),
}

impl WalkError {
    #[inline]
    pub fn from_errno(e: i32) -> Self {
        match e {
            libc::EACCES | libc::EPERM => WalkError::Denied,
            libc::ENOENT => WalkError::Gone,
            other => WalkError::Other(other),
        }
    }

    #[inline]
    pub fn is_denied(&self) -> bool {
        matches!(self, WalkError::Denied)
    }
}

impl std::fmt::Display for WalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalkError::Denied => write!(f, "permission denied (may need Full Disk Access)"),
            WalkError::Gone => write!(f, "path disappeared during scan"),
            WalkError::Other(e) => write!(f, "io error {e}"),
        }
    }
}

/// A directory listing plus anything that went wrong producing it.
#[derive(Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
}

/// Join without reallocating the parent each time.
#[inline]
pub fn join(parent: &Path, name: &str) -> PathBuf {
    let mut p = PathBuf::with_capacity(parent.as_os_str().len() + name.len() + 1);
    p.push(parent);
    p.push(name);
    p
}
