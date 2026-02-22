#!/usr/bin/env bash
set -euo pipefail

SESSION_NAME="codex-hook-bridge"

if ! command -v tmux >/dev/null 2>&1; then
  echo "stopped (tmux missing)"
  exit 1
fi

if tmux has-session -t "$SESSION_NAME" 2>/dev/null; then
  echo "running (tmux session '$SESSION_NAME')"
  exit 0
fi

echo "stopped"
exit 1
