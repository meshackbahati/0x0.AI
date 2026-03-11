use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use sysinfo::System;
use uuid::Uuid;

use crate::categories::{ChallengeCategory, infer_category};
use crate::cli::{
    ChatApprovalMode, ChatArgs, Cli, Commands, ConfigCommand, OutputFormat, ProviderCompat,
    ProvidersCommand, ProvidersConfigureArgs, RouteTask, SetupArgs, ToolsCommand, UpdateArgs,
    WebCommand,
};
use crate::config::{AppConfig, RuntimeConfig, load_runtime_config, save_config};
use crate::ingest::{ScanOptions, collect_signals_from_artifacts, scan_path};
use crate::output::OutputMode;
use crate::planner::{PlannerOptions, solve_loop};
use crate::plugins::PluginManager;
use crate::policy::{Approvals, PolicyEngine};
use crate::providers::{ProviderManager, ProviderRequest, TaskType};
use crate::report::{build_replay, build_writeup, write_writeup};
use crate::research::ResearchResult;
use crate::research::local::search_local;
use crate::research::web::WebResearcher;
use crate::storage::{NewAction, StateStore};
use crate::tools::package::{build_install_plan, detect_package_manager};
use crate::tools::{ToolManager, ToolRunRequest};
use crate::util::{confirm, shell_preview};
use crate::web_lab::{generate_templates_and_notebook, map_target, parse_target, replay_request};

