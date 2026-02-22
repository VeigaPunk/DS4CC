#!/bin/bash
# GamePadCC state hook for Claude Code
# Each agent session writes its own state file: gamepadcc_agent_<session_id>
# The daemon aggregates all agent files to determine lightbar color.
#
# "done" only fires if the task took longer than the threshold.
# Set GAMEPADCC_DONE_THRESHOLD_S to override (default: 600 = 10 minutes).

DONE_THRESHOLD_S="${GAMEPADCC_DONE_THRESHOLD_S:-600}"

INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name":"[^"]*"' | cut -d'"' -f4)
SESSION_ID=$(echo "$INPUT" | grep -o '"session_id":"[^"]*"' | cut -d'"' -f4)

# Fallback session ID if not found
if [ -z "$SESSION_ID" ]; then
    SESSION_ID="unknown_$$"
fi

# Resolve state directory (same temp dir the daemon watches)
if [ -d "/mnt/c" ] && [ -f /proc/version ] && grep -qi microsoft /proc/version 2>/dev/null; then
    WIN_USER=$(cmd.exe /C "echo %USERNAME%" 2>/dev/null | tr -d '\r')
    STATE_DIR="/mnt/c/Users/${WIN_USER}/AppData/Local/Temp"
elif [ -n "$TEMP" ]; then
    STATE_DIR="$TEMP"
else
    STATE_DIR="/tmp"
fi

AGENT_FILE="${STATE_DIR}/gamepadcc_agent_${SESSION_ID}"
TIMESTAMP_FILE="${AGENT_FILE}_start"

case "$EVENT" in
    UserPromptSubmit)
        # Record start time and set working
        date +%s > "$TIMESTAMP_FILE"
        printf '%s' "working" > "$AGENT_FILE"
        ;;
    Stop)
        # Only fire "done" if task exceeded threshold, otherwise go idle
        if [ -f "$TIMESTAMP_FILE" ]; then
            START=$(cat "$TIMESTAMP_FILE")
            NOW=$(date +%s)
            ELAPSED=$((NOW - START))
            rm -f "$TIMESTAMP_FILE"
            if [ "$ELAPSED" -ge "$DONE_THRESHOLD_S" ]; then
                printf '%s' "done" > "$AGENT_FILE"
            else
                printf '%s' "idle" > "$AGENT_FILE"
            fi
        else
            printf '%s' "idle" > "$AGENT_FILE"
        fi
        ;;
    PostToolUseFailure)
        printf '%s' "error" > "$AGENT_FILE"
        ;;
    *)
        exit 0
        ;;
esac

exit 0
