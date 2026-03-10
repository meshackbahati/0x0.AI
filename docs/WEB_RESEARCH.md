# Web Research Guide

`0x0 research` supports local and optional web-backed research.

## Local

```bash
0x0 research "rsa common modulus" --local --session-id <id>
```

## Web (Passive Only)

```bash
0x0 research "RFC 8017" --web --approve-network --session-id <id>
```

Behavior:
- caches fetched pages in local SQLite cache
- extracts readable text from HTML
- stores citation metadata (source, locator, snippet)
- supports domain allow/block rules from config
- attempts robots/rate-limit compliance

This subsystem is for documentation/reference retrieval, not unauthorized exploitation.