pub fn run(cli: Cli) -> Result<()> {
    let mut runtime = load_runtime_config(cli.config.clone())?;
    if cli.offline {
        runtime.config.safety.offline_only = true;
        runtime.config.safety.research_web_enabled = false;
    }

    let dry_run = cli.dry_run;
    let auto_yes = cli.yes;
    let no_install = cli.no_install;
    let output_mode = resolve_output_mode(&cli, &runtime);

    let store = StateStore::open(
        &runtime.paths.db_path,
        runtime.config.memory.max_actions_per_session,
        runtime.config.memory.max_artifacts_per_session,
        runtime.config.memory.max_cache_entries,
    )?;

    let policy = PolicyEngine::new(runtime.config.safety.clone())?;
    let tools = ToolManager::new(runtime.config.tools.clone(), dry_run);
    let providers = ProviderManager::new(runtime.config.clone());
    let _plugins = PluginManager::new(runtime.paths.plugins_dir.clone());
    let command = cli.command.unwrap_or(Commands::Chat(ChatArgs::default()));

    match command {
        Commands::Init(args) => cmd_init(args.path, args.force, &mut runtime, output_mode),
        Commands::Setup(args) => cmd_setup(args, &mut runtime, output_mode),
        Commands::Update(args) => {
            cmd_update(args, &runtime, &policy, &tools, auto_yes, output_mode)
        }
        Commands::Scan(args) => {
            policy.ensure_path_allowed(&args.path)?;
            let session_id = ensure_session(&store, args.session_id.as_deref(), &args.path)?;
            let report = scan_path(
                &args.path,
                &session_id,
                &ScanOptions {
                    recursive: args.recursive,
                    max_read_bytes: args.max_read_bytes,
                },
                &store,
            )?;

            let summary = format!(
                "scanned {} files, indexed {}, category {}",
                report.total_files_seen,
                report.indexed_files,
                report.detected_category.as_str()
            );
            store.touch_session(
                &session_id,
                Some("active"),
                Some(report.detected_category.as_str()),
                Some(&summary),
            )?;
            store.add_action(NewAction {
                session_id: &session_id,
                action_type: "scan",
                command: &format!("0x0 scan {}", args.path.display()),
                target: Some(&args.path.display().to_string()),
                status: "ok",
                stdout: Some(&summary),
                stderr: None,
                metadata: None,
            })?;

            emit(output_mode, &report, &summary)
        }
        Commands::Solve(args) => {
            policy.ensure_path_allowed(&args.path)?;
            let session_id = ensure_session(&store, args.session_id.as_deref(), &args.path)?;

            let scan = scan_path(
                &args.path,
                &session_id,
                &ScanOptions {
                    recursive: true,
                    max_read_bytes: 128 * 1024,
                },
                &store,
            )?;

            let category = scan.detected_category;
            store.touch_session(
                &session_id,
                Some("active"),
                Some(category.as_str()),
                Some("solve started"),
            )?;

            let mut approvals = Approvals {
                network: args.approve_network || auto_yes,
                exec: args.approve_exec || auto_yes,
                install: args.approve_install || auto_yes,
            };

            if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                approvals.exec = confirm(
                    "Allow local command execution for solve workflow?",
                    auto_yes,
                )?;
            }
            if args.web
                && runtime.config.safety.require_confirmation_for_network
                && !approvals.network
            {
                approvals.network =
                    confirm("Allow network actions against approved targets?", auto_yes)?;
            }

            let outcome = solve_loop(
                &session_id,
                &args.path,
                category,
                PlannerOptions {
                    max_steps: args.max_steps,
                    web_enabled: args.web,
                    approvals,
                },
                &policy,
                &store,
                &tools,
                &providers,
            )?;

            let solved = !outcome.candidate_flags.is_empty();
            let status = if solved { "solved" } else { "active" };
            store.touch_session(
                &session_id,
                Some(status),
                Some(category.as_str()),
                Some(&outcome.summary),
            )?;

            emit(
                output_mode,
                &outcome,
                &format!(
                    "session={} category={} steps={} flags={}\n{}",
                    outcome.session_id,
                    outcome.category,
                    outcome.steps_executed,
                    outcome.candidate_flags.len(),
                    outcome.summary
                ),
            )
        }
        Commands::SolveAll(args) => {
            policy.ensure_path_allowed(&args.path)?;
            let targets = collect_challenge_paths(&args.path, args.max_challenges)?;
            if targets.is_empty() {
                bail!(
                    "no challenge targets discovered under {}",
                    args.path.display()
                );
            }

            let mut approvals = Approvals {
                network: args.approve_network || auto_yes,
                exec: args.approve_exec || auto_yes,
                install: args.approve_install || auto_yes,
            };
            if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                approvals.exec = confirm(
                    "Allow local command execution for solve-all workflow?",
                    auto_yes,
                )?;
            }
            if args.web
                && runtime.config.safety.require_confirmation_for_network
                && !approvals.network
            {
                approvals.network =
                    confirm("Allow network actions against approved targets?", auto_yes)?;
            }

            let mut outcomes = Vec::new();
            for target in targets {
                let session_id = ensure_session(&store, None, &target)?;
                let scan = scan_path(
                    &target,
                    &session_id,
                    &ScanOptions {
                        recursive: true,
                        max_read_bytes: 128 * 1024,
                    },
                    &store,
                )?;
                let category = scan.detected_category;
                store.touch_session(
                    &session_id,
                    Some("active"),
                    Some(category.as_str()),
                    Some("solve-all started"),
                )?;

                let outcome = solve_loop(
                    &session_id,
                    &target,
                    category,
                    PlannerOptions {
                        max_steps: args.max_steps,
                        web_enabled: args.web,
                        approvals,
                    },
                    &policy,
                    &store,
                    &tools,
                    &providers,
                )?;
                let solved = !outcome.candidate_flags.is_empty();
                let status = if solved { "solved" } else { "active" };
                store.touch_session(
                    &session_id,
                    Some(status),
                    Some(category.as_str()),
                    Some(&outcome.summary),
                )?;
                outcomes.push(json!({
                    "target": target.display().to_string(),
                    "session_id": outcome.session_id,
                    "category": outcome.category,
                    "steps": outcome.steps_executed,
                    "flags": outcome.candidate_flags,
                    "blocked": outcome.blocked,
                    "summary": outcome.summary,
                }));
            }

            let solved_count = outcomes
                .iter()
                .filter(|o| {
                    o.get("flags")
                        .and_then(|v| v.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
                })
                .count();
            let payload = json!({
                "root": args.path.display().to_string(),
                "targets": outcomes.len(),
                "solved": solved_count,
                "outcomes": outcomes,
            });
            emit(
                output_mode,
                &payload,
                &format!(
                    "solve-all completed: {} targets, {} with candidate flags",
                    payload["targets"], solved_count
                ),
            )
        }
        Commands::Resume(args) => {
            let session = store
                .get_session(&args.session_id)?
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", args.session_id))?;

            if args.continue_solve {
                let path = PathBuf::from(&session.root_path);
                policy.ensure_path_allowed(&path)?;
                let artifacts = store.list_artifacts(&session.id, 1000)?;
                let signals = collect_signals_from_artifacts(&artifacts);
                let category = session
                    .category
                    .as_deref()
                    .and_then(parse_category)
                    .unwrap_or_else(|| infer_category(&signals));

                let mut approvals = Approvals {
                    network: args.approve_network || auto_yes,
                    exec: args.approve_exec || auto_yes,
                    install: false,
                };

                if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                    approvals.exec = confirm(
                        "Allow local command execution for resumed workflow?",
                        auto_yes,
                    )?;
                }
                if args.web
                    && runtime.config.safety.require_confirmation_for_network
                    && !approvals.network
                {
                    approvals.network =
                        confirm("Allow network actions against approved targets?", auto_yes)?;
                }

                let outcome = solve_loop(
                    &session.id,
                    &path,
                    category,
                    PlannerOptions {
                        max_steps: args.max_steps,
                        web_enabled: args.web,
                        approvals,
                    },
                    &policy,
                    &store,
                    &tools,
                    &providers,
                )?;

                store.touch_session(
                    &session.id,
                    Some("active"),
                    Some(category.as_str()),
                    Some(&outcome.summary),
                )?;

                emit(
                    output_mode,
                    &outcome,
                    &format!("resumed session {}", session.id),
                )
            } else {
                let actions = store.list_actions(&session.id, 20)?;
                let notes = store.list_notes(&session.id, 20)?;
                let hypotheses = store.list_hypotheses(&session.id, 20)?;
                let snapshot = json!({
                    "session": session,
                    "actions": actions,
                    "notes": notes,
                    "hypotheses": hypotheses,
                });
                emit(output_mode, &snapshot, "session snapshot emitted")
            }
        }
        Commands::Sessions(args) => {
            let limit = args.limit.clamp(1, 200);
            let status = args
                .status
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let category = normalize_category_filter(args.category.as_deref())?;

            let sessions = store.list_session_summaries(limit, status, category.as_deref())?;

            if output_mode == OutputMode::Json {
                let payload = json!({
                    "filters": {
                        "limit": limit,
                        "status": status,
                        "category": category,
                    },
                    "sessions": sessions,
                });
                emit(output_mode, &payload, "session list emitted")
            } else if sessions.is_empty() {
                println!("No sessions found.");
                Ok(())
            } else {
                println!("Recent sessions ({}):", sessions.len());
                for s in sessions {
                    println!(
                        "{}  status={}  category={}  actions={}  artifacts={}  notes={}  updated={}  root={}",
                        s.id,
                        s.status,
                        s.category.as_deref().unwrap_or("unknown"),
                        s.action_count,
                        s.artifact_count,
                        s.note_count,
                        s.updated_at,
                        s.root_path
                    );
                    if let Some(summary) = s.summary.as_deref() {
                        let summary = summary.trim();
                        if !summary.is_empty() {
                            println!("  summary: {}", truncate_for_prompt(summary, 160));
                        }
                    }
                }
                Ok(())
            }
        }
        Commands::Research(args) => {
            let cwd = std::env::current_dir().context("reading current directory")?;
            policy.ensure_path_allowed(&cwd)?;

            let mut local_hits = Vec::new();
            if args.local {
                local_hits = search_local(
                    &args.query,
                    &cwd,
                    &store,
                    args.session_id.as_deref(),
                    args.max_results,
                )?;
            }

            let mut web_hits = Vec::new();
            let mut approvals = Approvals {
                network: args.approve_network || auto_yes,
                exec: false,
                install: false,
            };

            if args.web {
                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        "Allow passive web research (docs/public references only)?",
                        auto_yes,
                    )?;
                }
                policy.ensure_network_allowed(approvals, "public-web", None, true)?;
                let researcher = WebResearcher::new(runtime.config.research.clone())?;
                web_hits = researcher.search(&args.query, args.max_results, &store)?;
            }

            if let Some(session_id) = args.session_id.as_deref() {
                for hit in local_hits.iter().chain(web_hits.iter()) {
                    let _ = store.add_citation(
                        session_id,
                        &hit.citation.source_type,
                        &hit.citation.source,
                        hit.citation.locator.as_deref(),
                        &hit.citation.snippet,
                    );
                }
            }

            let result = ResearchResult {
                query: args.query,
                local_hits,
                web_hits,
                inferences: vec![
                    "Sourced claims are citation-backed; any planning recommendations are model inference."
                        .to_string(),
                ],
            };

            emit(
                output_mode,
                &result,
                &format!(
                    "research results: local={} web={}",
                    result.local_hits.len(),
                    result.web_hits.len()
                ),
            )
        }
        Commands::Chat(args) => cmd_chat(
            args,
            &runtime,
            &store,
            &policy,
            &tools,
            &providers,
            auto_yes,
            output_mode,
        ),
        Commands::Note(args) => {
            let session = store.get_session(&args.session_id)?;
            if session.is_none() {
                bail!("session not found: {}", args.session_id);
            }
            let note = args.text.join(" ");
            let id = store.add_note(&args.session_id, &note)?;
            let result = json!({"session_id": args.session_id, "note_id": id, "note": note});
            emit(output_mode, &result, "note recorded")
        }
        Commands::Tools(args) => match args.command {
            ToolsCommand::Doctor(doctor_args) => {
                let statuses = tools.discover_default_tools();
                let manager = detect_package_manager();
                let available = statuses.iter().filter(|s| s.available).count();
                let payload = json!({
                    "package_manager": manager,
                    "available": available,
                    "total": statuses.len(),
                    "tools": if doctor_args.verbose { json!(statuses) } else { json!([]) },
                });

                let text = format!(
                    "tool doctor: {} / {} tools available, package_manager={:?}",
                    available,
                    statuses.len(),
                    manager
                );
                emit(output_mode, &payload, &text)
            }
            ToolsCommand::Install(install_args) => {
                if no_install {
                    bail!("install denied by --no-install");
                }

                let mut approvals = Approvals {
                    network: false,
                    exec: auto_yes,
                    install: install_args.approve_install || auto_yes,
                };

                if runtime.config.safety.require_confirmation_for_install && !approvals.install {
                    approvals.install = confirm(
                        &format!(
                            "Install tool '{}' using package manager?",
                            install_args.tool
                        ),
                        auto_yes,
                    )?;
                }

                policy.ensure_install_allowed(approvals, &install_args.tool)?;
                policy.ensure_exec_allowed(
                    Approvals {
                        exec: true,
                        ..approvals
                    },
                    "package-manager",
                )?;

                let plan = build_install_plan(&install_args.tool, None);
                if plan.command.is_empty() {
                    bail!("install plan empty for tool {}", install_args.tool);
                }
                let program = plan.command[0].clone();
                let args = plan.command[1..].to_vec();

                let res = tools.run(ToolRunRequest {
                    program: program.clone(),
                    args: args.clone(),
                    cwd: None,
                    timeout_secs: Some(3600),
                })?;

                let verify = tools.discover_tools(&[&install_args.tool]);
                let installed = verify.first().map(|s| s.available).unwrap_or(false);

                let payload = json!({
                    "plan": plan,
                    "execution": res,
                    "verified_installed": installed,
                });

                emit(
                    output_mode,
                    &payload,
                    &format!("install command executed for {}", install_args.tool),
                )
            }
        },
        Commands::Providers(args) => match args.command {
            ProvidersCommand::Test(test_args) => {
                let providers_list = providers.available_provider_names();
                if providers_list.is_empty() {
                    bail!("no providers available (including local fallback)");
                }

                let selected: Vec<String> = if let Some(name) = test_args.provider {
                    if !providers_list.iter().any(|p| p == &name) {
                        bail!("provider '{}' is not available", name);
                    }
                    vec![name]
                } else {
                    providers_list
                };

                let mut results = BTreeMap::new();

                for provider in selected {
                    let mut stream_buf = String::new();
                    let mut sink = |chunk: &str| {
                        stream_buf.push_str(chunk);
                        if output_mode == OutputMode::Text {
                            print!("{chunk}");
                            let _ = std::io::stdout().flush();
                        }
                    };

                    let response = providers.call_with_provider(
                        &provider,
                        ProviderRequest {
                            system: Some(
                                "You are running a provider connectivity and behavior test."
                                    .to_string(),
                            ),
                            prompt: test_args.prompt.clone(),
                            task_type: TaskType::Reasoning,
                            max_tokens: 120,
                            temperature: 0.1,
                            timeout_secs: runtime.config.providers.request_timeout_secs,
                            model_override: None,
                        },
                        Some(&mut sink),
                    );

                    if output_mode == OutputMode::Text {
                        println!();
                    }

                    match response {
                        Ok(r) => {
                            results.insert(
                                provider,
                                json!({
                                    "status": "ok",
                                    "model": r.model,
                                    "text": r.text,
                                    "streamed_preview": stream_buf,
                                }),
                            );
                        }
                        Err(err) => {
                            results.insert(
                                provider,
                                json!({
                                    "status": "error",
                                    "error": err.to_string(),
                                }),
                            );
                        }
                    }
                }

                emit(output_mode, &results, "provider test finished")
            }
            ProvidersCommand::Configure(cfg_args) => {
                configure_provider(&mut runtime.config, &cfg_args)?;

                if let Some(route) = cfg_args.route.clone() {
                    set_route(
                        &mut runtime.config,
                        route,
                        &cfg_args.provider,
                        cfg_args.model.clone(),
                    );
                }

                save_config(&runtime.paths.config_path, &runtime.config)?;
                let redacted = redacted_provider_view(&runtime.config, &cfg_args.provider);
                emit(
                    output_mode,
                    &redacted,
                    &format!("provider '{}' configuration updated", cfg_args.provider),
                )
            }
            ProvidersCommand::Models(model_args) => {
                let mut approvals = Approvals {
                    network: auto_yes,
                    exec: false,
                    install: false,
                };
                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        "Allow network access to provider APIs for model listing?",
                        auto_yes,
                    )?;
                }
                policy.ensure_network_allowed(approvals, "provider-apis", None, true)?;
                let models = providers.list_models(model_args.provider.as_deref())?;
                emit(output_mode, &models, "provider model list loaded")
            }
            ProvidersCommand::Use(use_args) => {
                let task_name = format!("{:?}", use_args.task).to_ascii_lowercase();
                set_route(
                    &mut runtime.config,
                    use_args.task,
                    &use_args.provider,
                    Some(use_args.model.clone()),
                );
                save_config(&runtime.paths.config_path, &runtime.config)?;
                let payload = json!({
                    "task": task_name,
                    "provider": use_args.provider,
                    "model": use_args.model
                });
                emit(output_mode, &payload, "task route updated")
            }
        },
        Commands::Web(args) => match args.command {
            WebCommand::Map(map_args) => {
                let target = parse_target(&map_args.target)?;
                let session_id =
                    ensure_session_label(&store, map_args.session_id.as_deref(), &target.base_url)?;

                let mut approvals = Approvals {
                    network: map_args.approve_network || auto_yes,
                    exec: map_args.approve_exec || auto_yes,
                    install: false,
                };

                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        &format!(
                            "Allow web mapping against approved target {}:{} ?",
                            target.host, target.port
                        ),
                        auto_yes,
                    )?;
                }
                if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                    approvals.exec =
                        confirm("Allow local command execution for web mapping?", auto_yes)?;
                }

                let out_dir = map_args
                    .out
                    .unwrap_or_else(|| runtime.paths.state_dir.join("payload_notebooks"));

                let report = map_target(&target, &tools, &policy, approvals, &out_dir)?;

                store.touch_session(
                    &session_id,
                    Some("active"),
                    Some("web"),
                    Some("web map completed"),
                )?;

                for probe in &report.probes {
                    let meta = json!({
                        "path": probe.path,
                        "status_code": probe.status_code,
                        "content_length": probe.content_length,
                    });
                    store.add_action(NewAction {
                        session_id: &session_id,
                        action_type: "web-map",
                        command: &probe.command_preview,
                        target: Some(&target.base_url),
                        status: &probe.status,
                        stdout: Some(&probe.excerpt),
                        stderr: None,
                        metadata: Some(&meta),
                    })?;
                    let citation_source = format!("{}{}", target.base_url, probe.path);
                    let _ = store.add_citation(
                        &session_id,
                        "web",
                        &citation_source,
                        None,
                        &probe.excerpt,
                    );
                }

                for template in &report.fuzz_templates {
                    let _ = store.add_note(
                        &session_id,
                        &format!(
                            "web-fuzz-template [{}]: {}",
                            template.name, template.command_preview
                        ),
                    );
                }

                emit(
                    output_mode,
                    &report,
                    &format!(
                        "web map complete: target={} probes={} params={} notebook={}",
                        report.target.base_url,
                        report.probes.len(),
                        report.discovered_params.len(),
                        report.payload_notebook_path
                    ),
                )
            }
            WebCommand::Replay(replay_args) => {
                let target = parse_target(&replay_args.target)?;
                let session_id = ensure_session_label(
                    &store,
                    replay_args.session_id.as_deref(),
                    &target.base_url,
                )?;

                let mut approvals = Approvals {
                    network: replay_args.approve_network || auto_yes,
                    exec: replay_args.approve_exec || auto_yes,
                    install: false,
                };

                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        &format!(
                            "Allow request replay against approved target {}:{} ?",
                            target.host, target.port
                        ),
                        auto_yes,
                    )?;
                }
                if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                    approvals.exec = confirm(
                        "Allow local command execution for request replay?",
                        auto_yes,
                    )?;
                }

                let report = replay_request(
                    &target,
                    &tools,
                    &policy,
                    approvals,
                    &replay_args.method,
                    &replay_args.path,
                    &replay_args.header,
                    replay_args.data.as_deref(),
                )?;

                let meta = json!({
                    "method": report.method,
                    "path": report.path,
                    "status_code": report.status_code,
                });
                store.add_action(NewAction {
                    session_id: &session_id,
                    action_type: "web-replay",
                    command: &report.command_preview,
                    target: Some(&target.base_url),
                    status: "ok",
                    stdout: Some(&report.stdout_excerpt),
                    stderr: Some(&report.stderr_excerpt),
                    metadata: Some(&meta),
                })?;

                emit(
                    output_mode,
                    &report,
                    &format!(
                        "web replay done: method={} path={} status={:?}",
                        report.method, report.path, report.status_code
                    ),
                )
            }
            WebCommand::Template(template_args) => {
                let target = parse_target(&template_args.target)?;
                let out_dir = template_args
                    .out
                    .unwrap_or_else(|| runtime.paths.state_dir.join("payload_notebooks"));
                let (templates, notebook_path) =
                    generate_templates_and_notebook(&target, &out_dir)?;
                let payload = json!({
                    "target": target,
                    "templates": templates,
                    "payload_notebook_path": notebook_path,
                });
                emit(output_mode, &payload, "web templates generated")
            }
        },
        Commands::Writeup(args) => {
            let bundle = build_writeup(&store, &args.session_id)?;
            let out = args.out.unwrap_or_else(|| {
                runtime
                    .paths
                    .writeups_dir
                    .join(format!("{}.md", args.session_id))
            });
            write_writeup(&out, &bundle.markdown)?;
            let payload =
                json!({"session_id": args.session_id, "path": out, "bytes": bundle.markdown.len()});
            emit(
                output_mode,
                &payload,
                &format!("writeup generated at {}", out.display()),
            )
        }
        Commands::Replay(args) => {
            let replay = build_replay(&store, &args.session_id, args.limit)?;
            if output_mode == OutputMode::Json {
                println!("{}", serde_json::to_string_pretty(&replay)?);
                Ok(())
            } else {
                println!("session: {}", replay.session_id);
                for action in replay.actions.into_iter().rev() {
                    println!("- [{}] {}", action.status, action.command);
                }
                Ok(())
            }
        }
        Commands::Config(args) => match args.command {
            ConfigCommand::Edit => {
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                let approvals = Approvals {
                    network: false,
                    exec: auto_yes
                        || confirm(&format!("Open config with editor '{}' ?", editor), auto_yes)?,
                    install: false,
                };
                policy.ensure_exec_allowed(approvals, &editor)?;

                let res = tools.run(ToolRunRequest {
                    program: editor.clone(),
                    args: vec![runtime.paths.config_path.display().to_string()],
                    cwd: None,
                    timeout_secs: None,
                })?;
                let payload = json!({"editor": editor, "result": res});
                emit(output_mode, &payload, "config edit command executed")
            }
            ConfigCommand::Show => {
                let text = fs::read_to_string(&runtime.paths.config_path)
                    .with_context(|| format!("reading {}", runtime.paths.config_path.display()))?;
                if output_mode == OutputMode::Json {
                    let cfg_json: serde_json::Value = toml::from_str::<toml::Value>(&text)
                        .map(serde_json::to_value)?
                        .context("converting config to json")?;
                    println!("{}", serde_json::to_string_pretty(&cfg_json)?);
                    Ok(())
                } else {
                    println!("{}", text);
                    Ok(())
                }
            }
        },
        Commands::Stats(args) => {
            let db_stats = if let Some(sid) = args.session_id.as_deref() {
                store.session_stats(sid)?
            } else {
                store.stats()?
            };

            let mut system = System::new_all();
            system.refresh_memory();

            let payload = json!({
                "db": db_stats,
                "memory": {
                    "total_bytes": system.total_memory(),
                    "used_bytes": system.used_memory(),
                    "available_bytes": system.available_memory(),
                },
                "cpu_count": num_cpus::get(),
                "config": {
                    "max_actions_per_session": runtime.config.memory.max_actions_per_session,
                    "max_artifacts_per_session": runtime.config.memory.max_artifacts_per_session,
                    "max_cache_entries": runtime.config.memory.max_cache_entries,
                }
            });

            emit(output_mode, &payload, "stats emitted")
        }
    }
}

