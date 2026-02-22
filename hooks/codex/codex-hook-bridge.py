#!/usr/bin/env python3
import argparse
import fnmatch
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


DEFAULT_SESSION_ROOT = Path.home() / ".codex" / "sessions"
DEFAULT_CONFIG_PATH = Path.home() / ".codex" / "hooks.json"
CLAUDE_SETTINGS_PATH = Path.home() / ".claude" / "settings.json"
DEFAULT_LOG_PATH = Path.home() / ".codex" / "hooks" / "bridge.log"
class HookBridge:
    def __init__(
        self,
        session_root: Path,
        config_path: Path,
        log_path: Path,
        poll_interval: float,
    ) -> None:
        self.session_root = session_root
        self.config_path = config_path
        self.log_path = log_path
        self.poll_interval = poll_interval

        self.offsets: dict[Path, int] = {}
        self.trailing_bytes: dict[Path, bytes] = {}
        self.call_names: dict[str, str] = {}
        self.session_context: dict[Path, dict[str, Any]] = {}

        self.hooks_config: dict[str, Any] = {}
        self.config_file_in_use: Path | None = None
        self.config_mtime_ns: int | None = None
        self._load_config(force=True)

    def log(self, message: str) -> None:
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        ts = time.strftime("%Y-%m-%d %H:%M:%S")
        with self.log_path.open("a", encoding="utf-8") as f:
            f.write(f"[{ts}] {message}\n")

    def _candidate_config_paths(self) -> list[Path]:
        return [self.config_path, CLAUDE_SETTINGS_PATH]

    def _read_hook_config(self, path: Path) -> dict[str, Any]:
        with path.open("r", encoding="utf-8") as f:
            raw = json.load(f)
        hooks = raw.get("hooks") if isinstance(raw, dict) and "hooks" in raw else raw
        if not isinstance(hooks, dict):
            raise ValueError(f"hooks config in {path} is not an object")
        return hooks

    def _load_config(self, force: bool = False) -> None:
        chosen: Path | None = None
        for candidate in self._candidate_config_paths():
            if candidate.exists():
                chosen = candidate
                break
        if chosen is None:
            if force or self.hooks_config:
                self.hooks_config = {}
                self.config_file_in_use = None
                self.config_mtime_ns = None
            return

        mtime_ns = chosen.stat().st_mtime_ns
        if (
            not force
            and self.config_file_in_use == chosen
            and self.config_mtime_ns == mtime_ns
        ):
            return

        self.hooks_config = self._read_hook_config(chosen)
        self.config_file_in_use = chosen
        self.config_mtime_ns = mtime_ns
        self.log(f"Loaded hook config from {chosen}")

    def _match(self, matcher: str, event_payload: dict[str, Any], match_value: str) -> bool:
        if not matcher:
            return True

        if match_value and fnmatch.fnmatch(match_value, matcher):
            return True
        if match_value:
            try:
                if re.search(matcher, match_value):
                    return True
            except re.error:
                pass

        payload_text = json.dumps(event_payload, separators=(",", ":"), ensure_ascii=False)
        if matcher in payload_text:
            return True
        try:
            return bool(re.search(matcher, payload_text))
        except re.error:
            return False

    def _run_command_hook(
        self,
        command: str,
        payload: dict[str, Any],
        timeout_s: float,
        event_name: str,
    ) -> None:
        stdin_data = json.dumps(payload, separators=(",", ":"), ensure_ascii=False)
        env = os.environ.copy()
        env["CODEX_HOOK_EVENT_NAME"] = event_name
        env["CODEX_SESSION_ID"] = str(payload.get("session_id", ""))
        env["CODEX_PROJECT_DIR"] = str(payload.get("cwd", ""))
        # Compatibility env vars for existing Claude hook scripts.
        env["CLAUDE_PROJECT_DIR"] = env["CODEX_PROJECT_DIR"]

        try:
            proc = subprocess.run(
                command,
                input=stdin_data,
                text=True,
                shell=True,
                executable="/bin/bash",
                env=env,
                capture_output=True,
                timeout=timeout_s,
            )
        except subprocess.TimeoutExpired:
            self.log(f"Hook timeout: event={event_name} command={command} timeout={timeout_s}s")
            return
        except Exception as exc:
            self.log(f"Hook crashed: event={event_name} command={command} error={exc}")
            return

        if proc.returncode != 0:
            self.log(
                f"Hook failed: event={event_name} rc={proc.returncode} command={command} "
                f"stderr={proc.stderr.strip()[:300]}"
            )
        elif proc.stdout.strip() or proc.stderr.strip():
            self.log(
                f"Hook output: event={event_name} command={command} "
                f"stdout={proc.stdout.strip()[:200]} stderr={proc.stderr.strip()[:200]}"
            )

    def fire_event(self, event_name: str, payload: dict[str, Any], match_value: str = "") -> None:
        self._load_config(force=False)
        rules = self.hooks_config.get(event_name, [])
        if not isinstance(rules, list):
            self.log(f"Invalid hook section for {event_name}: expected list")
            return
        if not rules:
            return

        payload = dict(payload)
        payload.setdefault("hook_event_name", event_name)

        for rule in rules:
            if not isinstance(rule, dict):
                continue
            matcher = str(rule.get("matcher", "") or "")
            if not self._match(matcher, payload, match_value):
                continue

            hooks = rule.get("hooks", [])
            if not isinstance(hooks, list):
                continue
            for hook in hooks:
                if not isinstance(hook, dict):
                    continue
                if hook.get("type") != "command":
                    self.log(
                        f"Skipping unsupported hook type for {event_name}: "
                        f"{hook.get('type')}"
                    )
                    continue
                command = hook.get("command")
                if not isinstance(command, str) or not command.strip():
                    continue
                timeout = hook.get("timeout", 60)
                try:
                    timeout_s = float(timeout)
                except (TypeError, ValueError):
                    timeout_s = 60.0
                self._run_command_hook(command, payload, timeout_s, event_name)

    def _session_info_for(self, session_file: Path) -> dict[str, Any]:
        info = self.session_context.get(session_file, {})
        if info:
            return info

        try:
            with session_file.open("rb") as f:
                first = f.readline().decode("utf-8", errors="replace")
        except OSError:
            return {}

        try:
            obj = json.loads(first)
        except json.JSONDecodeError:
            return {}

        payload = obj.get("payload", {})
        if obj.get("type") == "session_meta" and isinstance(payload, dict):
            info = {
                "session_id": payload.get("id", ""),
                "cwd": payload.get("cwd", ""),
                "timestamp": payload.get("timestamp", ""),
            }
            self.session_context[session_file] = info
        return info

    def _handle_record(self, session_file: Path, record: dict[str, Any]) -> None:
        top_type = record.get("type")
        payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}

        if top_type == "session_meta":
            self.session_context[session_file] = {
                "session_id": payload.get("id", ""),
                "cwd": payload.get("cwd", ""),
                "timestamp": payload.get("timestamp", ""),
            }
            return

        payload_type = payload.get("type")
        if not payload_type:
            return

        meta = self._session_info_for(session_file)
        session_id = meta.get("session_id", "")
        cwd = meta.get("cwd", "")
        timestamp = record.get("timestamp", "")

        if payload_type == "user_message":
            event_payload = {
                "hook_event_name": "UserPromptSubmit",
                "session_id": session_id,
                "cwd": cwd,
                "timestamp": timestamp,
                "message": payload.get("message", ""),
                "raw_event": payload,
            }
            self.fire_event("UserPromptSubmit", event_payload, match_value=payload.get("message", ""))
            return

        if payload_type == "task_complete":
            event_payload = {
                "hook_event_name": "Stop",
                "session_id": session_id,
                "cwd": cwd,
                "timestamp": timestamp,
                "turn_id": payload.get("turn_id", ""),
                "last_assistant_message": payload.get("last_agent_message", ""),
                "raw_event": payload,
            }
            self.fire_event("Stop", event_payload, match_value="")
            return

        if payload_type == "turn_aborted":
            event_payload = {
                "hook_event_name": "Stop",
                "session_id": session_id,
                "cwd": cwd,
                "timestamp": timestamp,
                "turn_id": payload.get("turn_id", ""),
                "last_assistant_message": "",
                "raw_event": payload,
            }
            self.fire_event("Stop", event_payload, match_value="")
            return

        if payload_type == "function_call":
            call_id = record.get("payload", {}).get("call_id")
            call_name = record.get("payload", {}).get("name")
            if isinstance(call_id, str) and isinstance(call_name, str):
                self.call_names[call_id] = call_name
            return

        if payload_type == "function_call_output":
            output_text = str(payload.get("output", ""))
            m = re.search(r"Process exited with code (\d+)", output_text)
            if not m:
                return
            exit_code = int(m.group(1))
            if exit_code == 0:
                return

            call_id = payload.get("call_id", "")
            tool_name = self.call_names.get(call_id, "")
            event_payload = {
                "hook_event_name": "PostToolUseFailure",
                "session_id": session_id,
                "cwd": cwd,
                "timestamp": timestamp,
                "call_id": call_id,
                "tool_name": tool_name,
                "exit_code": exit_code,
                "raw_event": payload,
            }
            self.fire_event("PostToolUseFailure", event_payload, match_value=tool_name)

    def _process_chunk(self, session_file: Path, chunk: bytes) -> None:
        data = self.trailing_bytes.get(session_file, b"") + chunk
        lines = data.split(b"\n")
        self.trailing_bytes[session_file] = lines.pop() if lines else b""
        for raw_line in lines:
            if not raw_line.strip():
                continue
            try:
                record = json.loads(raw_line.decode("utf-8", errors="replace"))
            except json.JSONDecodeError:
                continue
            if not isinstance(record, dict):
                continue
            self._handle_record(session_file, record)

    def _poll_file(self, session_file: Path) -> None:
        try:
            stat = session_file.stat()
        except FileNotFoundError:
            return

        size = stat.st_size
        old_offset = self.offsets.get(session_file)
        if old_offset is None:
            # Start tailing from EOF for existing files.
            self.trailing_bytes[session_file] = b""
            self.offsets[session_file] = size
            return

        if size < old_offset:
            old_offset = 0
            self.trailing_bytes[session_file] = b""

        if size == old_offset:
            return

        try:
            with session_file.open("rb") as f:
                f.seek(old_offset)
                chunk = f.read(size - old_offset)
        except OSError:
            return

        self.offsets[session_file] = size
        if chunk:
            self._process_chunk(session_file, chunk)

    def run_forever(self) -> None:
        self.log("Hook bridge started")
        while True:
            try:
                if self.session_root.exists():
                    for session_file in self.session_root.rglob("*.jsonl"):
                        self._poll_file(session_file)
                self._load_config(force=False)
                time.sleep(self.poll_interval)
            except KeyboardInterrupt:
                self.log("Hook bridge stopped (KeyboardInterrupt)")
                return
            except Exception as exc:
                self.log(f"Bridge loop error: {exc}")
                time.sleep(self.poll_interval)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Bridge Codex session events to Claude-style hooks")
    parser.add_argument("--session-root", type=Path, default=DEFAULT_SESSION_ROOT)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG_PATH)
    parser.add_argument("--log", type=Path, default=DEFAULT_LOG_PATH)
    parser.add_argument("--poll-interval", type=float, default=0.5)
    parser.add_argument("--trigger", type=str, default="")
    parser.add_argument("--session-id", type=str, default="manual")
    parser.add_argument("--cwd", type=str, default=os.getcwd())
    parser.add_argument("--message", type=str, default="")
    parser.add_argument("--tool-name", type=str, default="")
    parser.add_argument("--exit-code", type=int, default=1)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    bridge = HookBridge(
        session_root=args.session_root,
        config_path=args.config,
        log_path=args.log,
        poll_interval=args.poll_interval,
    )

    if args.trigger:
        event_payload: dict[str, Any] = {
            "hook_event_name": args.trigger,
            "session_id": args.session_id,
            "cwd": args.cwd,
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "message": args.message,
            "tool_name": args.tool_name,
            "exit_code": args.exit_code,
        }
        match_value = args.message if args.trigger == "UserPromptSubmit" else args.tool_name
        bridge.fire_event(args.trigger, event_payload, match_value=match_value)
        return 0

    bridge.run_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main())
