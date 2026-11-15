//! Lock manager — exclusive access control for Governor projects.
//!
//! Each active session holds a lock file (`session.lock`) in the project's
//! `.governor/` directory. The lock records the holder's PID, hostname,
//! and session start time.  Dead locks (stale PIDs) are automatically cleaned.

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

/// How long [`SessionLock::acquire_with_timeout`] sleeps between polls of the
/// lock file.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Error, Debug)]
pub enum LockError {
    #[error("lock already held by PID {pid} on {host} since {since}")]
    AlreadyLocked {
        pid: u32,
        host: String,
        since: String,
    },
    #[error("cannot acquire lock: {0}")]
    IoError(#[from] std::io::Error),
    #[error("no active session to release")]
    NotLocked,
    #[error("lock file corrupted: {0}")]
    Corrupted(String),
}

/// Information recorded in a session lock file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    /// PID of the process holding the lock.
    pub pid: u32,
    /// Hostname where the session runs.
    pub host: String,
    /// ISO-8601 timestamp when the session started.
    pub started_at: String,
    /// Governor version that created the lock.
    pub version: String,
}

/// A session lock handle — releases on drop if still held.
#[derive(Debug)]
pub struct SessionLock {
    lock_path: PathBuf,
    /// Whether this instance currently holds the lock.
    acquired: bool,
    /// Poll cycles spent by the last [`SessionLock::acquire_with_timeout`] call.
    retries: u32,
}

impl SessionLock {
    /// Create a new lock handle (does not acquire yet).
    pub fn new(lock_path: PathBuf) -> Self {
        Self {
            lock_path,
            acquired: false,
            retries: 0,
        }
    }

    /// Attempt to acquire the lock for this process.
    ///
    /// Returns an error if another live process already holds it.
    /// Cleans up stale locks automatically.
    pub fn acquire(&mut self) -> Result<(), LockError> {
        self.acquire_with_timeout(0)
    }