fn cmd_init(
    path: PathBuf,
    force: bool,
    runtime: &mut RuntimeConfig,
    mode: OutputMode,
) -> Result<()> {
    runtime.paths.ensure_dirs()?;

    if force {
        save_config(&runtime.paths.config_path, &runtime.config)?;
    }

    let project_meta = path.join(".0x0-ai");
    fs::create_dir_all(&project_meta)?;

    let project_cfg = project_meta.join("project.toml");
    if force || !project_cfg.exists() {
        let content = r#"name = "ctf-project"
notes = "local investigation workspace"

[safety]
offline_only = false
"#;
        fs::write(&project_cfg, content)?;
    }

    ensure_example_plugin(&runtime.paths.plugins_dir)?;

    let payload = json!({
        "config": runtime.paths.config_path,
        "db": runtime.paths.db_path,
        "cache": runtime.paths.cache_dir,
        "logs": runtime.paths.log_dir,
        "writeups": runtime.paths.writeups_dir,
        "plugins": runtime.paths.plugins_dir,
        "project_meta": project_meta,
    });

    emit(mode, &payload, "0x0.AI initialized")
}

fn cmd_setup(args: SetupArgs, runtime: &mut RuntimeConfig, mode: OutputMode) -> Result<()> {
    if args.non_interactive || args.provider.is_some() {
        let provider = args
            .provider
            .ok_or_else(|| anyhow::anyhow!("--provider is required for non-interactive setup"))?;
        let configure = ProvidersConfigureArgs {
            provider: provider.clone(),
            api_key: args.api_key.clone(),
            api_key_env: args.api_key_env.clone(),
            model: args.model.clone(),
            base_url: args.base_url.clone(),
            enable: true,
            disable: false,
            route: args.route.clone(),
            compat: args.compat.clone(),
        };
        configure_provider(&mut runtime.config, &configure)?;
        if let Some(route) = args.route {
            set_route(&mut runtime.config, route, &provider, args.model.clone());
        } else {
            bind_default_chat_routes(&mut runtime.config, &provider, args.model.clone());
        }
        save_config(&runtime.paths.config_path, &runtime.config)?;
        let payload = redacted_provider_view(&runtime.config, &provider);
        return emit(mode, &payload, "setup completed (non-interactive)");
    }

    println!("0x0.AI provider setup wizard");
    println!("Use Ctrl+C to abort.");

    let provider =
        prompt_required("Provider (openai/openrouter/together/gemini/moonshot/anthropic): ")?;
    let api_key = prompt_optional("API key (leave empty to use env var): ")?;
    let api_key_env = prompt_optional("API key env var (optional): ")?;
    let model = prompt_optional("Default model (optional): ")?;
    let base_url = prompt_optional("Base URL override (optional): ")?;
    let route = prompt_optional(
        "Route task [reasoning|coding|summarization|vision|classification] (optional): ",
    )?;
    let compat = prompt_optional(
        "Compatibility [openai|anthropic|generic] for custom providers (optional): ",
    )?;

    let route_task = route
        .as_deref()
        .and_then(|r| match r.trim().to_ascii_lowercase().as_str() {
            "reasoning" => Some(RouteTask::Reasoning),
            "coding" => Some(RouteTask::Coding),
            "summarization" => Some(RouteTask::Summarization),
            "vision" => Some(RouteTask::Vision),
            "classification" => Some(RouteTask::Classification),
            _ => None,
        });

    let configure = ProvidersConfigureArgs {
        provider: provider.clone(),
        api_key,
        api_key_env,
        model: model.clone(),
        base_url,
        enable: true,
        disable: false,
        route: route_task.clone(),
        compat: compat.as_deref().and_then(parse_compat),
    };

    configure_provider(&mut runtime.config, &configure)?;
    if let Some(task) = route_task {
        set_route(&mut runtime.config, task, &provider, model);
    } else {
        bind_default_chat_routes(&mut runtime.config, &provider, model);
    }
    save_config(&runtime.paths.config_path, &runtime.config)?;

    let payload = redacted_provider_view(&runtime.config, &provider);
    emit(mode, &payload, "setup completed")
}

fn cmd_update(
    args: UpdateArgs,
    runtime: &RuntimeConfig,
    policy: &PolicyEngine,
    tools: &ToolManager,
    auto_yes: bool,
    mode: OutputMode,
) -> Result<()> {
    if args.system && args.user {
        bail!("choose only one mode: --system or --user");
    }

    let mut approvals = Approvals {
        network: false,
        exec: auto_yes,
        install: auto_yes,
    };

    if runtime.config.safety.require_confirmation_for_install && !approvals.install {
        approvals.install = confirm(
            "Allow self-update (download and reinstall 0x0.AI from GitHub)?",
            auto_yes,
        )?;
    }
    policy.ensure_install_allowed(approvals, "0x0-ai-self-update")?;

    if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
        approvals.exec = confirm("Allow running updater script?", auto_yes)?;
    }
    policy.ensure_exec_allowed(approvals, "bash")?;

    let mut update_args = Vec::new();
    if args.system {
        update_args.push("--system".to_string());
    } else if args.user {
        update_args.push("--user".to_string());
    }
    if let Some(branch) = args.branch {
        update_args.push("--branch".to_string());
        update_args.push(branch);
    }
    if let Some(reference) = args.reference {
        update_args.push("--reference".to_string());
        update_args.push(reference);
    }
    if args.prefer_commit {
        update_args.push("--prefer-commit".to_string());
    }
    if args.dry_run {
        update_args.push("--dry-run".to_string());
    }

    let local_script = std::env::current_dir()?.join("scripts/update.sh");
    let (source, result) = if local_script.exists() {
        let mut cmd_args = vec![local_script.display().to_string()];
        cmd_args.extend(update_args.clone());
        let res = tools.run(ToolRunRequest {
            program: "bash".to_string(),
            args: cmd_args,
            cwd: None,
            timeout_secs: Some(3600),
        })?;
        ("local-script".to_string(), res)
    } else {
        let mut remote_cmd = String::from(
            "curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/update.sh | bash -s --",
        );
        for arg in &update_args {
            remote_cmd.push(' ');
            remote_cmd.push_str(&shell_escape::escape(arg.as_str().into()).to_string());
        }
        let res = tools.run(ToolRunRequest {
            program: "bash".to_string(),
            args: vec!["-lc".to_string(), remote_cmd],
            cwd: None,
            timeout_secs: Some(3600),
        })?;
        ("remote-script".to_string(), res)
    };

    let payload = json!({
        "source": source,
        "args": update_args,
        "result": result,
    });

    emit(mode, &payload, "update command executed")
}

#[derive(Debug, Deserialize)]
struct AgentDecision {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    message: String,
    action: Option<AgentAction>,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentAction {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    risk: String,
    #[serde(default)]
    requires_network: bool,
    host: Option<String>,
    port: Option<u16>,
    timeout_secs: Option<u64>,
}

#[derive(Debug, Clone)]
struct ChatProviderCandidate {
    name: String,
    enabled: bool,
    key_present: bool,
    loaded: bool,
    key_hint: Option<String>,
}

impl ChatProviderCandidate {
    fn ready(&self) -> bool {
        self.name == "local" || (self.enabled && self.key_present && self.loaded)
    }

    fn reason(&self) -> String {
        if self.name == "local" {
            return "ready".to_string();
        }
        if !self.enabled {
            return "disabled".to_string();
        }
        if !self.key_present {
            return format!(
                "missing API key{}",
                self.key_hint
                    .as_deref()
                    .map(|v| format!(" (set env {})", v))
                    .unwrap_or_default()
            );
        }
        if !self.loaded {
            return "not loaded in runtime".to_string();
        }
        "ready".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentRisk {
    Low,
    Medium,
    High,
}

impl AgentRisk {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

fn cmd_chat(
    args: ChatArgs,
    runtime: &RuntimeConfig,
    store: &StateStore,
    policy: &PolicyEngine,
    tools: &ToolManager,
    providers: &ProviderManager,
    auto_yes: bool,
    output_mode: OutputMode,
) -> Result<()> {
    let session_id = ensure_session_label(store, args.session_id.as_deref(), "chat://terminal")?;
    store.touch_session(
        &session_id,
        Some("active"),
        Some("misc"),
        Some("interactive chat session"),
    )?;
    let mut current_session_id = session_id;
    let mut flag_prefix: Option<String> = None;
    let mut active_provider = args.provider.clone();
    let mut active_model_override: Option<String> = None;
    if let Some(p) = active_provider.as_deref() {
        let candidates = chat_provider_candidates(runtime, providers);
        if let Some(candidate) = find_chat_provider_candidate(&candidates, p) {
            if !candidate.ready() {
                println!(
                    "Provider '{}' is not ready: {}. Falling back to task default provider.",
                    candidate.name,
                    candidate.reason()
                );
                active_provider = None;
            }
        } else {
            println!(
                "Provider '{}' is not recognized. Falling back to task default provider.",
                p
            );
            active_provider = None;
        }
    }

    let mut approvals = Approvals {
        network: args.approve_network || auto_yes,
        exec: args.approve_exec || auto_yes,
        install: false,
    };

    if args.web && runtime.config.safety.require_confirmation_for_network && !approvals.network {
        approvals.network = confirm(
            "Allow passive web research in chat mode (docs/public references only)?",
            auto_yes,
        )?;
    }

    if args.autonomous && runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
        approvals.exec = confirm(
            "Allow autonomous mode to execute local commands (risky actions still require approval)?",
            auto_yes,
        )?;
    }

    let mut web_researcher = if args.web {
        Some(WebResearcher::new(runtime.config.research.clone())?)
    } else {
        None
    };

    let cwd = std::env::current_dir()?;
    policy.ensure_path_allowed(&cwd)?;

    if let Some(one_shot) = args.prompt.as_deref() {
        if let Some(prefix) = extract_flag_prefix_hint(one_shot) {
            flag_prefix = Some(prefix.clone());
            let _ = store.add_note(
                &current_session_id,
                &format!("flag-prefix(auto-detected): {}", prefix),
            );
        }
        let reply = if args.autonomous {
            autonomous_chat_turn(
                one_shot,
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                output_mode,
                auto_yes,
                flag_prefix.as_deref(),
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?
        } else {
            chat_turn(
                one_shot,
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                output_mode,
                auto_yes,
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?
        };

        if output_mode == OutputMode::Json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "session_id": current_session_id,
                    "reply": reply,
                }))?
            );
        }
        return Ok(());
    }

    println!("Chat session: {}", current_session_id);
    print_chat_help(args.autonomous);
    print_chat_constraints(runtime, &args, &approvals);

