#!/bin/bash
# GamePadCC hook installer
# Run this from the GamePadCC repo directory to set up Claude Code hooks.
# Works from Windows (Git Bash) and WSL.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOK_SRC="$SCRIPT_DIR/hooks/gamepadcc-state.sh"

if [ ! -f "$HOOK_SRC" ]; then
    echo "Error: hooks/gamepadcc-state.sh not found. Run this from the GamePadCC repo root."
    exit 1
fi

# Install hook script
HOOK_DIR="$HOME/.claude/hooks"
mkdir -p "$HOOK_DIR"
cp "$HOOK_SRC" "$HOOK_DIR/gamepadcc-state.sh"
sed -i 's/\r$//' "$HOOK_DIR/gamepadcc-state.sh" 2>/dev/null || true
chmod +x "$HOOK_DIR/gamepadcc-state.sh"
echo "Installed hook script to $HOOK_DIR/gamepadcc-state.sh"

# Merge hooks into settings.json
SETTINGS="$HOME/.claude/settings.json"
if command -v python3 &>/dev/null; then
    python3 - "$SETTINGS" << 'PYEOF'
import json, sys, os

path = sys.argv[1]
cfg = {}
if os.path.exists(path):
    with open(path) as f:
        cfg = json.load(f)

hook_entry = [{"matcher": "", "hooks": [{"type": "command", "command": "~/.claude/hooks/gamepadcc-state.sh"}]}]
hooks = cfg.get("hooks", {})
hooks["UserPromptSubmit"] = hook_entry
hooks["Stop"] = hook_entry
hooks["PostToolUseFailure"] = hook_entry
cfg["hooks"] = hooks

with open(path, "w") as f:
    json.dump(cfg, f, indent=2)
    f.write("\n")

print(f"Updated {path}")
PYEOF
elif command -v node &>/dev/null; then
    node -e "
const fs = require('fs');
const path = process.argv[1];
let cfg = {};
try { cfg = JSON.parse(fs.readFileSync(path, 'utf8')); } catch {}
const hook = [{matcher: '', hooks: [{type: 'command', command: '~/.claude/hooks/gamepadcc-state.sh'}]}];
cfg.hooks = cfg.hooks || {};
cfg.hooks.UserPromptSubmit = hook;
cfg.hooks.Stop = hook;
cfg.hooks.PostToolUseFailure = hook;
fs.writeFileSync(path, JSON.stringify(cfg, null, 2) + '\n');
console.log('Updated ' + path);
" "$SETTINGS"
else
    echo "Warning: Neither python3 nor node found. Please manually add hooks to $SETTINGS"
    echo "See hooks/setup.json for the required configuration."
    exit 1
fi

# ── Codex hook bridge (optional) ──────────────────────────────────────

CODEX_DIR="$HOME/.codex"
if [ -d "$CODEX_DIR" ]; then
    echo ""
    echo "Codex detected — installing hook bridge..."
    CODEX_HOOK_DIR="$CODEX_DIR/hooks"
    mkdir -p "$CODEX_HOOK_DIR"

    # Copy all Codex hook scripts
    CODEX_SRC="$SCRIPT_DIR/hooks/codex"
    if [ -d "$CODEX_SRC" ]; then
        for f in codex-hook-bridge.py gamepadcc-state.sh start.sh stop.sh status.sh install-from-claude.sh; do
            if [ -f "$CODEX_SRC/$f" ]; then
                cp "$CODEX_SRC/$f" "$CODEX_HOOK_DIR/$f"
                sed -i 's/\r$//' "$CODEX_HOOK_DIR/$f" 2>/dev/null || true
                chmod +x "$CODEX_HOOK_DIR/$f"
            fi
        done
        echo "Installed Codex hook scripts to $CODEX_HOOK_DIR/"

        # Deploy hooks.json only if not already present
        CODEX_HOOKS_JSON="$CODEX_DIR/hooks.json"
        if [ ! -f "$CODEX_HOOKS_JSON" ]; then
            cp "$CODEX_SRC/hooks.json" "$CODEX_HOOKS_JSON"
            sed -i 's/\r$//' "$CODEX_HOOKS_JSON" 2>/dev/null || true
            echo "Installed $CODEX_HOOKS_JSON"
        else
            echo "hooks.json already exists, not overwriting"
        fi

        echo ""
        echo "To start the Codex hook bridge:"
        echo "  $CODEX_HOOK_DIR/start.sh"
    else
        echo "Warning: hooks/codex/ directory not found in repo. Skipping Codex setup."
    fi
else
    echo ""
    echo "(Codex not detected — skipping Codex hook bridge setup)"
fi

echo ""
echo "Done. Restart Claude Code for hooks to take effect."
