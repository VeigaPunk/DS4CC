#!/usr/bin/env bash
set -euo pipefail

HOOK_DIR="$HOME/.codex/hooks"
LOG_FILE="$HOOK_DIR/bridge.log"
BRIDGE="$HOOK_DIR/codex-hook-bridge.py"
SESSION_NAME="codex-hook-bridge"

mkdir -p "$HOOK_DIR"

if ! command -v tmux >/dev/null 2>&1; then
  echo "tmux is required but not installed"
  exit 1
fi

if tmux has-session -t "$SESSION_NAME" 2>/dev/null; then
  echo "codex-hook-bridge already running in tmux session '$SESSION_NAME'"
  exit 0
fi

tmux new-session -d -s "$SESSION_NAME" "python3 \"$BRIDGE\" >>\"$LOG_FILE\" 2>&1"
echo "started codex-hook-bridge in tmux session '$SESSION_NAME'"
