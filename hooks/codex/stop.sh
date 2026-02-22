#!/usr/bin/env bash
set -euo pipefail

SESSION_NAME="codex-hook-bridge"

if ! command -v tmux >/dev/null 2>&1; then
  echo "tmux is required but not installed"
  exit 1
fi

if tmux has-session -t "$SESSION_NAME" 2>/dev/null; then
  tmux kill-session -t "$SESSION_NAME"
  echo "stopped codex-hook-bridge (tmux session '$SESSION_NAME')"
else
  echo "codex-hook-bridge is not running"
fi
