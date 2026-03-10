# Tool Installation Guide

Use:

```bash
0x0 tools doctor
0x0 tools install <tool>
```

Install flow:
1. detect missing tool
2. detect package manager
3. build install command
4. require explicit approval
5. execute (or dry-run)
6. verify installation
7. log result

Supported managers:
- apt
- dnf
- pacman
- yay / paru
- zypper
- pip/pipx
- cargo
- go
- npm (explicit use only)

`--no-install` disables install actions.
