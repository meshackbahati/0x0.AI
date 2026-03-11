# Command Reference

This is the full CLI reference for `0x0` / `ox`.

## Global Flags

These can be used before any command:

- `--config <path>`: use a specific config file.
- `--json`: force JSON output.
- `--dry-run`: run in dry-run mode where supported.
- `--yes`: auto-approve prompts.
- `--offline`: force offline-only safety mode.
- `--no-install`: deny installation actions.
- `--output <text|json>`: explicit output mode.

## Core Commands

### `init [path]`

- `path` (optional, default `.`): project/workspace path.
- `--force`: overwrite/recreate default project/config files.

### `setup`

- `--provider <name>`
- `--api-key <value>`
- `--api-key-env <ENV_VAR>`
- `--model <model-id>`
- `--base-url <url>`
- `--route <reasoning|coding|summarization|vision|classification>`
- `--compat <openai|anthropic|generic>`
- `--non-interactive`

### `update`

- `--system`: target system install location.
- `--user`: target user-local install location.
- `--branch <branch>`
- `--reference <ref-or-tag>`
- `--prefer-commit`
- `--dry-run`

### `scan <path>`

- `path`: challenge root/file to scan.
- `--session-id <id>`: reuse/create explicit session ID.
- `--recursive` (default `true`)
- `--max-read-bytes <bytes>` (default `10485760`)

### `solve <path>`

- `path`: challenge root/file.
- `--session-id <id>`
- `--max-steps <n>` (default `8`)
- `--web`: allow web actions in solve loop.
- `--approve-network`
- `--approve-exec`
- `--approve-install`

### `solve-all <path>`

- `path`: parent folder containing many challenge dirs/files.
- `--max-steps <n>` (default `6`)
- `--web`
- `--approve-network`
- `--approve-exec`
- `--approve-install`
- `--max-challenges <n>` (default `40`)

### `resume <session-id>`

- `session-id`: existing session to continue/inspect.
- `--continue-solve`: continue solve loop from stored context.
- `--max-steps <n>` (default `4`)
- `--web`
- `--approve-network`
- `--approve-exec`

### `sessions`

- `--limit <n>` (default `20`)
- `--status <value>`: filter by session status.
- `--category <value>`: filter by category (`crypto`, `pwn`, `rev`, `web`, `forensics`, `stego`, `osint`, `mobile`, `hardware`, `blockchain`, `cloud`, `network`, `ai`, `misc`, `unknown`).

### `research <query>`

- `query`: search phrase.
- `--local` (default `true`)
- `--web` (default `false`)
- `--session-id <id>`
- `--max-results <n>` (default `5`)
- `--approve-network`

### `chat`

- `--session-id <id>`
- `--provider <name>`
- `--system <system-prompt>`
- `--prompt <one-shot-message>`
- `--autonomous <true|false>` (default `true`)
- `--approval-mode <all|risky>` (default `risky`)
- `--max-agent-steps <n>` (default `8`)
- `--web`
- `--approve-network`
- `--approve-exec`
- `--show-actions <true|false>` (default `true`)
- `--max-turns <n>` (default `200`)

Chat REPL slash commands:
- `/help`
- `/sessions`
- `/resume <session-id>`
- `/constraints`
- `/provider` (list provider readiness including API-key checks)
- `/provider <name>` (switch active provider if ready)
- `/model` (show active provider/model)
- `/model all` (fetch and list models for active provider API using configured provider `base_url`)
- `/model <provider>` (switch active provider and clear model override)
- `/model <model-id>` (set model override for current chat session)
- `/model <provider>:<model-id>` (switch provider+model together for current session)
- `/model default` (clear model override and use provider default)
- `/run <command>`
- `/ps`
- `/ls`
- `/pwd`
- `/clean`
- `/research <query>`
- `/ask <prompt>`
- `/auto <goal>`
- `/exit`

### `note <session-id> <text...>`

- `session-id`: target session.
- `text...`: note content.

### `writeup <session-id>`

- `session-id`
- `--out <path>`

### `replay <session-id>`

- `session-id`
- `--limit <n>` (default `100`)

### `stats`

- `--session-id <id>`: emit scoped stats for one session.

## Tools Commands

### `tools doctor`

- `--verbose`: include detailed tool rows.

### `tools install <tool>`

- `tool`: tool/program name to install.
- `--approve-install`

## Provider Commands

### `providers test`

- `--provider <name>`
- `--prompt <text>` (default: `Return a one-line status response.`)

### `providers configure <provider>`

- `provider`: provider key/name.
- `--api-key <value>`
- `--api-key-env <ENV_VAR>`
- `--model <model-id>`
- `--base-url <url>`
- `--enable`
- `--disable`
- `--route <reasoning|coding|summarization|vision|classification>`
- `--compat <openai|anthropic|generic>`

### `providers models`

- `--provider <name>`

### `providers use`

- `--task <reasoning|coding|summarization|vision|classification>`
- `--provider <name>`
- `--model <model-id>`

## Web Commands

### `web map <target>`

- `target`: target URL.
- `--session-id <id>`
- `--approve-network`
- `--approve-exec`
- `--out <dir>`

### `web replay <target>`

- `target`: target URL.
- `--method <verb>` (default `GET`)
- `--path <path>` (default `/`)
- `--header <k:v>` (repeatable)
- `--data <body>`
- `--session-id <id>`
- `--approve-network`
- `--approve-exec`

### `web template <target>`

- `target`: target URL.
- `--out <dir>`

## Config Commands

### `config show`

- no additional flags.

### `config edit`

- no additional flags.