    let mut turns = 0usize;
    loop {
        if turns >= args.max_turns {
            println!("Reached max turns ({})", args.max_turns);
            break;
        }
        turns += 1;

        print!("you> ");
        io::stdout().flush()?;
        let mut line = String::new();
        let n = io::stdin().read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(prefix) = extract_flag_prefix_hint(line)
            && flag_prefix.as_deref() != Some(prefix.as_str())
        {
            flag_prefix = Some(prefix.clone());
            let _ = store.add_note(
                &current_session_id,
                &format!("flag-prefix(auto-detected): {}", prefix),
            );
            println!("[agent] detected flag prefix: {}", prefix);
        }
        if matches!(line, "/exit" | "exit" | "quit") {
            break;
        }
        if line == "/help" {
            print_chat_help(args.autonomous);
            continue;
        }
        if let Some(rest) = line.strip_prefix("/provider") {
            let arg = rest.trim();
            let candidates = chat_provider_candidates(runtime, providers);
            if arg.is_empty() {
                println!("Provider status:");
                for c in &candidates {
                    let marker = if active_provider.as_deref() == Some(c.name.as_str()) {
                        "*"
                    } else {
                        " "
                    };
                    println!("[{}] {} => {}", marker, c.name, c.reason());
                }
                println!("Use: /provider <name>");
                continue;
            }

            if let Some(c) = find_chat_provider_candidate(&candidates, arg) {
                if c.ready() {
                    active_provider = Some(c.name.clone());
                    active_model_override = None;
                    let _ = store.add_note(
                        &current_session_id,
                        &format!("provider-selected {}", c.name),
                    );
                    println!(
                        "active provider set to '{}' (model override cleared)",
                        c.name
                    );
                } else {
                    println!("provider '{}' is not ready: {}", c.name, c.reason());
                }
            } else {
                println!("unknown provider '{}'", arg);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("/model") {
            let arg = rest.trim();
            if arg.is_empty() {
                let resolved = active_provider
                    .clone()
                    .unwrap_or_else(|| providers.provider_for_task(TaskType::Reasoning));
                let model_label = active_model_override
                    .as_deref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "(provider default)".to_string());
                println!("active provider={} model={}", resolved, model_label);
                continue;
            }
            if arg.eq_ignore_ascii_case("all") {
                let resolved = active_provider
                    .clone()
                    .unwrap_or_else(|| providers.provider_for_task(TaskType::Reasoning));
                if !providers
                    .available_provider_names()
                    .iter()
                    .any(|p| p == &resolved)
                {
                    println!(
                        "Provider '{}' is not available in current config/runtime.",
                        resolved
                    );
                    continue;
                }
                if resolved != "local" {
                    if runtime.config.safety.require_confirmation_for_network && !approvals.network
                    {
                        approvals.network = confirm(
                            "Allow network access to provider APIs for model listing?",
                            auto_yes,
                        )?;
                    }
                    policy.ensure_network_allowed(approvals, "provider-apis", None, true)?;
                }
                let listing = providers.list_models(Some(&resolved))?;
                let models = listing.get(&resolved).cloned().unwrap_or_default();
                if models.is_empty() {
                    println!("No models returned for provider '{}'.", resolved);
                } else {
                    println!("Models for '{}':", resolved);
                    for m in models {
                        println!("- {}", m);
                    }
                }
                let _ = store.add_note(
                    &current_session_id,
                    &format!("model-list provider={}", resolved),
                );
                continue;
            }

            if let Some((provider_name, model_name)) = arg.split_once(':') {
                let provider = provider_name.trim();
                let model = model_name.trim();
                if provider.is_empty() || model.is_empty() {
                    println!(
                        "Usage: /model <model-id> | /model all | /model <provider>:<model-id>"
                    );
                    continue;
                }
                let candidates = chat_provider_candidates(runtime, providers);
                if let Some(c) = find_chat_provider_candidate(&candidates, provider) {
                    if c.ready() {
                        active_provider = Some(c.name.clone());
                        active_model_override = Some(model.to_string());
                        let _ = store.add_note(
                            &current_session_id,
                            &format!("model-selected provider={} model={}", c.name, model),
                        );
                        println!("model override set: provider={} model={}", c.name, model);
                    } else {
                        println!("provider '{}' is not ready: {}", c.name, c.reason());
                    }
                } else {
                    println!("unknown provider '{}'", provider);
                }
                continue;
            }

            let candidates = chat_provider_candidates(runtime, providers);
            if let Some(c) = find_chat_provider_candidate(&candidates, arg) {
                if c.ready() {
                    active_provider = Some(c.name.clone());
                    active_model_override = None;
                    let _ = store.add_note(
                        &current_session_id,
                        &format!("provider-selected {}", c.name),
                    );
                    println!(
                        "active provider set to '{}' (model override cleared)",
                        c.name
                    );
                } else {
                    println!("provider '{}' is not ready: {}", c.name, c.reason());
                }
                continue;
            }

            if matches!(
                arg.to_ascii_lowercase().as_str(),
                "none" | "default" | "reset" | "clear"
            ) {
                active_model_override = None;
                let _ = store.add_note(&current_session_id, "model-selected: provider default");
                println!("model override cleared; using provider default");
                continue;
            }

            active_model_override = Some(arg.to_string());
            let resolved = active_provider
                .clone()
                .unwrap_or_else(|| providers.provider_for_task(TaskType::Reasoning));
            let _ = store.add_note(
                &current_session_id,
                &format!("model-selected provider={} model={}", resolved, arg),
            );
            println!("model override set: provider={} model={}", resolved, arg);
            continue;
        }
        if line == "/sessions" {
            let sessions = store.list_sessions(10)?;
            if sessions.is_empty() {
                println!("No sessions yet.");
            } else {
                println!("Recent sessions:");
                for s in sessions {
                    println!(
                        "{}  status={}  updated={}  root={}",
                        s.id, s.status, s.updated_at, s.root_path
                    );
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("/resume ") {
            let target = rest.trim();
            if target.is_empty() {
                println!("Usage: /resume <session-id>");
                continue;
            }
            if store.get_session(target)?.is_some() {
                current_session_id = target.to_string();
                println!("Resumed session: {}", current_session_id);
            } else {
                println!("Session not found: {}", target);
            }
            continue;
        }
        if line == "/constraints" {
            print_chat_constraints(runtime, &args, &approvals);
            continue;
        }
        if matches!(line, "/clean" | "/clear") {
            print!("\x1B[2J\x1B[H");
            io::stdout().flush()?;
            continue;
        }
        if line == "/ps" {
            let _ = chat_turn(
                "/run ps aux",
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?;
            continue;
        }
        if line == "/ls" {
            let _ = chat_turn(
                "/run ls -la",
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?;
            continue;
        }
        if line == "/pwd" {
            let _ = chat_turn(
                "/run pwd",
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?;
            continue;
        }
        if let Some(rest) = line.strip_prefix("/ask ") {
            let _ = chat_turn(
                rest,
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?;
            continue;
        }
        if let Some(rest) = line.strip_prefix("/auto ") {
            let _ = autonomous_chat_turn(
                rest,
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                flag_prefix.as_deref(),
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?;
            continue;
        }

        let _ = if args.autonomous {
            autonomous_chat_turn(
                line,
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                flag_prefix.as_deref(),
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?
        } else {
            chat_turn(
                line,
                &args,
                &current_session_id,
                store,
                policy,
                tools,
                providers,
                &cwd,
                &mut web_researcher,
                &mut approvals,
                OutputMode::Text,
                auto_yes,
                active_provider.as_deref(),
                active_model_override.as_deref(),
            )?
        };
    }

    Ok(())
}

fn print_chat_help(autonomous_default: bool) {
    println!("Commands:");
    println!("/help                    show command list");
    println!("/sessions                list recent session IDs");
    println!("/resume <session-id>     switch chat to an existing session");
    println!("/constraints             show active safety constraints");
    println!("/provider [name]         list providers or switch provider if ready");
    println!(
        "/model [all|<provider>|<id>|<provider>:<id>] show/switch model or list provider models"
    );
    println!("/run <command>           execute local command through policy wrapper");
    println!("/ps                      shortcut for /run ps aux");
    println!("/ls                      shortcut for /run ls -la");
    println!("/pwd                     shortcut for /run pwd");
    println!("/clean                   clear terminal screen");
    println!("/research <query>        local + optional web research");
    println!("/ask <prompt>            normal chat answer (no autonomous loop)");
    println!("/auto <goal>             autonomous action loop for one goal");
    println!("/exit                    leave chat");
    if autonomous_default {
        println!("Default mode: autonomous (/auto). Use /ask for direct answers.");
    } else {
        println!("Default mode: direct answers. Use /auto to enable autonomous steps.");
    }
}

fn print_chat_constraints(runtime: &RuntimeConfig, args: &ChatArgs, approvals: &Approvals) {
    let safety = &runtime.config.safety;
    let paths = safety
        .allowed_paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let hosts = if safety.allowed_hosts.is_empty() {
        "(none)".to_string()
    } else {
        safety.allowed_hosts.join(", ")
    };
    let ports = safety
        .allowed_ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    println!("Constraints:");
    println!("offline_only={}", safety.offline_only);
    println!("allowed_paths={paths}");
    println!("allowed_hosts={hosts}");
    println!("allowed_ports={ports}");
    println!(
        "approval_mode={} autonomous={} max_agent_steps={}",
        match args.approval_mode {
            ChatApprovalMode::All => "all",
            ChatApprovalMode::Risky => "risky",
        },
        args.autonomous,
        args.max_agent_steps
    );
    println!(
        "session_approvals: exec={} network={}",
        approvals.exec, approvals.network
    );
}

fn chat_provider_candidates(
    runtime: &RuntimeConfig,
    providers: &ProviderManager,
) -> Vec<ChatProviderCandidate> {
    let loaded: HashSet<String> = providers.available_provider_names().into_iter().collect();
    let mut out = BTreeMap::<String, ChatProviderCandidate>::new();

    out.insert(
        "local".to_string(),
        ChatProviderCandidate {
            name: "local".to_string(),
            enabled: true,
            key_present: true,
            loaded: loaded.contains("local"),
            key_hint: None,
        },
    );

    let insert_openai_like =
        |out: &mut BTreeMap<String, ChatProviderCandidate>,
         name: &str,
         cfg: &crate::config::OpenAiCompatProvider| {
            out.insert(
                name.to_string(),
                ChatProviderCandidate {
                    name: name.to_string(),
                    enabled: cfg.enabled,
                    key_present: provider_key_present(cfg.api_key.as_deref(), &cfg.api_key_env),
                    loaded: loaded.contains(name),
                    key_hint: Some(cfg.api_key_env.clone()),
                },
            );
        };

    let cfg = &runtime.config.providers;
    insert_openai_like(&mut out, "openai", &cfg.openai);
    insert_openai_like(&mut out, "openrouter", &cfg.openrouter);
    insert_openai_like(&mut out, "together", &cfg.together);
    insert_openai_like(&mut out, "moonshot", &cfg.moonshot);

    out.insert(
        "anthropic".to_string(),
        ChatProviderCandidate {
            name: "anthropic".to_string(),
            enabled: cfg.anthropic.enabled,
            key_present: provider_key_present(
                cfg.anthropic.api_key.as_deref(),
                &cfg.anthropic.api_key_env,
            ),
            loaded: loaded.contains("anthropic"),
            key_hint: Some(cfg.anthropic.api_key_env.clone()),
        },
    );

    out.insert(
        "gemini".to_string(),
        ChatProviderCandidate {
            name: "gemini".to_string(),
            enabled: cfg.gemini.enabled,
            key_present: provider_key_present(
                cfg.gemini.api_key.as_deref(),
                &cfg.gemini.api_key_env,
            ),
            loaded: loaded.contains("gemini"),
            key_hint: Some(cfg.gemini.api_key_env.clone()),
        },
    );

    for p in &cfg.anthropic_compatible {
        out.insert(
            p.name.clone(),
            ChatProviderCandidate {
                name: p.name.clone(),
                enabled: p.enabled,
                key_present: provider_key_present(p.api_key.as_deref(), &p.api_key_env),
                loaded: loaded.contains(&p.name),
                key_hint: Some(p.api_key_env.clone()),
            },
        );
    }

    for p in &cfg.custom_openai_compatible {
        out.insert(
            p.name.clone(),
            ChatProviderCandidate {
                name: p.name.clone(),
                enabled: p.enabled,
                key_present: provider_key_present(p.api_key.as_deref(), &p.api_key_env),
                loaded: loaded.contains(&p.name),
                key_hint: Some(p.api_key_env.clone()),
            },
        );
    }

    for p in &cfg.generic_http {
        out.insert(
            p.name.clone(),
            ChatProviderCandidate {
                name: p.name.clone(),
                enabled: p.enabled,
                key_present: provider_key_present(p.api_key.as_deref(), &p.api_key_env),
                loaded: loaded.contains(&p.name),
                key_hint: Some(p.api_key_env.clone()),
            },
        );
    }

    out.into_values().collect()
}

fn provider_key_present(api_key: Option<&str>, api_key_env: &str) -> bool {
    api_key.is_some_and(|v| !v.trim().is_empty())
        || std::env::var(api_key_env)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
}

fn find_chat_provider_candidate<'a>(
    candidates: &'a [ChatProviderCandidate],
    name: &str,
) -> Option<&'a ChatProviderCandidate> {
    let n = name.trim().to_ascii_lowercase();
    candidates.iter().find(|c| c.name.eq_ignore_ascii_case(&n))
}

#[allow(clippy::too_many_arguments)]
fn chat_turn(
    line: &str,
    args: &ChatArgs,
    session_id: &str,
    store: &StateStore,
    policy: &PolicyEngine,
    tools: &ToolManager,
    providers: &ProviderManager,
    cwd: &Path,
    web_researcher: &mut Option<WebResearcher>,
    approvals: &mut Approvals,
    output_mode: OutputMode,
    auto_yes: bool,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<String> {
    let _ = store.add_note(session_id, &format!("user: {}", line));

    if let Some(rest) = line.strip_prefix("/run ") {
        if policy.config().require_confirmation_for_exec && !approvals.exec {
            approvals.exec = confirm("Allow command execution for this chat turn?", auto_yes)?;
        }
        policy.ensure_exec_allowed(*approvals, "chat-run")?;

        let parts = rest
            .split_whitespace()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            bail!("empty /run command");
        }
        let program = parts[0].clone();
        let args_vec = parts[1..].to_vec();
        let res = tools.run(ToolRunRequest {
            program,
            args: args_vec,
            cwd: Some(cwd.to_path_buf()),
            timeout_secs: None,
        })?;

        let meta = json!({
            "duration_ms": res.duration_ms,
            "exit_code": res.exit_code,
            "timed_out": res.timed_out
        });
        store.add_action(NewAction {
            session_id,
            action_type: "chat-run",
            command: &res.command_preview,
            target: None,
            status: &res.status,
            stdout: Some(&res.stdout),
            stderr: Some(&res.stderr),
            metadata: Some(&meta),
        })?;
        println!("{}", res.stdout);
        if !res.stderr.trim().is_empty() {
            eprintln!("{}", res.stderr);
        }
        return Ok(res.stdout);
    }

    if let Some(query) = line.strip_prefix("/research ") {
        let mut local_hits = search_local(query, cwd, store, Some(session_id), 5)?;
        if args.show_actions && output_mode == OutputMode::Text {
            println!("[action] local-search hits={}", local_hits.len());
        }
        let mut web_hits = Vec::new();
        if let Some(researcher) = web_researcher.as_mut() {
            if policy.config().require_confirmation_for_network && !approvals.network {
                approvals.network = confirm(
                    "Allow passive web research in chat mode (docs/public references only)?",
                    auto_yes,
                )?;
            }
            policy.ensure_network_allowed(*approvals, "public-web", None, true)?;
            web_hits = researcher.search(query, 3, store)?;
            if args.show_actions && output_mode == OutputMode::Text {
                println!("[action] web-search hits={}", web_hits.len());
            }
        }

        for hit in local_hits.iter().chain(web_hits.iter()) {
            let _ = store.add_citation(
                session_id,
                &hit.citation.source_type,
                &hit.citation.source,
                hit.citation.locator.as_deref(),
                &hit.citation.snippet,
            );
        }

        let mut response = String::new();
        for hit in local_hits.drain(..).chain(web_hits.drain(..)) {
            response.push_str(&format!(
                "- {} :: {}\n",
                hit.citation.source,
                hit.snippet.replace('\n', " ")
            ));
        }
        if response.is_empty() {
            response.push_str("No matching local/web references found.\n");
        }

        println!("{}", response);
        let _ = store.add_note(session_id, &format!("assistant: {}", response.trim()));
        return Ok(response);
    }

    let local_hits = search_local(line, cwd, store, Some(session_id), 4)?;
    if args.show_actions && output_mode == OutputMode::Text {
        println!("[action] local-context hits={}", local_hits.len());
    }

    let mut context = String::new();
    for hit in &local_hits {
        context.push_str(&format!(
            "source={} locator={:?} snippet={}\n",
            hit.citation.source, hit.citation.locator, hit.citation.snippet
        ));
    }

    let prompt = if context.is_empty() {
        line.to_string()
    } else {
        format!(
            "User prompt:\n{}\n\nRelevant local context:\n{}",
            line, context
        )
    };

    let mut stream_buf = String::new();
    let mut sink = |chunk: &str| {
        stream_buf.push_str(chunk);
        if output_mode == OutputMode::Text {
            print!("{chunk}");
            let _ = io::stdout().flush();
        }
    };

    let req = ProviderRequest {
        system: args.system.clone().or_else(|| {
            Some("You are 0x0.AI, a transparent terminal copilot. Explain exactly what you did and why.".to_string())
        }),
        prompt,
        task_type: TaskType::Reasoning,
        max_tokens: 600,
        temperature: 0.2,
        timeout_secs: 45,
        model_override: model_override.map(ToString::to_string),
    };

    if args.show_actions && output_mode == OutputMode::Text {
        let provider_name = provider_override
            .map(ToString::to_string)
            .clone()
            .unwrap_or_else(|| providers.provider_for_task(TaskType::Reasoning));
        println!("[action] provider-call provider={provider_name}");
    }

    let response = if let Some(p) = provider_override {
        providers.call_with_provider(
            p,
            req,
            if output_mode == OutputMode::Text {
                Some(&mut sink)
            } else {
                None
            },
        )?
    } else {
        providers.call(
            req,
            if output_mode == OutputMode::Text {
                Some(&mut sink)
            } else {
                None
            },
        )?
    };

    if output_mode == OutputMode::Text {
        println!();
    }

    let assistant_text = if output_mode == OutputMode::Text && !stream_buf.is_empty() {
        stream_buf
    } else {
        response.text.clone()
    };

    let meta = json!({
        "provider": response.provider,
        "model": response.model,
        "prompt_tokens_est": response.prompt_tokens_est,
        "completion_tokens_est": response.completion_tokens_est,
    });
    store.add_action(NewAction {
        session_id,
        action_type: "chat-llm",
        command: "provider.generate",
        target: None,
        status: "ok",
        stdout: Some(&assistant_text),
        stderr: None,
        metadata: Some(&meta),
    })?;
    let _ = store.add_note(session_id, &format!("assistant: {}", assistant_text));

    if output_mode == OutputMode::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "provider": response.provider,
                "model": response.model,
                "text": assistant_text
            }))?
        );
    }

    Ok(assistant_text)
}

#[allow(clippy::too_many_arguments)]
fn autonomous_chat_turn(
    goal: &str,
    args: &ChatArgs,
    session_id: &str,
    store: &StateStore,
    policy: &PolicyEngine,
    tools: &ToolManager,
    providers: &ProviderManager,
    cwd: &Path,
    web_researcher: &mut Option<WebResearcher>,
    approvals: &mut Approvals,
    output_mode: OutputMode,
    auto_yes: bool,
    flag_prefix: Option<&str>,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<String> {
    let goal = goal.trim();
    if goal.is_empty() {
        return Ok(String::new());
    }

    let _ = store.add_note(session_id, &format!("user: {}", goal));

    let local_hits = search_local(goal, cwd, store, Some(session_id), 4)?;
    if args.show_actions && output_mode == OutputMode::Text {
        println!("[agent] local-context hits={}", local_hits.len());
    }
    let mut local_context = String::new();
    for hit in &local_hits {
        local_context.push_str(&format!(
            "source={} locator={:?} snippet={}\n",
            hit.citation.source, hit.citation.locator, hit.citation.snippet
        ));
    }

    let mut observations: Vec<String> = Vec::new();
    let mut concept_context = String::new();
    let mut failed_actions = HashSet::new();
    let mut consecutive_failures = 0usize;
    let available_tools = tools
        .discover_default_tools()
        .into_iter()
        .filter(|t| t.available)
        .map(|t| t.name)
        .collect::<Vec<_>>();
    if args.show_actions && output_mode == OutputMode::Text {
        println!("[agent] available-tools={}", available_tools.join(", "));
        if let Some(prefix) = flag_prefix {
            println!("[agent] expected-flag-prefix={prefix}");
        }
    }

    for step in 1..=args.max_agent_steps {
        let prompt = build_agent_prompt(
            goal,
            cwd,
            &local_context,
            &concept_context,
            &observations,
            &available_tools,
            step,
            args.max_agent_steps,
            consecutive_failures,
            flag_prefix,
        );
        if args.show_actions && output_mode == OutputMode::Text {
            let provider_name = provider_override
                .map(ToString::to_string)
                .clone()
                .unwrap_or_else(|| providers.provider_for_task(TaskType::Reasoning));
            println!("[agent] step={step} provider={provider_name}");
        }

        let req = ProviderRequest {
            system: args.system.clone().or_else(|| {
                Some("You are 0x0.AI autonomous mode. Plan one deterministic action at a time and output strict JSON only.".to_string())
            }),
            prompt,
            task_type: TaskType::Reasoning,
            max_tokens: 700,
            temperature: 0.1,
            timeout_secs: 60,
            model_override: model_override.map(ToString::to_string),
        };

        let response = with_terminal_spinner(
            args.show_actions && output_mode == OutputMode::Text,
            "[agent] thinking",
            || {
                if let Some(p) = provider_override {
                    providers.call_with_provider(p, req, None)
                } else {
                    providers.call(req, None)
                }
            },
        )?;

        let raw_text = response.text.clone();
        let meta = json!({
            "provider": response.provider,
            "model": response.model,
            "prompt_tokens_est": response.prompt_tokens_est,
            "completion_tokens_est": response.completion_tokens_est,
            "step": step,
        });
        store.add_action(NewAction {
            session_id,
            action_type: "chat-agent-plan",
            command: "provider.generate",
            target: None,
            status: "ok",
            stdout: Some(&raw_text),
            stderr: None,
            metadata: Some(&meta),
        })?;

        let decision = match parse_agent_decision(&raw_text) {
            Some(d) => d,
            None => {
                if let Some(fallback) = synthesize_autonomous_fallback(
                    goal,
                    step,
                    args.max_agent_steps,
                    &available_tools,
                    &observations,
                    &failed_actions,
                    flag_prefix,
                ) {
                    if args.show_actions && output_mode == OutputMode::Text {
                        println!(
                            "[agent] fallback-plan active (provider returned non-JSON at step {})",
                            step
                        );
                    }
                    fallback
                } else {
                    if output_mode == OutputMode::Text {
                        println!("{}", raw_text);
                    }
                    let _ = store.add_note(session_id, &format!("assistant: {}", raw_text));
                    return Ok(raw_text);
                }
            }
        };

        let mode = if decision.mode.trim().is_empty() {
            "respond".to_string()
        } else {
            decision.mode.trim().to_ascii_lowercase()
        };
        let message = decision.message.trim().to_string();
        if !message.is_empty() && output_mode == OutputMode::Text {
            println!("{}", message);
        }

        if mode != "act" || decision.action.is_none() {
            let final_text = if message.is_empty() {
                raw_text
            } else {
                message
            };
            let _ = store.add_note(session_id, &format!("assistant: {}", final_text));
            return Ok(final_text);
        }

        let action = decision.action.as_ref().expect("action checked");
        let preview = shell_preview(&action.program, &action.args);

        if failed_actions.contains(&preview) {
            let observation = format!(
                "blocked: repeated failed action skipped to force a different strategy ({preview})"
            );
            store.add_action(NewAction {
                session_id,
                action_type: "chat-agent-run",
                command: &preview,
                target: action.host.as_deref(),
                status: "skipped",
                stdout: None,
                stderr: Some(&observation),
                metadata: Some(&json!({"step": step, "reason": "repeated-failed-action"})),
            })?;
            if output_mode == OutputMode::Text {
                println!("[agent-action] {}", observation);
            }
            observations.push(observation);
            consecutive_failures += 1;
        } else {
            let observation = execute_agent_action(
                step,
                action,
                args,
                session_id,
                store,
                policy,
                tools,
                cwd,
                approvals,
                output_mode,
                auto_yes,
                flag_prefix,
            )?;

            if is_failure_observation(&observation) {
                failed_actions.insert(preview);
                consecutive_failures += 1;
            } else {
                consecutive_failures = 0;
            }
            observations.push(observation);
        }

        if consecutive_failures >= 2 {
            let research = run_autonomous_concept_research(
                goal,
                &observations,
                args,
                session_id,
                cwd,
                store,
                policy,
                web_researcher,
                approvals,
                output_mode,
                auto_yes,
            )?;
            if !research.trim().is_empty() {
                concept_context.push_str(&research);
                if !concept_context.ends_with('\n') {
                    concept_context.push('\n');
                }
            }
            consecutive_failures = 0;
        }
    }

    let final_text = format!(
        "Reached autonomous step limit ({}). Run /auto <goal> to continue.",
        args.max_agent_steps
    );
    if output_mode == OutputMode::Text {
        println!("{}", final_text);
    }
    let _ = store.add_note(session_id, &format!("assistant: {}", final_text));
    Ok(final_text)
}

#[allow(clippy::too_many_arguments)]
fn execute_agent_action(
    step: usize,
    action: &AgentAction,
    args: &ChatArgs,
    session_id: &str,
    store: &StateStore,
    policy: &PolicyEngine,
    tools: &ToolManager,
    cwd: &Path,
    approvals: &mut Approvals,
    output_mode: OutputMode,
    auto_yes: bool,
    flag_prefix: Option<&str>,
) -> Result<String> {
    let program = action.program.trim();
    if program.is_empty() {
        return Ok("blocked: empty program".to_string());
    }

    let preview = shell_preview(program, &action.args);
    let risk = classify_action_risk(action);
    let confirm_action =
        action_needs_confirmation(args.approval_mode, risk, action.requires_network);

    if args.show_actions && output_mode == OutputMode::Text {
        println!(
            "[agent-action] step={} risk={} network={} cmd={}",
            step,
            risk.as_str(),
            action.requires_network,
            preview
        );
        if !action.reason.trim().is_empty() {
            println!("[agent-action] reason={}", action.reason.trim());
        }
    }

    if confirm_action {
        let reason_suffix = if action.reason.trim().is_empty() {
            String::new()
        } else {
            format!(" reason: {}", action.reason.trim())
        };
        let approved = confirm(
            &format!(
                "Approve {}-risk action? {}{}",
                risk.as_str(),
                preview,
                reason_suffix
            ),
            auto_yes,
        )?;
        if !approved {
            let err = "action declined by user".to_string();
            store.add_action(NewAction {
                session_id,
                action_type: "chat-agent-run",
                command: &preview,
                target: action.host.as_deref(),
                status: "blocked",
                stdout: None,
                stderr: Some(&err),
                metadata: Some(&json!({"step": step, "risk": risk.as_str(), "network": action.requires_network})),
            })?;
            if output_mode == OutputMode::Text {
                println!("[agent-action] blocked: {}", err);
            }
            return Ok(format!("blocked: {}", err));
        }
    }

    if policy.config().require_confirmation_for_exec && !approvals.exec {
        approvals.exec = confirm("Allow command execution for autonomous mode?", auto_yes)?;
    }
    if let Err(err) = policy.ensure_exec_allowed(*approvals, program) {
        let err_text = err.to_string();
        store.add_action(NewAction {
            session_id,
            action_type: "chat-agent-run",
            command: &preview,
            target: action.host.as_deref(),
            status: "blocked",
            stdout: None,
            stderr: Some(&err_text),
            metadata: Some(
                &json!({"step": step, "risk": risk.as_str(), "network": action.requires_network}),
            ),
        })?;
        if output_mode == OutputMode::Text {
            println!("[agent-action] blocked: {}", err_text);
        }
        return Ok(format!("blocked: {}", err_text));
    }

    if action.requires_network {
        if policy.config().require_confirmation_for_network && !approvals.network {
            approvals.network = confirm("Allow network access for autonomous mode?", auto_yes)?;
        }
        let (inferred_host, inferred_port) = infer_host_port_from_args(&action.args);
        let host = action
            .host
            .clone()
            .or(inferred_host)
            .unwrap_or_else(|| "unknown-host".to_string());
        let port = action.port.or(inferred_port);
        if let Err(err) = policy.ensure_network_allowed(*approvals, &host, port, false) {
            let err_text = err.to_string();
            store.add_action(NewAction {
                session_id,
                action_type: "chat-agent-run",
                command: &preview,
                target: Some(&host),
                status: "blocked",
                stdout: None,
                stderr: Some(&err_text),
                metadata: Some(
                    &json!({"step": step, "risk": risk.as_str(), "network": true, "host": host, "port": port}),
                ),
            })?;
            if output_mode == OutputMode::Text {
                println!("[agent-action] blocked: {}", err_text);
            }
            return Ok(format!("blocked: {}", err_text));
        }
    }

    let max_timeout = policy.config().max_runtime_per_action_secs.max(1);
    let timeout_secs = action
        .timeout_secs
        .filter(|v| *v > 0)
        .map(|v| v.min(max_timeout))
        .unwrap_or(max_timeout);

    let run = with_terminal_spinner(
        args.show_actions && output_mode == OutputMode::Text,
        "[agent-action] running",
        || {
            tools.run(ToolRunRequest {
                program: program.to_string(),
                args: action.args.clone(),
                cwd: Some(cwd.to_path_buf()),
                timeout_secs: Some(timeout_secs),
            })
        },
    );

    match run {
        Ok(res) => {
            let meta = json!({
                "step": step,
                "risk": risk.as_str(),
                "network": action.requires_network,
                "exit_code": res.exit_code,
                "duration_ms": res.duration_ms,
                "timed_out": res.timed_out,
                "timeout_secs": timeout_secs,
            });
            store.add_action(NewAction {
                session_id,
                action_type: "chat-agent-run",
                command: &res.command_preview,
                target: action.host.as_deref(),
                status: &res.status,
                stdout: Some(&res.stdout),
                stderr: Some(&res.stderr),
                metadata: Some(&meta),
            })?;

            if output_mode == OutputMode::Text {
                if !res.stdout.trim().is_empty() {
                    println!("{}", res.stdout);
                }
                if !res.stderr.trim().is_empty() {
                    eprintln!("{}", res.stderr);
                }
            }

            let matched_flags =
                extract_flags_from_text(&format!("{}\n{}", res.stdout, res.stderr), flag_prefix);
            if !matched_flags.is_empty() {
                let joined = matched_flags.join(", ");
                let _ = store.add_note(session_id, &format!("candidate-flags: {}", joined));
                if output_mode == OutputMode::Text {
                    println!("[agent-flag] {}", joined);
                }
            }

            let mut observation = format!(
                "{} -> status={} exit={:?} stdout={} stderr={}",
                res.command_preview,
                res.status,
                res.exit_code,
                truncate_for_prompt(&res.stdout, 1000),
                truncate_for_prompt(&res.stderr, 500)
            );
            if !matched_flags.is_empty() {
                observation.push_str(&format!(" candidate_flags={}", matched_flags.join(",")));
            }
            Ok(observation)
        }
        Err(err) => {
            let err_text = err.to_string();
            store.add_action(NewAction {
                session_id,
                action_type: "chat-agent-run",
                command: &preview,
                target: action.host.as_deref(),
                status: "error",
                stdout: None,
                stderr: Some(&err_text),
                metadata: Some(&json!({"step": step, "risk": risk.as_str(), "network": action.requires_network})),
            })?;
            if output_mode == OutputMode::Text {
                eprintln!("[agent-action] error: {}", err_text);
            }
            Ok(format!("error: {}", err_text))
        }
    }
}

fn is_failure_observation(observation: &str) -> bool {
    let lower = observation.to_ascii_lowercase();
    lower.starts_with("blocked:")
        || lower.starts_with("error:")
        || lower.contains("status=error")
        || lower.contains("status=timeout")
}

#[allow(clippy::too_many_arguments)]
fn run_autonomous_concept_research(
    goal: &str,
    observations: &[String],
    args: &ChatArgs,
    session_id: &str,
    cwd: &Path,
    store: &StateStore,
    policy: &PolicyEngine,
    web_researcher: &mut Option<WebResearcher>,
    approvals: &mut Approvals,
    output_mode: OutputMode,
    auto_yes: bool,
) -> Result<String> {
    let queries = build_concept_queries(goal, observations);
    let mut context = String::new();

    for query in queries.into_iter().take(2) {
        let local_hits = search_local(&query, cwd, store, Some(session_id), 2).unwrap_or_default();
        let mut web_hits = Vec::new();

        if args.web {
            if let Some(researcher) = web_researcher.as_mut() {
                if policy.config().require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        "Allow autonomous web concept research (docs/public references only)?",
                        auto_yes,
                    )?;
                }
                if policy
                    .ensure_network_allowed(*approvals, "public-web", None, true)
                    .is_ok()
                {
                    web_hits = researcher.search(&query, 2, store).unwrap_or_default();
                }
            }
        }

        if args.show_actions && output_mode == OutputMode::Text {
            println!(
                "[agent-research] query='{}' local_hits={} web_hits={}",
                query,
                local_hits.len(),
                web_hits.len()
            );
        }

        for hit in local_hits.iter().chain(web_hits.iter()) {
            let _ = store.add_citation(
                session_id,
                &hit.citation.source_type,
                &hit.citation.source,
                hit.citation.locator.as_deref(),
                &hit.citation.snippet,
            );
        }

        if !local_hits.is_empty() || !web_hits.is_empty() {
            context.push_str(&format!("query={}\n", query));
            for hit in local_hits.iter().chain(web_hits.iter()).take(4) {
                context.push_str(&format!(
                    "- {} :: {}\n",
                    hit.citation.source,
                    hit.snippet.replace('\n', " ")
                ));
            }
        }
    }

    if !context.trim().is_empty() {
        let _ = store.add_note(
            session_id,
            &format!(
                "agent-concept-research: {}",
                truncate_for_prompt(&context, 700)
            ),
        );
    }

    Ok(context)
}

fn build_concept_queries(goal: &str, observations: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let goal = goal.trim();
    if !goal.is_empty() {
        out.push(goal.to_string());
        out.push(format!("{goal} ctf writeup strategy"));
    }

    if let Some(last) = observations.last() {
        let fragment = last
            .split_whitespace()
            .filter(|t| {
                t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./:".contains(c))
            })
            .take(10)
            .collect::<Vec<_>>()
            .join(" ");
        if !fragment.is_empty() {
            out.push(format!("{goal} {}", fragment));
        }
    }

    let mut dedup = HashSet::new();
    out.into_iter()
        .filter(|q| dedup.insert(q.clone()))
        .collect()
}

fn build_agent_prompt(
    goal: &str,
    cwd: &Path,
    local_context: &str,
    concept_context: &str,
    observations: &[String],
    available_tools: &[String],
    step: usize,
    max_steps: usize,
    stuck_score: usize,
    flag_prefix: Option<&str>,
) -> String {
    let mut prompt = format!(
        "Goal:\n{}\n\nCurrent working directory:\n{}\n\nStep: {}/{}\n",
        goal,
        cwd.display(),
        step,
        max_steps
    );

    if !local_context.trim().is_empty() {
        prompt.push_str("\nRelevant local context:\n");
        prompt.push_str(local_context);
    }

    if !observations.is_empty() {
        prompt.push_str("\nPrior observations:\n");
        for obs in observations.iter().rev().take(6).rev() {
            prompt.push_str("- ");
            prompt.push_str(obs);
            prompt.push('\n');
        }
    }

    if !available_tools.is_empty() {
        prompt.push_str("\nAvailable tools (installed):\n");
        prompt.push_str(&available_tools.join(", "));
        prompt.push('\n');
    }

    if let Some(prefix) = flag_prefix {
        prompt.push_str(&format!(
            "\nExpected flag prefix:\n{}\nTreat matching outputs as high-priority candidates.\n",
            prefix
        ));
    }

    if !concept_context.trim().is_empty() {
        prompt.push_str("\nConcept research context:\n");
        prompt.push_str(concept_context);
    }

    if stuck_score > 0 {
        prompt.push_str(&format!(
            "\nStuck indicator: {} recent failed/blocked attempts.\n",
            stuck_score
        ));
    }

    prompt.push_str(
        "\nReturn exactly one raw JSON object with this schema:\n\
{\"mode\":\"act|respond|done\",\"message\":\"short user-facing text\",\"action\":{\"program\":\"command\",\"args\":[\"arg1\"],\"reason\":\"why this action\",\"risk\":\"low|medium|high\",\"requires_network\":false,\"host\":null,\"port\":null,\"timeout_secs\":60}}\n\n\
Rules:\n\
- Use mode=\"act\" only when an action is needed.\n\
- Propose one action at a time.\n\
- Prefer deterministic local commands.\n\
- Use available tools autonomously; do not wait for explicit tool names from user.\n\
- Observe behavior from each command result and adapt the next action to that behavior.\n\
- If recent attempts failed, switch strategy family and do not repeat failed commands.\n\
- Think outside the obvious path: alternative tools, alternate assumptions, and verification steps.\n\
- Never propose destructive commands.\n\
- Output raw JSON only (no markdown or prose outside JSON).\n",
    );

    prompt
}

fn parse_agent_decision(raw: &str) -> Option<AgentDecision> {
    let mut candidates = Vec::new();
    candidates.push(raw.trim().to_string());

    if let Some(block) = extract_fenced_json(raw) {
        candidates.push(block);
    }

    if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}'))
        && start < end
    {
        candidates.push(raw[start..=end].trim().to_string());
    }

    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<AgentDecision>(&candidate) {
            return Some(parsed);
        }
    }
    None
}

