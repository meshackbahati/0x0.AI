# Troubleshooting

## No Provider Available

- Run `0x0 providers configure ...` or `0x0 setup`
- Verify env vars if using `api_key_env`
- Test with `0x0 providers test`

## Provider Model Listing Fails

- Check API key validity
- Check base URL and compatibility type
- Ensure network approval (`--yes` or interactive confirmation)
- Retry with explicit provider: `0x0 providers models --provider <name>`

## Actions Blocked by Policy

- Review `allowed_paths`, `allowed_hosts`, `allowed_ports`
- Check confirmation requirements in config
- Use explicit approvals (`--approve-*`) in authorized contexts

## Missing Tool Errors

- Run `0x0 tools doctor`
- Install with explicit approval: `0x0 tools install <tool>`

## Resume/State Issues

- Inspect sessions via `0x0 stats`
- Replay actions: `0x0 replay <session-id>`
- Keep DB path stable via your config location
