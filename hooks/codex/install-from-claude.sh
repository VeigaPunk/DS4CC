#!/usr/bin/env bash
set -euo pipefail

CLAUDE_SETTINGS="$HOME/.claude/settings.json"
CLAUDE_HOOK_SCRIPT="$HOME/.claude/hooks/gamepadcc-state.sh"
CODEX_HOOK_DIR="$HOME/.codex/hooks"
CODEX_HOOKS_JSON="$HOME/.codex/hooks.json"
CODEX_HOOK_SCRIPT="$CODEX_HOOK_DIR/gamepadcc-state.sh"

mkdir -p "$CODEX_HOOK_DIR"

if [[ ! -f "$CLAUDE_SETTINGS" ]]; then
  echo "missing $CLAUDE_SETTINGS"
  exit 1
fi

if command -v jq >/dev/null 2>&1; then
  jq '{hooks: (.hooks // {})}' "$CLAUDE_SETTINGS" > "$CODEX_HOOKS_JSON"
else
  python3 - <<'PY'
import json
from pathlib import Path

claude = Path.home() / ".claude" / "settings.json"
codex = Path.home() / ".codex" / "hooks.json"

data = json.loads(claude.read_text(encoding="utf-8"))
hooks = data.get("hooks", {})
codex.write_text(json.dumps({"hooks": hooks}, indent=2) + "\n", encoding="utf-8")
PY
fi

if [[ -f "$CLAUDE_HOOK_SCRIPT" && ! -f "$CODEX_HOOK_SCRIPT" ]]; then
  cp "$CLAUDE_HOOK_SCRIPT" "$CODEX_HOOK_SCRIPT"
  chmod +x "$CODEX_HOOK_SCRIPT"
fi

sed -i 's#~/.claude/hooks/#~/.codex/hooks/#g' "$CODEX_HOOKS_JSON"

chmod +x "$CODEX_HOOK_DIR"/start.sh "$CODEX_HOOK_DIR"/stop.sh "$CODEX_HOOK_DIR"/status.sh "$CODEX_HOOK_DIR"/codex-hook-bridge.py

echo "installed hooks to $CODEX_HOOKS_JSON"
echo "run: $CODEX_HOOK_DIR/start.sh"
