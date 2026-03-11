use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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

#[derive(Debug, Clone)]
struct GeneratedHelper {
    name: String,
    description: String,
    path: PathBuf,
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
        store.add_hypothesis(session_id, &hypothesis.text, hypothesis.confidence, "open")?;
    }
    if !plan.actions.is_empty() {
        let theory = plan
            .actions
            .iter()
            .map(|a| a.description.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = store.add_note(session_id, &format!("theory-checklist: {theory}"));
    }

    let available_tools = log_tool_visibility(session_id, store, tools)?;
    let generated_helpers = ensure_generated_helpers(target_path, category)
        .with_context(|| format!("generating helper tools in {}", target_path.display()))?;
    for helper in &generated_helpers {
        let _ = store.add_note(
            session_id,
            &format!(
                "generated-helper [{}]: {} ({})",
                helper.name,
                helper.path.display(),
                helper.description
            ),
        );
    }

    let mut flags = HashSet::new();
    let mut executed = 0usize;
    let mut blocked = false;
    let mut fail_counts: HashMap<String, usize> = HashMap::new();

    let reserved_steps = generated_helpers.len().min(options.max_steps);
    let mut actions = build_actions(
        category,
        target_path,
        &available_tools,
        options.max_steps.saturating_sub(reserved_steps),
    );
    for helper in generated_helpers.into_iter().rev() {
        actions.insert(
            0,
            planned_owned(
                "autotool",
                "python3",
                vec![
                    helper.path.display().to_string(),
                    target_path.display().to_string(),
                    category.as_str().to_string(),
                ],
            ),
        );
    }

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
        "Category: {}\nTheory cues: {}\nExecuted steps: {}\nCandidate flags: {:?}\nRecent actions:\n{}\n\nSummarize key findings and next deterministic steps.",
        category.as_str(),
        plan.hypotheses
            .iter()
            .map(|h| h.text.as_str())
            .collect::<Vec<_>>()
            .join(" | "),
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

fn build_actions(
    category: ChallengeCategory,
    target: &Path,
    available_tools: &HashSet<String>,
    max: usize,
) -> Vec<PlannedAction> {
    let mut actions = Vec::new();
    let target_arg = target.display().to_string();
    let discovered = discover_candidate_files(target, 300);
    let text_targets = select_targets(&discovered, is_text_or_source, 8);
    let code_targets = select_targets(&discovered, is_code_like, 8);
    let bin_targets = select_targets(&discovered, is_binary_like, 6);
    let image_targets = select_targets(&discovered, is_image_like, 6);
    let pcap_targets = select_targets(&discovered, is_pcap_like, 4);
    let mobile_targets = select_targets(&discovered, is_mobile_like, 4);
    let contract_targets = select_targets(&discovered, is_contract_like, 6);
    let infra_targets = select_targets(&discovered, is_infra_like, 6);
    let ai_targets = select_targets(&discovered, is_ai_like, 6);

    if has_tool(available_tools, "rg") {
        push_action(
            &mut actions,
            "observe",
            "rg",
            vec![
                "-n".to_string(),
                "flag|ctf|secret|password|token".to_string(),
                target_arg.clone(),
            ],
        );
    }
    for path in bin_targets.iter().take(2) {
        push_if_tool(
            &mut actions,
            available_tools,
            "triage",
            "file",
            vec![path.clone()],
        );
    }

    match category {
        ChallengeCategory::Crypto => {
            for path in pick_nonempty(&text_targets, &code_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "rg",
                    vec![
                        "-n".to_string(),
                        "(n|e|c|p|q|phi|mod)".to_string(),
                        path.clone(),
                    ],
                );
            }
        }
        ChallengeCategory::Pwn => {
            for path in bin_targets.iter().take(2) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "checksec",
                    vec!["--file".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "readelf",
                    vec!["-h".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "strings",
                    vec!["-n".to_string(), "4".to_string(), path.clone()],
                );
            }
        }
        ChallengeCategory::Reverse => {
            for path in bin_targets.iter().take(2) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "strings",
                    vec!["-n".to_string(), "4".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "objdump",
                    vec!["-x".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "radare2",
                    vec!["-A".to_string(), "-q".to_string(), path.clone()],
                );
            }
        }
        ChallengeCategory::Forensics => {
            for path in pick_nonempty(&pcap_targets, &bin_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "metadata",
                    "exiftool",
                    vec![path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "carve",
                    "binwalk",
                    vec![path.clone()],
                );
            }
        }
        ChallengeCategory::Stego => {
            for path in image_targets.iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "metadata",
                    "exiftool",
                    vec![path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "zsteg",
                    vec![path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "steghide",
                    vec![
                        "extract".to_string(),
                        "-sf".to_string(),
                        path.clone(),
                        "-f".to_string(),
                    ],
                );
            }
        }
        ChallengeCategory::Osint => {
            for path in pick_nonempty(&text_targets, &code_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "rg",
                    vec![
                        "-n".to_string(),
                        "@|https?://|dns|whois|twitter|discord|telegram".to_string(),
                        path.clone(),
                    ],
                );
            }
        }
        ChallengeCategory::Mobile => {
            for path in mobile_targets.iter().take(2) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "strings",
                    vec!["-n".to_string(), "4".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "decompile",
                    "jadx",
                    vec!["-d".to_string(), "jadx_out".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "decompile",
                    "apktool",
                    vec![
                        "d".to_string(),
                        path.clone(),
                        "-o".to_string(),
                        "apk_out".to_string(),
                    ],
                );
            }
        }
        ChallengeCategory::Hardware => {
            for path in pick_nonempty(&bin_targets, &text_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "carve",
                    "binwalk",
                    vec![path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "strings",
                    vec!["-n".to_string(), "4".to_string(), path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "simulate",
                    "iverilog",
                    vec!["-g2012".to_string(), path.clone()],
                );
            }
        }
        ChallengeCategory::Blockchain => {
            for path in pick_nonempty(&contract_targets, &code_targets)
                .iter()
                .take(3)
            {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "rg",
                    vec![
                        "-n".to_string(),
                        "require|revert|delegatecall|tx.origin|selfdestruct".to_string(),
                        path.clone(),
                    ],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "compile",
                    "solc",
                    vec!["--ast-compact-json".to_string(), path.clone()],
                );
            }
        }
        ChallengeCategory::Cloud => {
            for path in pick_nonempty(&infra_targets, &code_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "rg",
                    vec![
                        "-n".to_string(),
                        "iam|policy|bucket|secret|token|kube|docker|terraform|role".to_string(),
                        path.clone(),
                    ],
                );
            }
        }
        ChallengeCategory::Network => {
            for path in pcap_targets.iter().take(2) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "capinfos",
                    vec![path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "tshark",
                    vec!["-r".to_string(), path.clone(), "-q".to_string()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "tcpdump",
                    vec![
                        "-nr".to_string(),
                        path.clone(),
                        "-c".to_string(),
                        "120".to_string(),
                    ],
                );
            }
        }
        ChallengeCategory::Ai => {
            for path in pick_nonempty(&ai_targets, &text_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "rg",
                    vec![
                        "-n".to_string(),
                        "prompt|system|instruction|policy|secret|tool|jailbreak|model".to_string(),
                        path.clone(),
                    ],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "strings",
                    vec!["-n".to_string(), "4".to_string(), path.clone()],
                );
            }
        }
        ChallengeCategory::Web => {
            if let Some(host) = extract_host(target) {
                actions.push(PlannedAction {
                    action_type: "http-map".to_string(),
                    program: "curl".to_string(),
                    args: vec!["-i".to_string(), target_arg.clone()],
                    requires_network: true,
                    host: Some(host),
                    port: extract_port(target),
                });
            } else {
                for path in pick_nonempty(&code_targets, &text_targets).iter().take(3) {
                    push_if_tool(
                        &mut actions,
                        available_tools,
                        "triage",
                        "rg",
                        vec![
                            "-n".to_string(),
                            "route|endpoint|token|cookie|auth|jwt|password".to_string(),
                            path.clone(),
                        ],
                    );
                }
            }
        }
        ChallengeCategory::Misc | ChallengeCategory::Unknown => {
            for path in pick_nonempty(&text_targets, &bin_targets).iter().take(3) {
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "triage",
                    "file",
                    vec![path.clone()],
                );
                push_if_tool(
                    &mut actions,
                    available_tools,
                    "extract",
                    "strings",
                    vec!["-n".to_string(), "4".to_string(), path.clone()],
                );
            }
        }
    }

    if actions.is_empty() {
        push_action(&mut actions, "triage", "file", vec![target_arg]);
    }

    actions.truncate(max);
    actions
}

