/// Native Codex session JSONL poller.
///
/// Replaces the Python bridge entirely. Reads Codex session JSONL files
/// directly from the WSL filesystem via `\\wsl.localhost\` UNC paths and
/// writes `ds4cc_agent_*` state files to `%TEMP%` — the same format the
/// existing state aggregator already polls.
///
/// Skips silently if WSL is unavailable or Codex is not installed.

use crate::wsl::run_wsl;

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::time::{interval, Duration};

// ── Public API ──────────────────────────────────────────────────────

/// Resolve the Windows UNC path to the Codex sessions directory via WSL.
///
/// Returns `None` if WSL is unavailable or Codex is not installed.
pub fn resolve_sessions_dir() -> Option<PathBuf> {
    let output = run_wsl("test -d ~/.codex/sessions && wslpath -w ~/.codex/sessions")?;
    let path_str = output.trim();
    if path_str.is_empty() {
        return None;
    }
    let path = PathBuf::from(path_str);
    if path.exists() {
        log::info!("Codex sessions dir: {}", path.display());
        Some(path)
    } else {
        log::debug!("Codex sessions UNC path not accessible: {}", path.display());
        None
    }
}

/// Run the Codex JSONL poller loop. Scans for session files, reads new
/// JSONL records, and writes state files to `state_dir`.
pub async fn run(sessions_dir: PathBuf, state_dir: PathBuf, done_threshold_s: u64, poll_ms: u64) {
    let mut poller = CodexPoller::new(sessions_dir, state_dir, done_threshold_s);
    let mut ticker = interval(Duration::from_millis(poll_ms));

    loop {
        ticker.tick().await;
        // spawn_blocking because file I/O on UNC paths can block
        let mut poller_moved = poller;
        poller_moved = tokio::task::spawn_blocking(move || {
            poller_moved.poll();
            poller_moved
        })
        .await
        .unwrap_or_else(|_| {
            // If the blocking task panicked, create a fresh poller.
            // This should never happen, but prevents the task from dying.
            log::error!("Codex poller task panicked, resetting state");
            CodexPoller::new(
                PathBuf::new(), // will be replaced next iteration
                PathBuf::new(),
                done_threshold_s,
            )
        });
        poller = poller_moved;
    }
}

// ── Poller state ────────────────────────────────────────────────────

struct CodexPoller {
    sessions_dir: PathBuf,
    state_dir: PathBuf,
    done_threshold_s: u64,

    /// Per-file read offset (bytes already processed).
    offsets: HashMap<PathBuf, u64>,
    /// Incomplete trailing bytes from the last read (no newline yet).
    trailing: HashMap<PathBuf, Vec<u8>>,
    /// Cached session ID per JSONL file (from the `session_meta` record).
    session_ids: HashMap<PathBuf, String>,
    /// When each session entered "working" state (for done-threshold logic).
    working_since: HashMap<String, SystemTime>,
    /// Tracks function call_id → tool name for error attribution.
    call_names: HashMap<String, String>,
    /// Whether the initial scan has completed. Files discovered during the
    /// first poll jump to EOF (old sessions). Files discovered later are
    /// processed from line 2 (new sessions started after daemon).
    initial_scan_done: bool,
}

impl CodexPoller {
    fn new(sessions_dir: PathBuf, state_dir: PathBuf, done_threshold_s: u64) -> Self {
        Self {
            sessions_dir,
            state_dir,
            done_threshold_s,
            offsets: HashMap::new(),
            trailing: HashMap::new(),
            session_ids: HashMap::new(),
            working_since: HashMap::new(),
            call_names: HashMap::new(),
            initial_scan_done: false,
        }
    }

    /// One poll cycle: scan for JSONL files, read new data, process records.
    fn poll(&mut self) {
        let jsonl_files = match collect_jsonl_files(&self.sessions_dir) {
            Ok(files) => files,
            Err(_) => return, // sessions dir not accessible (WSL may be down)
        };

        for file_path in jsonl_files {
            self.poll_file(&file_path);
        }
        self.initial_scan_done = true;
    }

