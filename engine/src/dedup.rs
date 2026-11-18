//! Hardlink and APFS-clone de-duplication.
//!
//! ## Why this exists
//!
//! `pnpm` is a content-addressable store that **hardlinks** packages into every
//! project's `node_modules` from `~/Library/pnpm/store`. APFS additionally clones
//! files copy-on-write. So the same physical bytes appear under many paths.
//!
//! Summing allocated size naively therefore reports space that deleting will not
//! give back. The Python prototype hits this: `du -sk` de-duplicates by inode, but
//! only *within a single invocation*, and the prototype calls it once per
//! candidate directory — so N projects sharing one store each claim the full size.
//!
//! [`InodeSet`] charges each `(dev, ino)` exactly once per scan. Findings then
//! carry two numbers:
//!
//! * `exclusive` — first sighting. Deleting this really returns these bytes.
//! * `shared`    — a repeat sighting. Returned only when the last referrer goes.
//!
//! This makes us look *less* impressive than the prototype and be *more* correct,
//! which is the right trade for a tool that deletes things.

use dashmap::DashMap;

/// Shard count. Power of two; sized so 8–12 rayon workers rarely collide.
const SHARDS: usize = 128;

/// Set of file identities already charged in this scan.
///
/// `DashMap` rather than `DashSet` because only the former exposes
/// `shard_amount`, and shard count is the whole point: it is what keeps the
/// rayon workers off each other's locks in the hot summing loop.
pub struct InodeSet {
    seen: DashMap<(i64, u64), ()>,
}

/// Outcome of charging one entry's bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Charge {
    /// Bytes reclaimed by deleting this path.
    pub exclusive: u64,
    /// Bytes shared with another path already counted in this scan.
    pub shared: u64,
}

impl Charge {
    #[inline]
    pub fn total(&self) -> u64 {
        self.exclusive + self.shared
    }
}

impl std::ops::AddAssign for Charge {
    fn add_assign(&mut self, rhs: Self) {
        self.exclusive += rhs.exclusive;
        self.shared += rhs.shared;
    }
}

impl Default for InodeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl InodeSet {
    pub fn new() -> Self {
        Self {
            seen: DashMap::with_capacity_and_shard_amount(1 << 16, SHARDS),
        }
    }

    /// Charge `bytes` for file identity `(dev, ino)`.
    ///
    /// `ino == 0` means the backend could not supply an identity; such entries are
    /// charged as exclusive rather than silently dropped — over-reporting a rare
    /// edge case is safer than hiding real bytes from the user.
    #[inline]
    pub fn charge(&self, dev: i64, ino: u64, bytes: u64) -> Charge {
        if bytes == 0 {
            return Charge::default();
        }
        if ino == 0 || self.seen.insert((dev, ino), ()).is_none() {
            Charge {
                exclusive: bytes,
                shared: 0,
            }
        } else {
            Charge {
                exclusive: 0,
                shared: bytes,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sighting_is_exclusive_repeat_is_shared() {
        let set = InodeSet::new();
        assert_eq!(
            set.charge(1, 42, 1000),
            Charge {
                exclusive: 1000,
                shared: 0
            }
        );
        assert_eq!(
            set.charge(1, 42, 1000),
            Charge {
                exclusive: 0,
                shared: 1000
            }
        );
    }

    #[test]
    fn same_inode_on_different_devices_does_not_collide() {
        let set = InodeSet::new();
        assert_eq!(set.charge(1, 42, 500).exclusive, 500);
        // Without the dev component this would be wrongly reported as shared,
        // hiding real reclaimable bytes on a second volume.
        assert_eq!(set.charge(2, 42, 500).exclusive, 500);
    }

    #[test]
    fn unknown_inode_is_charged_not_dropped() {
        let set = InodeSet::new();
        assert_eq!(set.charge(1, 0, 700).exclusive, 700);
        assert_eq!(set.charge(1, 0, 700).exclusive, 700);
    }

    #[test]
    fn zero_byte_entries_do_not_consume_identity() {
        let set = InodeSet::new();
        assert_eq!(set.charge(1, 9, 0), Charge::default());
        // The identity was never claimed, so real bytes still count later.
        assert_eq!(set.charge(1, 9, 123).exclusive, 123);
    }
}
