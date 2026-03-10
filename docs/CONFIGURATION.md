# Configuration

Config file location is printed by:

```bash
0x0 init
```

View/edit:

```bash
0x0 config show
0x0 config edit
```

## Provider Configuration Methods

1. Env vars only
2. CLI config updates (`providers configure`)
3. First-time setup wizard (`setup`)

Examples:

```bash
0x0 providers configure openai --enable --api-key-env OPENAI_API_KEY --model gpt-4.1-mini
0x0 providers models --provider openai
0x0 providers use --task reasoning --provider openai --model gpt-4.1
```

Custom endpoints:

```bash
0x0 providers configure myproxy --compat openai --base-url https://proxy.example/v1 --api-key-env PROXY_KEY --model x-model --enable
0x0 providers configure myclaude --compat anthropic --base-url https://claude-proxy.example/v1 --api-key-env CLAUDE_PROXY_KEY --model claude-3-5-sonnet --enable
```

## Safety Controls

Key policy flags in config:
- `allowed_paths`
- `allowed_hosts`
- `allowed_ports`
- `offline_only`
- `research_web_enabled`
- confirmation requirements for network/exec/install