    fn poll_file(&mut self, file_path: &Path) {
        let size = match std::fs::metadata(file_path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };

        if !self.offsets.contains_key(file_path) {
            // First time seeing this file. Read line 1 for session_id.
            let first_line_end = self.extract_session_id(file_path);
            self.trailing.insert(file_path.to_path_buf(), Vec::new());

            if !self.initial_scan_done {
                // Initial scan: old session file — jump to EOF, don't replay.
                self.offsets.insert(file_path.to_path_buf(), size);
                return;
            }

            // New session appeared after daemon started — process from
            // after session_meta so we catch the first user_message.
            let start_offset = first_line_end.unwrap_or(0);
            self.offsets.insert(file_path.to_path_buf(), start_offset);
            if size <= start_offset {
                return; // only session_meta so far, nothing else to read
            }
        }

        let mut offset = self.offsets.get(file_path).copied().unwrap_or(0);

        // File was truncated/replaced — reset
        if size < offset {
            offset = 0;
            self.trailing.insert(file_path.to_path_buf(), Vec::new());
        }

        // No new data
        if size == offset {
            return;
        }

        // Read new bytes
        let chunk = match read_chunk(file_path, offset, size) {
            Some(c) => c,
            None => return,
        };

        self.offsets.insert(file_path.to_path_buf(), size);
        self.process_chunk(file_path, &chunk);
    }

    /// Read the first line of a JSONL file to extract the session_id from
    /// the `session_meta` record. Returns the byte offset just past the
    /// first newline (i.e., where line 2 starts).
    fn extract_session_id(&mut self, file_path: &Path) -> Option<u64> {
        let mut file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => return None,
        };
        let mut buf = vec![0u8; 4096]; // session_meta is typically < 2KB
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return None,
        };
        let data = &buf[..n];
        // Find the first newline to delimit line 1
        let newline_pos = data.iter().position(|&b| b == b'\n');
        let first_line_bytes = match newline_pos {
            Some(pos) => &data[..pos],
            None => data, // no newline yet — file may still be tiny
        };
        let first_line = String::from_utf8_lossy(first_line_bytes);
        if let Ok(record) = serde_json::from_str::<serde_json::Value>(&first_line) {
            if record.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
                if let Some(id) = record
                    .get("payload")
                    .and_then(|p| p.get("id"))
                    .and_then(|v| v.as_str())
                {
                    self.session_ids
                        .insert(file_path.to_path_buf(), id.to_string());
                }
            }
        }
        // Return offset past the newline (start of line 2)
        newline_pos.map(|pos| (pos + 1) as u64)
    }

    /// Process a chunk of bytes: split on newlines, parse complete JSON lines.
    fn process_chunk(&mut self, file_path: &Path, chunk: &[u8]) {
        let trailing = self
            .trailing
            .remove(&file_path.to_path_buf())
            .unwrap_or_default();

        let mut data = trailing;
        data.extend_from_slice(chunk);

        let mut lines: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();

        // Last element is either empty (line ended with \n) or incomplete
        let remainder = lines.pop().unwrap_or(&[]);
        self.trailing
            .insert(file_path.to_path_buf(), remainder.to_vec());

        for raw_line in lines {
            if raw_line.is_empty() {
                continue;
            }
            let line_str = String::from_utf8_lossy(raw_line);
            if let Ok(record) = serde_json::from_str::<serde_json::Value>(&line_str) {
                self.handle_record(file_path, &record);
            }
        }
    }

    /// Map a single JSONL record to a state file write.
    fn handle_record(&mut self, file_path: &Path, record: &serde_json::Value) {
        let top_type = record.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let payload = record.get("payload").and_then(|v| v.as_object());

        // Handle session_meta (first record in file)
        if top_type == "session_meta" {
            if let Some(p) = payload {
                if let Some(id) = p.get("id").and_then(|v| v.as_str()) {
                    self.session_ids
                        .insert(file_path.to_path_buf(), id.to_string());
                }
            }
            return;
        }

        let payload = match payload {
            Some(p) => p,
            None => return,
        };

        let payload_type = match payload.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return,
        };

        let session_id = match self.session_ids.get(&file_path.to_path_buf()) {
            Some(id) => id.clone(),
            None => return, // no session_meta seen yet
        };

        match payload_type {
            "user_message" => {
                self.working_since
                    .insert(session_id.clone(), SystemTime::now());
                self.write_state(&session_id, "working");
                self.write_start_timestamp(&session_id);
            }
            "task_complete" | "turn_aborted" => {
                let state = self.compute_done_state(&session_id);
                self.write_state(&session_id, state);
                self.working_since.remove(&session_id);
                self.remove_start_timestamp(&session_id);
            }
            "function_call" => {
                // Track call_id → tool name for error attribution
                if let (Some(call_id), Some(name)) = (
                    payload.get("call_id").and_then(|v| v.as_str()),
                    payload.get("name").and_then(|v| v.as_str()),
                ) {
                    self.call_names
                        .insert(call_id.to_string(), name.to_string());
                }
            }
            "function_call_output" => {
                // Non-zero exit codes transition the session to "error" state.
                if let Some(output) = payload.get("output").and_then(|v| v.as_str()) {
                    if has_nonzero_exit(output) {
                        self.write_state(&session_id, "error");
                    }
                }
            }
            _ => {}
        }
    }

    /// Decide whether a completed task should be "done" or "idle" based on
    /// how long it was "working".
    fn compute_done_state(&self, session_id: &str) -> &'static str {
        if let Some(start) = self.working_since.get(session_id) {
            if let Ok(elapsed) = start.elapsed() {
                if elapsed.as_secs() >= self.done_threshold_s {
                    return "done";
                }
            }
        }
        "idle"
    }

    fn write_state(&self, session_id: &str, state: &str) {
        let path = self.state_dir.join(format!("ds4cc_agent_{session_id}"));
        if let Err(e) = std::fs::write(&path, state) {
            log::debug!("Failed to write state file {}: {e}", path.display());
        }
    }

    fn write_start_timestamp(&self, session_id: &str) {
        let path = self
            .state_dir
            .join(format!("ds4cc_agent_{session_id}_start"));
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();
        let _ = std::fs::write(&path, ts);
    }

    fn remove_start_timestamp(&self, session_id: &str) {
        let path = self
            .state_dir
            .join(format!("ds4cc_agent_{session_id}_start"));
        let _ = std::fs::remove_file(&path);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Recursively collect all `.jsonl` files under a directory.
fn collect_jsonl_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    collect_jsonl_recursive(dir, &mut result)?;
    Ok(result)
}

