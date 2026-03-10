# Install

## Prerequisites

- Rust toolchain (`cargo`, `rustc`)
- Linux host (Arch, Ubuntu, Debian, Fedora, Kali, Parrot, etc.)

## Build

```bash
git clone <repo>
cd 0x0.AI
cargo build --release
```

Binary path:

```bash
./target/release/0x0
./target/release/ox
```

## One-line Install (GitHub)

User-local:

```bash
curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/install.sh | bash
```

System-wide:

```bash
curl -fsSL https://raw.githubusercontent.com/meshackbahati/0x0.AI/main/scripts/install.sh | bash -s -- --system
```

Start with:

```bash
ox --help
```

Installer behavior:
- clones repo into a temporary directory
- installs binaries (`ox`, `0x0`)
- deletes the temporary clone automatically
- optional: keep source via `--keep-source`

## Initialize

```bash
./target/release/0x0 init
```

This creates:
- config file
- SQLite state DB
- cache/log/writeup directories
- sample plugin

## First-time API Setup

Interactive:

```bash
0x0 setup
```

Non-interactive:

```bash
0x0 setup --non-interactive --provider openai --api-key-env OPENAI_API_KEY --model gpt-4.1-mini
```
