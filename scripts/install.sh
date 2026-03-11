#!/usr/bin/env bash
set -euo pipefail

REPO_URL="${REPO_URL:-https://github.com/meshackbahati/0x0.AI.git}"
BRANCH="${BRANCH:-main}"
MODE="user"
KEEP_SOURCE=0

for arg in "$@"; do
  case "$arg" in
    --system)
      MODE="system"
      ;;
    --user)
      MODE="user"
      ;;
    --keep-source)
      KEEP_SOURCE=1
      ;;
    *)
      echo "Unknown option: $arg" >&2
      echo "Usage: install.sh [--user|--system] [--keep-source]" >&2
      exit 1
      ;;
  esac
done

if ! command -v git >/dev/null 2>&1; then
  echo "git is required" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Install Rust first: https://rustup.rs/" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[+] Cloning $REPO_URL ($BRANCH)"
git clone --depth 1 --branch "$BRANCH" "$REPO_URL" "$TMP_DIR/repo"

if [[ "$MODE" == "system" ]]; then
  ROOT="/usr/local"
  echo "[+] Installing system-wide to $ROOT (requires sudo)"
  sudo cargo install --path "$TMP_DIR/repo" --locked --force --root "$ROOT"
  BIN_DIR="$ROOT/bin"
else
  ROOT="${HOME}/.local"
  echo "[+] Installing for current user to $ROOT"
  cargo install --path "$TMP_DIR/repo" --locked --force --root "$ROOT"
  BIN_DIR="$ROOT/bin"
fi

if [[ "$KEEP_SOURCE" -eq 1 ]]; then
  KEEP_DIR="${HOME}/.local/share/0x0-ai-source"
  rm -rf "$KEEP_DIR"
  mkdir -p "$KEEP_DIR"
  cp -R "$TMP_DIR/repo/." "$KEEP_DIR/"
  echo "[+] Source preserved at $KEEP_DIR"
else
  echo "[+] Temporary clone will be deleted automatically"
fi

if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
  echo "[!] $BIN_DIR is not on PATH"
  echo "    Add this to your shell config:"
  echo "    export PATH=\"$BIN_DIR:\$PATH\""
fi

echo "[+] Installed successfully"
echo "    Run: ox --help"
echo "    Alias also available as: 0x0 --help"
echo "    Update later with: ox update"
