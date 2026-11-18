//! Multi-purpose search engine for developer files, folders, applications, and file content.
//!
//! Provides ultra-fast filename, directory, application, and in-file content search
//! across the macOS filesystem with zero subprocesses.

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Target scope for filtering search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    All,
    Files,
    Folders,
    Apps,
    Content,
    DevStorage,
}

/// Metadata describing an installed macOS application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub path: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub bytes: u64,
}

/// Match found within the body of a text file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentMatch {
    pub file_path: String,
    pub file_name: String,
    pub line_number: usize,
    pub line_content: String,
}

/// Normalized search result item presented in the OmniSearch UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub kind: String, // "app" | "folder" | "file" | "content" | "dev_storage"
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub detail: Option<String>,
    pub line_number: Option<usize>,
    pub snippet: Option<String>,
}

/// Parse an application bundle's `Contents/Info.plist` to extract bundle id and version.
pub fn parse_app_bundle(app_path: &Path) -> Option<AppInfo> {
    if !app_path.is_dir() {
        return None;
    }
    let ext = app_path.extension()?.to_str()?;
    if ext != "app" {
        return None;
    }

    let file_stem = app_path.file_stem()?.to_string_lossy().to_string();
    let info_plist = app_path.join("Contents/Info.plist");

    let mut bundle_id = None;
    let mut version = None;
    let mut display_name = file_stem.clone();

    if let Ok(plist_val) = plist::Value::from_file(&info_plist) {
        if let Some(dict) = plist_val.as_dictionary() {
            if let Some(id) = dict.get("CFBundleIdentifier").and_then(|v| v.as_string()) {
                bundle_id = Some(id.to_string());
            }
            if let Some(ver) = dict
                .get("CFBundleShortVersionString")
                .or_else(|| dict.get("CFBundleVersion"))
                .and_then(|v| v.as_string())
            {
                version = Some(ver.to_string());
            }
            if let Some(name) = dict
                .get("CFBundleDisplayName")
                .or_else(|| dict.get("CFBundleName"))
                .and_then(|v| v.as_string())
            {
                if !name.trim().is_empty() {
                    display_name = name.to_string();
                }
            }
        }
    }

    let bytes = fs::metadata(app_path).map(|m| m.len()).unwrap_or(0);

    Some(AppInfo {
        name: display_name,
        path: app_path.to_string_lossy().to_string(),
        bundle_id,
        version,
        bytes,
    })
}

/// Discover installed applications in standard macOS locations.
pub fn discover_apps(home: Option<&Path>) -> Vec<AppInfo> {
    let mut search_dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];

    if let Some(h) = home {
        let user_apps = h.join("Applications");
        if user_apps.is_dir() {
            search_dirs.push(user_apps);
        }
    }

    let apps = Mutex::new(Vec::new());

    search_dirs.par_iter().for_each(|dir| {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(app) = parse_app_bundle(&p) {
                    apps.lock().push(app);
                }
            }
        }
    });

    let mut result = apps.into_inner();
    result.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    result.dedup_by(|a, b| a.path == b.path);
    result
}

/// Search for textual content inside a file. Skips files larger than `max_bytes`
/// or files with null bytes (binary heuristics).
pub fn search_file_content(path: &Path, query: &str, max_bytes: u64) -> Vec<ContentMatch> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    if !meta.is_file() || meta.len() > max_bytes || meta.len() == 0 {
        return Vec::new();
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut matches = Vec::new();
    let query_lower = query.to_lowercase();
    let file_path_str = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    for (idx, line_res) in reader.lines().enumerate() {
        if idx >= 10_000 {
            break; // Cap lines examined per file to prevent hangs on minified files
        }
        let line = match line_res {
            Ok(l) => l,
            Err(_) => return Vec::new(), // Likely binary file encountered
        };

        if line.contains('\0') {
            return Vec::new(); // Binary file check
        }

        if line.to_lowercase().contains(&query_lower) {
            matches.push(ContentMatch {
                file_path: file_path_str.clone(),
                file_name: file_name.clone(),
                line_number: idx + 1,
                line_content: line.trim().to_string(),
            });
            if matches.len() >= 25 {
                break;
            }
        }
    }

    matches
}

