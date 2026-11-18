//! Zero-overhead persistent cache for Woosh scan results and findings.
//!
//! Cold start target: **< 3 ms** to load 400,000+ indexed items from disk
//! on Apple Silicon, completely eliminating full rescan delays upon app launch.

use crate::ScanResult;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Magic identifier at start of Woosh index files ("WOOSHIDX").
pub const MAGIC_BYTES: &[u8; 8] = b"WOOSHIDX";

/// Current binary index format schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Packed binary header written at the start of the index file.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub timestamp: i64,
    pub fsevent_id: u64,
    pub payload_len: u64,
}

impl IndexHeader {
    pub fn new(fsevent_id: u64, payload_len: u64) -> Self {
        Self {
            magic: *MAGIC_BYTES,
            version: SCHEMA_VERSION,
            timestamp: chrono::Utc::now().timestamp(),
            fsevent_id,
            payload_len,
        }
    }

    pub fn to_bytes(&self) -> [u8; 36] {
        let mut buf = [0u8; 36];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..20].copy_from_slice(&self.timestamp.to_le_bytes());
        buf[20..28].copy_from_slice(&self.fsevent_id.to_le_bytes());
        buf[28..36].copy_from_slice(&self.payload_len.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8; 36]) -> Option<Self> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&buf[0..8]);
        if &magic != MAGIC_BYTES {
            return None;
        }

        let version = u32::from_le_bytes(buf[8..12].try_into().ok()?);
        if version != SCHEMA_VERSION {
            return None;
        }

        let timestamp = i64::from_le_bytes(buf[12..20].try_into().ok()?);
        let fsevent_id = u64::from_le_bytes(buf[20..28].try_into().ok()?);
        let payload_len = u64::from_le_bytes(buf[28..36].try_into().ok()?);

        Some(Self {
            magic,
            version,
            timestamp,
            fsevent_id,
            payload_len,
        })
    }
}

/// Cached scan container loaded from disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedScan {
    pub timestamp: i64,
    pub fsevent_id: u64,
    pub scan_result: ScanResult,
}

/// Returns the default path for the Woosh index cache.
pub fn default_cache_path() -> PathBuf {
    crate::paths::resolve("~/Library/Application Support/Woosh/index.bin", None)
}

/// Query the system-wide monotonic FSEvent stream counter.
///
/// Returns the current 64-bit event ID from the macOS kernel via CoreServices.
/// On non-macOS platforms (e.g. Linux Docker verification), returns 0.
#[cfg(target_os = "macos")]
pub fn current_fsevent_id() -> u64 {
    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn FSEventsGetCurrentEventId() -> u64;
    }
    unsafe { FSEventsGetCurrentEventId() }
}

#[cfg(not(target_os = "macos"))]
pub fn current_fsevent_id() -> u64 {
    0
}

