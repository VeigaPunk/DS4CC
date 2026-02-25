#!/bin/bash
# DS4CC state hook for Claude Code
# Each agent session writes its own state file: ds4cc_agent_<session_id>
# The daemon aggregates all agent files to determine lightbar color.
#
# "done" only fires if the task took longer than the threshold.
# Set DS4CC_DONE_THRESHOLD_S to override (default: 600 = 10 minutes).

DONE_THRESHOLD_S="${DS4CC_DONE_THRESHOLD_S:-600}"
STALE_WORKING_S="${DS4CC_STALE_WORKING_S:-900}"

INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name":"[^"]*"' | cut -d'"' -f4)
SESSION_ID=$(echo "$INPUT" | grep -o '"session_id":"[^"]*"' | cut -d'"' -f4)

# Fallback session ID if not found
if [ -z "$SESSION_ID" ]; then
    SESSION_ID="unknown_$$"
fi

# Resolve state directory (same temp dir the daemon watches)
if [ -n "${DS4CC_STATE_DIR:-}" ]; then
    STATE_DIR="$DS4CC_STATE_DIR"
fi

if [ -z "${STATE_DIR:-}" ] && [ -d "/mnt/c" ] && [ -f /proc/version ] && grep -qi microsoft /proc/version 2>/dev/null; then
    # Fast path: scan for the dedicated DS4CC subdir under each user's Temp folder.
    for _d in /mnt/c/Users/*/AppData/Local/Temp/DS4CC; do
        if [ -d "$_d" ]; then
            STATE_DIR="$_d"
            break
        fi
    done
fi

if [ -z "${STATE_DIR:-}" ] && [ -n "${TEMP:-}" ]; then
    STATE_DIR="$TEMP/DS4CC"
fi

if [ -z "${STATE_DIR:-}" ]; then
    STATE_DIR="/tmp/DS4CC"
fi

mkdir -p "$STATE_DIR"

AGENT_FILE="${STATE_DIR}/ds4cc_agent_${SESSION_ID}"
TIMESTAMP_FILE="${AGENT_FILE}_start"

prune_stale_agents() {
    local now state start_file start_ts mtime age f
    now=$(date +%s)
    for f in "${STATE_DIR}"/ds4cc_agent_*; do
        [ -e "$f" ] || continue
        case "$f" in
            *_start) continue ;;
        esac
        state=$(cat "$f" 2>/dev/null || true)
        [ "$state" = "working" ] || continue
        start_file="${f}_start"
        if [ -f "$start_file" ]; then
            start_ts=$(cat "$start_file" 2>/dev/null || true)
        else
            start_ts=""
        fi
        if ! [[ "$start_ts" =~ ^[0-9]+$ ]]; then
            mtime=$(stat -c %Y "$f" 2>/dev/null || echo "$now")
            start_ts="$mtime"
        fi
        age=$((now - start_ts))
        if [ "$age" -gt "$STALE_WORKING_S" ]; then
            printf '%s' "idle" > "$f"
            rm -f "$start_file"
        fi
    done
}

prune_stale_agents

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