fn push_if_tool(
    actions: &mut Vec<PlannedAction>,
    available_tools: &HashSet<String>,
    action_type: &str,
    program: &str,
    args: Vec<String>,
) -> bool {
    if !has_tool(available_tools, program) {
        return false;
    }
    push_action(actions, action_type, program, args);
    true
}

fn push_action(
    actions: &mut Vec<PlannedAction>,
    action_type: &str,
    program: &str,
    args: Vec<String>,
) {
    actions.push(planned_owned(action_type, program, args));
}

fn has_tool(available_tools: &HashSet<String>, program: &str) -> bool {
    available_tools.contains(program)
}

fn discover_candidate_files(target: &Path, max_files: usize) -> Vec<PathBuf> {
    if target.is_file() {
        return vec![target.to_path_buf()];
    }
    if !target.is_dir() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(target)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        out.push(entry.path().to_path_buf());
        if out.len() >= max_files {
            break;
        }
    }
    out
}

fn select_targets<F>(paths: &[PathBuf], mut pred: F, limit: usize) -> Vec<String>
where
    F: FnMut(&Path) -> bool,
{
    paths
        .iter()
        .filter(|p| pred(p))
        .take(limit)
        .map(|p| p.display().to_string())
        .collect()
}

fn pick_nonempty(preferred: &[String], fallback: &[String]) -> Vec<String> {
    if preferred.is_empty() {
        fallback.to_vec()
    } else {
        preferred.to_vec()
    }
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_text_or_source(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "txt"
            | "md"
            | "rst"
            | "json"
            | "yaml"
            | "yml"
            | "csv"
            | "tsv"
            | "xml"
            | "ini"
            | "cfg"
            | "toml"
            | "log"
            | "html"
            | "js"
            | "ts"
            | "py"
            | "rb"
            | "php"
            | "java"
            | "go"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "rs"
            | "sql"
            | "sh"
    )
}

