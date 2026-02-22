#!/bin/bash
# GamePadCC state hook for Claude Code
# Writes agent state to the GamePadCC state file on hook events.
# Works from both Windows (Git Bash) and WSL.
#
# "done" only fires if the task took longer than the threshold.
# Set GAMEPADCC_DONE_THRESHOLD_S to override (default: 600 = 10 minutes).

DONE_THRESHOLD_S="${GAMEPADCC_DONE_THRESHOLD_S:-600}"

INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name":"[^"]*"' | cut -d'"' -f4)

# Resolve state file path
if [ -d "/mnt/c" ] && [ -f /proc/version ] && grep -qi microsoft /proc/version 2>/dev/null; then
    WIN_USER=$(cmd.exe /C "echo %USERNAME%" 2>/dev/null | tr -d '\r')
    STATE_FILE="/mnt/c/Users/${WIN_USER}/AppData/Local/Temp/gamepadcc_state"
elif [ -n "$TEMP" ]; then
    STATE_FILE="$TEMP/gamepadcc_state"
else
    STATE_FILE="/tmp/gamepadcc_state"
fi

TIMESTAMP_FILE="${STATE_FILE}_start"

case "$EVENT" in
    UserPromptSubmit)
        # Record start time and set working
        date +%s > "$TIMESTAMP_FILE"
        printf '%s' "working" > "$STATE_FILE"
        ;;
    Stop)
        # Only fire "done" if task exceeded threshold, otherwise go idle
        if [ -f "$TIMESTAMP_FILE" ]; then
            START=$(cat "$TIMESTAMP_FILE")
            NOW=$(date +%s)
            ELAPSED=$((NOW - START))
            rm -f "$TIMESTAMP_FILE"
            if [ "$ELAPSED" -ge "$DONE_THRESHOLD_S" ]; then
                printf '%s' "done" > "$STATE_FILE"
            else
                printf '%s' "idle" > "$STATE_FILE"
            fi
        else
            printf '%s' "idle" > "$STATE_FILE"
        fi
        ;;
    PostToolUseFailure)
        printf '%s' "error" > "$STATE_FILE"
        ;;
    *)
        exit 0
        ;;
esac

exit 0
