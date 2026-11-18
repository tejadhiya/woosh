//! Docker storage — port of `core/docker_manager.py`.
//!
//! **Δ: talks to `/var/run/docker.sock` directly** instead of shelling out to
//! `docker system df`. The prompt asks for socket inspection, and it removes the
//! prototype's `_parse_docker_size` helper, which did float arithmetic on display
//! strings like `"1.5GB (50%)"`. The API returns exact `i64` bytes.

use super::{days_since, Domain, Finding, Risk};
use crate::http::{request, Endpoint};
use crate::paths;
use crate::telemetry::human_bytes;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Docker Engine API version floor that still ships `/system/df`.
const API: &str = "/v1.44";

pub fn socket_path() -> PathBuf {
    // Docker Desktop on macOS also exposes a per-user socket; prefer the standard
    // path and fall back to the user one.
    let standard = PathBuf::from("/var/run/docker.sock");
    if standard.exists() {
        return standard;
    }
    paths::resolve("~/.docker/run/docker.sock", None)
}

fn api_get(path: &str) -> Option<serde_json::Value> {
    let sock = socket_path();
    let body = request(Endpoint::Unix(&sock), "GET", &format!("{API}{path}"), None).ok()?;
    serde_json::from_str(&body).ok()
}

pub fn daemon_running() -> bool {
    api_get("/_ping").is_some()
        || request(
            Endpoint::Unix(&socket_path()),
            "GET",
            &format!("{API}/_ping"),
            None,
        )
        .is_ok()
}

/// Docker Desktop's VM disk image.
#[derive(Debug, Clone, Serialize)]
pub struct VmDisk {
    pub path: PathBuf,
    /// Blocks actually allocated — what you would really reclaim.
    pub real_bytes: u64,
    /// Sparse apparent length.
    pub apparent_bytes: u64,
    pub is_sparse: bool,
}

/// Locate and size `Docker.raw`.
///
/// Sparse-file handling comes free here: `blocks * 512` is the allocated size, so
/// a 64 GB apparent / 12 GB real image reports 12 GB with no special case. The
/// prototype needed a dedicated `du -sk` call to get this right.
pub fn vm_disk() -> Option<VmDisk> {
    use std::os::unix::fs::MetadataExt;
    const CANDIDATES: &[&str] = &[
        "~/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw",
        "~/Library/Containers/com.docker.docker/Data/vms/0/Docker.raw",
        "~/Library/Containers/com.docker.docker/Data/Docker.raw",
    ];
    for spec in CANDIDATES {
        let p = paths::resolve(spec, None);
        if let Ok(md) = p.metadata() {
            let real = md.blocks().saturating_mul(512);
            let apparent = md.len();
            return Some(VmDisk {
                path: p,
                real_bytes: real,
                apparent_bytes: apparent,
                is_sparse: apparent > real,
            });
        }
    }
    None
}

/// One prunable Docker resource class.
pub struct Prunable {
    pub id: &'static str,
    pub name: &'static str,
    /// Engine API prune endpoint.
    pub endpoint: &'static str,
    /// Only remove unreferenced objects, never anything in use.
    pub query: &'static str,
}

pub const PRUNABLES: &[Prunable] = &[
    Prunable {
        id: "docker:build_cache",
        name: "Docker build cache",
        endpoint: "/build/prune",
        query: "",
    },
    Prunable {
        id: "docker:images",
        name: "Docker unused images",
        endpoint: "/images/prune",
        query: "?filters=%7B%22dangling%22%3A%5B%22false%22%5D%7D",
    },
    Prunable {
        id: "docker:containers",
        name: "Docker stopped containers",
        endpoint: "/containers/prune",
        query: "",
    },
    Prunable {
        id: "docker:volumes",
        name: "Docker dangling volumes",
        endpoint: "/volumes/prune",
        query: "",
    },
];

#[derive(Debug, Clone, Serialize, Default)]
pub struct DockerReport {
    pub daemon_running: bool,
    pub vm_disk: Option<VmDisk>,
    pub total_reclaimable: u64,
}

