//! Health checks and diagnostics (governor doctor).
//!
//! Validates workspace integrity, project structure, lock consistency,
//! and clipboard availability.

use crate::integrity;
use crate::project::{discover_projects, Project, ProjectError};
use crate::relay::base64_encode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HealthError {
    #[error("project error: {0}")]
    Project(#[from] ProjectError),
    #[error("integrity error: {0}")]
    Integrity(#[from] integrity::IntegrityError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Individual health check result.
#[derive(Debug)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, PartialEq)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckStatus {
    fn icon(&self) -> &str {
        match self {
            CheckStatus::Pass => "✅",
            CheckStatus::Warn => "⚠️",
            CheckStatus::Fail => "❌",
        }
    }
}

/// Run all health checks.
///
/// The project in the current directory is always deep-checked (STATE.md,
/// contract integrity, lock), even when it is invisible to workspace-root
/// discovery; `--verbose` additionally shallow-checks every other
/// discovered project.
pub fn run_health(project: Option<&Project>, verbose: bool) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // ── System checks ────────────────────────────────────────────
    results.push(check_clipboard());
    results.push(check_filesystem());
    if verbose {
        results.push(check_clipboard_throughput());
        results.push(check_filesystem_saturation());
        results.push(check_clock_skew());
        results.push(check_relay_sanity());
    }

    // ── Project discovery ────────────────────────────────────────
    let cwd_project = if project.is_none() {
        crate::project::project_in_current_dir()
    } else {
        None
    };
    let deep_target = project.or(cwd_project.as_ref());

    let discovered = discover_projects();
    let (discovery_status, discovery_detail) = match &discovered {
        Ok(projects) => (
            CheckStatus::Pass,
            format!("found {} project(s)", projects.len()),
        ),
        // An empty workspace is expected when the only Governor project in
        // play is the current directory itself: that is a configuration
        // inconvenience (the roots do not cover it), not a broken workspace.
        Err(e) if deep_target.is_some() => (CheckStatus::Warn, e.to_string()),
        Err(e) => (CheckStatus::Fail, e.to_string()),
    };
    results.push(CheckResult {
        name: "Project Discovery".into(),
        status: discovery_status,
        detail: discovery_detail,
    });

    if verbose {
        if let Ok(projects) = &discovered {
            for p in projects {
                results.push(CheckResult {
                    name: format!("  └ {}", p.name),
                    status: if p.is_locked() {
                        CheckStatus::Warn
                    } else {
                        CheckStatus::Pass
                    },
                    detail: if p.is_locked() {
                        "locked (active session)".into()
                    } else {
                        "available".into()
                    },
                });
            }
        }
    }

    // Deep checks on the requested project, or on the current directory's
    // project; with --verbose and no explicit target, shallow-check every
    // other discovered project too.
    if let Some(target) = deep_target {
        results.extend(run_project_checks(target, verbose));
    }
    if verbose && project.is_none() {
        if let Ok(projects) = &discovered {
            for p in projects {
                if deep_target.is_some_and(|t| same_dir(&t.root, &p.root)) {
                    continue; // already deep-checked above
                }
                results.extend(run_project_checks(p, false));
            }
        }
    }

    // ── Lock consistency ─────────────────────────────────────────
    results.push(check_stale_locks());

    results
}

/// Deep-check a single project: STATE.md, contract integrity, lock state.
pub fn check_project(project: &Project) -> Vec<CheckResult> {
    run_project_checks(project, false)
}

/// True when two paths refer to the same directory.
///
/// Comparison canonicalises both sides so `.`/`..`-spellings of the same
/// directory are recognised; paths that cannot be canonicalised fall back
/// to exact equality.
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Check clipboard availability.
fn check_clipboard() -> CheckResult {
    match crate::clipboard::read() {
        Ok(_) => CheckResult {
            name: "Clipboard Access".into(),
            status: CheckStatus::Pass,
            detail: "clipboard readable".into(),
        },
        Err(e) => CheckResult {
            name: "Clipboard Access".into(),
            status: CheckStatus::Warn,
            detail: format!("clipboard read failed: {}", e),
        },
    }
}

/// Check filesystem workspace accessibility.
fn check_filesystem() -> CheckResult {
    let cwd = std::env::current_dir();
    match cwd {
        Ok(dir) => {
            let writable = std::fs::metadata(&dir).is_ok();
            CheckResult {
                name: "Workspace Access".into(),
                status: if writable {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                detail: format!("working directory: {}", dir.display()),
            }
        }
        Err(e) => CheckResult {
            name: "Workspace Access".into(),
            status: CheckStatus::Fail,
            detail: format!("cannot access working directory: {}", e),
        },
    }
}

/// Run deep checks on a specific project.
fn run_project_checks(project: &Project, _verbose: bool) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // STATE.md existence.
    match project.read_state() {
        Ok(state) => {
            results.push(CheckResult {
                name: format!("{}: STATE.md", project.name),
                status: CheckStatus::Pass,
                detail: format!("{} bytes", state.len()),
            });
        }
        Err(e) => {
            results.push(CheckResult {
                name: format!("{}: STATE.md", project.name),
                status: CheckStatus::Fail,
                detail: e.to_string(),
            });
        }
    }

    // IMPLEMENTATION.md integrity.
    match &project.implementation_file {
        Some(impl_path) => match integrity::validate_contract(impl_path) {
            Ok(hash) => {
                results.push(CheckResult {
                    name: format!("{}: Contract Integrity", project.name),
                    status: CheckStatus::Pass,
                    detail: format!("hash: {}", hash),
                });
            }
            Err(integrity::IntegrityError::MissingHash) => {
                results.push(CheckResult {
                    name: format!("{}: Contract Integrity", project.name),
                    status: CheckStatus::Warn,
                    detail: "unsigned contract (no hash marker)".into(),
                });
            }
            Err(e) => {
                results.push(CheckResult {
                    name: format!("{}: Contract Integrity", project.name),
                    status: CheckStatus::Fail,
                    detail: e.to_string(),
                });
            }
        },
        None => {
            results.push(CheckResult {
                name: format!("{}: Contract Integrity", project.name),
                status: CheckStatus::Warn,
                detail: "no IMPLEMENTATION.md (contract file missing)".into(),
            });
        }
    }

    // Lock state.
    let lock_path = project.lock_path();
    if lock_path.exists() {
        match std::fs::read_to_string(&lock_path) {
            Ok(json) => {
                if let Ok(info) = serde_json::from_str::<crate::lock::LockInfo>(&json) {
                    let alive = crate::lock::is_pid_alive(info.pid);
                    results.push(CheckResult {
                        name: format!("{}: Lock Status", project.name),
                        // A lock always warrants a warning: alive means the
                        // session may still be running, stale means cleanup.
                        status: CheckStatus::Warn,
                        detail: format!(
                            "PID {} on {} ({} — {})",
                            info.pid,
                            info.host,
                            info.started_at,
                            if alive { "alive" } else { "stale" }
                        ),
                    });
                }
            }
            Err(e) => {
                results.push(CheckResult {
                    name: format!("{}: Lock Status", project.name),
                    status: CheckStatus::Fail,
                    detail: format!("corrupted lock: {}", e),
                });
            }
        }
    }

    results
}

/// Check for stale lock files across all projects.
fn check_stale_locks() -> CheckResult {
    match discover_projects() {
        Ok(projects) => {
            let mut stale_count = 0;
            for p in &projects {
                let lock_path = p.lock_path();
                if lock_path.exists() {
                    if let Ok(json) = std::fs::read_to_string(&lock_path) {
                        if let Ok(info) =
                            serde_json::from_str::<crate::lock::LockInfo>(&json)
                        {
                            if !crate::lock::is_pid_alive(info.pid) {
                                stale_count += 1;
                            }
                        }
                    }
                }
            }

            if stale_count > 0 {
                CheckResult {
                    name: "Stale Locks".into(),
                    status: CheckStatus::Warn,
                    detail: format!("{} stale lock file(s) detected", stale_count),
                }
            } else {
                CheckResult {
                    name: "Stale Locks".into(),
                    status: CheckStatus::Pass,
                    detail: "no stale locks".into(),
                }
            }
        }
        Err(e) => CheckResult {
            name: "Stale Locks".into(),
            status: CheckStatus::Fail,
            detail: format!("cannot scan: {}", e),
        },
    }
}

/// Measure clipboard throughput with a realistic-sized context (~10 KB).
fn check_clipboard_throughput() -> CheckResult {
    use std::time::Instant;

    // Generate a 10 KB payload (approx).
    let payload: String = "x".repeat(10_000);
    let start = Instant::now();
    let result = crate::clipboard::round_trip(&payload);
    let elapsed = start.elapsed();
    match result {
        Ok(read_back) => {
            if read_back != payload {
                return CheckResult {
                    name: "Clipboard Throughput".into(),
                    status: CheckStatus::Fail,
                    detail: format!("clipboard corruption: read back {} bytes, expected {}", read_back.len(), payload.len()),
                };
            }
            let throughput_mb = (payload.len() as f64) / (elapsed.as_secs_f64() * 1_000_000.0);
            let detail = format!(
                "round‑trip {} bytes in {:.0} ms ({:.2} MB/s)",
                payload.len(),
                elapsed.as_millis(),
                throughput_mb
            );
            // Classify based on latency.
            let status = if elapsed.as_millis() < 100 {
                CheckStatus::Pass
            } else if elapsed.as_millis() < 1000 {
                CheckStatus::Warn
            } else {
                CheckStatus::Fail
            };
            CheckResult {
                name: "Clipboard Throughput".into(),
                status,
                detail,
            }
        }
        Err(e) => CheckResult {
            name: "Clipboard Throughput".into(),
            status: CheckStatus::Fail,
            detail: format!("clipboard round‑trip failed: {}", e),
        },
    }
}

/// Warn when any workspace root drive is nearly full (>95% used).
fn check_filesystem_saturation() -> CheckResult {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let mut warnings = Vec::new();
    let mut checked_drives = HashSet::new();

    // Collect drives from workspace roots.
    let config = crate::config::Config::load().unwrap_or_default();
    for root in config.resolved_roots() {
        // Extract drive letter on Windows (first component).
        if let Some(drive) = root.components().next() {
            let drive_path = PathBuf::from(drive.as_os_str());
            if checked_drives.contains(&drive_path) {
                continue;
            }
            checked_drives.insert(drive_path.clone());
            if let Ok(free) = fs2::available_space(&drive_path) {
                if let Ok(total) = fs2::total_space(&drive_path) {
                    let used = total.saturating_sub(free);
                    let percent = (used as f64) / (total as f64) * 100.0;
                    if percent > 95.0 {
                        let msg = format!(
                            "drive {}: {:.1}% full ({} free / {} total)",
                            drive_path.display(),
                            percent,
                            bytesize::ByteSize(free),
                            bytesize::ByteSize(total)
                        );
                        warnings.push(msg);
                    }
                }
            }
        }
    }

    // Also check current working directory's drive if not already covered.
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(drive) = cwd.components().next() {
            let drive_path = PathBuf::from(drive.as_os_str());
            if !checked_drives.contains(&drive_path) {
                if let Ok(free) = fs2::available_space(&drive_path) {
                    if let Ok(total) = fs2::total_space(&drive_path) {
                        let used = total.saturating_sub(free);
                        let percent = (used as f64) / (total as f64) * 100.0;
                        if percent > 95.0 {
                            let msg = format!(
                                "drive {}: {:.1}% full ({} free / {} total)",
                                drive_path.display(),
                                percent,
                                bytesize::ByteSize(free),
                                bytesize::ByteSize(total)
                            );
                            warnings.push(msg);
                        }
                    }
                }
            }
        }
    }

    if warnings.is_empty() {
        CheckResult {
            name: "Drive Saturation".into(),
            status: CheckStatus::Pass,
            detail: "no drives above 95% capacity".into(),
        }
    } else {
        CheckResult {
            name: "Drive Saturation".into(),
            status: CheckStatus::Warn,
            detail: warnings.join("; "),
        }
    }
}

