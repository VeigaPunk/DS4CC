#!/bin/bash
# GamePadCC state hook for Claude Code
# Writes agent state to the GamePadCC state file on hook events.
# Works from both Windows (Git Bash) and WSL.

INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name":"[^"]*"' | cut -d'"' -f4)

case "$EVENT" in
    UserPromptSubmit) STATE="working" ;;
    Stop)             STATE="done" ;;
    PostToolUseFailure) STATE="error" ;;
    *) exit 0 ;;
esac

# Resolve state file path
if [ -d "/mnt/c" ] && [ -f /proc/version ] && grep -qi microsoft /proc/version 2>/dev/null; then
    # WSL: write to Windows temp via /mnt/c
    WIN_USER=$(cmd.exe /C "echo %USERNAME%" 2>/dev/null | tr -d '\r')
    STATE_FILE="/mnt/c/Users/${WIN_USER}/AppData/Local/Temp/gamepadcc_state"
elif [ -n "$TEMP" ]; then
    # Windows (Git Bash / MSYS2): $TEMP is set
    STATE_FILE="$TEMP/gamepadcc_state"
else
    # Fallback
    STATE_FILE="/tmp/gamepadcc_state"
fi

printf '%s' "$STATE" > "$STATE_FILE"
exit 0