fn extract_fenced_json(raw: &str) -> Option<String> {
    let start = raw.find("```")?;
    let rest = &raw[start + 3..];
    let end = rest.find("```")?;
    let mut block = rest[..end].trim().to_string();
    let lower = block.to_ascii_lowercase();
    if lower.starts_with("json") {
        block = block[4..].trim().to_string();
    }
    if block.is_empty() { None } else { Some(block) }
}

fn action_needs_confirmation(
    mode: ChatApprovalMode,
    risk: AgentRisk,
    requires_network: bool,
) -> bool {
    match mode {
        ChatApprovalMode::All => true,
        ChatApprovalMode::Risky => requires_network || !matches!(risk, AgentRisk::Low),
    }
}

fn classify_action_risk(action: &AgentAction) -> AgentRisk {
    if let Some(parsed) = AgentRisk::from_label(&action.risk) {
        return parsed;
    }

    let program = action.program.to_ascii_lowercase();
    let joined = format!("{} {}", program, action.args.join(" ").to_ascii_lowercase());

    let high_risk = ["rm", "mkfs", "dd", "shutdown", "reboot", "poweroff", "halt"];
    if high_risk.iter().any(|x| program == *x) || joined.contains("rm -rf") {
        return AgentRisk::High;
    }

    let medium_risk = [
        "curl", "wget", "nc", "ncat", "nmap", "ffuf", "sqlmap", "nikto", "http", "ssh", "scp",
        "ftp", "telnet",
    ];
    if action.requires_network || medium_risk.iter().any(|x| program == *x) {
        return AgentRisk::Medium;
    }

    AgentRisk::Low
}

