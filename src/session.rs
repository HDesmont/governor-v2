//! Session lifecycle: start, close, and state management.
//!
//! A Governor session represents one continuous AI-assisted development
//! cycle: lock → assemble context → work → validate → close → unlock.

use crate::clipboard;
use crate::context;
use crate::integrity;
use crate::lock::{LockError, SessionLock};
use crate::project::{Project, ProjectError};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("project error: {0}")]
    Project(#[from] ProjectError),
    #[error("lock error: {0}")]
    Lock(#[from] LockError),
    #[error("context error: {0}")]
    Context(#[from] context::ContextError),
    #[error("clipboard error: {0}")]
    Clipboard(#[from] clipboard::ClipboardError),
    #[error("integrity error: {0}")]
    Integrity(#[from] integrity::IntegrityError),
    #[error("session error: {0}")]
    Other(String),
}

/// Result of a successful session start.
#[derive(Debug)]
pub struct SessionStartResult {
    pub project_name: String,
    pub context_size: usize,
    pub context_hash: String,
    /// `true` when the context came from the assembly cache (PLAN §5.1).
    pub context_reused: bool,
    /// Path of the context cache, or `None` for an in-memory cache.
    pub cache_path: Option<PathBuf>,
}

/// Start a session for the given project.
///
/// 1. Acquire an exclusive lock
/// 2. Sign the implementation contract (embed hash)
/// 3. Assemble AI context (reused from cache when the state is unchanged)
/// 4. Write context to clipboard
/// 5. Return lock handle (caller must hold until close)
pub fn start_session(project: &Project, lock_timeout_seconds: u64) -> Result<(SessionStartResult, SessionLock), SessionError> {
    let lock_path = project.lock_path();

    // ── 1. Acquire lock ──────────────────────────────────────────
    let mut lock = SessionLock::new(lock_path);
    lock.acquire_with_timeout(lock_timeout_seconds)?;

    // ── 2. Sign implementation contract ──────────────────────────
    if let Some(ref impl_path) = project.implementation_file {
        integrity::sign_contract(impl_path)?;
    }

    // ── 3. Assemble context ──────────────────────────────────────
    let cache = context::ContextCache::open_default();
    let cache_path = cache.path().map(PathBuf::from);
    let cached = context::assemble_context_cached(project, &cache)?;
    if cached.reused {
        eprintln!("♻️  Context unchanged — reusing assembled payload ({}).", cached.fingerprint);
    }
    let payload = cached.payload;

    // ── 4. Write to clipboard ────────────────────────────────────
    clipboard::write(&payload.text)?;

    let result = SessionStartResult {
        project_name: project.name.clone(),
        context_size: payload.size_bytes,
        context_hash: payload.hash,
        context_reused: cached.reused,
        cache_path,
    };

    Ok((result, lock))
}

/// Result of contract validation during a session close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractCheck {
    /// The embedded hash matched the recomputed one (or there is no contract).
    Validated,
    /// The contract had no hash yet and was signed during this close.
    SignedNow,
    /// Validation failed: hash mismatch or an unreadable contract.
    Failed,
}

/// Outcome summary of a session close.
#[derive(Debug, Clone, Copy)]
pub struct CloseOutcome {
    pub contract_check: ContractCheck,
}

/// Close a session for the given project.
///
/// 1. Validate implementation contract integrity
/// 2. Assemble close/verification context
/// 3. Write close context to clipboard
/// 4. Release the lock
pub fn close_session(
    project: &Project,
    lock: &mut SessionLock,
) -> Result<CloseOutcome, SessionError> {
    // ── 1. Validate contract integrity ───────────────────────────
    let mut contract_check = ContractCheck::Validated;
    if let Some(ref impl_path) = project.implementation_file {
        match integrity::validate_contract(impl_path) {
            Ok(hash) => {
                eprintln!("✅ Contract integrity verified: {}", hash);
            }
            Err(integrity::IntegrityError::MissingHash) => {
                // No hash — first close without signing. Sign it now.
                integrity::sign_contract(impl_path)?;
                eprintln!("📝 Contract signed for future verification.");
                contract_check = ContractCheck::SignedNow;
            }
            Err(e) => {
                eprintln!("⚠️  Contract integrity check failed: {}", e);
                // Continue with close anyway — the verification context will
                // include the warning for the Architect.
                contract_check = ContractCheck::Failed;
            }
        }
    }

    // ── 2. Assemble close context ────────────────────────────────
    let cache = context::ContextCache::open_default();
    let cached = context::assemble_close_context_cached(project, &cache)?;
    if cached.reused {
        eprintln!("♻️  Verification context unchanged — reusing assembled payload.");
    }
    let payload = cached.payload;

    // ── 3. Write close context to clipboard ──────────────────────
    clipboard::write(&payload.text)?;

    // ── 4. Release lock ──────────────────────────────────────────
    lock.release()?;

    Ok(CloseOutcome { contract_check })
}

/// Check whether a project has an active session.
pub fn is_session_active(project: &Project) -> bool {
    project.is_locked()
}

/// Get the lock path for a project (used by health checks).
pub fn lock_path_for(project: &Project) -> PathBuf {
    project.lock_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_path_consistency() {
        // Verify lock path is always under .governor/
        let p = PathBuf::from("/tmp/test-project");
        let lock = p.join(".governor").join("session.lock");
        assert!(lock.ends_with("session.lock"));
        assert!(lock.to_string_lossy().contains(".governor"));
    }
}