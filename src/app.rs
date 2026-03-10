use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use sysinfo::System;
use uuid::Uuid;

use crate::categories::{ChallengeCategory, infer_category};
use crate::cli::{
    ChatArgs, Cli, Commands, ConfigCommand, OutputFormat, ProvidersCommand, RouteTask, SetupArgs,
    ToolsCommand, WebCommand, ProvidersConfigureArgs, ProviderCompat,
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
use crate::util::confirm;
use crate::web_lab::{generate_templates_and_notebook, map_target, parse_target, replay_request};

pub fn run(cli: Cli) -> Result<()> {
    let mut runtime = load_runtime_config(cli.config.clone())?;
    if cli.offline {
        runtime.config.safety.offline_only = true;
        runtime.config.safety.research_web_enabled = false;
    }

    let output_mode = resolve_output_mode(&cli, &runtime);

    let store = StateStore::open(
        &runtime.paths.db_path,
        runtime.config.memory.max_actions_per_session,
        runtime.config.memory.max_artifacts_per_session,
        runtime.config.memory.max_cache_entries,
    )?;

    let policy = PolicyEngine::new(runtime.config.safety.clone())?;
    let tools = ToolManager::new(runtime.config.tools.clone(), cli.dry_run);
    let providers = ProviderManager::new(runtime.config.clone());
    let _plugins = PluginManager::new(runtime.paths.plugins_dir.clone());

    match cli.command {
        Commands::Init(args) => cmd_init(args.path, args.force, &mut runtime, output_mode),
        Commands::Setup(args) => cmd_setup(args, &mut runtime, output_mode),
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
                network: args.approve_network || cli.yes,
                exec: args.approve_exec || cli.yes,
                install: args.approve_install || cli.yes,
            };

            if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                approvals.exec = confirm(
                    "Allow local command execution for solve workflow?",
                    cli.yes,
                )?;
            }
            if args.web && runtime.config.safety.require_confirmation_for_network && !approvals.network {
                approvals.network =
                    confirm("Allow network actions against approved targets?", cli.yes)?;
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
                bail!("no challenge targets discovered under {}", args.path.display());
            }

            let mut approvals = Approvals {
                network: args.approve_network || cli.yes,
                exec: args.approve_exec || cli.yes,
                install: args.approve_install || cli.yes,
            };
            if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                approvals.exec = confirm(
                    "Allow local command execution for solve-all workflow?",
                    cli.yes,
                )?;
            }
            if args.web && runtime.config.safety.require_confirmation_for_network && !approvals.network
            {
                approvals.network =
                    confirm("Allow network actions against approved targets?", cli.yes)?;
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
                    network: args.approve_network || cli.yes,
                    exec: args.approve_exec || cli.yes,
                    install: false,
                };

                if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                    approvals.exec = confirm(
                        "Allow local command execution for resumed workflow?",
                        cli.yes,
                    )?;
                }
                if args.web && runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network =
                        confirm("Allow network actions against approved targets?", cli.yes)?;
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

                emit(output_mode, &outcome, &format!("resumed session {}", session.id))
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
                network: args.approve_network || cli.yes,
                exec: false,
                install: false,
            };

            if args.web {
                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        "Allow passive web research (docs/public references only)?",
                        cli.yes,
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
            cli.yes,
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
                if cli.no_install {
                    bail!("install denied by --no-install");
                }

                let mut approvals = Approvals {
                    network: false,
                    exec: cli.yes,
                    install: install_args.approve_install || cli.yes,
                };

                if runtime.config.safety.require_confirmation_for_install && !approvals.install {
                    approvals.install = confirm(
                        &format!("Install tool '{}' using package manager?", install_args.tool),
                        cli.yes,
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
                                "You are running a provider connectivity and behavior test.".to_string(),
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
                    set_route(&mut runtime.config, route, &cfg_args.provider, cfg_args.model.clone());
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
                    network: cli.yes,
                    exec: false,
                    install: false,
                };
                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network =
                        confirm("Allow network access to provider APIs for model listing?", cli.yes)?;
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
                let session_id = ensure_session_label(
                    &store,
                    map_args.session_id.as_deref(),
                    &target.base_url,
                )?;

                let mut approvals = Approvals {
                    network: map_args.approve_network || cli.yes,
                    exec: map_args.approve_exec || cli.yes,
                    install: false,
                };

                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        &format!(
                            "Allow web mapping against approved target {}:{} ?",
                            target.host, target.port
                        ),
                        cli.yes,
                    )?;
                }
                if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                    approvals.exec =
                        confirm("Allow local command execution for web mapping?", cli.yes)?;
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
                        &format!("web-fuzz-template [{}]: {}", template.name, template.command_preview),
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
                    network: replay_args.approve_network || cli.yes,
                    exec: replay_args.approve_exec || cli.yes,
                    install: false,
                };

                if runtime.config.safety.require_confirmation_for_network && !approvals.network {
                    approvals.network = confirm(
                        &format!(
                            "Allow request replay against approved target {}:{} ?",
                            target.host, target.port
                        ),
                        cli.yes,
                    )?;
                }
                if runtime.config.safety.require_confirmation_for_exec && !approvals.exec {
                    approvals.exec =
                        confirm("Allow local command execution for request replay?", cli.yes)?;
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
                let (templates, notebook_path) = generate_templates_and_notebook(&target, &out_dir)?;
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
            let out = args
                .out
                .unwrap_or_else(|| runtime.paths.writeups_dir.join(format!("{}.md", args.session_id)));
            write_writeup(&out, &bundle.markdown)?;
            let payload = json!({"session_id": args.session_id, "path": out, "bytes": bundle.markdown.len()});
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
                    exec: cli.yes
                        || confirm(
                            &format!("Open config with editor '{}' ?", editor),
                            cli.yes,
                        )?,
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
                let text = fs::read_to_string(&runtime.paths.config_path).with_context(|| {
                    format!("reading {}", runtime.paths.config_path.display())
                })?;
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

fn cmd_init(path: PathBuf, force: bool, runtime: &mut RuntimeConfig, mode: OutputMode) -> Result<()> {
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
        }
        save_config(&runtime.paths.config_path, &runtime.config)?;
        let payload = redacted_provider_view(&runtime.config, &provider);
        return emit(mode, &payload, "setup completed (non-interactive)");
    }

    println!("0x0.AI provider setup wizard");
    println!("Use Ctrl+C to abort.");

    let provider = prompt_required("Provider (openai/openrouter/together/gemini/moonshot/anthropic): ")?;
    let api_key = prompt_optional("API key (leave empty to use env var): ")?;
    let api_key_env = prompt_optional("API key env var (optional): ")?;
    let model = prompt_optional("Default model (optional): ")?;
    let base_url = prompt_optional("Base URL override (optional): ")?;
    let route = prompt_optional("Route task [reasoning|coding|summarization|vision|classification] (optional): ")?;
    let compat = prompt_optional("Compatibility [openai|anthropic] for custom providers (optional): ")?;

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
    }
    save_config(&runtime.paths.config_path, &runtime.config)?;

    let payload = redacted_provider_view(&runtime.config, &provider);
    emit(mode, &payload, "setup completed")
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

    let mut web_researcher = if args.web {
        Some(WebResearcher::new(runtime.config.research.clone())?)
    } else {
        None
    };

    let cwd = std::env::current_dir()?;
    policy.ensure_path_allowed(&cwd)?;

    if let Some(one_shot) = args.prompt.as_deref() {
        let reply = chat_turn(
            one_shot,
            &args,
            &session_id,
            store,
            policy,
            tools,
            providers,
            &cwd,
            &mut web_researcher,
            approvals,
            output_mode,
            auto_yes,
        )?;

        if output_mode == OutputMode::Json {
            println!("{}", serde_json::to_string_pretty(&json!({
                "session_id": session_id,
                "reply": reply,
            }))?);
        }
        return Ok(());
    }

    println!("Chat session: {}", session_id);
    println!("Commands: /help, /exit, /run <cmd>, /research <query>");

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
        if matches!(line, "/exit" | "exit" | "quit") {
            break;
        }
        if line == "/help" {
            println!("Available:");
            println!("/run <command>          execute local command through policy wrapper");
            println!("/research <query>       local + optional web research");
            println!("/exit                   leave chat");
            continue;
        }

        let _ = chat_turn(
            line,
            &args,
            &session_id,
            store,
            policy,
            tools,
            providers,
            &cwd,
            &mut web_researcher,
            approvals,
            OutputMode::Text,
            auto_yes,
        )?;
    }

    Ok(())
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
    mut approvals: Approvals,
    output_mode: OutputMode,
    auto_yes: bool,
) -> Result<String> {
    let _ = store.add_note(session_id, &format!("user: {}", line));

    if let Some(rest) = line.strip_prefix("/run ") {
        if policy.config().require_confirmation_for_exec && !approvals.exec {
            approvals.exec = confirm("Allow command execution for this chat turn?", auto_yes)?;
        }
        policy.ensure_exec_allowed(approvals, "chat-run")?;

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
        if args.show_actions {
            println!("[action] local-search hits={}", local_hits.len());
        }
        let mut web_hits = Vec::new();
        if let Some(researcher) = web_researcher.as_mut() {
            policy.ensure_network_allowed(approvals, "public-web", None, true)?;
            web_hits = researcher.search(query, 3, store)?;
            if args.show_actions {
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
    if args.show_actions {
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
        format!("User prompt:\n{}\n\nRelevant local context:\n{}", line, context)
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
        system: args
            .system
            .clone()
            .or_else(|| Some("You are 0x0.AI, a transparent terminal copilot. Explain exactly what you did and why.".to_string())),
        prompt,
        task_type: TaskType::Reasoning,
        max_tokens: 600,
        temperature: 0.2,
        timeout_secs: 45,
        model_override: None,
    };

    if args.show_actions {
        let provider_name = args
            .provider
            .clone()
            .unwrap_or_else(|| providers.provider_for_task(TaskType::Reasoning));
        println!("[action] provider-call provider={provider_name}");
    }

    let response = if let Some(p) = &args.provider {
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

            let compat = args.compat.clone().unwrap_or(ProviderCompat::Openai);
            let base_url = args.base_url.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown provider '{other}'; pass --base-url and optionally --compat openai|anthropic"
                )
            })?;
            let api_key_env = args.api_key_env.clone().unwrap_or_else(|| {
                format!("{}_API_KEY", other.to_ascii_uppercase().replace('-', "_"))
            });

            match compat {
                ProviderCompat::Openai => cfg
                    .providers
                    .custom_openai_compatible
                    .push(crate::config::NamedOpenAiCompatProvider {
                        name: other.to_string(),
                        enabled: !args.disable,
                        base_url,
                        api_key_env,
                        api_key: args.api_key.clone(),
                        default_model: args
                            .model
                            .clone()
                            .unwrap_or_else(|| "gpt-4.1-mini".to_string()),
                    }),
                ProviderCompat::Anthropic => cfg
                    .providers
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
                    }),
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

fn ensure_session_label(store: &StateStore, session_id: Option<&str>, root_label: &str) -> Result<String> {
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

    for entry in walkdir::WalkDir::new(root).max_depth(3).into_iter().filter_map(Result::ok) {
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
        "web" => Some(ChallengeCategory::Web),
        "misc" => Some(ChallengeCategory::Misc),
        "forensics" => Some(ChallengeCategory::Forensics),
        "stego" => Some(ChallengeCategory::Stego),
        "osint" => Some(ChallengeCategory::Osint),
        "unknown" => Some(ChallengeCategory::Unknown),
        _ => None,
    }
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