fn is_code_like(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "js" | "ts"
            | "py"
            | "rb"
            | "php"
            | "java"
            | "go"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "rs"
            | "sql"
            | "sh"
            | "sol"
            | "vy"
    )
}

fn is_binary_like(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "elf" | "exe" | "dll" | "so" | "bin" | "out" | "class" | "apk"
    )
}

fn is_image_like(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
    )
}

fn is_pcap_like(path: &Path) -> bool {
    matches!(ext(path).as_str(), "pcap" | "pcapng")
}

fn is_mobile_like(path: &Path) -> bool {
    matches!(ext(path).as_str(), "apk" | "ipa" | "dex" | "smali")
}

fn is_contract_like(path: &Path) -> bool {
    matches!(ext(path).as_str(), "sol" | "vy")
}

fn is_infra_like(path: &Path) -> bool {
    let e = ext(path);
    matches!(e.as_str(), "tf" | "tfvars" | "yaml" | "yml" | "json")
        || path
            .file_name()
            .and_then(|x| x.to_str())
            .is_some_and(|name| {
                let lower = name.to_ascii_lowercase();
                lower == "dockerfile"
                    || lower.contains("kube")
                    || lower.contains("helm")
                    || lower.contains("compose")
            })
}

fn is_ai_like(path: &Path) -> bool {
    matches!(
        ext(path).as_str(),
        "onnx" | "pt" | "pth" | "safetensors" | "json" | "txt" | "md"
    ) || path
        .file_name()
        .and_then(|x| x.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.contains("tokenizer") || lower.contains("prompt") || lower.contains("model")
        })
}

fn planned_owned(action_type: &str, program: &str, args: Vec<String>) -> PlannedAction {
    PlannedAction {
        action_type: action_type.to_string(),
        program: program.to_string(),
        args,
        requires_network: false,
        host: None,
        port: None,
    }
}

fn log_tool_visibility(
    session_id: &str,
    store: &StateStore,
    tools: &ToolManager,
) -> Result<HashSet<String>> {
    let statuses = tools.discover_default_tools();
    let available = statuses
        .iter()
        .filter(|t| t.available)
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();
    let missing = statuses
        .iter()
        .filter(|t| !t.available)
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();

    let summary = format!(
        "available({}): {}\nmissing({}): {}",
        available.len(),
        available.join(", "),
        missing.len(),
        missing.join(", ")
    );
    let metadata = json!({
        "available_count": available.len(),
        "missing_count": missing.len(),
    });
    store.add_action(NewAction {
        session_id,
        action_type: "tool-discovery",
        command: "auto-discover-tools",
        target: None,
        status: "ok",
        stdout: Some(&summary),
        stderr: None,
        metadata: Some(&metadata),
    })?;
    Ok(available.into_iter().collect())
}

