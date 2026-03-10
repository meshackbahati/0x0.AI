#!/usr/bin/env bash
set -euo pipefail

MODE="user"
for arg in "$@"; do
  case "$arg" in
    --system)
      MODE="system"
      ;;
    --user)
      MODE="user"
      ;;
    *)
      echo "Unknown option: $arg" >&2
      echo "Usage: uninstall.sh [--user|--system]" >&2
      exit 1
      ;;
  esac
done

if [[ "$MODE" == "system" ]]; then
  ROOT="/usr/local"
  if command -v sudo >/dev/null 2>&1; then
    sudo rm -f "$ROOT/bin/ox" "$ROOT/bin/0x0"
  else
    rm -f "$ROOT/bin/ox" "$ROOT/bin/0x0"
  fi
else
  ROOT="${HOME}/.local"
  rm -f "$ROOT/bin/ox" "$ROOT/bin/0x0"
fi

echo "Removed binaries from $ROOT/bin"