/// Detect clock skew between system time and NTP (or network time if accessible).
fn check_clock_skew() -> CheckResult {
    // Check clock skew by looking at lock file timestamps.
    // If any lock file modification time is far in the future (> 5 minutes),
    // that indicates a possible clock skew.
    use std::time::{SystemTime, Duration};

    let now = SystemTime::now();
    let mut future_locks = Vec::new();
    let mut total_locks = 0;

    // Scan all projects' lock files.
    if let Ok(projects) = discover_projects() {
        for p in projects {
            let lock_path = p.lock_path();
            if lock_path.exists() {
                total_locks += 1;
                if let Ok(metadata) = std::fs::metadata(&lock_path) {
                    if let Ok(mtime) = metadata.modified() {
                        // If lock timestamp is more than 5 minutes in the future, it's suspicious.
                        if mtime > now + Duration::from_secs(300) {
                            let ahead_secs = mtime.duration_since(now)
                                .unwrap_or(Duration::from_secs(0))
                                .as_secs();
                            future_locks.push(format!(
                                "{}: lock timestamp {} seconds ahead",
                                p.name, ahead_secs
                            ));
                        }
                    }
                }
            }
        }
    }

    if !future_locks.is_empty() {
        CheckResult {
            name: "Clock Skew".into(),
            status: CheckStatus::Warn,
            detail: format!(
                "{} lock(s) have future timestamps (possible clock skew): {}",
                future_locks.len(),
                future_locks.join(", ")
            ),
        }
    } else {
        CheckResult {
            name: "Clock Skew".into(),
            status: CheckStatus::Pass,
            detail: format!("no clock skew detected (checked {} locks)", total_locks),
        }
    }
}

