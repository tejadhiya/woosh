//! CoreSimulator devices detection and reclamation.
//!
//! On developer Macs, `~/Library/Developer/CoreSimulator/Devices` frequently grows
//! to 20–60 GB. When Xcode is updated or old simulator runtimes are removed,
//! simulator devices tied to those missing runtimes become "unavailable" and can
//! never be booted again — but their multi-gigabyte filesystems remain on disk.
//!
//! This module scans simulator devices, parses `device.plist` to read their metadata,
//! cross-references installed runtimes, and categorises:
//! - **Unavailable devices** (runtime missing): [`Risk::Safe`], can be deleted.
//! - **Available shutdown devices**: [`Risk::Caution`], recoverable via Xcode or `simctl`.
//! - **Booted devices**: [`Risk::Advisory`], never deleted while running.

use crate::dedup::InodeSet;
use crate::domains::{Domain, Finding, Risk};
use crate::stats::Stats;
use crate::sweep;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Minimum size to report a simulator device (10 MB).
pub const MIN_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct DevicePlist {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "deviceType", default)]
    pub device_type: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub state: Option<i64>,
    #[serde(rename = "UDID", default)]
    pub udid: Option<String>,
    #[serde(rename = "isAvailable", default)]
    pub is_available: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimDevice {
    pub udid: String,
    pub name: String,
    pub device_type: String,
    pub runtime: String,
    pub runtime_name: String,
    pub state: i64,
    pub is_available: bool,
    pub path: PathBuf,
}

/// Convert a runtime identifier like `com.apple.CoreSimulator.SimRuntime.iOS-16-4`
/// or `com.apple.CoreSimulator.SimRuntime.watchOS-9-1` to a human-readable string
/// like `iOS 16.4` or `watchOS 9.1`.
pub fn human_runtime(runtime_id: &str) -> String {
    if let Some(s) = runtime_id.strip_prefix("com.apple.CoreSimulator.SimRuntime.") {
        if let Some((os, ver)) = s.split_once('-') {
            let clean_ver = ver.replace('-', ".");
            return format!("{os} {clean_ver}");
        }
        return s.to_string();
    }
    runtime_id.to_string()
}

