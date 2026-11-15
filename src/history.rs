//! Session history log (`<project>/.governor/history.jsonl`).
//!
//! An append-only JSON Lines log: each line is one session event, so a
//! session is written incrementally without ever rewriting the file — the
//! reason JSONL was chosen over a single JSON document. `governor start`
//! appends a `start` event, `governor close` appends a `close` event with
//! the final status, and `governor history` pairs the events back up for
//! display.
//!
//! Event schema:
//!
//! ```text
//! {"event":"start","ts":"2026-11-15T14:30:00Z","pid":4242,"host":"DESKTOP","contract_hash":"a1b2…"}
//! {"event":"close","ts":"2026-11-15T15:05:12Z","status":"success"}
//! ```

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HistoryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("history file {} line {}: malformed JSON — {source}", path.display(), line)]
    MalformedLine {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot serialize history event: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Final status of a session, derived from contract validation at close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Close completed and the contract validated (or there was none).
    Success,
    /// Close completed, but something needed attention — e.g. an unsigned
    /// contract was signed during close.
    Warning,
    /// Contract validation failed (e.g. hash mismatch).
    Fail,
}

impl SessionStatus {
    /// Table label for this status.
    pub fn label(self) -> &'static str {
        match self {
            SessionStatus::Success => "Success",
            SessionStatus::Warning => "Warning",
            SessionStatus::Fail => "Fail",
        }
    }
}

/// One line of `history.jsonl`: a session start or a session close.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HistoryEvent {
    /// Emitted by `governor start`.
    Start {
        /// RFC 3339 UTC timestamp.
        ts: DateTime<Utc>,
        pid: u32,
        host: String,
        /// SHA-256 of `IMPLEMENTATION.md` at session start, if any.
        contract_hash: Option<String>,
    },
    /// Emitted by `governor close`.
    Close {
        /// RFC 3339 UTC timestamp.
        ts: DateTime<Utc>,
        status: SessionStatus,
    },
}

/// Path of the history file inside a project root.
pub fn history_path(project_root: &Path) -> PathBuf {
    project_root.join(".governor").join("history.jsonl")
}

/// Append a session-start event (timestamp, PID, host, contract hash).
pub fn record_start(project_root: &Path, contract_hash: Option<&str>) -> Result<(), HistoryError> {
    let event = HistoryEvent::Start {
        ts: Utc::now(),
        pid: std::process::id(),
        host: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
        contract_hash: contract_hash.map(str::to_string),
    };
    append(project_root, &event)
}

/// Append a session-close event with the final status.
pub fn record_close(project_root: &Path, status: SessionStatus) -> Result<(), HistoryError> {
    append(
        project_root,
        &HistoryEvent::Close {
            ts: Utc::now(),
            status,
        },
    )
}

/// Append one event as a new JSONL line (creating the file when needed).
fn append(project_root: &Path, event: &HistoryEvent) -> Result<(), HistoryError> {
    let path = history_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(event)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Read and parse every event from the project's history file.
///
/// A missing file is not an error — it simply means "no history yet" and
/// yields an empty list. A malformed line *is* an error: silently skipping
/// the damage would misreport sessions, so the reader stops with the file
/// path and the offending line number instead.
pub fn load(project_root: &Path) -> Result<Vec<HistoryEvent>, HistoryError> {
    let path = history_path(project_root);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut events = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue; // tolerate blank / trailing lines
        }
        let event = serde_json::from_str(line).map_err(|source| HistoryError::MalformedLine {
            path: path.clone(),
            line: index + 1,
            source,
        })?;
        events.push(event);
    }
    Ok(events)
}

/// A paired start/close record ready for display.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub started: Option<DateTime<Utc>>,
    pub closed: Option<DateTime<Utc>>,
    pub pid: Option<u32>,
    pub host: Option<String>,
    pub contract_hash: Option<String>,
    pub status: Option<SessionStatus>,
}

impl SessionRecord {
    /// Table label: the final status, or `Active` while the session is open.
    pub fn status_label(&self) -> &str {
        self.status.map_or("Active", SessionStatus::label)
    }
}

/// Pair `start`/`close` events into session records, preserving order.
///
/// A `close` attaches to the most recent still-open session — the one the
/// running `governor close` would actually end. A close with no open start
/// is kept as an orphan record so the status is not lost; starts that never
/// see a close stay open and render as `Active`.
pub fn collect_sessions(events: &[HistoryEvent]) -> Vec<SessionRecord> {
    let mut open: Vec<SessionRecord> = Vec::new();
    let mut sessions: Vec<SessionRecord> = Vec::new();

    for event in events {
        match event {
            HistoryEvent::Start {
                ts,
                pid,
                host,
                contract_hash,
            } => open.push(SessionRecord {
                started: Some(*ts),
                closed: None,
                pid: Some(*pid),
                host: Some(host.clone()),
                contract_hash: contract_hash.clone(),
                status: None,
            }),
            HistoryEvent::Close { ts, status } => match open.pop() {
                Some(mut session) => {
                    session.closed = Some(*ts);
                    session.status = Some(*status);
                    sessions.push(session);
                }
                None => sessions.push(SessionRecord {
                    started: None,
                    closed: Some(*ts),
                    pid: None,
                    host: None,
                    contract_hash: None,
                    status: Some(*status),
                }),
            },
        }
    }

    // Still-open sessions are the newest rows, so they come last.
    sessions.extend(open);
    sessions
}