fn collect_jsonl_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Ignore errors in subdirectories (e.g., permission issues)
            let _ = collect_jsonl_recursive(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
    Ok(())
}

/// Returns true if the tool output string contains a non-zero process exit code,
/// e.g. "Process exited with code 1" or "Process exited with code 127".
fn has_nonzero_exit(output: &str) -> bool {
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("Process exited with code ") {
            if let Ok(code) = rest.trim().parse::<i32>() {
                return code != 0;
            }
        }
    }
    false
}

/// Read bytes from `offset` to `size` in a file.
fn read_chunk(path: &Path, offset: u64, size: u64) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let to_read = (size - offset) as usize;
    let mut buf = vec![0u8; to_read];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_nonzero_exit() {
        assert!(has_nonzero_exit("Process exited with code 1"));
        assert!(has_nonzero_exit("some output\nProcess exited with code 127\n"));
        assert!(!has_nonzero_exit("Process exited with code 0"));
        assert!(!has_nonzero_exit("no exit code here"));
    }

    #[test]
    fn test_collect_jsonl_nonexistent_dir() {
        let result = collect_jsonl_files(Path::new(r"C:\nonexistent\path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_poller_full_lifecycle() {
        let test_dir = std::env::temp_dir().join("ds4cc_codex_poll_test");
        let sessions_dir = test_dir.join("sessions");
        let state_dir = test_dir.join("state");
        let _ = std::fs::create_dir_all(&sessions_dir);
        let _ = std::fs::create_dir_all(&state_dir);

        let mut poller = CodexPoller::new(sessions_dir.clone(), state_dir.clone(), 600);

        // Create a JSONL session file
        let session_file = sessions_dir.join("test-session.jsonl");
        std::fs::write(
            &session_file,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"test-123\",\"cwd\":\"/tmp\"}}\n",
        )
        .unwrap();

        // First poll: discovers file, jumps to EOF, extracts session_id
        poller.poll();
        assert_eq!(
            poller.session_ids.get(&session_file),
            Some(&"test-123".to_string())
        );
        // No state file yet (jumped to EOF)
        assert!(!state_dir.join("ds4cc_agent_test-123").exists());

        // Append user_message event
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&session_file)
            .unwrap();
        writeln!(
            f,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"test\"}}}}"
        )
        .unwrap();
        drop(f);

        // Second poll: should read the new line and write "working"
        poller.poll();
        assert_eq!(
            std::fs::read_to_string(state_dir.join("ds4cc_agent_test-123")).unwrap(),
            "working"
        );
        assert!(state_dir.join("ds4cc_agent_test-123_start").exists());

        // Append task_complete (quick task → should go to "idle")
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&session_file)
            .unwrap();
        writeln!(
            f,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"t1\"}}}}"
        )
        .unwrap();
        drop(f);

        poller.poll();
        assert_eq!(
            std::fs::read_to_string(state_dir.join("ds4cc_agent_test-123")).unwrap(),
            "idle"
        );
        assert!(!state_dir.join("ds4cc_agent_test-123_start").exists());

        // Append another user_message then function_call_output with error
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&session_file)
            .unwrap();
        writeln!(
            f,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"fix bug\"}}}}"
        )
        .unwrap();
        writeln!(
            f,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"function_call_output\",\"output\":\"Process exited with code 1\"}}}}"
        )
        .unwrap();
        drop(f);

        poller.poll();
        // user_message sets "working"; function_call_output with non-zero exit writes "error".
        assert_eq!(
            std::fs::read_to_string(state_dir.join("ds4cc_agent_test-123")).unwrap(),
            "error"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_realistic_codex_jsonl_format() {
        // Uses the exact JSONL format that Codex CLI produces, including
        // timestamps, extra fields, and nested directory structure.
        let test_dir = std::env::temp_dir().join("ds4cc_codex_realistic_test");
        let sessions_dir = test_dir.join("sessions").join("2026").join("02").join("22");
        let state_dir = test_dir.join("state");
        let _ = std::fs::create_dir_all(&sessions_dir);
        let _ = std::fs::create_dir_all(&state_dir);

        // Use the top-level sessions dir (recursive scan should find the file)
        let mut poller = CodexPoller::new(test_dir.join("sessions"), state_dir.clone(), 600);

        let session_file = sessions_dir.join("rollout-2026-02-22T08-16-51-test.jsonl");

        use std::io::Write;
        // Write realistic session_meta (first line, with all extra fields)
        let mut f = std::fs::File::create(&session_file).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:57.670Z","type":"session_meta","payload":{{"id":"019c846c-3bd5-7593-bdef-de03296a30b1","timestamp":"2026-02-22T08:16:51.669Z","cwd":"/home/vhpnk/CodexCli","originator":"codex_cli_rs","cli_version":"0.105.0-alpha.10","source":"cli","model_provider":"openai"}}}}"#).unwrap();
        drop(f);

        // First poll: discover + extract session_id
        poller.poll();
        assert_eq!(
            poller.session_ids.get(&session_file),
            Some(&"019c846c-3bd5-7593-bdef-de03296a30b1".to_string())
        );

        // Append response_item (should be ignored), then user_message
        let mut f = std::fs::OpenOptions::new().append(true).open(&session_file).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:57.670Z","type":"response_item","payload":{{"type":"message","role":"developer","content":[{{"type":"input_text","text":"test"}}]}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:57.670Z","type":"turn_context","payload":{{"turn_id":"019c846c-5340","cwd":"/home/vhpnk/CodexCli","model":"gpt-5.3-codex"}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:57.671Z","type":"event_msg","payload":{{"type":"user_message","message":"hello","images":[],"local_images":[],"text_elements":[]}}}}"#).unwrap();
        drop(f);

        poller.poll();
        let sid = "019c846c-3bd5-7593-bdef-de03296a30b1";
        assert_eq!(
            std::fs::read_to_string(state_dir.join(format!("ds4cc_agent_{sid}"))).unwrap(),
            "working"
        );

        // Append agent_message + token_count + task_complete (all realistic)
        let mut f = std::fs::OpenOptions::new().append(true).open(&session_file).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:59.205Z","type":"event_msg","payload":{{"type":"agent_message","message":"Hi. What do you want to work on?","phase":"final_answer"}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:59.216Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":8338}}}}}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-22T08:16:59.216Z","type":"event_msg","payload":{{"type":"task_complete","turn_id":"019c846c-5340-7633-9584-6ab41cb8832d","last_agent_message":"Hi. What do you want to work on?"}}}}"#).unwrap();
        drop(f);

        poller.poll();
        assert_eq!(
            std::fs::read_to_string(state_dir.join(format!("ds4cc_agent_{sid}"))).unwrap(),
            "idle" // quick task → idle, not done
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    /// Live integration test: simulate a new Codex session appearing after
    /// daemon startup. Verifies the poller processes events from the start
    /// (not jumping to EOF like it does for pre-existing sessions).
    #[test]
    fn test_live_unc_new_session_detection() {
        let unc = PathBuf::from(r"\\wsl.localhost\Ubuntu\home\vhpnk\.codex\sessions");
        if !unc.exists() {
            eprintln!("Skipping live UNC test: WSL path not accessible");
            return;
        }

        let state_dir = std::env::temp_dir();
        let test_session_dir = unc.join("_test");
        let _ = std::fs::create_dir_all(&test_session_dir);
        let session_file = test_session_dir.join("new-session-test.jsonl");

        // Clean up any previous test artifacts
        let _ = std::fs::remove_file(&session_file);
        let _ = std::fs::remove_file(state_dir.join("ds4cc_agent_new-sess-001"));
        let _ = std::fs::remove_file(state_dir.join("ds4cc_agent_new-sess-001_start"));

        let mut poller = CodexPoller::new(unc.clone(), state_dir.clone(), 600);

        // First poll: initial scan, discovers existing files, jumps to EOF
        poller.poll();
        assert!(poller.initial_scan_done, "initial_scan_done should be set");

        // Now create a NEW session file (simulates Codex CLI starting)
        use std::io::Write;
        let mut f = std::fs::File::create(&session_file).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-23T00:00:00.000Z","type":"session_meta","payload":{{"id":"new-sess-001","cwd":"/tmp"}}}}"#).unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-23T00:00:01.000Z","type":"event_msg","payload":{{"type":"user_message","message":"hello","images":[]}}}}"#).unwrap();
        drop(f);

        // Second poll: should discover new file and process ALL events
        // (not jump to EOF since initial_scan_done = true)
        poller.poll();
        assert_eq!(
            poller.session_ids.get(&session_file),
            Some(&"new-sess-001".to_string()),
            "Should extract session ID from new file"
        );
        let state_path = state_dir.join("ds4cc_agent_new-sess-001");
        assert!(
            state_path.exists(),
            "State file should exist — new session events were processed"
        );
        assert_eq!(
            std::fs::read_to_string(&state_path).unwrap(),
            "working",
            "State should be 'working' from user_message in new session"
        );

        // Append task_complete
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&session_file)
            .unwrap();
        writeln!(f, r#"{{"timestamp":"2026-02-23T00:00:05.000Z","type":"event_msg","payload":{{"type":"task_complete","turn_id":"t1","last_agent_message":"done"}}}}"#).unwrap();
        drop(f);

        poller.poll();
        assert_eq!(
            std::fs::read_to_string(&state_path).unwrap(),
            "idle",
            "State should be 'idle' after quick task_complete"
        );

        // Clean up
        let _ = std::fs::remove_file(&session_file);
        let _ = std::fs::remove_dir(&test_session_dir);
        let _ = std::fs::remove_file(&state_path);
        let _ = std::fs::remove_file(state_dir.join("ds4cc_agent_new-sess-001_start"));
        eprintln!("Live UNC new-session detection test passed!");
    }

    /// Integration test: verify the poller can read real Codex JSONL files
    /// from the WSL UNC path (only runs if WSL is available).
    #[test]
    fn test_read_real_codex_sessions_via_unc() {
        let unc = PathBuf::from(r"\\wsl.localhost\Ubuntu\home\vhpnk\.codex\sessions");
        if !unc.exists() {
            eprintln!("Skipping UNC test: WSL path not accessible");
            return;
        }

        let files = collect_jsonl_files(&unc).expect("Should read UNC sessions dir");
        assert!(!files.is_empty(), "Should find at least one JSONL file");

        // Try to parse the first line of the first file
        let first_file = &files[0];
        let mut file = std::fs::File::open(first_file).expect("Should open JSONL file via UNC");
        let mut buf = vec![0u8; 8192];
        let n = file.read(&mut buf).expect("Should read from UNC");
        let text = String::from_utf8_lossy(&buf[..n]);
        let first_line = text.lines().next().expect("Should have at least one line");
        let record: serde_json::Value =
            serde_json::from_str(first_line).expect("First line should be valid JSON");
        assert_eq!(
            record.get("type").and_then(|v| v.as_str()),
            Some("session_meta"),
            "First record should be session_meta"
        );
        assert!(
            record.get("payload").and_then(|p| p.get("id")).is_some(),
            "session_meta should have payload.id"
        );
        eprintln!(
            "Successfully read {} JSONL files via UNC, first session: {}",
            files.len(),
            record["payload"]["id"].as_str().unwrap_or("?")
        );
    }
}
