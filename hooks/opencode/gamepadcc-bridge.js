// OpenCode plugin for GamePadCC lightbar state integration.
//
// Listens to OpenCode session events and writes GamePadCC state files
// via the shared gamepadcc-state.sh hook script.
//
// Events mapped:
//   session.status "active"  → UserPromptSubmit  (working)
//   session.status "idle"    → Stop              (done)
//   session.status "error"   → PostToolUseFailure(error)
//   tool.execute.before      → UserPromptSubmit  (working)
//
// Deployed to: ~/.config/opencode/plugins/gamepadcc-bridge.js
// State script: ~/.config/opencode/plugins/gamepadcc-state.sh

import { execSync } from "child_process";
import { existsSync } from "fs";
import { join } from "path";

const PLUGINS_DIR = join(process.env.HOME || "~", ".config/opencode/plugins");
const STATE_SCRIPT = join(PLUGINS_DIR, "gamepadcc-state.sh");

function fireHook(eventName, sessionID, cwd) {
  if (!existsSync(STATE_SCRIPT)) return;

  const payload = JSON.stringify({
    hook_event_name: eventName,
    session_id: sessionID || "opencode",
    cwd: cwd || process.cwd(),
  });

  try {
    execSync(`bash "${STATE_SCRIPT}"`, {
      input: payload,
      env: {
        ...process.env,
        CLAUDE_PROJECT_DIR: cwd || process.cwd(),
      },
      timeout: 10000,
      stdio: ["pipe", "pipe", "pipe"],
    });
  } catch (_) {
    // silently ignore hook failures
  }
}

export default async ({ directory }) => ({
  event: async ({ event }) => {
    const props = event.properties || {};
    const sid = props.sessionID || "";

    if (event.type === "session.status") {
      if (props.status === "active") fireHook("UserPromptSubmit", sid, directory);
      else if (props.status === "idle") fireHook("Stop", sid, directory);
      else if (props.status === "error") fireHook("PostToolUseFailure", sid, directory);
    }
  },

  "tool.execute.before": async (input) => {
    fireHook("UserPromptSubmit", input.sessionID, directory);
  },
});
