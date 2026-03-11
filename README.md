# 0x0.AI

`0x0.AI` is a production-grade, local-first, Linux-first autonomous CTF assistant for **authorized environments only**.

It combines:
- coding agent workflows
- local/web research workflows
- structured CTF solve planning
- safe tool execution and action logging

## Safety First

`0x0.AI` is intentionally constrained.

It will only operate on:
- local files
- local containers/VMs
- explicitly approved lab/CTF targets

It blocks or requires explicit confirmation for:
- unauthorized network interaction
- execution actions
- installation actions

See [Safety Model](docs/SAFETY_MODEL.md).

## Key Features

- CLI-only, Linux-first runtime (Rust)
- SQLite-backed persistent sessions and action logs
- resumable investigations and session replay
- broad category-aware solving with adaptive command synthesis
- major CTF category coverage: `crypto`, `pwn`, `rev`, `web`, `forensics`, `stego`, `osint`, `mobile`, `hardware`, `blockchain`, `cloud`, `network`, `ai`, `misc`
- local artifact ingestion/indexing
- local + web research with citations and cache
- safe tool wrapper with dry-run, timeout, capture
- package-manager-aware install planning
- pluggable provider abstraction with fallback local mode
- provider model routing by task type
- custom API provider support
- autonomous terminal REPL mode (`0x0 chat`) with per-action visibility and approval gates

## Commands

Core:
- `0x0 init`
- `0x0 setup`
- `0x0 update`
- `0x0 scan <path>`
- `0x0 solve <path>`
- `0x0 solve-all <path>`
- `0x0 resume <session-id>`
- `0x0 sessions [--limit ... --status ... --category ...]`
- `0x0 research <query>`
- `0x0 chat`
- `0x0 note <session-id> ...`
- `0x0 writeup <session-id>`
- `0x0 replay <session-id>`
- `0x0 stats`

Providers:
- `0x0 providers configure ...`
- `0x0 providers models [--provider ...]`
- `0x0 providers use --task ... --provider ... --model ...`
- `0x0 providers test`

Tools:
- `0x0 tools doctor`
- `0x0 tools install <tool>`

Web (authorized lab only):
- `0x0 web map <target-url>`
- `0x0 web replay <target-url> --method ... --path ...`
- `0x0 web template <target-url>`

Config:
- `0x0 config show`
- `0x0 config edit`

## Quick Start

One-line install from GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/install.sh | bash
```

System-wide one-line install:

```bash
curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/install.sh | bash -s -- --system
```

After install, run `ox --help` (alias `0x0 --help` is also installed).
The installer clones to a temporary directory and removes it automatically after installation.

1. Build:

```bash
cargo build --release
```

2. Initialize runtime paths and sample plugin:

```bash
./target/release/0x0 init
./target/release/ox init
```

3. First-time provider setup:

```bash
./target/release/0x0 setup
```

4. Scan and solve a challenge directory:

```bash
./target/release/0x0 scan ./challenge
./target/release/0x0 solve ./challenge --yes
```

5. Recursive solving for many challenge folders:

```bash
./target/release/0x0 solve-all ./ctf --yes
```

6. Generate writeup:

```bash
./target/release/0x0 writeup <session-id>
```

## Interactive REPL Mode

```bash
ox
# or: 0x0
# optional: ox chat --show-actions
```

Inside chat:
- type a normal prompt to run autonomous mode (`/auto`) by default
- `/ask <prompt>` for direct response only (no autonomous action loop)
- `/auto <goal>` to force autonomous loop for a single goal
- `/sessions` to list recent session IDs
- `/resume <session-id>` to continue a previous chat session
- `/provider` to list provider readiness (enabled/key/runtime)
- `/provider <name>` to switch provider interactively (checks API key presence)
- `/model` to show active provider/model
- `/model all` to fetch and list live models from current provider API
- `/model <provider>` to switch active provider
- `/model <model-id>` to switch model during the session
- `/model <provider>:<model-id>` to switch provider and model together
- `/run <command>` to execute one command manually
- `/ps`, `/ls`, `/pwd` shortcuts for common terminal actions
- `/clean` to clear the terminal
- `/research <query>` for local + optional web research
- `/constraints` to show current safety boundaries and approvals
- `/exit` to quit

Approval behavior:
- default `--approval-mode risky`: asks only for risky/network actions
- `--approval-mode all`: asks before every proposed action

Autonomous behavior notes:
- the agent discovers available tools each run and can switch tools dynamically
- it observes command behavior/output per step and adapts follow-up commands
- mention expected flag format naturally in chat (example: `flag prefix is HTB`) and it is auto-detected
- it can auto-generate helper scripts under `.0x0-ai/generated-tools/` during solve loops

Update behavior:
- `0x0 update` checks stable release tags first, then falls back to branch commit tracking
- `0x0 update --prefer-commit` always tracks branch commit
- `0x0 update --dry-run` shows selected ref/commit without installing

## Provider and Model Selection

Configure provider:

```bash
0x0 providers configure openai --enable --api-key-env OPENAI_API_KEY --model gpt-4.1-mini
```

Gemini quick setup/test:

```bash
export GEMINI_API_KEY="your_key_here"
0x0 providers configure gemini --enable --api-key-env GEMINI_API_KEY --model gemini-2.5-flash
0x0 providers test --provider gemini
0x0 providers use --task reasoning --provider gemini --model gemini-2.5-flash
```

List available provider models:

```bash
0x0 providers models --provider openai
```

Bind model to a task mode:

```bash
0x0 providers use --task reasoning --provider openai --model gpt-4.1
```

Add a custom API endpoint:

```bash
0x0 providers configure mygateway \
  --compat openai \
  --base-url https://my.gateway.example/v1 \
  --api-key-env MY_GATEWAY_KEY \
  --model my-default-model \
  --enable
```

## Docs

- [Architecture](docs/ARCHITECTURE.md)
- [User Guide](docs/USER_GUIDE.md)
- [Command Reference](docs/COMMANDS.md)
- [Install](docs/INSTALL.md)
- [Distro Notes](docs/DISTRO_NOTES.md)
- [Configuration](docs/CONFIGURATION.md)
- [Safety Model](docs/SAFETY_MODEL.md)
- [Plugin Guide](docs/PLUGIN_GUIDE.md)
- [Web Research Guide](docs/WEB_RESEARCH.md)
- [Tool Installation Guide](docs/TOOL_INSTALL.md)
- [Example Workflows](docs/WORKFLOWS.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
