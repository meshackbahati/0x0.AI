use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::categories::{ArtifactSignal, ChallengeCategory, infer_category};
use crate::storage::{ArtifactRecord, StateStore};
use crate::util::{hash_file_sha256, now_utc_rfc3339, read_text_prefix};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    pub recursive: bool,
    pub max_read_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub root: String,
    pub session_id: String,
    pub total_files_seen: usize,
    pub indexed_files: usize,
    pub detected_category: ChallengeCategory,
    pub artifacts: Vec<ArtifactRecord>,
}

pub fn scan_path(
    root: &Path,
    session_id: &str,
    options: &ScanOptions,
    store: &StateStore,
) -> Result<ScanReport> {
    let walker = if options.recursive {
        WalkDir::new(root)
    } else {
        WalkDir::new(root).max_depth(1)
    };

    let mut total = 0_usize;
    let mut indexed = 0_usize;
    let mut artifacts = Vec::new();
    let mut signals = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        total += 1;
        let path = entry.path().to_path_buf();
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let (kind, mime) = classify_file(&path);
        let sha256 = hash_file_sha256(&path).ok();
        let summary = summarize_file(&path, &kind, options.max_read_bytes).ok();

        let record = ArtifactRecord {
            session_id: session_id.to_string(),
            path: path.display().to_string(),
            kind: kind.clone(),
            size: meta.len(),
            sha256,
            mime,
            indexed_at: now_utc_rfc3339(),
            summary: summary.clone(),
        };

        store.upsert_artifact(&record)?;

        signals.push(ArtifactSignal {
            path: record.path.clone(),
            kind: record.kind.clone(),
            size: record.size,
            summary: record.summary.clone(),
        });

        artifacts.push(record);
        indexed += 1;
    }

    let detected_category = infer_category(&signals);

    Ok(ScanReport {
        root: root.display().to_string(),
        session_id: session_id.to_string(),
        total_files_seen: total,
        indexed_files: indexed,
        detected_category,
        artifacts,
    })
}

pub fn summarize_file(path: &Path, kind: &str, max_bytes: usize) -> Result<String> {
    if kind == "text" || kind == "source" || kind == "json" || kind == "yaml" || kind == "csv" {
        let text = read_text_prefix(path, max_bytes.min(64 * 1024))?;
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        return Ok(truncate(line, 240));
    }

    if kind == "pdf" {
        return Ok("PDF artifact (extract with pdftotext if installed)".to_string());
    }

    if kind == "binary" {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.starts_with(&[0x7F, b'E', b'L', b'F']) {
            return Ok("ELF binary".to_string());
        }
        return Ok("Binary artifact".to_string());
    }

    Ok(format!("{} artifact", kind))
}

pub fn classify_file(path: &Path) -> (String, Option<String>) {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let kind = match ext.as_str() {
        "txt" | "md" | "rst" => "text",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "csv" | "tsv" => "csv",
        "c" | "cc" | "cpp" | "h" | "hpp" | "rs" | "py" | "js" | "ts" | "go" | "java" | "php"
        | "rb" | "sh" => "source",
        "zip" | "tar" | "gz" | "xz" | "7z" | "rar" => "archive",
        "pcap" | "pcapng" => "pcap",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => "image",
        "wav" | "mp3" | "ogg" | "flac" => "audio",
        "pdf" => "pdf",
        "elf" | "exe" | "dll" | "so" | "bin" => "binary",
        "sql" => "source",
        _ => {
            if is_probably_text(path) {
                "text"
            } else {
                "binary"
            }
        }
    }
    .to_string();

    let mime = Some(
        match kind.as_str() {
            "text" | "source" | "json" | "yaml" | "csv" => "text/plain",
            "archive" => "application/octet-stream",
            "pcap" => "application/vnd.tcpdump.pcap",
            "image" => "image/*",
            "audio" => "audio/*",
            "pdf" => "application/pdf",
            _ => "application/octet-stream",
        }
        .to_string(),
    );

    (kind, mime)
}

fn is_probably_text(path: &Path) -> bool {
    let Ok(data) = fs::read(path) else {
        return false;
    };

    let take = data.len().min(512);
    if take == 0 {
        return true;
    }

    data[..take]
        .iter()
        .all(|b| b.is_ascii() || matches!(b, b'\n' | b'\r' | b'\t'))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

pub fn collect_signals_from_artifacts(artifacts: &[ArtifactRecord]) -> Vec<ArtifactSignal> {
    artifacts
        .iter()
        .map(|a| ArtifactSignal {
            path: a.path.clone(),
            kind: a.kind.clone(),
            size: a.size,
            summary: a.summary.clone(),
        })
        .collect()
}

pub fn path_is_within(root: &Path, child: &Path) -> bool {
    let r = canonicalize_lossy(root);
    let c = canonicalize_lossy(child);
    c.starts_with(r)
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
