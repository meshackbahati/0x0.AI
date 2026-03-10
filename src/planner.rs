use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::categories::{ChallengeCategory, plan_for};
use crate::policy::{Approvals, PolicyEngine};
use crate::providers::{ProviderManager, ProviderRequest, TaskType};
use crate::storage::{NewAction, StateStore};
use crate::tools::{ToolManager, ToolRunRequest};
use crate::util::shell_preview;

#[derive(Debug, Clone)]
pub struct PlannerOptions {
    pub max_steps: usize,
    pub web_enabled: bool,
    pub approvals: Approvals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveOutcome {
    pub session_id: String,
    pub category: String,
    pub steps_executed: usize,
    pub candidate_flags: Vec<String>,
    pub summary: String,
    pub blocked: bool,
}

#[derive(Debug, Clone)]
struct PlannedAction {
    action_type: String,
    program: String,
    args: Vec<String>,
    requires_network: bool,
    host: Option<String>,
    port: Option<u16>,
}

pub fn solve_loop(
    session_id: &str,
    target_path: &Path,
    category: ChallengeCategory,
    options: PlannerOptions,
    policy: &PolicyEngine,
    store: &StateStore,
    tools: &ToolManager,
    providers: &ProviderManager,
) -> Result<SolveOutcome> {
    let plan = plan_for(category);

    for hypothesis in &plan.hypotheses {
        store.add_hypothesis(
            session_id,
            &hypothesis.text,
            hypothesis.confidence,
            "open",
        )?;
    }

    let mut flags = HashSet::new();
    let mut executed = 0usize;
    let mut blocked = false;
    let mut fail_counts: HashMap<String, usize> = HashMap::new();

    let actions = build_actions(category, target_path, options.max_steps);

    for action in actions {
        if executed >= options.max_steps {
            break;
        }

        if action.requires_network {
            if !options.web_enabled {
                let meta = json!({"reason": "web disabled"});
                store.add_action(NewAction {
                    session_id,
                    action_type: &action.action_type,
                    command: &shell_preview(&action.program, &action.args),
                    target: action.host.as_deref(),
                    status: "skipped",
                    stdout: None,
                    stderr: Some("network action skipped: web disabled"),
                    metadata: Some(&meta),
                })?;
                continue;
            }
            if let Some(host) = action.host.as_deref() {
                if policy
                    .ensure_network_allowed(options.approvals, host, action.port, false)
                    .is_err()
                {
                    blocked = true;
                    store.add_action(NewAction {
                        session_id,
                        action_type: &action.action_type,
                        command: &shell_preview(&action.program, &action.args),
                        target: Some(host),
                        status: "blocked",
                        stdout: None,
                        stderr: Some("blocked by policy"),
                        metadata: None,
                    })?;
                    continue;
                }
            }
        }

        if let Err(err) = policy.ensure_exec_allowed(options.approvals, &action.program) {
            blocked = true;
            let err_text = err.to_string();
            store.add_action(NewAction {
                session_id,
                action_type: &action.action_type,
                command: &shell_preview(&action.program, &action.args),
                target: action.host.as_deref(),
                status: "blocked",
                stdout: None,
                stderr: Some(&err_text),
                metadata: None,
            })?;
            continue;
        }

        let preview = shell_preview(&action.program, &action.args);
        if fail_counts.get(&preview).copied().unwrap_or(0) >= 1 {
            store.add_action(NewAction {
                session_id,
                action_type: &action.action_type,
                command: &preview,
                target: action.host.as_deref(),
                status: "skipped",
                stdout: None,
                stderr: Some("skipped repeated failed action"),
                metadata: None,
            })?;
            continue;
        }

        let run = tools.run(ToolRunRequest {
            program: action.program.clone(),
            args: action.args.clone(),
            cwd: Some(target_path.to_path_buf()),
            timeout_secs: None,
        });

        match run {
            Ok(res) => {
                executed += 1;
                if res.status != "ok" {
                    *fail_counts.entry(preview.clone()).or_default() += 1;
                }

                for f in extract_candidate_flags(&format!("{}\n{}", res.stdout, res.stderr)) {
                    flags.insert(f);
                }

                let meta = json!({
                    "exit_code": res.exit_code,
                    "duration_ms": res.duration_ms,
                    "timed_out": res.timed_out,
                    "dry_run": res.dry_run,
                });
                store.add_action(NewAction {
                    session_id,
                    action_type: &action.action_type,
                    command: &res.command_preview,
                    target: action.host.as_deref(),
                    status: &res.status,
                    stdout: Some(&res.stdout),
                    stderr: Some(&res.stderr),
                    metadata: Some(&meta),
                })?;
            }
            Err(err) => {
                *fail_counts.entry(preview.clone()).or_default() += 1;
                let err_text = err.to_string();
                store.add_action(NewAction {
                    session_id,
                    action_type: &action.action_type,
                    command: &preview,
                    target: action.host.as_deref(),
                    status: "error",
                    stdout: None,
                    stderr: Some(&err_text),
                    metadata: None,
                })?;
            }
        }
    }

    let action_tail = store
        .list_actions(session_id, 10)?
        .into_iter()
        .map(|a| {
            format!(
                "[{}] {} => {}",
                a.status,
                a.command,
                a.stdout.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let summary_prompt = format!(
        "Category: {}\nExecuted steps: {}\nCandidate flags: {:?}\nRecent actions:\n{}\n\nSummarize key findings and next deterministic steps.",
        category.as_str(),
        executed,
        flags,
        action_tail
    );

    let summary = providers
        .call(
            ProviderRequest {
                system: Some("You are a CTF analyst. Keep output concise, reproducible, and safety-compliant.".to_string()),
                prompt: summary_prompt,
                task_type: TaskType::Summarization,
                max_tokens: 400,
                temperature: 0.2,
                timeout_secs: 45,
                model_override: None,
            },
            None,
        )
        .map(|r| r.text)
        .unwrap_or_else(|_| {
            "Summary unavailable from provider. Review replay and hypotheses for next deterministic action."
                .to_string()
        });

    Ok(SolveOutcome {
        session_id: session_id.to_string(),
        category: category.as_str().to_string(),
        steps_executed: executed,
        candidate_flags: flags.into_iter().collect(),
        summary,
        blocked,
    })
}

fn build_actions(category: ChallengeCategory, target: &Path, max: usize) -> Vec<PlannedAction> {
    let t = target.display().to_string();
    let mut actions = Vec::new();

    match category {
        ChallengeCategory::Crypto => {
            actions.push(planned("triage", "rg", vec!["-n", "(n|e|c|p|q)", &t]));
            actions.push(planned("triage", "file", vec![&t]));
        }
        ChallengeCategory::Pwn => {
            actions.push(planned("triage", "file", vec![&t]));
            actions.push(planned("triage", "checksec", vec!["--file", &t]));
            actions.push(planned("triage", "readelf", vec!["-h", &t]));
            actions.push(planned("extract", "strings", vec!["-n", "4", &t]));
        }
        ChallengeCategory::Reverse => {
            actions.push(planned("triage", "file", vec![&t]));
            actions.push(planned("extract", "strings", vec!["-n", "4", &t]));
            actions.push(planned("triage", "objdump", vec!["-x", &t]));
        }
        ChallengeCategory::Forensics | ChallengeCategory::Stego => {
            actions.push(planned("triage", "file", vec![&t]));
            actions.push(planned("metadata", "exiftool", vec![&t]));
            actions.push(planned("carve", "binwalk", vec![&t]));
        }
        ChallengeCategory::Web => {
            actions.push(PlannedAction {
                action_type: "http-map".to_string(),
                program: "curl".to_string(),
                args: vec!["-i".to_string(), t],
                requires_network: true,
                host: extract_host(target),
                port: extract_port(target),
            });
        }
        ChallengeCategory::Misc | ChallengeCategory::Osint | ChallengeCategory::Unknown => {
            actions.push(planned("triage", "file", vec![&t]));
            actions.push(planned("grep", "rg", vec!["-n", "flag|ctf|secret|password", &t]));
        }
    }

    actions.truncate(max);
    actions
}

fn planned(action_type: &str, program: &str, args: Vec<&str>) -> PlannedAction {
    PlannedAction {
        action_type: action_type.to_string(),
        program: program.to_string(),
        args: args.into_iter().map(ToString::to_string).collect(),
        requires_network: false,
        host: None,
        port: None,
    }
}

fn extract_candidate_flags(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?i)([a-z0-9_\-]{2,16}\{[^\n\r\}]{1,180}\})").expect("regex");
    let mut out = HashSet::new();
    for cap in re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            out.insert(m.as_str().to_string());
        }
    }
    out.into_iter().collect()
}

fn extract_host(target: &Path) -> Option<String> {
    let s = target.display().to_string();
    if let Ok(url) = url::Url::parse(&s) {
        return url.host_str().map(ToString::to_string);
    }
    None
}

fn extract_port(target: &Path) -> Option<u16> {
    let s = target.display().to_string();
    if let Ok(url) = url::Url::parse(&s) {
        return url.port_or_known_default();
    }
    None
}