fn infer_host_port_from_args(args: &[String]) -> (Option<String>, Option<u16>) {
    for arg in args {
        if let Ok(url) = url::Url::parse(arg) {
            let host = url.host_str().map(ToString::to_string);
            let port = url.port_or_known_default();
            if host.is_some() {
                return (host, port);
            }
        }
    }
    (None, None)
}

fn extract_flag_prefix_hint(text: &str) -> Option<String> {
    let prefixed_flag = Regex::new(r"(?i)\b([a-z][a-z0-9_-]{1,24})\{").expect("prefix regex");
    if let Some(cap) = prefixed_flag.captures(text)
        && let Some(m) = cap.get(1)
    {
        return Some(m.as_str().to_string());
    }

    let stated_prefix = Regex::new(
        r"(?i)\b(?:flag\s*prefix|prefix|flag\s*format)\b\s*(?:is|=|:)?\s*([a-z][a-z0-9_-]{1,24})\b",
    )
    .expect("stated prefix regex");
    if let Some(cap) = stated_prefix.captures(text)
        && let Some(m) = cap.get(1)
    {
        let candidate = m.as_str().to_ascii_lowercase();
        if !matches!(
            candidate.as_str(),
            "flag" | "prefix" | "format" | "is" | "the"
        ) {
            return Some(m.as_str().to_string());
        }
    }

    None
}

