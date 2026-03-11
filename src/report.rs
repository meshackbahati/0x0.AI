use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::storage::{ActionRecord, SessionRecord, StateStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteupBundle {
    pub session: SessionRecord,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayBundle {
    pub session_id: String,
    pub actions: Vec<ActionRecord>,
}

pub fn build_writeup(store: &StateStore, session_id: &str) -> Result<WriteupBundle> {
    let session = store
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;

    let actions = store.list_actions(session_id, 500)?;
    let notes = store.list_notes(session_id, 200)?;
    let hypotheses = store.list_hypotheses(session_id, 100)?;
    let citations = store.list_citations(session_id, 200)?;
    let artifacts = store.list_artifacts(session_id, 200)?;

    let flags = extract_flags_from_actions(&actions);

    let mut md = String::new();
    md.push_str(&format!("# 0x0.AI Writeup: `{}`\n\n", session.id));
    md.push_str("## Session Metadata\n");
    md.push_str(&format!("- Created: {}\n", session.created_at));
    md.push_str(&format!("- Updated: {}\n", session.updated_at));
    md.push_str(&format!("- Status: {}\n", session.status));
    md.push_str(&format!("- Root Path: `{}`\n", session.root_path));
    md.push_str(&format!(
        "- Category: `{}`\n",
        session
            .category
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    ));
    if let Some(summary) = &session.summary {
        md.push_str(&format!("- Summary: {}\n", summary));
    }
    md.push('\n');

    md.push_str("## Candidate Flags\n");
    if flags.is_empty() {
        md.push_str("- None extracted\n\n");
    } else {
        for f in flags {
            md.push_str(&format!("- `{}`\n", f));
        }
        md.push('\n');
    }

    md.push_str("## Hypotheses\n");
    if hypotheses.is_empty() {
        md.push_str("- No hypotheses recorded\n\n");
    } else {
        for h in hypotheses {
            md.push_str(&format!(
                "- ({:.2}) [{}] {}\n",
                h.confidence, h.status, h.text
            ));
        }
        md.push('\n');
    }

    md.push_str("## Artifact Index\n");
    for a in artifacts.into_iter().take(100) {
        md.push_str(&format!("- `{}` ({}, {} bytes)\n", a.path, a.kind, a.size));
        if let Some(summary) = a.summary {
            md.push_str(&format!("  - summary: {}\n", summary));
        }
    }
    md.push('\n');

    md.push_str("## Action Replay\n");
    if actions.is_empty() {
        md.push_str("- No actions recorded\n\n");
    } else {
        for action in actions.iter().rev() {
            md.push_str(&format!("### {} [{}]\n", action.command, action.status));
            md.push_str(&format!("- Timestamp: {}\n", action.ts));
            md.push_str(&format!("- Type: {}\n", action.action_type));
            if let Some(target) = &action.target {
                md.push_str(&format!("- Target: {}\n", target));
            }
            if let Some(stdout) = &action.stdout
                && !stdout.trim().is_empty()
            {
                md.push_str("- Stdout excerpt:\n\n```text\n");
                md.push_str(&truncate(stdout, 800));
                md.push_str("\n```\n");
            }
            if let Some(stderr) = &action.stderr
                && !stderr.trim().is_empty()
            {
                md.push_str("- Stderr excerpt:\n\n```text\n");
                md.push_str(&truncate(stderr, 500));
                md.push_str("\n```\n");
            }
            md.push('\n');
        }
    }

    md.push_str("## Research Citations\n");
    if citations.is_empty() {
        md.push_str("- No citations recorded\n\n");
    } else {
        for c in citations {
            md.push_str(&format!(
                "- [{}] {} {}\n",
                c.source_type,
                c.source,
                c.locator.unwrap_or_default()
            ));
            md.push_str(&format!("  - {}\n", c.snippet));
        }
        md.push('\n');
    }

    md.push_str("## Analyst Notes\n");
    if notes.is_empty() {
        md.push_str("- No notes recorded\n");
    } else {
        for n in notes {
            md.push_str(&format!("- {}: {}\n", n.ts, n.note));
        }
    }

    Ok(WriteupBundle {
        session,
        markdown: md,
    })
}

pub fn write_writeup(path: &Path, markdown: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, markdown)?;
    Ok(())
}

pub fn build_replay(store: &StateStore, session_id: &str, limit: usize) -> Result<ReplayBundle> {
    let actions = store.list_actions(session_id, limit)?;
    Ok(ReplayBundle {
        session_id: session_id.to_string(),
        actions,
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

fn extract_flags_from_actions(actions: &[ActionRecord]) -> Vec<String> {
    let re = regex::Regex::new(r"(?i)([a-z0-9_\-]{2,16}\{[^\n\r\}]{1,180}\})").expect("flag regex");

    let mut out = BTreeSet::new();
    for a in actions {
        let text = format!(
            "{}\n{}",
            a.stdout.as_deref().unwrap_or(""),
            a.stderr.as_deref().unwrap_or("")
        );
        for cap in re.captures_iter(&text) {
            if let Some(m) = cap.get(1) {
                out.insert(m.as_str().to_string());
            }
        }
    }

    out.into_iter().collect()
}
