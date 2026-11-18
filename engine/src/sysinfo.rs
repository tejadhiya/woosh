//! Disk and host facts — port of `core/system_info.py`.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiskStats {
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub percent_used: f64,
    pub arch: String,
    pub hostname: String,
}

/// `statvfs` on the volume containing `path`.
///
/// `f_bavail` (available to an unprivileged user), not `f_bfree`, because the
/// reserved blocks are not space the user can actually use.
///
/// The `as u64` casts look redundant and are not: `fsblkcnt_t` is `u32` on macOS
/// and `u64` on Linux, so they are load-bearing on the target we actually ship.
/// Clippy only sees the Linux widths inside the verification container.
#[allow(clippy::unnecessary_cast)]
pub fn disk_stats(path: &Path) -> DiskStats {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let mut out = DiskStats {
        mount: path.display().to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: hostname(),
        ..Default::default()
    };

    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return out;
    };
    // SAFETY: zeroed statvfs is a valid initial state; `c` outlives the call.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return out;
    }

    let frsize = if s.f_frsize > 0 {
        s.f_frsize as u64
    } else {
        s.f_bsize as u64
    };
    out.total = (s.f_blocks as u64).saturating_mul(frsize);
    out.free = (s.f_bavail as u64).saturating_mul(frsize);
    out.used = out.total.saturating_sub(out.free);
    out.percent_used = if out.total > 0 {
        (out.used as f64 / out.total as f64) * 100.0
    } else {
        0.0
    };
    out
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for its own length.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "this Mac".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_volume_reports_plausible_numbers() {
        let d = disk_stats(Path::new("/"));
        assert!(d.total > 0, "total must be non-zero");
        assert!(d.used <= d.total);
        assert!(d.free <= d.total);
        assert!((0.0..=100.0).contains(&d.percent_used));
    }

    #[test]
    fn nonexistent_path_degrades_instead_of_panicking() {
        let d = disk_stats(Path::new("/no/such/volume/anywhere"));
        assert_eq!(d.total, 0);
    }
}
