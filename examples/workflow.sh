#!/usr/bin/env bash
set -euo pipefail

# Example end-to-end local workflow
ox init
ox setup --non-interactive --provider openai --api-key-env OPENAI_API_KEY --model gpt-4.1-mini

SESSION_JSON=$(ox scan ./tests/fixtures/misc --json)
SESSION_ID=$(echo "$SESSION_JSON" | jq -r '.session_id')

ox solve ./tests/fixtures/misc --session-id "$SESSION_ID" --yes
ox writeup "$SESSION_ID"
ox replay "$SESSION_ID"