/// Discover installed runtime bundle identifiers from user, system, and Xcode platforms.
pub fn discover_installed_runtimes(home: &Path) -> HashSet<String> {
    let mut runtimes = HashSet::new();

    let candidate_dirs = [
        home.join("Library/Developer/CoreSimulator/Profiles/Runtimes"),
        PathBuf::from("/Library/Developer/CoreSimulator/Profiles/Runtimes"),
    ];

    for dir in &candidate_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|ext| ext == "simruntime") {
                    if let Some(id) = read_runtime_id(&p) {
                        runtimes.insert(id);
                    }
                }
            }
        }
    }

    // Xcode built-in platform runtimes
    let xcode_platforms = PathBuf::from("/Applications/Xcode.app/Contents/Developer/Platforms");
    if xcode_platforms.is_dir() {
        if let Ok(platforms) = std::fs::read_dir(xcode_platforms) {
            for plat in platforms.flatten() {
                let rdir = plat
                    .path()
                    .join("Library/Developer/CoreSimulator/Profiles/Runtimes");
                if let Ok(entries) = std::fs::read_dir(rdir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().is_some_and(|ext| ext == "simruntime") {
                            if let Some(id) = read_runtime_id(&p) {
                                runtimes.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }

    runtimes
}

fn read_runtime_id(bundle_path: &Path) -> Option<String> {
    let info_plist = bundle_path.join("Contents/Info.plist");
    if let Ok(val) = plist::Value::from_file(&info_plist) {
        if let Some(dict) = val.as_dictionary() {
            if let Some(id) = dict
                .get("CFBundleIdentifier")
                .or_else(|| dict.get("SimRuntimeIdentifier"))
                .and_then(|v| v.as_string())
            {
                return Some(id.to_string());
            }
        }
    }
    // Fallback: derive from folder name, e.g. "iOS 17.2.simruntime" -> "com.apple.CoreSimulator.SimRuntime.iOS-17-2"
    let file_stem = bundle_path.file_stem()?.to_string_lossy();
    if let Some((os, ver)) = file_stem.split_once(' ') {
        let slug_ver = ver.replace('.', "-");
        return Some(format!(
            "com.apple.CoreSimulator.SimRuntime.{os}-{slug_ver}"
        ));
    }
    None
}

/// Parse a simulator device from its directory.
pub fn parse_device(device_dir: &Path, installed_runtimes: &HashSet<String>) -> Option<SimDevice> {
    let plist_path = device_dir.join("device.plist");
    let val = plist::Value::from_file(&plist_path).ok()?;
    let parsed: DevicePlist = plist::from_value(&val).ok()?;

    let udid = parsed.udid.or_else(|| {
        device_dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
    })?;

    let name = parsed
        .name
        .unwrap_or_else(|| "Unknown Simulator".to_string());
    let device_type = parsed
        .device_type
        .unwrap_or_else(|| "Unknown Device Type".to_string());
    let runtime = parsed.runtime.unwrap_or_default();
    let runtime_name = human_runtime(&runtime);
    let state = parsed.state.unwrap_or(1); // Default 1 = Shutdown

    // Availability: if explicit in plist, use it; otherwise check against installed runtimes.
    // If no runtimes exist on the system at all, any device is unavailable.
    let is_available = parsed.is_available.unwrap_or_else(|| {
        if runtime.is_empty() {
            false
        } else if installed_runtimes.is_empty() {
            // No runtimes installed anywhere on this machine
            false
        } else {
            installed_runtimes.contains(&runtime)
        }
    });

    Some(SimDevice {
        udid,
        name,
        device_type,
        runtime,
        runtime_name,
        state,
        is_available,
        path: device_dir.to_path_buf(),
    })
}

/// Probe CoreSimulator devices under `home`.
pub fn probe_simulators(home: &Path, stats: &Stats, inodes: &InodeSet) -> Vec<Finding> {
    let devices_dir = home.join("Library/Developer/CoreSimulator/Devices");
    if !devices_dir.is_dir() {
        return Vec::new();
    }

    let installed_runtimes = discover_installed_runtimes(home);
    let entries = match std::fs::read_dir(&devices_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(dev) = parse_device(&p, &installed_runtimes) {
                devices.push(dev);
            }
        }
    }

    devices
        .into_iter()
        .filter_map(|dev| {
            let m = sweep::measure(&dev.path, stats, inodes);
            if m.charge.total() < MIN_BYTES {
                return None;
            }

            let is_booted = dev.state == 3;
            let (risk, detail, restore) = if is_booted {
                (
                    Risk::Advisory,
                    "Simulator is currently running (Booted). Protected from deletion.".to_string(),
                    None,
                )
            } else if !dev.is_available {
                (
                    Risk::Safe,
                    format!(
                        "Orphaned simulator — {} runtime is no longer installed on this Mac.",
                        dev.runtime_name
                    ),
                    Some("xcrun simctl delete unavailable".to_string()),
                )
            } else {
                (
                    Risk::Caution,
                    format!("Simulator device ({}, Shutdown).", dev.runtime_name),
                    Some(format!(
                        "xcrun simctl create \"{}\" \"{}\" \"{}\"",
                        dev.name, dev.device_type, dev.runtime
                    )),
                )
            };

            let display_name = if !dev.is_available {
                format!("{} ({}, unavailable)", dev.name, dev.runtime_name)
            } else {
                format!("{} ({})", dev.name, dev.runtime_name)
            };

            let meta = serde_json::json!({
                "udid": dev.udid,
                "device_name": dev.name,
                "device_type": dev.device_type,
                "runtime": dev.runtime,
                "runtime_name": dev.runtime_name,
                "state": if is_booted { "Booted" } else { "Shutdown" },
                "is_available": dev.is_available,
            });

            stats.add_finding();
            Some(
                Finding::new(Domain::Cache, format!("sim:{}", dev.udid), display_name)
                    .at(dev.path)
                    .detail(detail)
                    .sized(m.charge.exclusive, m.charge.shared, m.files)
                    .aged(m.newest_mtime)
                    .risk(risk)
                    .restore(restore.unwrap_or_default())
                    .meta(meta),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_runtime_decodes_cleanly() {
        assert_eq!(
            human_runtime("com.apple.CoreSimulator.SimRuntime.iOS-16-4"),
            "iOS 16.4"
        );
        assert_eq!(
            human_runtime("com.apple.CoreSimulator.SimRuntime.watchOS-9-1"),
            "watchOS 9.1"
        );
        assert_eq!(
            human_runtime("com.apple.CoreSimulator.SimRuntime.tvOS-17-0"),
            "tvOS 17.0"
        );
        assert_eq!(human_runtime("custom-runtime"), "custom-runtime");
    }

    #[test]
    fn parses_device_plist_correctly() {
        let temp = tempfile::tempdir().unwrap();
        let plist_path = temp.path().join("device.plist");
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>UDID</key>
    <string>4A17B79D-F4D3-4CDA-8E02-86BA6A3AC84F</string>
    <key>name</key>
    <string>iPhone 14 Pro</string>
    <key>deviceType</key>
    <string>com.apple.CoreSimulator.SimDeviceType.iPhone-14-Pro</string>
    <key>runtime</key>
    <string>com.apple.CoreSimulator.SimRuntime.iOS-16-4</string>
    <key>state</key>
    <integer>1</integer>
</dict>
</plist>"#;
        std::fs::write(&plist_path, xml).unwrap();

        let mut runtimes = HashSet::new();
        runtimes.insert("com.apple.CoreSimulator.SimRuntime.iOS-17-0".to_string());

        let dev = parse_device(temp.path(), &runtimes).expect("must parse device");
        assert_eq!(dev.udid, "4A17B79D-F4D3-4CDA-8E02-86BA6A3AC84F");
        assert_eq!(dev.name, "iPhone 14 Pro");
        assert_eq!(dev.runtime_name, "iOS 16.4");
        assert_eq!(dev.state, 1);
        // iOS 16.4 is not in runtimes -> is_available must be false
        assert!(!dev.is_available);

        // When runtime is present
        runtimes.insert("com.apple.CoreSimulator.SimRuntime.iOS-16-4".to_string());
        let dev2 = parse_device(temp.path(), &runtimes).expect("must parse device");
        assert!(dev2.is_available);
    }

    #[test]
    fn probe_simulators_detects_unavailable_and_booted_devices() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let dev_dir = home.join("Library/Developer/CoreSimulator/Devices");
        std::fs::create_dir_all(&dev_dir).unwrap();

        // 1. Unavailable device (runtime iOS-16-4 not installed)
        let unavail_dir = dev_dir.join("11111111-2222-3333-4444-555555555555");
        std::fs::create_dir_all(unavail_dir.join("data")).unwrap();
        let unavail_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>UDID</key>
    <string>11111111-2222-3333-4444-555555555555</string>
    <key>name</key>
    <string>iPhone 14 Pro</string>
    <key>deviceType</key>
    <string>com.apple.CoreSimulator.SimDeviceType.iPhone-14-Pro</string>
    <key>runtime</key>
    <string>com.apple.CoreSimulator.SimRuntime.iOS-16-4</string>
    <key>state</key>
    <integer>1</integer>
</dict>
</plist>"#;
        std::fs::write(unavail_dir.join("device.plist"), unavail_xml).unwrap();
        // 12 MB payload to exceed MIN_BYTES (10 MB)
        std::fs::write(
            unavail_dir.join("data/blob.bin"),
            vec![0u8; 12 * 1024 * 1024],
        )
        .unwrap();

        // 2. Booted device (state = 3)
        let booted_dir = dev_dir.join("99999999-8888-7777-6666-555555555555");
        std::fs::create_dir_all(booted_dir.join("data")).unwrap();
        let booted_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>UDID</key>
    <string>99999999-8888-7777-6666-555555555555</string>
    <key>name</key>
    <string>iPhone 15</string>
    <key>deviceType</key>
    <string>com.apple.CoreSimulator.SimDeviceType.iPhone-15</string>
    <key>runtime</key>
    <string>com.apple.CoreSimulator.SimRuntime.iOS-17-0</string>
    <key>state</key>
    <integer>3</integer>
</dict>
</plist>"#;
        std::fs::write(booted_dir.join("device.plist"), booted_xml).unwrap();
        std::fs::write(
            booted_dir.join("data/blob.bin"),
            vec![0u8; 12 * 1024 * 1024],
        )
        .unwrap();

        let stats = Stats::default();
        let inodes = InodeSet::new();
        let findings = probe_simulators(home, &stats, &inodes);

        assert_eq!(findings.len(), 2);
        let f_unavail = findings
            .iter()
            .find(|f| f.id == "sim:11111111-2222-3333-4444-555555555555")
            .expect("must find unavailable device");
        assert_eq!(f_unavail.risk, Risk::Safe);
        assert!(f_unavail.name.contains("unavailable"));
        assert!(f_unavail.bytes >= 12 * 1024 * 1024);
        assert_eq!(
            f_unavail.restore.as_deref(),
            Some("xcrun simctl delete unavailable")
        );

        let f_booted = findings
            .iter()
            .find(|f| f.id == "sim:99999999-8888-7777-6666-555555555555")
            .expect("must find booted device");
        assert_eq!(f_booted.risk, Risk::Advisory);
        assert!(f_booted.detail.contains("Booted"));
    }
}
