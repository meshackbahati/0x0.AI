# Safety Model

`0x0.AI` is designed for authorized environments only.

## Allowed Scope

- local files and artifacts
- user-owned local targets
- explicitly allowed lab/CTF hosts and ports

## Disallowed Behavior

- unauthorized exploitation
- indiscriminate scanning
- credential theft
- malware deployment
- stealth/persistence behavior
- autonomous internet attacks

## Enforcement

- per-action policy checks
- host/port/path allowlists
- optional offline-only mode
- explicit confirmation for network/exec/install
- action logging and replay
- dry-run mode

If an action violates policy, it is blocked and logged.