    /// Attempt to acquire the lock, waiting up to `timeout_seconds` seconds.
    ///
    /// If `timeout_seconds` is 0, behaves like [`SessionLock::acquire`].
    /// While waiting, polls the lock file every [`POLL_INTERVAL`].
    /// Returns `Ok(())` when the lock is acquired, `Err(LockError::AlreadyLocked)`
    /// if the timeout expires before the lock becomes available.
    pub fn acquire_with_timeout(&mut self, timeout_seconds: u64) -> Result<(), LockError> {
        let start_time = std::time::Instant::now();
        let timeout_duration = Duration::from_secs(timeout_seconds);
        self.retries = 0;

        loop {
            match self.try_acquire_once() {
                Ok(()) => return Ok(()),
                Err(LockError::AlreadyLocked { pid, host, since }) => {
                    // Check if timeout has expired
                    if start_time.elapsed() >= timeout_duration {
                        return Err(LockError::AlreadyLocked { pid, host, since });
                    }
                    // Wait a bit before retrying
                    self.retries += 1;
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Poll cycles the last [`SessionLock::acquire_with_timeout`] call spent
    /// waiting, i.e. how many times it slept and tried again.
    ///
    /// `0` means the outcome was decided on the first attempt without any
    /// waiting — which is what "timeout 0 equals no wait" actually promises,
    /// and it can be asserted without timing the call (wall-clock assertions on
    /// this path are machine-load dependent).
    pub fn retry_count(&self) -> u32 {
        self.retries
    }

    /// Try to acquire the lock once, without waiting.
    fn try_acquire_once(&mut self) -> Result<(), LockError> {
        if self.lock_path.exists() {
            match self.read_lock_info() {
                Ok(info) => {
                    if is_pid_alive(info.pid) {
                        return Err(LockError::AlreadyLocked {
                            pid: info.pid,
                            host: info.host,
                            since: info.started_at,
                        });
                    }
                    // Stale lock — clean it up.
                    std::fs::remove_file(&self.lock_path)?;
                }
                Err(_) => {
                    // Corrupted lock file — clean it up.
                    std::fs::remove_file(&self.lock_path)?;
                }
            }
        }

        // Ensure parent directory exists.
        if let Some(parent) = self.lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let info = LockInfo {
            pid: std::process::id(),
            host: hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            started_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let json = serde_json::to_string_pretty(&info)
            .map_err(|e| LockError::Corrupted(e.to_string()))?;
        std::fs::write(&self.lock_path, json)?;

        self.acquired = true;
        Ok(())
    }

    /// Release the lock (removes the lock file).
    pub fn release(&mut self) -> Result<(), LockError> {
        if !self.acquired {
            return Err(LockError::NotLocked);
        }
        if self.lock_path.exists() {
            std::fs::remove_file(&self.lock_path)?;
        }
        self.acquired = false;
        Ok(())
    }

    /// Read current lock info from disk.
    fn read_lock_info(&self) -> Result<LockInfo, LockError> {
        let content = std::fs::read_to_string(&self.lock_path)?;
        serde_json::from_str(&content).map_err(|e| LockError::Corrupted(e.to_string()))
    }

    /// Check whether this handle currently holds the lock.
    pub fn is_acquired(&self) -> bool {
        self.acquired
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        // Best-effort release on drop (don't panic in destructor).
        if self.acquired {
            let _ = std::fs::remove_file(&self.lock_path);
        }
    }
}

/// Check whether a process with the given PID is still running.
///
/// Implemented on top of the `sysinfo` crate (cross-platform). Unlike the raw
/// `OpenProcess`/`GetExitCodeProcess` probe, this does not depend on the
/// `STILL_ACTIVE` exit-code sentinel, which can be misreported for
/// partially-dereferenced handles, and unlike the earlier `tasklist.exe`
/// shell-out it spawns no process. Only the one PID of interest is refreshed,
/// so the cost stays in the microsecond range.
pub fn is_pid_alive(pid: u32) -> bool {
    use sysinfo::{ProcessesToUpdate, System};

    let target = sysinfo::Pid::from_u32(pid);
    let mut system = System::new();
    // Refresh just this PID: a full process-table walk is orders of magnitude
    // more expensive and unnecessary for a single existence check.
    system.refresh_processes(ProcessesToUpdate::Some(&[target]));
    system.process(target).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    #[test]
    fn acquire_with_zero_timeout_is_immediate() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("test.lock");
        let mut lock = SessionLock::new(lock_path.clone());
        
        // First acquire should succeed
        lock.acquire().unwrap();
        assert!(lock.is_acquired());
        
        // Create a second lock for the same path
        let mut lock2 = SessionLock::new(lock_path);
        
        // With zero timeout, should fail immediately
        let start = Instant::now();
        let result = lock2.acquire_with_timeout(0);
        let elapsed = start.elapsed();
        
        assert!(matches!(result, Err(LockError::AlreadyLocked { .. })));
        assert_eq!(lock2.retry_count(), 0, "zero timeout must not poll");
        assert!(elapsed < Duration::from_millis(500)); // Should be fast (generous: no retry cycle fits in 500 ms)
    }
    
    #[test]
    fn acquire_with_timeout_succeeds_when_lock_released() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("test.lock");
        
        // First, создаем файл блокировки от имени "другого процесса"
        // Для этого нужно создать файл с информацией о блокировке
        let lock_info = LockInfo {
            pid: std::process::id(),
            host: "test-host".to_string(),
            started_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            version: "test".to_string(),
        };
        
        // Убедимся, что родительская директория существует
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }

        // Файл блокировки создаём здесь, синхронно: иначе поток мог бы не успеть
        // его записать до первого опроса, и тест зависел бы от гонки.
        let json = serde_json::to_string_pretty(&lock_info).unwrap();
        std::fs::write(&lock_path, json).unwrap();

        let lock_path_clone = lock_path.clone();

        // Создаем фоновый поток, который удалит файл блокировки через 100 мс
        std::thread::spawn(move || {
            // Ждем 100 мс, затем удаляем файл (симулируем освобождение блокировки)
            std::thread::sleep(Duration::from_millis(100));
            let _ = std::fs::remove_file(&lock_path_clone);
        });

        // Пытаемся захватить блокировку с таймаутом 1 секунда
        let mut lock = SessionLock::new(lock_path);
        let start = Instant::now();
        let result = lock.acquire_with_timeout(1);
        let elapsed = start.elapsed();

        // Должно успешно захватить блокировку после того, как файл будет удален
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        assert!(lock.is_acquired());
        // Positive control for `retry_count`: this call really did wait.
        assert!(lock.retry_count() >= 1, "waiting acquire must poll at least once");
        // Должен был подождать хотя бы ~100 мс, но не более 500 мс
        assert!(elapsed >= Duration::from_millis(90), "Should have waited for file deletion");
        assert!(elapsed < Duration::from_millis(500), "Should have succeeded before timeout");
    }
    
    #[test]
    fn is_pid_alive_distinguishes_running_and_absent_pids() {
        // The whole stale-lock cleanup depends on this: the current process must
        // always count as alive, otherwise a live session's lock gets deleted.
        assert!(is_pid_alive(std::process::id()));
        // Not a valid process id: must be reported dead so the lock is treated
        // as stale.
        assert!(!is_pid_alive(u32::MAX));
    }

    #[test]
    fn timeout_zero_equals_no_wait() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("test.lock");
        let mut lock1 = SessionLock::new(lock_path.clone());
        lock1.acquire().unwrap();
        
        let mut lock2 = SessionLock::new(lock_path);
        let start = Instant::now();
        let result = lock2.acquire_with_timeout(0);
        let elapsed = start.elapsed();
        
        assert!(matches!(result, Err(LockError::AlreadyLocked { .. })));
        // The contract is "no waiting": the decision comes from the first probe,
        // with zero poll cycles. Asserting on the poll count is deterministic;
        // the old wall-clock bound (`< 100 ms`) failed on Windows because the
        // probe shelled out to tasklist.exe, which alone can exceed 100 ms under
        // load. Timing a sub-millisecond operation is also meaningless: one
        // retry cycle costs at least POLL_INTERVAL anyway.
        assert_eq!(lock2.retry_count(), 0, "zero timeout must not poll");
        assert!(
            elapsed < Duration::from_millis(500),
            "single probe took {elapsed:?}, expected well under a poll cycle"
        );
    }
}