/// Parse `/system/df` payload into detailed, selectable findings.
pub fn parse_df(df: &serde_json::Value, vm: Option<&VmDisk>) -> (DockerReport, Vec<Finding>) {
    let mut report = DockerReport {
        daemon_running: true,
        vm_disk: vm.cloned(),
        total_reclaimable: 0,
    };
    let mut findings = Vec::new();

    // 1. Build volume <-> container maps
    let mut container_to_vols: HashMap<String, Vec<String>> = HashMap::new();
    let mut vol_to_containers: HashMap<String, Vec<(String, String)>> = HashMap::new();

    let containers_arr = df.get("Containers").and_then(|v| v.as_array());
    if let Some(arr) = containers_arr {
        for c in arr {
            let full_id = c
                .get("Id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let raw_name = c
                .get("Names")
                .and_then(|n| n.as_array())
                .and_then(|a| a.first())
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let clean_name = raw_name.trim_start_matches('/');
            let display_name = if clean_name.is_empty() {
                full_id.chars().take(12).collect::<String>()
            } else {
                clean_name.to_string()
            };
            let state = c
                .get("State")
                .and_then(|s| s.as_str())
                .unwrap_or("exited")
                .to_string();

            let mut mounted = Vec::new();
            if let Some(mounts) = c.get("Mounts").and_then(|m| m.as_array()) {
                for m in mounts {
                    if let Some(v_name) = m.get("Name").and_then(|n| n.as_str()) {
                        mounted.push(v_name.to_string());
                        vol_to_containers
                            .entry(v_name.to_string())
                            .or_default()
                            .push((display_name.clone(), state.clone()));
                    }
                }
            }
            container_to_vols.insert(full_id, mounted);
        }
    }

    // 2. Containers (Active & Stopped)
    if let Some(arr) = containers_arr {
        for c in arr {
            let full_id = c
                .get("Id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let short_id: String = full_id.chars().take(12).collect();
            let raw_name = c
                .get("Names")
                .and_then(|n| n.as_array())
                .and_then(|a| a.first())
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let clean_name = raw_name.trim_start_matches('/');
            let name = if clean_name.is_empty() {
                short_id.clone()
            } else {
                clean_name.to_string()
            };
            let image = c
                .get("Image")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            let state = c
                .get("State")
                .and_then(|s| s.as_str())
                .unwrap_or("exited")
                .to_string();
            let status = c
                .get("Status")
                .and_then(|s| s.as_str())
                .unwrap_or(&state)
                .to_string();
            let size_rw = c.get("SizeRw").and_then(|s| s.as_i64()).unwrap_or(0).max(0) as u64;
            let size_root_fs = c
                .get("SizeRootFs")
                .and_then(|s| s.as_i64())
                .unwrap_or(0)
                .max(0) as u64;
            let created = c.get("Created").and_then(|s| s.as_i64()).unwrap_or(0);

            let vols = container_to_vols.get(&full_id).cloned().unwrap_or_default();
            let vols_summary = if vols.is_empty() {
                "none".to_string()
            } else {
                vols.join(", ")
            };

            let is_running = state == "running";
            let mut f = Finding::new(
                Domain::Docker,
                format!("docker:container:{short_id}"),
                if is_running {
                    format!("Active: {name}")
                } else {
                    format!("Stopped: {name}")
                },
            );
            f.detail = format!(
                "{} · Image: {image} · Attached volumes: {vols_summary}",
                if is_running {
                    format!("Active ({status})")
                } else {
                    format!("Stopped ({status})")
                }
            );
            f.bytes = size_rw;
            f.shared_bytes = size_root_fs.saturating_sub(size_rw);
            f.files = 1;
            f = f.aged(created);

            if is_running {
                f.risk = Risk::Advisory;
                f.meta = Some(serde_json::json!({
                    "docker_type": "container",
                    "sub_type": "active_container",
                    "state": "running",
                    "status": status,
                    "image": image,
                    "container_id": full_id,
                    "short_id": short_id,
                    "name": name,
                    "volumes": vols,
                    "size_rw": size_rw,
                    "size_root_fs": size_root_fs,
                }));
            } else {
                f.risk = Risk::Caution;
                f.restore = Some(format!("docker start {name}"));
                f.meta = Some(serde_json::json!({
                    "docker_type": "container",
                    "sub_type": "stopped_container",
                    "action": "delete_container",
                    "target": full_id,
                    "state": state,
                    "status": status,
                    "image": image,
                    "container_id": full_id,
                    "short_id": short_id,
                    "name": name,
                    "volumes": vols,
                    "size_rw": size_rw,
                    "size_root_fs": size_root_fs,
                }));
            }
            findings.push(f);
        }
    }

    // 3. Volumes (Dangling & Attached)
    if let Some(arr) = df.get("Volumes").and_then(|v| v.as_array()) {
        for v in arr {
            let v_name = v
                .get("Name")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            if v_name.is_empty() {
                continue;
            }
            let driver = v
                .get("Driver")
                .and_then(|s| s.as_str())
                .unwrap_or("local")
                .to_string();
            let mountpoint = v
                .get("Mountpoint")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let size = v
                .get("UsageData")
                .and_then(|u| u.get("Size"))
                .and_then(|s| s.as_i64())
                .unwrap_or(0)
                .max(0) as u64;

            let attached_info = vol_to_containers.get(&v_name);
            let is_attached_to_active = attached_info
                .map(|conts| conts.iter().any(|(_, st)| st == "running"))
                .unwrap_or(false);
            let cont_names: Vec<String> = attached_info
                .map(|conts| conts.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default();
            let conts_summary = if cont_names.is_empty() {
                "none (unattached)".to_string()
            } else {
                cont_names.join(", ")
            };

            let mut f = Finding::new(
                Domain::Docker,
                format!("docker:volume:{v_name}"),
                format!("Volume: {v_name}"),
            );
            f.bytes = size;
            f.files = 1;

            if is_attached_to_active {
                f.detail =
                    format!("Attached volume · Driver: {driver} · In use by: {conts_summary}");
                f.risk = Risk::Advisory;
                f.meta = Some(serde_json::json!({
                    "docker_type": "volume",
                    "sub_type": "attached_volume",
                    "volume_name": v_name,
                    "driver": driver,
                    "mountpoint": mountpoint,
                    "attached": true,
                    "is_active": true,
                    "containers": cont_names,
                }));
            } else {
                f.detail = format!(
                    "Dangling volume · Driver: {driver} · {}",
                    if cont_names.is_empty() {
                        "Unattached".to_string()
                    } else {
                        format!("Attached to stopped: {conts_summary}")
                    }
                );
                f.risk = Risk::Caution;
                f.meta = Some(serde_json::json!({
                    "docker_type": "volume",
                    "sub_type": "dangling_volume",
                    "action": "delete_volume",
                    "target": v_name,
                    "volume_name": v_name,
                    "driver": driver,
                    "mountpoint": mountpoint,
                    "attached": !cont_names.is_empty(),
                    "is_active": false,
                    "containers": cont_names,
                }));
            }
            findings.push(f);
        }
    }

    // 4. Images (Unused & In-Use)
    if let Some(arr) = df.get("Images").and_then(|v| v.as_array()) {
        for img in arr {
            let full_id = img
                .get("Id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let short_id: String = full_id
                .trim_start_matches("sha256:")
                .chars()
                .take(12)
                .collect();
            let tags: Vec<String> = img
                .get("RepoTags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            let repo_digests: Vec<String> = img
                .get("RepoDigests")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            // An image is pullable if any RepoDigest contains "@sha256:" and does not start with "<none>"
            let is_pullable = repo_digests
                .iter()
                .any(|d| d.contains("@sha256:") && !d.starts_with("<none>"));

            let labels: std::collections::HashMap<String, String> = img
                .get("Labels")
                .and_then(|v| v.as_object())
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            let compose_project = labels.get("com.docker.compose.project").cloned();
            let is_untagged = tags.is_empty()
                || tags
                    .iter()
                    .all(|t| t == "<none>:<none>" || t.starts_with("<none>"));

            let origin = if is_pullable {
                "registry"
            } else if is_untagged {
                "dangling_build"
            } else {
                "local_build"
            };

            let primary_tag = if is_untagged {
                format!("Untagged ({short_id})")
            } else {
                tags.iter()
                    .find(|t| !t.starts_with("<none>"))
                    .cloned()
                    .or_else(|| {
                        repo_digests
                            .first()
                            .and_then(|s| s.split('@').next())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| format!("Image {short_id}"))
            };

            let containers_count = img.get("Containers").and_then(|c| c.as_i64()).unwrap_or(0);
            let size = img.get("Size").and_then(|s| s.as_i64()).unwrap_or(0).max(0) as u64;
            let shared_size = img
                .get("SharedSize")
                .and_then(|s| s.as_i64())
                .unwrap_or(0)
                .max(0) as u64;
            let created = img.get("Created").and_then(|c| c.as_i64()).unwrap_or(0);
            let exclusive = size.saturating_sub(shared_size);

            let is_unused = containers_count <= 0;
            let mut f = Finding::new(
                Domain::Docker,
                format!("docker:image:{short_id}"),
                if is_unused {
                    primary_tag.clone()
                } else {
                    format!("{primary_tag} (in use)")
                },
            );
            f.bytes = if exclusive > 0 { exclusive } else { size };
            f.shared_bytes = shared_size;
            f.files = 1;
            f = f.aged(created);

            let origin_desc = match origin {
                "registry" => "Pullable from registry",
                "local_build" => {
                    if let Some(proj) = &compose_project {
                        &format!("Built on device (Compose: {proj})")
                    } else {
                        "Built on device"
                    }
                }
                _ => "Untagged build layer",
            };

            if is_unused {
                f.detail = format!(
                    "{origin_desc} · Unused · Created {}d ago · Shared {}",
                    days_since(created),
                    human_bytes(shared_size)
                );
                f.risk = Risk::Caution;
                if is_pullable && !primary_tag.starts_with("Untagged") {
                    f.restore = Some(format!("docker pull {primary_tag}"));
                } else if origin == "local_build" {
                    if let Some(proj) = &compose_project {
                        f.restore = Some(format!("docker compose build (project: {proj})"));
                    } else {
                        f.restore = Some(format!("docker build -t {primary_tag} <source-path>"));
                    }
                }
                f.meta = Some(serde_json::json!({
                    "docker_type": "image",
                    "sub_type": "unused_image",
                    "action": "delete_image",
                    "target": full_id,
                    "image_id": full_id,
                    "short_id": short_id,
                    "tag": primary_tag,
                    "all_tags": tags,
                    "repo_digests": repo_digests,
                    "containers": 0,
                    "size": size,
                    "shared_size": shared_size,
                    "is_pullable": is_pullable,
                    "origin": origin,
                    "compose_project": compose_project,
                }));
            } else {
                f.detail = format!(
                    "{origin_desc} · In use by {containers_count} container{} · Shared {}",
                    if containers_count == 1 { "" } else { "s" },
                    human_bytes(shared_size)
                );
                f.risk = Risk::Advisory;
                f.meta = Some(serde_json::json!({
                    "docker_type": "image",
                    "sub_type": "in_use_image",
                    "image_id": full_id,
                    "short_id": short_id,
                    "tag": primary_tag,
                    "all_tags": tags,
                    "repo_digests": repo_digests,
                    "containers": containers_count,
                    "size": size,
                    "shared_size": shared_size,
                    "is_pullable": is_pullable,
                    "origin": origin,
                    "compose_project": compose_project,
                }));
            }
            findings.push(f);
        }
    }

    // 5. Build cache
    let bc_arr = df.get("BuildCache").and_then(|v| v.as_array());
    if let Some(arr) = bc_arr {
        let reclaimable: Vec<_> = arr
            .iter()
            .filter(|c| !c.get("InUse").and_then(|u| u.as_bool()).unwrap_or(false))
            .collect();
        let bc_bytes: i64 = reclaimable
            .iter()
            .filter_map(|c| c.get("Size").and_then(|s| s.as_i64()))
            .sum();

        if bc_bytes > 0 || !arr.is_empty() {
            let mut f = Finding::new(Domain::Docker, "docker:build_cache", "Docker build cache");
            f.detail = format!(
                "{} build cache layers ({} reclaimable). Rebuilt on next build.",
                arr.len(),
                human_bytes(bc_bytes.max(0) as u64)
            );
            f.bytes = bc_bytes.max(0) as u64;
            f.files = arr.len() as u64;
            f.risk = Risk::Safe;
            f.meta = Some(serde_json::json!({
                "docker_type": "build_cache",
                "action": "prune_build_cache",
                "total_layers": arr.len(),
                "reclaimable_layers": reclaimable.len(),
            }));
            findings.push(f);
        }
    }

    // 6. VM Disk
    if let Some(d) = vm {
        findings.push(
            Finding::new(Domain::Docker, "docker:vm_disk", "Docker.raw virtual disk")
                .at(d.path.clone())
                .detail(format!(
                    "Docker Desktop VM image{}. Reclaim space inside Docker.",
                    if d.is_sparse { " (sparse)" } else { "" }
                ))
                .sized(d.real_bytes, 0, 1)
                .risk(Risk::Advisory),
        );
    }

    report.total_reclaimable = findings
        .iter()
        .filter(|f| f.risk != Risk::Advisory)
        .map(|f| f.bytes)
        .sum();
    (report, findings)
}

/// `GET /system/df` → findings, one per detailed resource class.
pub fn scan() -> (DockerReport, Vec<Finding>) {
    let vm = vm_disk();
    let Some(df) = api_get("/system/df") else {
        // Daemon down: report the VM disk alone, as the prototype does.
        let report = DockerReport {
            daemon_running: false,
            vm_disk: vm.clone(),
            total_reclaimable: 0,
        };
        let mut findings = Vec::new();
        if let Some(d) = &vm {
            findings.push(
                Finding::new(Domain::Docker, "docker:vm_disk", "Docker.raw virtual disk")
                    .at(d.path.clone())
                    .detail(format!(
                        "Docker Desktop VM image{}. Start Docker to reclaim space inside it.",
                        if d.is_sparse { " (sparse)" } else { "" }
                    ))
                    .sized(d.real_bytes, 0, 1)
                    .risk(Risk::Advisory),
            );
        }
        return (report, findings);
    };
    parse_df(&df, vm.as_ref())
}

/// Delete an individual image by ID or tag.
pub fn delete_image(target: &str) -> Result<u64, String> {
    let sock = socket_path();
    let url = format!("{API}/images/{target}");
    let _ = request(Endpoint::Unix(&sock), "DELETE", &url, None).map_err(|e| e.to_string())?;
    Ok(0)
}

/// Delete an individual container by ID.
pub fn delete_container(target: &str) -> Result<(), String> {
    let sock = socket_path();
    let url = format!("{API}/containers/{target}?v=1");
    let _ = request(Endpoint::Unix(&sock), "DELETE", &url, None).map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete an individual volume by name.
pub fn delete_volume(target: &str) -> Result<(), String> {
    let sock = socket_path();
    let url = format!("{API}/volumes/{target}");
    let _ = request(Endpoint::Unix(&sock), "DELETE", &url, None).map_err(|e| e.to_string())?;
    Ok(())
}

/// Run one bulk prune action. Returns bytes the daemon reports as freed.
pub fn prune(action: &str) -> Result<u64, String> {
    let p = PRUNABLES
        .iter()
        .find(|p| p.id == action || p.endpoint.contains(action))
        .ok_or_else(|| format!("unknown docker action '{action}'"))?;
    let sock = socket_path();
    let url = format!("{API}{}{}", p.endpoint, p.query);
    let body =
        request(Endpoint::Unix(&sock), "POST", &url, Some("{}")).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    Ok(v.get("SpaceReclaimed")
        .and_then(|s| s.as_i64())
        .unwrap_or(0)
        .max(0) as u64)
}

/// Is this path Docker's VM disk? The reaper must never delete it directly —
/// that destroys every image, container and volume at once.
pub fn is_vm_disk(p: &Path) -> bool {
    p.file_name().map(|n| n == "Docker.raw").unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_disk_is_recognised_and_protected() {
        assert!(is_vm_disk(Path::new("/x/vms/0/data/Docker.raw")));
        assert!(!is_vm_disk(Path::new("/x/vms/0/data/other.raw")));
    }

    #[test]
    fn prune_rejects_unknown_actions() {
        assert!(prune("rm -rf /").is_err());
        assert!(prune("prune_everything").is_err());
    }

    #[test]
    fn image_prune_filter_excludes_in_use_images() {
        // The query must never be a bare prune-all: dangling=false limits removal
        // to images no container references.
        let p = PRUNABLES.iter().find(|p| p.id == "docker:images").unwrap();
        assert!(p.query.contains("dangling"));
    }

    #[test]
    fn parse_df_extracts_detailed_docker_items() {
        let json = serde_json::json!({
            "Containers": [
                {
                    "Id": "c1_active_1234567890",
                    "Names": ["/my_active_app"],
                    "Image": "my_image:v1",
                    "State": "running",
                    "Status": "Up 2 hours",
                    "SizeRw": 1024,
                    "SizeRootFs": 2048,
                    "Created": 1700000000,
                    "Mounts": [{"Name": "my_data_vol", "Type": "volume"}]
                },
                {
                    "Id": "c2_stopped_1234567890",
                    "Names": ["/my_stopped_app"],
                    "Image": "old_image:v1",
                    "State": "exited",
                    "Status": "Exited (0) 2 days ago",
                    "SizeRw": 512,
                    "SizeRootFs": 1024,
                    "Created": 1700000000,
                    "Mounts": []
                }
            ],
            "Volumes": [
                {
                    "Name": "my_data_vol",
                    "Driver": "local",
                    "Mountpoint": "/var/lib/docker/volumes/my_data_vol/_data",
                    "UsageData": {"Size": 10000000, "RefCount": 1}
                },
                {
                    "Name": "dangling_vol",
                    "Driver": "local",
                    "Mountpoint": "/var/lib/docker/volumes/dangling_vol/_data",
                    "UsageData": {"Size": 5000000, "RefCount": 0}
                }
            ],
            "Images": [
                {
                    "Id": "sha256:img111111111111",
                    "RepoTags": ["my_image:v1"],
                    "Size": 20000000,
                    "SharedSize": 5000000,
                    "Containers": 1,
                    "Created": 1700000000
                },
                {
                    "Id": "sha256:img222222222222",
                    "RepoTags": ["unused_image:latest"],
                    "RepoDigests": ["unused_image@sha256:794285b5467e452a265b53f6562d9fb031fc0f8cf83a30eb6a0a75f284e3eab3"],
                    "Size": 30000000,
                    "SharedSize": 10000000,
                    "Containers": 0,
                    "Created": 1700000000
                },
                {
                    "Id": "sha256:img333333333333",
                    "RepoTags": ["<none>:<none>"],
                    "Size": 8000000,
                    "SharedSize": 0,
                    "Containers": 0,
                    "Created": 1700000000
                }
            ],
            "BuildCache": [
                {
                    "ID": "bc1",
                    "Size": 15000000,
                    "InUse": false
                }
            ]
        });

        let (report, findings) = parse_df(&json, None);
        assert!(report.daemon_running);
        assert_eq!(findings.len(), 8);

        // 1. Active container is Advisory with attached volume
        let active_c = findings
            .iter()
            .find(|f| f.id == "docker:container:c1_active_12")
            .unwrap();
        assert_eq!(active_c.risk, Risk::Advisory);
        assert!(active_c.name.contains("my_active_app"));
        assert!(active_c.detail.contains("my_data_vol"));

        // 2. Stopped container is Caution
        let stopped_c = findings
            .iter()
            .find(|f| f.id == "docker:container:c2_stopped_1")
            .unwrap();
        assert_eq!(stopped_c.risk, Risk::Caution);
        assert!(stopped_c.name.contains("Stopped: my_stopped_app"));

        // 3. Attached volume is Advisory because it is used by active container
        let att_vol = findings
            .iter()
            .find(|f| f.id == "docker:volume:my_data_vol")
            .unwrap();
        assert_eq!(att_vol.risk, Risk::Advisory);
        assert!(att_vol.detail.contains("my_active_app"));

        // 4. Dangling volume is Caution
        let dang_vol = findings
            .iter()
            .find(|f| f.id == "docker:volume:dangling_vol")
            .unwrap();
        assert_eq!(dang_vol.risk, Risk::Caution);

        // 5. In-use image (built on device, no RepoDigests) is Advisory
        let in_use_img = findings
            .iter()
            .find(|f| f.id == "docker:image:img111111111")
            .unwrap();
        assert_eq!(in_use_img.risk, Risk::Advisory);
        let m1 = in_use_img.meta.as_ref().unwrap();
        assert_eq!(m1["origin"], "local_build");
        assert_eq!(m1["is_pullable"], false);

        // 6. Unused pullable image is Caution with delete_image action
        let unused_img = findings
            .iter()
            .find(|f| f.id == "docker:image:img222222222")
            .unwrap();
        assert_eq!(unused_img.risk, Risk::Caution);
        assert_eq!(unused_img.name, "unused_image:latest");
        let m2 = unused_img.meta.as_ref().unwrap();
        assert_eq!(m2["origin"], "registry");
        assert_eq!(m2["is_pullable"], true);
        assert_eq!(
            unused_img.restore.as_deref(),
            Some("docker pull unused_image:latest")
        );

        // 7. Dangling untagged build image
        let untagged_img = findings
            .iter()
            .find(|f| f.id == "docker:image:img333333333")
            .unwrap();
        assert_eq!(untagged_img.risk, Risk::Caution);
        assert_eq!(untagged_img.name, "Untagged (img333333333)");
        let m3 = untagged_img.meta.as_ref().unwrap();
        assert_eq!(m3["origin"], "dangling_build");
        assert_eq!(m3["is_pullable"], false);

        // 8. Build cache is Safe
        let bc = findings
            .iter()
            .find(|f| f.id == "docker:build_cache")
            .unwrap();
        assert_eq!(bc.risk, Risk::Safe);

        // Total reclaimable only sums Caution and Safe items
        assert_eq!(
            report.total_reclaimable,
            512 + 5000000 + 20000000 + 8000000 + 15000000
        );
    }
}
