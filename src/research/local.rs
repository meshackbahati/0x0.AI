use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::research::{Citation, ResearchHit};
use crate::storage::StateStore;

pub fn search_local(
    query: &str,
    root: &Path,
    store: &StateStore,
    session_id: Option<&str>,
    limit: usize,
) -> Result<Vec<ResearchHit>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let q = query.to_ascii_lowercase();

    for note in store.search_local_notes(query, limit)? {
        let snippet = truncate(&note.note, 220);
        let key = format!("note:{}", note.id);
        if seen.insert(key) {
            out.push(ResearchHit {
                title: Some(format!("Note {}", note.id)),
                snippet: snippet.clone(),
                citation: Citation {
                    source_type: "note".to_string(),
                    source: format!("session:{}", note.session_id),
                    locator: Some(format!("note-id:{}", note.id)),
                    snippet,
                },
            });
        }
        if out.len() >= limit {
            return Ok(out);
        }
    }

    if let Some(sid) = session_id {
        for artifact in store.list_artifacts(sid, limit * 8)? {
            let hay = format!(
                "{} {} {}",
                artifact.path,
                artifact.kind,
                artifact.summary.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase();
            if hay.contains(&q) {
                let snippet = artifact
                    .summary
                    .clone()
                    .unwrap_or_else(|| format!("{} ({})", artifact.path, artifact.kind));
                let key = format!("artifact:{}", artifact.path);
                if seen.insert(key) {
                    out.push(ResearchHit {
                        title: Some(format!("Artifact {}", artifact.kind)),
                        snippet: truncate(&snippet, 220),
                        citation: Citation {
                            source_type: "artifact".to_string(),
                            source: artifact.path.clone(),
                            locator: None,
                            snippet: truncate(&snippet, 220),
                        },
                    });
                }
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
    }

    let max_files = 400usize;
    let mut scanned = 0usize;

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if out.len() >= limit || scanned > max_files {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        scanned += 1;

        let path = entry.path();
        if !is_textish(path) {
            continue;
        }

        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };

        for (idx, line) in content.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&q) {
                let snippet = truncate(line.trim(), 220);
                let key = format!("{}:{}", path.display(), idx + 1);
                if seen.insert(key) {
                    out.push(ResearchHit {
                        title: Some(path.file_name().unwrap_or_default().to_string_lossy().to_string()),
                        snippet: snippet.clone(),
                        citation: Citation {
                            source_type: "file".to_string(),
                            source: path.display().to_string(),
                            locator: Some(format!("line:{}", idx + 1)),
                            snippet,
                        },
                    });
                }
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
    }

    Ok(out)
}

fn is_textish(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "json"
            | "yaml"
            | "yml"
            | "csv"
            | "toml"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "go"
            | "java"
            | "php"
            | "sh"
            | "sql"
            | "log"
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
