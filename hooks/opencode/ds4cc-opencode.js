// DS4CC state plugin for OpenCode
// Writes ds4cc_agent_<sessionID> state files consumed by the DS4CC daemon.
//
// Install (global — works for all projects):
//   cp ds4cc-opencode.js ~/.config/opencode/plugins/
//
// Or use install-hooks.sh from the DS4CC repo root which handles this automatically.
//
// Environment variables:
//   DS4CC_STATE_DIR         Override state file directory
//   DS4CC_DONE_THRESHOLD_S  Minimum working seconds before "done" fires (default: 600)
//   DS4CC_STALE_WORKING_S   Stale working threshold in seconds (default: 900)
//
// State file protocol (same as Claude Code and Codex integrations):
//   ds4cc_agent_<sessionID>        → "working" | "idle" | "done" | "error"
//   ds4cc_agent_<sessionID>_start  → unix timestamp (seconds) of when working started

import {
  writeFileSync,
  mkdirSync,
  existsSync,
  readdirSync,
  statSync,
  unlinkSync,
  readFileSync,
} from "fs";
import { join } from "path";

const DONE_THRESHOLD_S = parseInt(process.env.DS4CC_DONE_THRESHOLD_S ?? "600", 10);
const STALE_WORKING_S = parseInt(process.env.DS4CC_STALE_WORKING_S ?? "900", 10);

// ── State directory resolution ──────────────────────────────────────
// Two supported contexts:
//
//   WSL CLI  — OpenCode is a Linux/WSL process. process.env.TEMP = /tmp (useless).
//              We resolve the Windows temp dir via the /mnt/c mount point so the
//              Windows ds4cc daemon can read the state files.
//
//   Windows Desktop — OpenCode is a native Windows process. /mnt/c doesn't exist.
//              process.env.TEMP = C:\Users\...\AppData\Local\Temp — use it directly.

// Windows user accounts that are never real users (skip in directory scan).
const SYSTEM_USERS = new Set(["All Users", "Default", "Default User", "Public", "desktop.ini"]);

function resolveStateDir() {
  if (process.env.DS4CC_STATE_DIR) {
    return process.env.DS4CC_STATE_DIR;
  }

  // WSL: scan /mnt/c/Users/*/AppData/Local/Temp (skipping system accounts).
  // Always return the DS4CC subdir — the daemon creates it on startup.
  const wslBase = "/mnt/c/Users";
  if (existsSync(wslBase)) {
    try {
      const users = readdirSync(wslBase).filter((u) => !SYSTEM_USERS.has(u));
      for (const user of users) {
        const tmpDir = join(wslBase, user, "AppData/Local/Temp");
        if (existsSync(tmpDir)) return join(tmpDir, "DS4CC");
      }
    } catch {}
  }

  // Windows Desktop fallback: TEMP/TMP are proper Windows paths here
  if (process.env.TEMP && process.env.TEMP !== "/tmp") return join(process.env.TEMP, "DS4CC");
  if (process.env.TMP && process.env.TMP !== "/tmp") return join(process.env.TMP, "DS4CC");

  return "/tmp/DS4CC";
}

// ── File helpers ────────────────────────────────────────────────────

function writeState(stateDir, sessionId, state) {
  try {
    mkdirSync(stateDir, { recursive: true });
    writeFileSync(join(stateDir, `ds4cc_agent_${sessionId}`), state);
  } catch {}
}

function writeTimestamp(stateDir, sessionId) {
  try {
    writeFileSync(
      join(stateDir, `ds4cc_agent_${sessionId}_start`),
      String(Math.floor(Date.now() / 1000))
    );
  } catch {}
}

function removeTimestamp(stateDir, sessionId) {
  try {
    unlinkSync(join(stateDir, `ds4cc_agent_${sessionId}_start`));
  } catch {}
}

// ── Stale agent pruning ─────────────────────────────────────────────

function pruneStaleAgents(stateDir) {
  const nowS = Math.floor(Date.now() / 1000);
  try {
    for (const file of readdirSync(stateDir)) {
      if (!file.startsWith("ds4cc_agent_") || file.endsWith("_start")) continue;
      const filePath = join(stateDir, file);
      try {
        if (readFileSync(filePath, "utf8").trim() !== "working") continue;
        const sessionId = file.slice("ds4cc_agent_".length);
        const startFile = join(stateDir, `ds4cc_agent_${sessionId}_start`);
        let startTs;
        try {
          startTs = parseInt(readFileSync(startFile, "utf8").trim(), 10);
        } catch {
          startTs = Math.floor(statSync(filePath).mtimeMs / 1000);
        }
        if (!isFinite(startTs)) continue;
        if (nowS - startTs > STALE_WORKING_S) {
          writeFileSync(filePath, "idle");
          try { unlinkSync(startFile); } catch {}
        }
      } catch {}
    }
  } catch {}
}

// ── State transition helpers ────────────────────────────────────────

function setWorking(stateDir, sessionId, workingStart) {
  if (!sessionId || workingStart.has(sessionId)) return;
  workingStart.set(sessionId, Date.now());
  writeState(stateDir, sessionId, "working");
  writeTimestamp(stateDir, sessionId);
}

function setDoneOrIdle(stateDir, sessionId, workingStart) {
  if (!sessionId) return;
  const startMs = workingStart.get(sessionId);
  workingStart.delete(sessionId);
  removeTimestamp(stateDir, sessionId);
  if (startMs != null) {
    const elapsedS = (Date.now() - startMs) / 1000;
    writeState(stateDir, sessionId, elapsedS >= DONE_THRESHOLD_S ? "done" : "idle");
  } else {
    writeState(stateDir, sessionId, "idle");
  }
}

// ── Plugin export ───────────────────────────────────────────────────

export const DS4CCPlugin = async () => {
  const stateDir = resolveStateDir();

  // sessionId → working start timestamp (ms). Cleared on idle/error.
  const workingStart = new Map();

  return {
    // ── Generic event bus ──────────────────────────────────────────
    event: async ({ event }) => {
      // Event payload is under event.properties; session ID varies by event type:
      //   session.status/idle/error  → props.sessionID
      //   session.created/deleted    → props.info.id
      const props = event.properties ?? {};
      const sessionId =
        props.sessionID ||
        props.session_id ||
        props.info?.id ||
        "";
      if (!sessionId) return;

      pruneStaleAgents(stateDir);

      switch (event.type) {
        case "session.status": {
          // status is an object: { type: "busy" | "idle", ... }
          const statusType = props.status?.type;
          if (statusType === "busy") {
            setWorking(stateDir, sessionId, workingStart);
          } else if (statusType === "idle") {
            setDoneOrIdle(stateDir, sessionId, workingStart);
          }
          break;
        }

        case "session.idle": {
          setDoneOrIdle(stateDir, sessionId, workingStart);
          break;
        }

        case "session.error": {
          writeState(stateDir, sessionId, "error");
          workingStart.delete(sessionId);
          removeTimestamp(stateDir, sessionId);
          break;
        }

        case "session.deleted": {
          workingStart.delete(sessionId);
          try { unlinkSync(join(stateDir, `ds4cc_agent_${sessionId}`)); } catch {}
          removeTimestamp(stateDir, sessionId);
          break;
        }
      }
    },

    // ── Tool execution: reliable working signal ────────────────────
    // Fires before every tool call, giving us a second path to detect
    // the "working" state even if session.status fires late.
    "tool.execute.before": async (input) => {
      const sessionId = input.sessionID || input.session_id || "";
      if (!sessionId) return;
      pruneStaleAgents(stateDir);
      setWorking(stateDir, sessionId, workingStart);
    },
  };
};