fn ensure_generated_helpers(
    target_path: &Path,
    category: ChallengeCategory,
) -> Result<Vec<GeneratedHelper>> {
    let base_dir = if target_path.is_dir() {
        target_path.to_path_buf()
    } else {
        target_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let helper_dir = base_dir.join(".0x0-ai").join("generated-tools");
    fs::create_dir_all(&helper_dir)?;

    let mut out = Vec::new();
    let probe = helper_dir.join("ctf_probe.py");
    fs::write(&probe, PROBE_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&probe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&probe, perms)?;
    }
    out.push(GeneratedHelper {
        name: "ctf-probe".to_string(),
        description: "Behavior and artifact observation helper".to_string(),
        path: probe,
    });

    if matches!(
        category,
        ChallengeCategory::Crypto
            | ChallengeCategory::Blockchain
            | ChallengeCategory::Ai
            | ChallengeCategory::Misc
    ) {
        let extractor = helper_dir.join("pattern_probe.py");
        fs::write(&extractor, PATTERN_SCRIPT)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&extractor)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&extractor, perms)?;
        }
        out.push(GeneratedHelper {
            name: "pattern-probe".to_string(),
            description: "Pattern and token extractor helper".to_string(),
            path: extractor,
        });
    }

    Ok(out)
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

const PROBE_SCRIPT: &str = r#"#!/usr/bin/env python3
import os
import stat
import subprocess
import sys
from collections import Counter

root = sys.argv[1] if len(sys.argv) > 1 else "."
category = sys.argv[2] if len(sys.argv) > 2 else "unknown"

if not os.path.exists(root):
    print(f"[probe] missing target: {root}")
    sys.exit(0)

files = []
if os.path.isfile(root):
    files = [root]
else:
    for base, _, names in os.walk(root):
        for name in names:
            files.append(os.path.join(base, name))
            if len(files) >= 250:
                break
        if len(files) >= 250:
            break

ext_counts = Counter()
exec_targets = []
for path in files:
    ext = os.path.splitext(path)[1].lower() or "<none>"
    ext_counts[ext] += 1
    try:
        st = os.stat(path)
        if st.st_mode & stat.S_IXUSR:
            exec_targets.append(path)
    except OSError:
        pass

print(f"[probe] category={category} files={len(files)}")
print("[probe] top extensions:")
for ext, count in ext_counts.most_common(10):
    print(f"  - {ext}: {count}")

for path in exec_targets[:2]:
    for args in ([], ["--help"], ["-h"]):
        try:
            proc = subprocess.run([path] + args, capture_output=True, text=True, timeout=2)
            out = (proc.stdout or proc.stderr or "").strip().replace("\n", " ")
            out = out[:180]
            print(f"[behavior] {os.path.basename(path)} {' '.join(args) if args else '<no-args>'} => code={proc.returncode} out={out}")
        except Exception as exc:
            print(f"[behavior] {os.path.basename(path)} {' '.join(args) if args else '<no-args>'} => error={exc}")
"#;

const PATTERN_SCRIPT: &str = r#"#!/usr/bin/env python3
import os
import re
import sys

root = sys.argv[1] if len(sys.argv) > 1 else "."
patterns = [
    re.compile(r"(?i)[a-z0-9_-]{2,16}\{[^\n\r\}]{1,180}\}"),
    re.compile(r"(?i)(api[_-]?key|token|secret|password)\s*[:=]\s*['\"]?([^\s'\";]+)"),
    re.compile(r"(?i)(flag|ctf|challenge)[^a-z0-9]{0,6}([a-z0-9_\-\{\}]{4,})"),
]

if not os.path.exists(root):
    print(f"[pattern] missing target: {root}")
    sys.exit(0)

files = []
if os.path.isfile(root):
    files = [root]
else:
    for base, _, names in os.walk(root):
        for name in names:
            path = os.path.join(base, name)
            if os.path.getsize(path) > 2_000_000:
                continue
            files.append(path)
            if len(files) >= 200:
                break
        if len(files) >= 200:
            break

hits = 0
for path in files:
    try:
        with open(path, "r", errors="ignore") as f:
            text = f.read()
    except OSError:
        continue
    for pat in patterns:
        for m in pat.finditer(text):
            print(f"[pattern-hit] {path}: {m.group(0)[:180]}")
            hits += 1
            if hits >= 40:
                break
        if hits >= 40:
            break
    if hits >= 40:
        break

print(f"[pattern] files={len(files)} hits={hits}")
"#;