/// Save a scan result to disk atomically with verified header and FSEvent ID.
pub fn save_cache(path: &Path, scan_result: &ScanResult, fsevent_id: u64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = serde_json::to_vec(scan_result)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let header = IndexHeader::new(fsevent_id, payload.len() as u64);
    let header_bytes = header.to_bytes();

    // Write to a temporary file in the same directory, then rename atomically.
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(&header_bytes)?;
        file.write_all(&payload)?;
        file.sync_data()?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Load a cached scan result from disk.
///
/// Validates magic bytes, schema version, and payload integrity. If the file
/// is missing, invalid, or obsolete, returns `Ok(None)` gracefully.
pub fn load_cache(path: &Path) -> std::io::Result<Option<CachedScan>> {
    if !path.is_file() {
        return Ok(None);
    }

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };

    let mut header_buf = [0u8; 36];
    if file.read_exact(&mut header_buf).is_err() {
        return Ok(None);
    }

    let header = match IndexHeader::from_bytes(&header_buf) {
        Some(h) => h,
        None => return Ok(None), // Bad magic or unsupported version
    };

    let mut payload = Vec::with_capacity(header.payload_len as usize);
    file.read_to_end(&mut payload)?;

    if payload.len() as u64 != header.payload_len {
        return Ok(None); // Truncated payload
    }

    match serde_json::from_slice::<ScanResult>(&payload) {
        Ok(scan_result) => Ok(Some(CachedScan {
            timestamp: header.timestamp,
            fsevent_id: header.fsevent_id,
            scan_result,
        })),
        Err(_) => Ok(None), // Deserialization failure (schema mismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::{Domain, Finding, Risk};
    use crate::sysinfo::DiskStats;
    use crate::Totals;
    use tempfile::tempdir;

    fn mock_scan_result() -> ScanResult {
        let findings = vec![Finding {
            id: "test:1".into(),
            domain: Domain::NodeModules,
            name: "node_modules".into(),
            path: Some(PathBuf::from("/Users/test/project/node_modules")),
            detail: "Test project dependencies".into(),
            bytes: 1024 * 1024 * 150,
            shared_bytes: 0,
            files: 12000,
            mtime: 1700000000,
            days_idle: 14,
            risk: Risk::Safe,
            restore: Some("npm install".into()),
            meta: None,
        }];

        ScanResult {
            disk: DiskStats::default(),
            findings,
            totals: Totals {
                reclaimable: 1024 * 1024 * 150,
                shared: 0,
                items: 1,
                by_domain: vec![("node_modules".into(), 1024 * 1024 * 150, 1)],
            },
            denied: Vec::new(),
            duration_us: 15000,
            entries_scanned: 12000,
            roots: vec![PathBuf::from("/Users/test")],
        }
    }

    #[test]
    fn header_roundtrip_serialization() {
        let header = IndexHeader::new(4289104, 1048576);
        let bytes = header.to_bytes();
        let parsed = IndexHeader::from_bytes(&bytes).expect("valid header");

        assert_eq!(parsed.magic, *MAGIC_BYTES);
        assert_eq!(parsed.version, SCHEMA_VERSION);
        assert_eq!(parsed.fsevent_id, 4289104);
        assert_eq!(parsed.payload_len, 1048576);
    }

    #[test]
    fn invalid_magic_rejected() {
        let mut bytes = [0u8; 36];
        bytes[0..8].copy_from_slice(b"BADMAGIC");
        assert!(IndexHeader::from_bytes(&bytes).is_none());
    }

    #[test]
    fn save_and_load_cache_roundtrip() {
        let dir = tempdir().unwrap();
        let cache_file = dir.path().join("index.bin");
        let initial_scan = mock_scan_result();

        save_cache(&cache_file, &initial_scan, 987654321).expect("save should succeed");
        assert!(cache_file.is_file());

        let loaded = load_cache(&cache_file)
            .expect("load should succeed")
            .expect("should return cached scan");

        assert_eq!(loaded.fsevent_id, 987654321);
        assert_eq!(loaded.scan_result.findings.len(), 1);
        assert_eq!(loaded.scan_result.findings[0].name, "node_modules");
        assert_eq!(loaded.scan_result.findings[0].bytes, 1024 * 1024 * 150);
        assert_eq!(loaded.scan_result.totals.reclaimable, 1024 * 1024 * 150);
    }

    #[test]
    fn non_existent_cache_returns_none() {
        let dir = tempdir().unwrap();
        let cache_file = dir.path().join("does_not_exist.bin");

        let loaded = load_cache(&cache_file).expect("should not error on missing file");
        assert!(loaded.is_none());
    }

    #[test]
    fn corrupted_cache_returns_none() {
        let dir = tempdir().unwrap();
        let cache_file = dir.path().join("corrupted.bin");
        fs::write(&cache_file, b"corrupted random data that is short").unwrap();

        let loaded = load_cache(&cache_file).expect("should not panic on corrupted file");
        assert!(loaded.is_none());
    }
}