/// Print the session history table (the most recent `limit` sessions).
///
/// With no history at all prints a friendly notice instead of an error.
pub fn print_history(project_root: &Path, limit: usize) -> Result<(), HistoryError> {
    let events = load(project_root)?;
    if events.is_empty() {
        println!("No session history found for this project");
        return Ok(());
    }

    let sessions = collect_sessions(&events);
    let total = sessions.len();
    let shown = &sessions[total.saturating_sub(limit)..];

    println!();
    println!(
        "  Session history — last {} of {} session(s):",
        shown.len(),
        total
    );
    println!();

    let header = ["Date", "Start", "Duration", "Status", "Contract hash"].map(String::from);
    let rows: Vec<[String; 5]> = shown.iter().map(row_cells).collect();

    // Column widths = the longest cell (header included), by char count.
    let mut widths = [0usize; 5];
    for (w, cell) in widths.iter_mut().zip(header.iter()) {
        *w = cell.chars().count();
    }
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row.iter()) {
            *w = (*w).max(cell.chars().count());
        }
    }

    print_row(&header, &widths);
    let rule = widths.iter().sum::<usize>() + 2 * (widths.len() - 1);
    println!("  {}", "─".repeat(rule));
    for row in &rows {
        print_row(row, &widths);
    }
    println!();
    Ok(())
}

/// Left-aligned one line of the table.
fn print_row(cells: &[String; 5], widths: &[usize; 5]) {
    let mut line = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            line.push_str("  ");
        }
        line.push_str(&format!("{:<width$}", cell, width = widths[i]));
    }
    println!("  {}", line.trim_end());
}

/// Format one record into table cells: date, start time, duration, status,
/// contract hash. Timestamps render in the local time zone; an orphan close
/// borrows its close timestamp for the date column, an open session has no
/// duration yet.
fn row_cells(session: &SessionRecord) -> [String; 5] {
    let (date, start) = match session.started.or(session.closed) {
        Some(ts) => {
            let local = ts.with_timezone(&Local);
            (
                local.format("%Y-%m-%d").to_string(),
                local.format("%H:%M:%S").to_string(),
            )
        }
        None => ("-".to_string(), "-".to_string()),
    };
    let duration = match (session.started, session.closed) {
        (Some(started), Some(closed)) => format_duration(closed.signed_duration_since(started)),
        _ => "-".to_string(),
    };
    [
        date,
        start,
        duration,
        session.status_label().to_string(),
        hash_label(session.contract_hash.as_deref()),
    ]
}

/// Shorten a contract hash for the table: the first 8 hex characters.
fn hash_label(hash: Option<&str>) -> String {
    match hash {
        Some(h) if h.len() > 8 => format!("{}…", &h[..8]),
        Some(h) => h.to_string(),
        None => "-".to_string(),
    }
}

