# Plugin Guide

Plugins are local manifest files in the plugin directory created by `0x0 init`.

Manifest format (TOML):

```toml
name = "extract-flag"
description = "Extract CTF-style flag patterns from text files"
command = "python3"
args = ["/path/to/script.py"]
categories = ["misc", "forensics"]
```

Common category tags:
- `crypto`, `pwn`, `rev`, `web`, `forensics`, `stego`, `osint`
- `mobile`, `hardware`, `blockchain`, `cloud`, `network`, `ai`, `misc`

Notes:
- Plugins run through the safe subprocess wrapper.
- Keep plugins local and auditable.
- Do not add unauthorized offensive automation.