fn synthesize_autonomous_fallback(
    goal: &str,
    step: usize,
    max_steps: usize,
    available_tools: &[String],
    observations: &[String],
    failed_actions: &HashSet<String>,
    flag_prefix: Option<&str>,
) -> Option<AgentDecision> {
    let goal_lc = goal.to_ascii_lowercase();
    let has_tool = |name: &str| available_tools.iter().any(|t| t == name);

    let mut candidates: Vec<AgentAction> = Vec::new();
    candidates.push(AgentAction {
        program: "ls".to_string(),
        args: vec!["-la".to_string()],
        reason: "Map current challenge directory structure".to_string(),
        risk: "low".to_string(),
        requires_network: false,
        host: None,
        port: None,
        timeout_secs: Some(20),
    });

    if has_tool("rg") {
        candidates.push(AgentAction {
            program: "rg".to_string(),
            args: vec!["--files".to_string(), ".".to_string()],
            reason: "List candidate files quickly for challenge triage".to_string(),
            risk: "low".to_string(),
            requires_network: false,
            host: None,
            port: None,
            timeout_secs: Some(20),
        });
    }

    candidates.push(AgentAction {
        program: "find".to_string(),
        args: vec![
            ".".to_string(),
            "-maxdepth".to_string(),
            "3".to_string(),
            "-type".to_string(),
            "f".to_string(),
        ],
        reason: "Enumerate challenge artifacts recursively".to_string(),
        risk: "low".to_string(),
        requires_network: false,
        host: None,
        port: None,
        timeout_secs: Some(20),
    });

    if has_tool("rg") {
        let pattern = if goal_lc.contains("rsa") || goal_lc.contains("crypto") {
            "(rsa|n\\s*=|e\\s*=|c\\s*=|phi|mod|flag|ctf)"
        } else if goal_lc.contains("pwn") || goal_lc.contains("overflow") {
            "(flag|win|system\\(|gets\\(|strcpy|canary|rop)"
        } else if goal_lc.contains("web") {
            "(route|endpoint|auth|jwt|token|cookie|flag)"
        } else if goal_lc.contains("rev") || goal_lc.contains("reverse") {
            "(flag|check|verify|decrypt|xor|main)"
        } else {
            "(flag|ctf|secret|password|token)"
        };
        candidates.push(AgentAction {
            program: "rg".to_string(),
            args: vec!["-n".to_string(), pattern.to_string(), ".".to_string()],
            reason: "Extract high-signal strings and flag clues from files".to_string(),
            risk: "low".to_string(),
            requires_network: false,
            host: None,
            port: None,
            timeout_secs: Some(30),
        });
    }

    if let Some(prefix) = flag_prefix
        && has_tool("rg")
        && !prefix.trim().is_empty()
    {
        let pattern = format!("{}\\{{", regex::escape(prefix.trim()));
        candidates.push(AgentAction {
            program: "rg".to_string(),
            args: vec!["-n".to_string(), pattern, ".".to_string()],
            reason: "Prioritize expected flag prefix candidates from user context".to_string(),
            risk: "low".to_string(),
            requires_network: false,
            host: None,
            port: None,
            timeout_secs: Some(20),
        });
    }

    if has_tool("python3") {
        let script = r#"import os,stat,subprocess
files=[]
for b,_,ns in os.walk('.'):
  for n in ns:
    p=os.path.join(b,n); files.append(p)
    if len(files)>=200: break
  if len(files)>=200: break
print('[behavior] files=',len(files))
shown=0
for p in files:
  try:
    st=os.stat(p)
    if st.st_mode & stat.S_IXUSR:
      for a in ([],['--help'],['-h']):
        try:
          r=subprocess.run([p]+a,capture_output=True,text=True,timeout=2)
          o=(r.stdout or r.stderr or '').strip().replace('\n',' ')[:140]
          print('[behavior]',p,'args=',a if a else ['<none>'],'code=',r.returncode,'out=',o)
          shown+=1
        except Exception as e:
          print('[behavior]',p,'args=',a if a else ['<none>'],'err=',e)
        if shown>=6: break
      if shown>=6: break
  except Exception:
    pass
"#;
        candidates.push(AgentAction {
            program: "python3".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            reason: "Observe executable behavior to guide next exploit hypothesis".to_string(),
            risk: "low".to_string(),
            requires_network: false,
            host: None,
            port: None,
            timeout_secs: Some(45),
        });
    }

    let next = candidates.into_iter().find(|a| {
        let preview = shell_preview(&a.program, &a.args);
        !failed_actions.contains(&preview) && !observations.iter().any(|o| o.contains(&preview))
    });

    let action = next?;
    Some(AgentDecision {
        mode: "act".to_string(),
        message: format!(
            "Using fallback autonomous planner (step {}/{}): {}",
            step, max_steps, action.reason
        ),
        action: Some(action),
    })
}

fn extract_flags_from_text(text: &str, flag_prefix: Option<&str>) -> Vec<String> {
    let generic_re =
        Regex::new(r"(?i)([a-z0-9_\-]{2,16}\{[^\n\r\}]{1,180}\})").expect("flag regex");

    let mut out = BTreeMap::new();
    if let Some(prefix) = flag_prefix
        && !prefix.trim().is_empty()
    {
        let escaped = regex::escape(prefix.trim());
        let pattern = format!(r"(?i)({}\{{[^\n\r\}}]{{1,220}}\}})", escaped);
        if let Ok(prefixed_re) = Regex::new(&pattern) {
            for cap in prefixed_re.captures_iter(text) {
                if let Some(m) = cap.get(1) {
                    out.insert(m.as_str().to_ascii_lowercase(), m.as_str().to_string());
                }
            }
        }
    }

    for cap in generic_re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            out.insert(m.as_str().to_ascii_lowercase(), m.as_str().to_string());
        }
    }
    out.into_values().collect()
}

fn truncate_for_prompt(input: &str, max_chars: usize) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() <= max_chars {
        return trimmed.to_string();
    }
    format!("{}...[truncated]", &trimmed[..max_chars])
}

fn with_terminal_spinner<T, F>(enabled: bool, label: &str, op: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    if !enabled {
        return op();
    }

    let done = Arc::new(AtomicBool::new(false));
    let done_bg = Arc::clone(&done);
    let label = label.to_string();

    let handle = thread::spawn(move || {
        let frames = ["|", "/", "-", "\\"];
        let mut idx = 0usize;
        while !done_bg.load(Ordering::Relaxed) {
            print!("\r{} {}", label, frames[idx % frames.len()]);
            let _ = io::stdout().flush();
            idx = idx.wrapping_add(1);
            thread::sleep(Duration::from_millis(90));
        }
        print!("\r{} done    \n", label);
        let _ = io::stdout().flush();
    });

    let result = op();
    done.store(true, Ordering::Relaxed);
    let _ = handle.join();
    result
}

fn configure_provider(cfg: &mut AppConfig, args: &ProvidersConfigureArgs) -> Result<()> {
    let name = args.provider.to_ascii_lowercase();
    match name.as_str() {
        "openai" => configure_openai_compat(&mut cfg.providers.openai, args),
        "openrouter" => configure_openai_compat(&mut cfg.providers.openrouter, args),
        "together" => configure_openai_compat(&mut cfg.providers.together, args),
        "moonshot" => configure_openai_compat(&mut cfg.providers.moonshot, args),
        "anthropic" => {
            if let Some(v) = args.api_key.clone() {
                cfg.providers.anthropic.api_key = Some(v);
            }
            if let Some(v) = args.api_key_env.clone() {
                cfg.providers.anthropic.api_key_env = v;
            }
            if let Some(v) = args.model.clone() {
                cfg.providers.anthropic.default_model = v;
            }
            if let Some(v) = args.base_url.clone() {
                cfg.providers.anthropic.base_url = v;
            }
            if args.enable {
                cfg.providers.anthropic.enabled = true;
            }
            if args.disable {
                cfg.providers.anthropic.enabled = false;
            }
            Ok(())
        }
        "gemini" => {
            if let Some(v) = args.api_key.clone() {
                cfg.providers.gemini.api_key = Some(v);
            }
            if let Some(v) = args.api_key_env.clone() {
                cfg.providers.gemini.api_key_env = v;
            }
            if let Some(v) = args.model.clone() {
                cfg.providers.gemini.default_model = v;
            }
            if let Some(v) = args.base_url.clone() {
                cfg.providers.gemini.base_url = v;
            }
            if args.enable {
                cfg.providers.gemini.enabled = true;
            }
            if args.disable {
                cfg.providers.gemini.enabled = false;
            }
            Ok(())
        }
        other => {
            if let Some(p) = cfg
                .providers
                .custom_openai_compatible
                .iter_mut()
                .find(|p| p.name.eq_ignore_ascii_case(other))
            {
                if let Some(v) = args.api_key.clone() {
                    p.api_key = Some(v);
                }
                if let Some(v) = args.api_key_env.clone() {
                    p.api_key_env = v;
                }
                if let Some(v) = args.model.clone() {
                    p.default_model = v;
                }
                if let Some(v) = args.base_url.clone() {
                    p.base_url = v;
                }
                if args.enable {
                    p.enabled = true;
                }
                if args.disable {
                    p.enabled = false;
                }
                return Ok(());
            }

            if let Some(p) = cfg
                .providers
                .anthropic_compatible
                .iter_mut()
                .find(|p| p.name.eq_ignore_ascii_case(other))
            {
                if let Some(v) = args.api_key.clone() {
                    p.api_key = Some(v);
                }
                if let Some(v) = args.api_key_env.clone() {
                    p.api_key_env = v;
                }
                if let Some(v) = args.model.clone() {
                    p.default_model = v;
                }
                if let Some(v) = args.base_url.clone() {
                    p.base_url = v;
                }
                if args.enable {
                    p.enabled = true;
                }
                if args.disable {
                    p.enabled = false;
                }
                return Ok(());
            }

            if let Some(p) = cfg
                .providers
                .generic_http
                .iter_mut()
                .find(|p| p.name.eq_ignore_ascii_case(other))
            {
                if let Some(v) = args.api_key.clone() {
                    p.api_key = Some(v);
                }
                if let Some(v) = args.api_key_env.clone() {
                    p.api_key_env = v;
                }
                if let Some(v) = args.model.clone() {
                    p.default_model = v;
                }
                if let Some(v) = args.base_url.clone() {
                    p.base_url = v;
                }
                if args.enable {
                    p.enabled = true;
                }
                if args.disable {
                    p.enabled = false;
                }
                return Ok(());
            }

            let compat = args.compat.clone().unwrap_or(ProviderCompat::Openai);
            let base_url = args.base_url.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown provider '{other}'; pass --base-url and optionally --compat openai|anthropic|generic"
                )
            })?;
            let api_key_env = args.api_key_env.clone().unwrap_or_else(|| {
                format!("{}_API_KEY", other.to_ascii_uppercase().replace('-', "_"))
            });

            match compat {
                ProviderCompat::Openai => cfg.providers.custom_openai_compatible.push(
                    crate::config::NamedOpenAiCompatProvider {
                        name: other.to_string(),
                        enabled: !args.disable,
                        base_url,
                        api_key_env,
                        api_key: args.api_key.clone(),
                        default_model: args
                            .model
                            .clone()
                            .unwrap_or_else(|| "gpt-4.1-mini".to_string()),
                    },
                ),
                ProviderCompat::Anthropic => {
                    cfg.providers
                        .anthropic_compatible
                        .push(crate::config::AnthropicProvider {
                            name: other.to_string(),
                            enabled: !args.disable,
                            base_url,
                            api_key_env,
                            api_key: args.api_key.clone(),
                            default_model: args
                                .model
                                .clone()
                                .unwrap_or_else(|| "claude-3-5-sonnet-latest".to_string()),
                        })
                }
                ProviderCompat::Generic => {
                    cfg.providers
                        .generic_http
                        .push(crate::config::GenericHttpProvider {
                            name: other.to_string(),
                            enabled: !args.disable,
                            base_url,
                            api_key_env,
                            api_key: args.api_key.clone(),
                            default_model: args
                                .model
                                .clone()
                                .unwrap_or_else(|| "generic-default".to_string()),
                        })
                }
            }
            Ok(())
        }
    }
}