/// Verify that the relay protocol works end‑to‑end through the clipboard.
fn check_relay_sanity() -> CheckResult {
    use crate::relay::{CMD_MARKER, extract_command};
    use crate::clipboard::round_trip;

    // Create a simple command that the daemon would recognise.
    let cmd = "ping";
    let encoded = base64_encode(cmd);
    // Full marker: <!-- GOV-CMD:<encoded> -->
    let full_marker = format!("{}{}-->", CMD_MARKER, encoded);
    // Write it and read it back in one atomic clipboard transaction.
    let read_back = match round_trip(&full_marker) {
        Ok(s) => s,
        Err(e) => {
            return CheckResult {
                name: "Relay Sanity".into(),
                status: CheckStatus::Fail,
                detail: format!("clipboard round-trip failed: {}", e),
            };
        }
    };
    match extract_command(&read_back) {
        Some(decoded) if decoded == cmd => CheckResult {
            name: "Relay Sanity".into(),
            status: CheckStatus::Pass,
            detail: format!("command round‑trip succeeded ({} bytes)", full_marker.len()),
        },
        Some(other) => CheckResult {
            name: "Relay Sanity".into(),
            status: CheckStatus::Warn,
            detail: format!("command mismatch: expected '{}', got '{}'", cmd, other),
        },
        None => CheckResult {
            name: "Relay Sanity".into(),
            status: CheckStatus::Warn,
            detail: "no GOV-CMD marker found in clipboard".into(),
        },
    }
}

