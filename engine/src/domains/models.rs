//! AI/ML model caches — port of `core/models_manager.py`.
//!
//! **Δ: Ollama via its HTTP API** (`GET /api/tags`, `DELETE /api/delete`) instead
//! of parsing `ollama list` stdout. The prototype reconstructs sizes from
//! `parts[2] + parts[3]` of a column-split display string (`"2.2 GB"` → float
//! arithmetic); the API returns exact bytes. A manifest walk remains as the
//! fallback for when the daemon is down.
//!
//! **Δ: Hugging Face `blobs/` are hardlinked from `snapshots/`**, so sizing them
//! naively double-counts every model. [`crate::dedup`] handles it.

use super::{Domain, Finding, Risk};
use crate::http::{request, Endpoint};
use crate::paths;
use std::path::PathBuf;

const OLLAMA_ADDR: &str = "127.0.0.1:11434";

/// Other ML caches, ported verbatim from the prototype's `ml_dirs`.
pub const ML_CACHES: &[(&str, &str, &str)] = &[
    ("insightface", "InsightFace weights", "~/.insightface"),
    ("deepface", "DeepFace checkpoints", "~/.deepface"),
    ("keras", "Keras model cache", "~/.keras"),
    ("easyocr", "EasyOCR weights", "~/.EasyOCR"),
    ("pytorch", "PyTorch Hub cache", "~/.cache/torch"),
    ("whisper", "Whisper audio models", "~/.cache/whisper"),
    ("nltk", "NLTK language datasets", "~/nltk_data"),
    ("stanza", "Stanza CoreNLP resources", "~/stanza_resources"),
    (
        "scikit-learn",
        "scikit-learn datasets",
        "~/scikit_learn_data",
    ),
];

pub fn huggingface_hub() -> PathBuf {
    if let Ok(v) = std::env::var("HF_HOME") {
        let p = PathBuf::from(v).join("hub");
        if p.is_dir() {
            return p;
        }
    }
    paths::resolve("~/.cache/huggingface/hub", None)
}

/// One cached Hugging Face repo, from a `models--<org>--<repo>` directory.
pub struct HfModel {
    pub repo_id: String,
    pub path: PathBuf,
    pub snapshots: usize,
}

/// Decode `models--org--repo` → `org/repo`.
pub fn decode_hf_folder(folder: &str) -> Option<String> {
    let rest = folder.strip_prefix("models--")?;
    Some(rest.replace("--", "/"))
}

pub fn list_hf_models() -> Vec<HfModel> {
    let hub = huggingface_hub();
    let Ok(rd) = std::fs::read_dir(&hub) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let folder = e.file_name().to_string_lossy().to_string();
        if !folder.starts_with("models--") {
            continue;
        }
        let Some(repo_id) = decode_hf_folder(&folder) else {
            continue;
        };
        let snapshots = std::fs::read_dir(e.path().join("snapshots"))
            .map(|r| {
                r.flatten()
                    .filter(|s| !s.file_name().to_string_lossy().starts_with('.'))
                    .count()
            })
            .unwrap_or(0);
        out.push(HfModel {
            repo_id,
            path: e.path(),
            snapshots,
        });
    }
    out
}

pub fn hf_finding(m: &HfModel, bytes: u64, shared: u64, files: u64, mtime: i64) -> Finding {
    Finding::new(
        Domain::AiModel,
        format!("hf:{}", m.repo_id),
        m.repo_id.clone(),
    )
    .at(m.path.clone())
    .detail(format!(
        "Hugging Face · {} snapshot{}",
        m.snapshots,
        if m.snapshots == 1 { "" } else { "s" }
    ))
    .sized(bytes, shared, files)
    .aged(mtime)
    .risk(Risk::Caution)
    .restore(format!(
        "huggingface-cli download {} # or re-run your script",
        m.repo_id
    ))
    .meta(serde_json::json!({ "source": "huggingface", "snapshots": m.snapshots }))
}

/// An Ollama model as reported by `GET /api/tags`.
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub digest: String,
    pub modified: String,
}

/// Query the Ollama daemon. Exact byte sizes, no stdout parsing.
pub fn list_ollama_models() -> Option<Vec<OllamaModel>> {
    let body = request(Endpoint::Tcp(OLLAMA_ADDR), "GET", "/api/tags", None).ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let arr = v.get("models")?.as_array()?;
    Some(
        arr.iter()
            .map(|m| OllamaModel {
                name: m
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                size: m.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                digest: m
                    .get("digest")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(12)
                    .collect(),
                modified: m
                    .get("modified_at")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(10)
                    .collect(),
            })
            .collect(),
    )
}

/// Delete an Ollama model through the API.
pub fn delete_ollama_model(name: &str) -> Result<(), String> {
    let payload = serde_json::json!({ "model": name }).to_string();
    request(
        Endpoint::Tcp(OLLAMA_ADDR),
        "DELETE",
        "/api/delete",
        Some(&payload),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn ollama_finding(m: &OllamaModel) -> Finding {
    Finding::new(
        Domain::AiModel,
        format!("ollama:{}", m.name),
        m.name.clone(),
    )
    .detail(format!("Ollama · {} · {}", m.digest, m.modified))
    .sized(m.size, 0, 0)
    .risk(Risk::Caution)
    .restore(format!("ollama pull {}", m.name))
    .meta(serde_json::json!({
        "source": "ollama",
        "action": "delete_ollama",
        "model": m.name,
    }))
}

/// Ollama blob directory, for the daemon-down fallback.
pub fn ollama_blobs() -> PathBuf {
    paths::resolve("~/.ollama/models/blobs", None)
}

/// LM Studio models live at `~/.lmstudio/models/<publisher>/<model>`.
pub fn lmstudio_models() -> Vec<(String, PathBuf)> {
    let root = paths::resolve("~/.lmstudio/models", None);
    let mut out = Vec::new();
    let Ok(pubs) = std::fs::read_dir(&root) else {
        return out;
    };
    for p in pubs.flatten() {
        let pub_name = p.file_name().to_string_lossy().to_string();
        if pub_name.starts_with('.') || !p.path().is_dir() {
            continue;
        }
        if let Ok(models) = std::fs::read_dir(p.path()) {
            for m in models.flatten() {
                let m_name = m.file_name().to_string_lossy().to_string();
                if m_name.starts_with('.') {
                    continue;
                }
                out.push((format!("{pub_name}/{m_name}"), m.path()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hf_folder_decoding_matches_prototype() {
        assert_eq!(
            decode_hf_folder("models--meta-llama--Llama-3-8B"),
            Some("meta-llama/Llama-3-8B".into())
        );
        assert_eq!(
            decode_hf_folder("models--bert-base-uncased"),
            Some("bert-base-uncased".into())
        );
        assert_eq!(decode_hf_folder("datasets--squad"), None);
    }

    #[test]
    fn every_prototype_ml_cache_is_present() {
        for src in [
            "insightface",
            "deepface",
            "keras",
            "easyocr",
            "pytorch",
            "whisper",
            "nltk",
            "stanza",
            "scikit-learn",
        ] {
            assert!(ML_CACHES.iter().any(|(s, _, _)| *s == src), "missing {src}");
        }
    }

    #[test]
    fn models_are_never_marked_safe_to_auto_delete() {
        // A 7 GB model is a long re-download. It must never be pre-selected.
        let m = OllamaModel {
            name: "llama3:8b".into(),
            size: 4_700_000_000,
            digest: "abc123".into(),
            modified: "2026-01-01".into(),
        };
        assert_eq!(ollama_finding(&m).risk, Risk::Caution);
    }
}