fn configure_openai_compat(
    provider: &mut crate::config::OpenAiCompatProvider,
    args: &ProvidersConfigureArgs,
) -> Result<()> {
    if let Some(v) = args.api_key.clone() {
        provider.api_key = Some(v);
    }
    if let Some(v) = args.api_key_env.clone() {
        provider.api_key_env = v;
    }
    if let Some(v) = args.model.clone() {
        provider.default_model = v;
    }
    if let Some(v) = args.base_url.clone() {
        provider.base_url = v;
    }
    if args.enable {
        provider.enabled = true;
    }
    if args.disable {
        provider.enabled = false;
    }
    Ok(())
}

fn set_route(cfg: &mut AppConfig, route: RouteTask, provider: &str, model: Option<String>) {
    let target = match route {
        RouteTask::Reasoning => &mut cfg.model_routing.reasoning,
        RouteTask::Coding => &mut cfg.model_routing.coding,
        RouteTask::Summarization => &mut cfg.model_routing.summarization,
        RouteTask::Vision => &mut cfg.model_routing.vision,
        RouteTask::Classification => &mut cfg.model_routing.classification,
    };
    target.provider = provider.to_string();
    if model.is_some() {
        target.model = model;
    }
}

fn bind_default_chat_routes(cfg: &mut AppConfig, provider: &str, model: Option<String>) {
    let model_clone = model.clone();
    if cfg
        .model_routing
        .reasoning
        .provider
        .eq_ignore_ascii_case("local")
    {
        cfg.model_routing.reasoning.provider = provider.to_string();
        if model_clone.is_some() {
            cfg.model_routing.reasoning.model = model_clone.clone();
        }
    }
    if cfg
        .model_routing
        .coding
        .provider
        .eq_ignore_ascii_case("local")
    {
        cfg.model_routing.coding.provider = provider.to_string();
        if model_clone.is_some() {
            cfg.model_routing.coding.model = model_clone.clone();
        }
    }
    if cfg
        .model_routing
        .summarization
        .provider
        .eq_ignore_ascii_case("local")
    {
        cfg.model_routing.summarization.provider = provider.to_string();
        if model_clone.is_some() {
            cfg.model_routing.summarization.model = model_clone;
        }
    }
}

fn redacted_provider_view(cfg: &AppConfig, provider: &str) -> serde_json::Value {
    let p = provider.to_ascii_lowercase();
    match p.as_str() {
        "openai" => json!({
            "provider": "openai",
            "enabled": cfg.providers.openai.enabled,
            "base_url": cfg.providers.openai.base_url,
            "api_key_env": cfg.providers.openai.api_key_env,
            "api_key_configured": cfg.providers.openai.api_key.as_deref().is_some_and(|v| !v.is_empty()),
            "default_model": cfg.providers.openai.default_model,
        }),
        "openrouter" => json!({
            "provider": "openrouter",
            "enabled": cfg.providers.openrouter.enabled,
            "base_url": cfg.providers.openrouter.base_url,
            "api_key_env": cfg.providers.openrouter.api_key_env,
            "api_key_configured": cfg.providers.openrouter.api_key.as_deref().is_some_and(|v| !v.is_empty()),
            "default_model": cfg.providers.openrouter.default_model,
        }),
        "together" => json!({
            "provider": "together",
            "enabled": cfg.providers.together.enabled,
            "base_url": cfg.providers.together.base_url,
            "api_key_env": cfg.providers.together.api_key_env,
            "api_key_configured": cfg.providers.together.api_key.as_deref().is_some_and(|v| !v.is_empty()),
            "default_model": cfg.providers.together.default_model,
        }),
        "moonshot" => json!({
            "provider": "moonshot",
            "enabled": cfg.providers.moonshot.enabled,
            "base_url": cfg.providers.moonshot.base_url,
            "api_key_env": cfg.providers.moonshot.api_key_env,
            "api_key_configured": cfg.providers.moonshot.api_key.as_deref().is_some_and(|v| !v.is_empty()),
            "default_model": cfg.providers.moonshot.default_model,
        }),
        "anthropic" => json!({
            "provider": "anthropic",
            "enabled": cfg.providers.anthropic.enabled,
            "base_url": cfg.providers.anthropic.base_url,
            "api_key_env": cfg.providers.anthropic.api_key_env,
            "api_key_configured": cfg.providers.anthropic.api_key.as_deref().is_some_and(|v| !v.is_empty()),
            "default_model": cfg.providers.anthropic.default_model,
        }),
        "gemini" => json!({
            "provider": "gemini",
            "enabled": cfg.providers.gemini.enabled,
            "base_url": cfg.providers.gemini.base_url,
            "api_key_env": cfg.providers.gemini.api_key_env,
            "api_key_configured": cfg.providers.gemini.api_key.as_deref().is_some_and(|v| !v.is_empty()),
            "default_model": cfg.providers.gemini.default_model,
        }),
        other => {
            if let Some(p) = cfg
                .providers
                .custom_openai_compatible
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(other))
            {
                return json!({
                    "provider": p.name,
                    "compat": "openai",
                    "enabled": p.enabled,
                    "base_url": p.base_url,
                    "api_key_env": p.api_key_env,
                    "api_key_configured": p.api_key.as_deref().is_some_and(|v| !v.is_empty()),
                    "default_model": p.default_model,
                });
            }
            if let Some(p) = cfg
                .providers
                .anthropic_compatible
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(other))
            {
                json!({
                    "provider": p.name,
                    "compat": "anthropic",
                    "enabled": p.enabled,
                    "base_url": p.base_url,
                    "api_key_env": p.api_key_env,
                    "api_key_configured": p.api_key.as_deref().is_some_and(|v| !v.is_empty()),
                    "default_model": p.default_model,
                })
            } else if let Some(p) = cfg
                .providers
                .generic_http
                .iter()
                .find(|x| x.name.eq_ignore_ascii_case(other))
            {
                json!({
                    "provider": p.name,
                    "compat": "generic",
                    "enabled": p.enabled,
                    "base_url": p.base_url,
                    "api_key_env": p.api_key_env,
                    "api_key_configured": p.api_key.as_deref().is_some_and(|v| !v.is_empty()),
                    "default_model": p.default_model,
                })
            } else {
                json!({"provider": provider, "status": "unknown"})
            }
        }
    }
}

fn prompt_required(prompt: &str) -> Result<String> {
    loop {
        let value = prompt_optional(prompt)?;
        if let Some(v) = value {
            if !v.trim().is_empty() {
                return Ok(v);
            }
        }
        println!("A value is required.");
    }
}

fn prompt_optional(prompt: &str) -> Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

fn parse_compat(value: &str) -> Option<ProviderCompat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai-compatible" | "openai_compatible" => Some(ProviderCompat::Openai),
        "anthropic" | "anthropic-compatible" | "anthropic_compatible" => {
            Some(ProviderCompat::Anthropic)
        }
        "generic" | "http" | "rest" => Some(ProviderCompat::Generic),
        _ => None,
    }
}

fn ensure_example_plugin(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;

    let manifest = dir.join("extract_flag.toml");
    let script = dir.join("extract_flag.py");

    if !script.exists() {
        fs::write(
            &script,
            r#"#!/usr/bin/env python3
import re
import sys

if len(sys.argv) < 2:
    print("usage: extract_flag.py <file>")
    sys.exit(1)

with open(sys.argv[1], "r", errors="ignore") as f:
    text = f.read()

m = re.findall(r"[A-Za-z0-9_\-]{2,16}\{[^\n\r\}]{1,180}\}", text)
for x in m:
    print(x)
"#,
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms)?;
        }
    }

    if !manifest.exists() {
        let content = format!(
            "name = \"extract-flag\"\ndescription = \"Extract CTF-style flag patterns from text files\"\ncommand = \"python3\"\nargs = [\"{}\"]\ncategories = [\"misc\", \"forensics\", \"reverse\"]\n",
            script.display()
        );
        fs::write(manifest, content)?;
    }

    Ok(())
}

fn resolve_output_mode(cli: &Cli, runtime: &RuntimeConfig) -> OutputMode {
    match cli.output {
        Some(OutputFormat::Json) => OutputMode::Json,
        Some(OutputFormat::Text) => OutputMode::Text,
        None => {
            if cli.json || runtime.config.general.default_json_output {
                OutputMode::Json
            } else {
                OutputMode::Text
            }
        }
    }
}

fn ensure_session(store: &StateStore, session_id: Option<&str>, root: &Path) -> Result<String> {
    match session_id {
        Some(id) => {
            if store.get_session(id)?.is_none() {
                store.create_session(id, &root.display().to_string())?;
            }
            Ok(id.to_string())
        }
        None => {
            let id = Uuid::new_v4().to_string();
            store.create_session(&id, &root.display().to_string())?;
            Ok(id)
        }
    }
}

fn ensure_session_label(
    store: &StateStore,
    session_id: Option<&str>,
    root_label: &str,
) -> Result<String> {
    match session_id {
        Some(id) => {
            if store.get_session(id)?.is_none() {
                store.create_session(id, root_label)?;
            }
            Ok(id.to_string())
        }
        None => {
            let id = Uuid::new_v4().to_string();
            store.create_session(&id, root_label)?;
            Ok(id)
        }
    }
}

fn collect_challenge_paths(root: &Path, max: usize) -> Result<Vec<PathBuf>> {
    if root.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }
    if !root.exists() {
        bail!("path does not exist: {}", root.display());
    }

    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for entry in walkdir::WalkDir::new(root)
        .max_depth(3)
        .into_iter()
        .filter_map(Result::ok)
    {
        if out.len() >= max {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(parent) = path.parent() {
            let parent = parent.to_path_buf();
            let key = parent.display().to_string();
            if seen.insert(key) {
                out.push(parent);
            }
        }
    }

    if out.is_empty() {
        out.push(root.to_path_buf());
    }
    Ok(out)
}

fn parse_category(s: &str) -> Option<ChallengeCategory> {
    match s {
        "crypto" => Some(ChallengeCategory::Crypto),
        "pwn" => Some(ChallengeCategory::Pwn),
        "rev" => Some(ChallengeCategory::Reverse),
        "reverse" => Some(ChallengeCategory::Reverse),
        "web" => Some(ChallengeCategory::Web),
        "misc" => Some(ChallengeCategory::Misc),
        "forensics" => Some(ChallengeCategory::Forensics),
        "forensic" => Some(ChallengeCategory::Forensics),
        "stego" => Some(ChallengeCategory::Stego),
        "osint" => Some(ChallengeCategory::Osint),
        "mobile" => Some(ChallengeCategory::Mobile),
        "hardware" => Some(ChallengeCategory::Hardware),
        "blockchain" | "chain" => Some(ChallengeCategory::Blockchain),
        "cloud" => Some(ChallengeCategory::Cloud),
        "network" | "net" => Some(ChallengeCategory::Network),
        "ai" | "ml" | "llm" => Some(ChallengeCategory::Ai),
        "unknown" => Some(ChallengeCategory::Unknown),
        _ => None,
    }
}

fn normalize_category_filter(raw: Option<&str>) -> Result<Option<String>> {
    let Some(value) = raw else {
        return Ok(None);
    };
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Ok(None);
    }

    let canonical = match normalized.as_str() {
        "reverse" => "rev",
        "forensic" => "forensics",
        "chain" => "blockchain",
        "net" => "network",
        "ml" | "llm" => "ai",
        _ => normalized.as_str(),
    };
    if parse_category(canonical).is_none() {
        bail!("unsupported category '{}'", value.trim());
    }
    Ok(Some(canonical.to_string()))
}

fn emit<T: Serialize>(mode: OutputMode, payload: &T, text: &str) -> Result<()> {
    match mode {
        OutputMode::Text => {
            println!("{text}");
        }
        OutputMode::Json => {
            println!("{}", serde_json::to_string_pretty(payload)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_json_from_fenced_block() {
        let raw = "```json\n{\"mode\":\"act\",\"message\":\"triage\",\"action\":{\"program\":\"rg\",\"args\":[\"-n\",\"flag\",\".\"],\"risk\":\"low\"}}\n```";
        let parsed = parse_agent_decision(raw).expect("agent decision");
        assert_eq!(parsed.mode, "act");
        let action = parsed.action.expect("action");
        assert_eq!(action.program, "rg");
        assert_eq!(action.args, vec!["-n", "flag", "."]);
    }

    #[test]
    fn classifies_high_risk_commands() {
        let action = AgentAction {
            program: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/demo".to_string()],
            reason: String::new(),
            risk: String::new(),
            requires_network: false,
            host: None,
            port: None,
            timeout_secs: None,
        };
        assert_eq!(classify_action_risk(&action), AgentRisk::High);
    }

    #[test]
    fn risky_mode_skips_prompt_for_low_risk_local_action() {
        assert!(!action_needs_confirmation(
            ChatApprovalMode::Risky,
            AgentRisk::Low,
            false
        ));
        assert!(action_needs_confirmation(
            ChatApprovalMode::Risky,
            AgentRisk::Medium,
            false
        ));
        assert!(action_needs_confirmation(
            ChatApprovalMode::Risky,
            AgentRisk::Low,
            true
        ));
    }
}