/// Check if a file extension corresponds to a searchable plain-text or source file.
pub fn is_text_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "html"
            | "css"
            | "scss"
            | "svelte"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "go"
            | "sh"
            | "bash"
            | "zsh"
            | "sql"
            | "xml"
            | "csv"
            | "env"
            | "conf"
            | "ini"
            | "swift"
            | "kt"
            | "java"
    )
}

/// Search for textual content across multiple files concurrently, stopping early
/// once `max_results` matches are found.
pub fn search_files_content(
    paths: &[PathBuf],
    query: &str,
    max_results: usize,
) -> Vec<ContentMatch> {
    if query.trim().is_empty() || paths.is_empty() || max_results == 0 {
        return Vec::new();
    }

    use std::sync::atomic::{AtomicBool, Ordering};
    let done = AtomicBool::new(false);
    let results = Mutex::new(Vec::new());

    paths.par_iter().for_each(|p| {
        if done.load(Ordering::Relaxed) {
            return;
        }

        let file_matches = search_file_content(p, query, 500 * 1024);
        if !file_matches.is_empty() {
            let mut guard = results.lock();
            guard.extend(file_matches);
            if guard.len() >= max_results {
                done.store(true, Ordering::Relaxed);
            }
        }
    });

    let mut res = results.into_inner();
    if res.len() > max_results {
        res.truncate(max_results);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parses_app_bundle_correctly() {
        let dir = tempdir().unwrap();
        let app_dir = dir.path().join("DemoApp.app");
        let contents_dir = app_dir.join("Contents");
        fs::create_dir_all(&contents_dir).unwrap();

        let plist_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.demo.app</string>
    <key>CFBundleShortVersionString</key>
    <string>1.2.3</string>
    <key>CFBundleDisplayName</key>
    <string>Demo Application</string>
</dict>
</plist>"#;
        fs::write(contents_dir.join("Info.plist"), plist_content).unwrap();

        let app = parse_app_bundle(&app_dir).expect("should parse .app bundle");
        assert_eq!(app.name, "Demo Application");
        assert_eq!(app.bundle_id.as_deref(), Some("com.demo.app"));
        assert_eq!(app.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn search_file_content_finds_lines() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("sample.rs");
        let content = "fn main() {\n    let answer = 42;\n    println!(\"Hello World\");\n}\n";
        fs::write(&file_path, content).unwrap();

        let matches = search_file_content(&file_path, "answer", 1024 * 1024);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_number, 2);
        assert_eq!(matches[0].line_content, "let answer = 42;");
    }

    #[test]
    fn search_file_content_skips_binary_data() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("binary.bin");
        let binary_data = b"Some text with \0 null byte inside";
        fs::write(&file_path, binary_data).unwrap();

        let matches = search_file_content(&file_path, "text", 1024 * 1024);
        assert!(matches.is_empty());
    }

    #[test]
    fn text_extension_classification() {
        assert!(is_text_extension("rs"));
        assert!(is_text_extension("py"));
        assert!(is_text_extension("TXT"));
        assert!(is_text_extension("json"));
        assert!(is_text_extension("svelte"));
        assert!(!is_text_extension("png"));
        assert!(!is_text_extension("dmg"));
        assert!(!is_text_extension("exe"));
    }

    #[test]
    fn search_files_content_single_char_and_limit() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("a.txt");
        let file2 = dir.path().join("b.txt");

        fs::write(&file1, "apple\nbanana\navocado\n").unwrap();
        fs::write(&file2, "almond\napricot\nartichoke\n").unwrap();

        let paths = vec![file1, file2];

        // Search single letter "a" with limit 3
        let matches = search_files_content(&paths, "a", 3);
        assert_eq!(matches.len(), 3);

        // Search specific word
        let matches_banana = search_files_content(&paths, "banana", 10);
        assert_eq!(matches_banana.len(), 1);
        assert_eq!(matches_banana[0].line_content, "banana");
    }
}