/// Human-readable duration: `45s`, `20m 15s`, `1h 2m 3s`.
fn format_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds().max(0);
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a fixed RFC 3339 timestamp for deterministic tests.
    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn start_event(s: &str, pid: u32, hash: Option<&str>) -> HistoryEvent {
        HistoryEvent::Start {
            ts: ts(s),
            pid,
            host: "test-host".to_string(),
            contract_hash: hash.map(str::to_string),
        }
    }

    fn close_event(s: &str, status: SessionStatus) -> HistoryEvent {
        HistoryEvent::Close {
            ts: ts(s),
            status,
        }
    }

    #[test]
    fn record_start_appends_a_start_event() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        record_start(root, Some("a1b2c3d4")).unwrap();

        assert!(history_path(root).exists());
        let events = load(root).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            HistoryEvent::Start {
                pid,
                host,
                contract_hash,
                ..
            } => {
                assert_eq!(*pid, std::process::id());
                assert!(!host.is_empty(), "hostname must be recorded");
                assert_eq!(contract_hash.as_deref(), Some("a1b2c3d4"));
            }
            other => panic!("expected Start, got {other:?}"),
        }
    }

    #[test]
    fn record_close_appends_a_close_event() {
        let tmp = tempfile::tempdir().unwrap();
        record_start(tmp.path(), None).unwrap();
        record_close(tmp.path(), SessionStatus::Fail).unwrap();

        let events = load(tmp.path()).unwrap();
        assert_eq!(events.len(), 2);
        match &events[1] {
            HistoryEvent::Close { status, .. } => assert_eq!(*status, SessionStatus::Fail),
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn start_and_close_pair_into_one_finished_session() {
        let tmp = tempfile::tempdir().unwrap();
        record_start(tmp.path(), None).unwrap();
        record_close(tmp.path(), SessionStatus::Success).unwrap();

        let sessions = collect_sessions(&load(tmp.path()).unwrap());
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.status, Some(SessionStatus::Success));
        assert_eq!(session.status_label(), "Success");
        let started = session.started.unwrap();
        let closed = session.closed.unwrap();
        assert!(closed >= started, "close must not precede start");
    }

    #[test]
    fn duration_matches_start_and_close_gap() {
        let events = vec![
            start_event("2026-11-15T10:00:00Z", 1, None),
            close_event("2026-11-15T10:20:15Z", SessionStatus::Success),
        ];
        let session = &collect_sessions(&events)[0];
        let gap = session.closed.unwrap().signed_duration_since(session.started.unwrap());
        assert_eq!(format_duration(gap), "20m 15s");
    }

    #[test]
    fn format_duration_units() {
        assert_eq!(format_duration(chrono::Duration::seconds(45)), "45s");
        assert_eq!(format_duration(chrono::Duration::seconds(1215)), "20m 15s");
        assert_eq!(format_duration(chrono::Duration::seconds(3723)), "1h 2m 3s");
        // Defensive: a negative gap must not render as "-1s".
        assert_eq!(format_duration(chrono::Duration::seconds(-5)), "0s");
    }

    #[test]
    fn malformed_line_reports_file_and_line_number() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".governor")).unwrap();
        let path = history_path(root);
        std::fs::write(
            &path,
            "{\"event\":\"close\",\"ts\":\"2026-11-15T10:00:00Z\",\"status\":\"success\"}\nnot-json-at-all\n",
        )
        .unwrap();

        let err = load(root).unwrap_err();
        match &err {
            HistoryError::MalformedLine { path: p, line, .. } => {
                assert_eq!(p, &path);
                assert_eq!(*line, 2, "the corrupt line is the 2nd one");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
        let msg = err.to_string();
        assert!(
            msg.contains("malformed JSON") && msg.contains("line 2"),
            "message must name the problem and the line: {msg}"
        );
    }

    #[test]
    fn missing_history_file_is_an_empty_log() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn blank_lines_are_tolerated() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".governor")).unwrap();
        std::fs::write(
            history_path(tmp.path()),
            "\n{\"event\":\"close\",\"ts\":\"2026-11-15T11:00:00Z\",\"status\":\"fail\"}\n\n",
        )
        .unwrap();

        let events = load(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn unclosed_start_stays_active() {
        let events = vec![start_event("2026-11-15T10:00:00Z", 7, Some("cafe"))];
        let sessions = collect_sessions(&events);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status_label(), "Active");
        assert_eq!(sessions[0].contract_hash.as_deref(), Some("cafe"));
        assert!(sessions[0].closed.is_none());
    }

    #[test]
    fn orphan_close_is_kept_as_a_row() {
        let events = vec![close_event("2026-11-15T11:00:00Z", SessionStatus::Fail)];
        let sessions = collect_sessions(&events);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status_label(), "Fail");
        assert!(sessions[0].started.is_none());
    }

    #[test]
    fn close_attaches_to_the_latest_open_start() {
        let events = vec![
            start_event("2026-11-15T10:00:00Z", 1, None),
            start_event("2026-11-15T10:05:00Z", 2, None),
            close_event("2026-11-15T10:06:00Z", SessionStatus::Success),
        ];
        let sessions = collect_sessions(&events);
        assert_eq!(sessions.len(), 2);
        // The close pairs with the 10:05 start — the running session.
        assert_eq!(sessions[0].started, Some(ts("2026-11-15T10:05:00Z")));
        assert_eq!(sessions[0].status_label(), "Success");
        assert_eq!(sessions[1].started, Some(ts("2026-11-15T10:00:00Z")));
        assert_eq!(sessions[1].status_label(), "Active");
    }

    #[test]
    fn hash_label_shortens_long_hashes() {
        let long = "a".repeat(64);
        assert_eq!(hash_label(Some(&long)), "aaaaaaaa…");
        assert_eq!(hash_label(Some("abc")), "abc");
        assert_eq!(hash_label(None), "-");
    }

    #[test]
    fn print_history_renders_a_real_log() {
        // Smoke test: rendering a mixed log (finished + active session) must
        // not error; output goes to the captured test stdout.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        record_start(root, Some(&"f".repeat(64))).unwrap();
        record_close(root, SessionStatus::Warning).unwrap();
        record_start(root, None).unwrap();

        print_history(root, 10).unwrap();
        print_history(root, 1).unwrap(); // limit cuts the finished session
    }

    #[test]
    fn print_history_with_empty_project_prints_a_notice() {
        // A project without history must render the friendly notice path,
        // not an error. Output goes to the captured test stdout.
        let tmp = tempfile::tempdir().unwrap();
        print_history(tmp.path(), 10).unwrap();
    }
}
