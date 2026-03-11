# Architecture

`0x0.AI` uses a modular Rust architecture designed for low memory usage, auditable actions, and safe automation.

## Layers

- `src/cli.rs`: command schema and argument parsing
- `src/app.rs`: orchestration and command handlers
- `src/config.rs`: config loading/saving and runtime path management
- `src/policy.rs`: safety policy enforcement
- `src/storage.rs`: SQLite persistence (sessions/actions/artifacts/hypotheses/notes/citations/cache)
- `src/ingest.rs`: artifact scanning and indexing
- `src/planner.rs`: adaptive planner/executor/review solving loop
- `src/tools/`: capability discovery, subprocess runner, package manager abstraction
- `src/providers/`: provider abstraction, routing, retries, timeout, token budget
- `src/research/`: local search + web research/caching/citation extraction
- `src/categories/`: category plans/heuristics for crypto, pwn, rev, web, forensics, stego, osint, mobile, hardware, blockchain, cloud, network, ai, misc
- `src/report.rs`: writeup and replay report generation
- `src/web_lab.rs`: authorized web challenge mapping/replay/fuzz templates
- `src/plugins.rs`: local plugin discovery and execution

## Runtime Principles

- Local-first operation
- Bounded memory growth via pruning limits
- Lazy and incremental processing where possible
- Explicit policy checks before risky actions
- Strong action logging for auditability
- Observe-and-adapt loops: each action output influences next action selection
- Tool-aware autonomy: only installed tools are proposed/executed by planner

## Persistence Model

SQLite stores:
- sessions
- actions
- artifacts
- hypotheses
- notes
- citations
- web cache

All command workflows can be resumed by session ID.

## Provider Routing

Task routes are configured per mode:
- reasoning
- coding
- summarization
- vision
- classification

Routes map to provider + model, allowing deterministic selection.

## Safety Pipeline

Before execution, each action is evaluated against policy:
1. path constraints
2. network constraints (host/port/confirmation/offline)
3. execution confirmation
4. install confirmation

Blocked actions are logged with reason.