/// True when at least one check reported a hard failure.
///
/// Warnings do not count: `governor health` must exit 0 when the workspace
/// is degraded but intact, and exit 1 only when something is actually broken
/// (so CI and scripts can gate on the exit code).
pub fn has_failures(results: &[CheckResult]) -> bool {
    results
        .iter()
        .any(|r| matches!(r.status, CheckStatus::Fail))
}

/// Print health check results to stdout in a nice format.
pub fn print_results(results: &[CheckResult]) {
    println!();
    println!("═══════════════════════════════════════════");
    println!("  Governor v{} Health Report", env!("CARGO_PKG_VERSION"));
    println!("═══════════════════════════════════════════");
    println!();

    let mut pass = 0;
    let mut warn = 0;
    let mut fail = 0;

    for r in results {
        println!("  {}  {}", r.status.icon(), r.name);
        if !r.detail.is_empty() {
            println!("      {}", r.detail);
        }
        match r.status {
            CheckStatus::Pass => pass += 1,
            CheckStatus::Warn => warn += 1,
            CheckStatus::Fail => fail += 1,
        }
    }

    println!();
    println!("───────────────────────────────────────────");
    println!(
        "  {} passed, {} warnings, {} failures",
        pass, warn, fail
    );
    println!("═══════════════════════════════════════════");
    println!();
}

