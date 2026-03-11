#!/usr/bin/env bash
set -euo pipefail

REPO_URL="${REPO_URL:-https://github.com/meshackbahati/0x0.AI.git}"
BRANCH="${BRANCH:-main}"
MODE="user"
REFERENCE=""
PREFER_COMMIT=0
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage: update.sh [options]

Options:
  --user               Install under ~/.local (default)
  --system             Install under /usr/local (requires sudo)
  --branch <name>      Branch to track for commit updates (default: main)
  --reference <ref>    Explicit git ref (tag or commit) to install
  --prefer-commit      Skip release tags and always track branch commit
  --repo-url <url>     Override repository URL
  --dry-run            Resolve target ref without installing
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --system)
      MODE="system"
      shift
      ;;
    --user)
      MODE="user"
      shift
      ;;
    --branch)
      BRANCH="${2:-}"
      shift 2
      ;;
    --reference)
      REFERENCE="${2:-}"
      shift 2
      ;;
    --prefer-commit)
      PREFER_COMMIT=1
      shift
      ;;
    --repo-url)
      REPO_URL="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
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

latest_stable_tag() {
  git ls-remote --tags --refs "$REPO_URL" \
    | awk '{print $2}' \
    | sed 's@refs/tags/@@' \
    | grep -E '^[vV]?[0-9]+(\.[0-9]+){1,3}$' \
    | sort -V \
    | tail -n1 || true
}

latest_branch_commit() {
  git ls-remote "$REPO_URL" "refs/heads/$BRANCH" | awk '{print $1}' | head -n1
}

TARGET_REF=""
TARGET_KIND=""

if [[ -n "$REFERENCE" ]]; then
  TARGET_REF="$REFERENCE"
  TARGET_KIND="explicit-ref"
else
  if [[ "$PREFER_COMMIT" -eq 0 ]]; then
    TAG="$(latest_stable_tag)"
    if [[ -n "$TAG" ]]; then
      TARGET_REF="$TAG"
      TARGET_KIND="release-tag"
    fi
  fi

  if [[ -z "$TARGET_REF" ]]; then
    COMMIT="$(latest_branch_commit)"
    if [[ -z "$COMMIT" ]]; then
      echo "Could not resolve branch head for $BRANCH" >&2
      exit 1
    fi
    TARGET_REF="$COMMIT"
    TARGET_KIND="commit"
  fi
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[+] Updating from $REPO_URL"
echo "[+] Selected target: $TARGET_REF ($TARGET_KIND)"

git clone --filter=blob:none "$REPO_URL" "$TMP_DIR/repo"
git -C "$TMP_DIR/repo" checkout --detach "$TARGET_REF"
RESOLVED_COMMIT="$(git -C "$TMP_DIR/repo" rev-parse HEAD)"
echo "[+] Resolved commit: $RESOLVED_COMMIT"

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[+] Dry run complete (no install performed)"
  exit 0
fi

if [[ "$MODE" == "system" ]]; then
  ROOT="/usr/local"
  echo "[+] Installing system-wide to $ROOT (requires sudo)"
  sudo cargo install --path "$TMP_DIR/repo" --locked --force --root "$ROOT"
else
  ROOT="${HOME}/.local"
  echo "[+] Installing for current user to $ROOT"
  cargo install --path "$TMP_DIR/repo" --locked --force --root "$ROOT"
fi

BIN_DIR="$ROOT/bin"
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
  echo "[!] $BIN_DIR is not on PATH"
  echo "    Add this to your shell config:"
  echo "    export PATH=\"$BIN_DIR:\$PATH\""
fi

echo "[+] Update completed"
echo "[+] Installed commit: $RESOLVED_COMMIT"
echo "[+] Run: ox --help"
