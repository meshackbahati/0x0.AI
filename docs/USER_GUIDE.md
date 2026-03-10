# User Guide

This guide covers practical daily use of `ox` (`0x0` alias also works).

## 1) Install

User-local one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/install.sh | bash
```

System-wide one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/install.sh | bash -s -- --system
```

Notes:
- Installer clones into a temporary directory and deletes it automatically after install.
- Use `--keep-source` if you explicitly want a local source copy.

## 2) First Run

```bash
ox init
ox setup
```

`setup` wizard configures provider, API key/env, model, and optional task route.

## 3) Configure Providers and Models

Enable a provider:

```bash
ox providers configure openai --enable --api-key-env OPENAI_API_KEY --model gpt-4.1-mini
```

List models from provider API:

```bash
ox providers models --provider openai
```

Route task mode to a selected provider/model:

```bash
ox providers use --task reasoning --provider openai --model gpt-4.1
ox providers use --task coding --provider openai --model gpt-4.1-mini
ox providers use --task summarization --provider openai --model gpt-4.1-mini
```

Custom API endpoint (OpenAI-compatible):

```bash
ox providers configure myproxy \
  --compat openai \
  --base-url https://proxy.example/v1 \
  --api-key-env MYPROXY_KEY \
  --model my-default-model \
  --enable
```

Custom API endpoint (Anthropic-compatible):

```bash
ox providers configure myclaude \
  --compat anthropic \
  --base-url https://claude-proxy.example/v1 \
  --api-key-env MYCLAUDE_KEY \
  --model claude-3-5-sonnet \
  --enable
```

## 4) Solve Workflows

Single target:

```bash
ox scan ./challenge
ox solve ./challenge --yes
```

Batch across directories:

```bash
ox solve-all ./ctf --yes --max-challenges 40
```

Resume and replay:

```bash
ox resume <session-id>
ox replay <session-id>
ox writeup <session-id>
```

## 5) Chat Mode (Transparent Actions)

```bash
ox chat --show-actions --yes
```

Inside chat:
- Normal prompt: reasoning/coding response with streamed output
- `/research <query>`: local + optional web research
- `/run <command>`: executes local command via safety wrapper
- `/exit`: leave chat

## 6) Web Challenge (Authorized Lab Only)

Map target:

```bash
ox web map http://127.0.0.1:8080 --approve-network --approve-exec
```

Replay requests:

```bash
ox web replay http://127.0.0.1:8080 --method POST --path /login --data 'u=test&p=test' --approve-network --approve-exec
```

Generate templates and payload notebook:

```bash
ox web template http://127.0.0.1:8080
```

## 7) Get Most Out of It

- Set explicit safety allowlists (`allowed_paths`, `allowed_hosts`, `allowed_ports`).
- Use `--dry-run` to preview actions before execution.
- Keep notes during exploration:

```bash
ox note <session-id> "Hypothesis: weak nonce reuse"
```

- Use provider routes by task mode for cost/performance control.
- Generate writeups immediately after solving while context is fresh.

## 8) Safety and Scope

`ox` is for authorized CTF/lab environments only.

It is intentionally not designed for unauthorized exploitation or indiscriminate internet attacks.