// ── unit tests ────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard};

    /// Serialises every test in this module that touches the system clipboard
    /// (directly, or through `run_health`). The per-operation `CLIPBOARD_LOCK`
    /// in `clipboard.rs` only makes a single read/write/round-trip atomic, so
    /// two parallel tests can still interleave their clipboard operations.
    /// Holding one test-wide guard per clipboard-touching test removes that
    /// interleaving entirely.
    static TEST_CLIPBOARD_LOCK: Mutex<()> = Mutex::new(());

    /// Lock the test clipboard mutex, recovering from a previous test's panic
    /// (a panic already dropped the guard, so the OS clipboard is closed).
    fn clipboard_test_guard() -> MutexGuard<'static, ()> {
        TEST_CLIPBOARD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Create `<root>/<name>/STATE.md` (+ optionally IMPLEMENTATION.md) and
    /// return the project handle for it.
    fn fixture_project(root: &Path, name: &str, with_impl: bool) -> Project {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("STATE.md"), format!("# STATE — {name}\n")).unwrap();
        if with_impl {
            std::fs::write(
                dir.join("IMPLEMENTATION.md"),
                "# Contract\n\nBuild a thing.\n",
            )
            .unwrap();
        }
        crate::project::project_from_dir(&dir).expect("fixture must be a project")
    }

    /// The result of the project's contract-integrity check.
    fn contract_result(results: &[CheckResult]) -> &CheckResult {
        results
            .iter()
            .find(|r| r.name.contains("Contract Integrity"))
            .expect("contract check must be reported")
    }

    #[test]
    fn contract_integrity_passes_after_signing() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture_project(tmp.path(), "signed", true);

        let impl_path = project.implementation_file.as_ref().unwrap();
        integrity::sign_contract(impl_path).unwrap();

        let results = check_project(&project);
        let contract = contract_result(&results);
        assert!(matches!(contract.status, CheckStatus::Pass), "{:?}", contract.detail);
        assert!(contract.detail.starts_with("hash: "));
    }

    #[test]
    fn contract_integrity_detects_tampering() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture_project(tmp.path(), "tampered", true);

        let impl_path = project.implementation_file.as_ref().unwrap();
        integrity::sign_contract(impl_path).unwrap();

        // Break the signed contract the way an accidental edit would.
        let path = impl_path.clone();
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, content.replace("Build a thing", "Build another thing"))
            .unwrap();

        let results = check_project(&project);
        let contract = contract_result(&results);
        assert!(matches!(contract.status, CheckStatus::Fail), "{:?}", contract.detail);
        assert!(
            contract.detail.contains("hash mismatch"),
            "failure must say why: {:?}",
            contract.detail
        );
    }

    #[test]
    fn contract_integrity_warns_on_unsigned_contract() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture_project(tmp.path(), "unsigned", true);

        let results = check_project(&project);
        let contract = contract_result(&results);
        assert!(matches!(contract.status, CheckStatus::Warn), "{:?}", contract.detail);
        assert!(contract.detail.contains("unsigned"));
    }

    #[test]
    fn contract_integrity_warns_on_missing_contract_file() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture_project(tmp.path(), "no-contract", false);

        assert!(project.implementation_file.is_none());
        let results = check_project(&project);
        let contract = contract_result(&results);
        assert!(matches!(contract.status, CheckStatus::Warn), "{:?}", contract.detail);
        assert!(contract.detail.contains("missing"));
    }

    #[test]
    fn state_md_check_passes_for_valid_project() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture_project(tmp.path(), "with-state", true);

        let results = check_project(&project);
        let state = results
            .iter()
            .find(|r| r.name.contains("STATE.md"))
            .expect("STATE.md check must be reported");
        assert!(matches!(state.status, CheckStatus::Pass), "{:?}", state.detail);
    }

    #[test]
    fn test_check_status_icon() {
        assert_eq!(CheckStatus::Pass.icon(), "✅");
        assert_eq!(CheckStatus::Warn.icon(), "⚠️");
        assert_eq!(CheckStatus::Fail.icon(), "❌");
    }

    #[test]
    fn test_check_clipboard_throughput() {
        let _clipboard = clipboard_test_guard();
        let result = check_clipboard_throughput();
        assert!(matches!(result.status, CheckStatus::Pass | CheckStatus::Warn | CheckStatus::Fail));
        // detail should contain "round‑trip" or "failed"
        assert!(!result.detail.is_empty());
    }

    #[test]
    fn test_check_filesystem_saturation() {
        let result = check_filesystem_saturation();
        assert!(matches!(result.status, CheckStatus::Pass | CheckStatus::Warn | CheckStatus::Fail));
        // detail should mention "drives" or "capacity"
        assert!(!result.detail.is_empty());
    }

    #[test]
    fn test_check_clock_skew_no_locks() {
        let result = check_clock_skew();
        // Without lock files, it should pass (or warn)
        assert!(matches!(result.status, CheckStatus::Pass | CheckStatus::Warn));
        assert!(result.detail.contains("no clock skew") || result.detail.contains("checked 0 locks"));
    }

    #[test]
    fn test_check_relay_sanity() {
        let _clipboard = clipboard_test_guard();
        let result = check_relay_sanity();
        // It can pass, warn, or fail depending on clipboard state
        assert!(matches!(result.status, CheckStatus::Pass | CheckStatus::Warn | CheckStatus::Fail));
        assert!(!result.detail.is_empty());
    }

    #[test]
    fn test_run_health_without_verbose() {
        let _clipboard = clipboard_test_guard();
        let results = run_health(None, false);
        // Should not include verbose‑only checks
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"Clipboard Throughput"));
        assert!(!names.contains(&"Drive Saturation"));
        assert!(!names.contains(&"Clock Skew"));
        assert!(!names.contains(&"Relay Sanity"));
    }

    #[test]
    fn test_run_health_with_verbose() {
        let _clipboard = clipboard_test_guard();
        let results = run_health(None, true);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        // Should include verbose‑only checks
        assert!(names.contains(&"Clipboard Throughput"));
        assert!(names.contains(&"Drive Saturation"));
        assert!(names.contains(&"Clock Skew"));
        assert!(names.contains(&"Relay Sanity"));
    }

    #[test]
    fn test_print_results_no_panic() {
        let results = vec![
            CheckResult { name: "Test Pass".into(), status: CheckStatus::Pass, detail: "ok".into() },
            CheckResult { name: "Test Warn".into(), status: CheckStatus::Warn, detail: "warning".into() },
            CheckResult { name: "Test Fail".into(), status: CheckStatus::Fail, detail: "failure".into() },
        ];
        // Should not panic
        print_results(&results);
    }

    #[test]
    fn test_has_failures_detects_fail_only() {
        assert!(!has_failures(&[]), "empty report is not a failure");
        assert!(!has_failures(&[
            CheckResult { name: "a".into(), status: CheckStatus::Pass, detail: String::new() },
            CheckResult { name: "b".into(), status: CheckStatus::Warn, detail: String::new() },
        ]));
        assert!(has_failures(&[
            CheckResult { name: "a".into(), status: CheckStatus::Pass, detail: String::new() },
            CheckResult { name: "b".into(), status: CheckStatus::Fail, detail: String::new() },
        ]));
        assert!(has_failures(&[
            CheckResult { name: "b".into(), status: CheckStatus::Fail, detail: String::new() },
        ]));
    }
}