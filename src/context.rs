//! AI context assembly — builds the prompt payload sent to the AI.
//!
//! Combines STATE.md (operational memory), IMPLEMENTATION.md (active contract),
//! and protocol metadata into a single clipboard-ready payload.
//!
//! Assembly is guarded by a context cache (PLAN §5.1): the SHA-256 fingerprint
//! of the inputs decides whether the payload is rebuilt or reused, and the
//! assembled payload can be persisted to `~/.governor/cache.json` so repeated
//! runs with an unchanged project state skip re-assembly entirely.

use crate::integrity;
use crate::project::{Project, ProjectError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("integrity error: {0}")]
    Integrity(#[from] integrity::IntegrityError),
    #[error("project error: {0}")]
    Project(#[from] ProjectError),
    #[error("cache serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Context payload ready for clipboard delivery.
#[derive(Debug, Clone)]
pub struct ContextPayload {
    /// The full text to be placed in the clipboard.
    pub text: String,
    /// Number of bytes in the payload.
    pub size_bytes: usize,
    /// SHA-256 of the payload for verification.
    pub hash: String,
}

/// Protocol role markers used in context assembly.
pub mod roles {
    pub const ARCHITECT: &str = "Architect";
    pub const ENGINEER: &str = "Engineer";
    pub const READY_MARKER: &str = "## READY";
    pub const VERIFIED_MARKER: &str = "## VERIFIED";
}

/// Header of the session-metadata footer block.
const SESSION_METADATA_HEADER: &str = "### Session Metadata";

/// Prefix of the timestamp line inside the footer block.
const TIMESTAMP_PREFIX: &str = "- Timestamp: ";

/// Render the footer timestamp. Kept in one place so the cache can refresh
/// exactly the same format on a hit.
fn now_timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S %:z").to_string()
}

/// Assemble the full AI context for a project.
///
/// The context includes:
/// 1. Protocol header with role routing
/// 2. Operational state (STATE.md)
/// 3. Implementation contract (IMPLEMENTATION.md) with integrity hash
/// 4. Footer with session metadata
pub fn assemble_context(project: &Project) -> Result<ContextPayload, ContextError> {
    let state = project.read_state()?;
    let implementation = project.read_implementation();

    let mut parts: Vec<String> = Vec::new();

    // ── Protocol header ──────────────────────────────────────────
    parts.push(format!("## Governor v{} Session Context", env!("CARGO_PKG_VERSION")));
    parts.push(format!("Project: {}", project.name));
    parts.push(String::new());
    parts.push("### Role Routing".to_string());
    parts.push(format!(
        "- If the task is ready for implementation → respond as **{}**",
        roles::ENGINEER
    ));
    parts.push(format!(
        "- If the implementation is verified complete → respond as **{}**",
        roles::ARCHITECT
    ));
    parts.push(String::new());
    parts.push("---".to_string());
    parts.push(String::new());

    // ── Operational state ────────────────────────────────────────
    parts.push("### STATE.md (Operational Memory)".to_string());
    parts.push(String::new());
    parts.push(state);
    parts.push(String::new());
    parts.push("---".to_string());
    parts.push(String::new());

    // ── Implementation contract ──────────────────────────────────
    if let Some(ref impl_content) = implementation {
        parts.push("### IMPLEMENTATION.md (Active Contract)".to_string());

        // Validate integrity if hash is present.
        let impl_path = project.implementation_file.as_ref().unwrap();
        let hash_ok = integrity::validate_contract(impl_path).is_ok();
        if !hash_ok {
            parts.push("⚠️  WARNING: Contract integrity check FAILED".to_string());
        }

        parts.push(String::new());
        parts.push(impl_content.clone());
        parts.push(String::new());
        parts.push("---".to_string());
        parts.push(String::new());
    }

    // ── Footer ───────────────────────────────────────────────────
    parts.push(SESSION_METADATA_HEADER.to_string());
    parts.push(format!(
        "{TIMESTAMP_PREFIX}{}",
        now_timestamp()
    ));
    parts.push(format!("- Project: {}", project.name));
    parts.push(format!("- Governor: v{}", env!("CARGO_PKG_VERSION")));
    parts.push(String::new());
    parts.push("<!-- End of Governor context -->".to_string());

    let text = parts.join("\n");
    let hash = integrity::compute_hash(&text);
    let size_bytes = text.len();

    Ok(ContextPayload {
        text,
        size_bytes,
        hash,
    })
}

/// Assemble a close-session validation payload.
///
/// Used when closing a session: provides the final contract state
/// for the Architect role to verify.
pub fn assemble_close_context(project: &Project) -> Result<ContextPayload, ContextError> {
    let state = project.read_state()?;
    let implementation = project.read_implementation();

    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "## Governor v{} — Session Close / Verification",
        env!("CARGO_PKG_VERSION")
    ));
    parts.push(format!("Project: {}", project.name));
    parts.push(String::new());
    parts.push("The implementation session is complete.".to_string());
    parts.push("Please verify the final state as Architect:".to_string());
    parts.push(String::new());
    parts.push("### Final STATE.md".to_string());
    parts.push(String::new());
    parts.push(state);
    parts.push(String::new());

    if let Some(ref impl_content) = implementation {
        parts.push("### Final IMPLEMENTATION.md".to_string());
        parts.push(String::new());
        parts.push(impl_content.clone());
    }

    let text = parts.join("\n");
    let hash = integrity::compute_hash(&text);

    Ok(ContextPayload {
        size_bytes: text.len(),
        text,
        hash,
    })
}

// ── Context-assembly cache (PLAN §5.1) ───────────────────────────

/// Cache file name under `~/.governor/`.
pub const CACHE_FILE_NAME: &str = "cache.json";

/// Environment variable pointing at an explicit cache file.
pub const ENV_CACHE_PATH: &str = "GOVERNOR_CACHE";

/// Bumped whenever the cache schema or the assembled payload layout changes, so
/// entries written by an older binary are ignored instead of replayed.
const CACHE_FORMAT_VERSION: u32 = 1;

/// Payloads above this size are assembled but not cached: storing a megabyte of
/// prompt text per project costs more than the assembly it would save.
pub const MAX_CACHED_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Upper bound on cached entries; the oldest are pruned on insert.
pub const MAX_CACHE_ENTRIES: usize = 64;

/// Which assembly a cache entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextKind {
    /// `governor start` context.
    Session,
    /// `governor close` verification context.
    Close,
}

impl ContextKind {
    /// Stable tag used in cache keys and on disk.
    pub fn as_str(self) -> &'static str {
        match self {
            ContextKind::Session => "session",
            ContextKind::Close => "close",
        }
    }

    /// One entry per (kind, project root): start and close payloads must never
    /// be served for each other.
    fn cache_key(self, root: &Path) -> String {
        format!("{}::{}", self.as_str(), normalize_root(root))
    }
}

/// Normalize a project root for use as a cache key.
///
/// Separators are unified and trailing ones dropped so `C:\work\p` and
/// `C:/work/p/` map to the same entry within a platform.
fn normalize_root(root: &Path) -> String {
    let unified = root.display().to_string().replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    if trimmed.is_empty() {
        unified
    } else {
        trimmed.to_string()
    }
}

/// Expand a leading `~/` or `~\` against the home directory.
fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// Fingerprint of every input the assembled context is derived from.
///
/// Content hashes, not mtimes: touching a file without editing it must not
/// invalidate the cache, while any byte change must.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSignature {
    /// Cache/payload schema version ([`CACHE_FORMAT_VERSION`]).
    pub format: u32,
    /// Governor version that produced the payload.
    pub governor_version: String,
    /// Which assembly this signature guards.
    pub kind: ContextKind,
    /// Project name as printed in the payload.
    pub project: String,
    /// Normalized project root.
    pub root: String,
    /// SHA-256 of STATE.md content.
    pub state_hash: String,
    /// SHA-256 of IMPLEMENTATION.md content, `None` when absent.
    pub implementation_hash: Option<String>,
}

impl ContextSignature {
    /// Hash the project's context inputs as they are on disk right now.
    pub fn from_project(project: &Project, kind: ContextKind) -> Result<Self, ContextError> {
        let state = project.read_state()?;
        let implementation = project.read_implementation();
        Ok(Self::from_contents(
            project,
            kind,
            &state,
            implementation.as_deref(),
        ))
    }

    /// Pure half of [`ContextSignature::from_project`] — hashing already-read
    /// contents, so tests need no filesystem.
    pub fn from_contents(
        project: &Project,
        kind: ContextKind,
        state: &str,
        implementation: Option<&str>,
    ) -> Self {
        Self {
            format: CACHE_FORMAT_VERSION,
            governor_version: env!("CARGO_PKG_VERSION").to_string(),
            kind,
            project: project.name.clone(),
            root: normalize_root(&project.root),
            state_hash: integrity::compute_hash(state),
            implementation_hash: implementation.map(integrity::compute_hash),
        }
    }

    /// SHA-256 over the canonical serialization of this signature.
    pub fn fingerprint(&self) -> String {
        match serde_json::to_string(self) {
            Ok(raw) => integrity::compute_hash(&raw),
            // A signature is plain data; serialization cannot realistically
            // fail, and a debug dump still hashes to a stable value.
            Err(_) => integrity::compute_hash(&format!("{self:?}")),
        }
    }
}

/// One assembled payload plus the signature it was built from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub signature: ContextSignature,
    pub fingerprint: String,
    /// Unix seconds when the payload was assembled.
    pub stored_at: i64,
    pub text: String,
    pub hash: String,
    pub size_bytes: usize,
}

impl CacheEntry {
    fn new(signature: ContextSignature, fingerprint: String, payload: &ContextPayload) -> Self {
        Self {
            signature,
            fingerprint,
            stored_at: chrono::Utc::now().timestamp(),
            text: payload.text.clone(),
            hash: payload.hash.clone(),
            size_bytes: payload.size_bytes,
        }
    }

    /// Payload served from the cache.
    ///
    /// The body is returned byte-for-byte as assembled; only the footer
    /// timestamp is refreshed to the current time, with the hash recomputed so
    /// it still describes the delivered text.
    fn payload(&self) -> ContextPayload {
        let (text, refreshed) = refresh_timestamp(&self.text);
        if !refreshed {
            return ContextPayload {
                text: self.text.clone(),
                size_bytes: self.size_bytes,
                hash: self.hash.clone(),
            };
        }
        let size_bytes = text.len();
        ContextPayload {
            hash: integrity::compute_hash(&text),
            size_bytes,
            text,
        }
    }
}

/// On-disk shape of `~/.governor/cache.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    format: u32,
    entries: BTreeMap<String, CacheEntry>,
}

/// Replace the footer timestamp line with the current time.
///
/// Only the block after the **last** [`SESSION_METADATA_HEADER`] is considered,
/// so a lookalike line inside user-authored STATE.md content is never rewritten.
/// Returns the new text and whether anything changed.
fn refresh_timestamp(text: &str) -> (String, bool) {
    let Some(header_at) = text.rfind(SESSION_METADATA_HEADER) else {
        return (text.to_string(), false);
    };
    let (head, tail) = text.split_at(header_at);

    let trailing_newline = tail.ends_with('\n');
    let mut lines: Vec<String> = tail.lines().map(str::to_string).collect();
    let Some(idx) = lines.iter().position(|l| l.starts_with(TIMESTAMP_PREFIX)) else {
        return (text.to_string(), false);
    };

    lines[idx] = format!("{TIMESTAMP_PREFIX}{}", now_timestamp());

    let mut out = String::with_capacity(text.len() + 32);
    out.push_str(head);
    out.push_str(&lines.join("\n"));
    if trailing_newline {
        out.push('\n');
    }
    (out, true)
}

/// Whether a payload of this size is worth caching.
pub fn is_cacheable(size_bytes: usize) -> bool {
    size_bytes <= MAX_CACHED_PAYLOAD_BYTES
}

/// Cache location: `GOVERNOR_CACHE` wins, else `~/.governor/cache.json`.
///
/// Takes the override explicitly so it is testable without mutating the process
/// environment. `None` means "no usable location" (no home directory).
pub fn cache_path_for(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(expand_tilde(raw));
    }
    let home = dirs::home_dir()?;
    Some(home.join(".governor").join(CACHE_FILE_NAME))
}

/// Cache path for the current environment.
pub fn default_cache_path() -> Option<PathBuf> {
    cache_path_for(std::env::var(ENV_CACHE_PATH).ok().as_deref())
}

/// Load cached entries from `path`.
///
/// A missing, unreadable, malformed, or version-mismatched file is an empty
/// cache: losing cached prompts is cheap, failing a session because of them is
/// not.
fn read_entries(path: &Path) -> BTreeMap<String, CacheEntry> {
    let read = (|| -> Result<BTreeMap<String, CacheEntry>, ContextError> {
        let raw = std::fs::read_to_string(path)?;
        let file: CacheFile = serde_json::from_str(&raw)?;
        if file.format != CACHE_FORMAT_VERSION {
            return Ok(BTreeMap::new());
        }
        Ok(file.entries)
    })();
    read.unwrap_or_default()
}

/// Context-assembly cache: an in-process map, optionally persisted to JSON.
///
/// Reuse rule: the inputs' [`ContextSignature::fingerprint`] must match
/// exactly, so any change to STATE.md or IMPLEMENTATION.md — including a
/// contract being signed for the first time — invalidates the entry.
#[derive(Debug, Default)]
pub struct ContextCache {
    path: Option<PathBuf>,
    entries: Mutex<BTreeMap<String, CacheEntry>>,
}

impl ContextCache {
    /// In-memory cache that never touches the filesystem.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Cache persisted at `path`, seeded with whatever is readable there.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = read_entries(&path);
        ContextCache {
            path: Some(path),
            entries: Mutex::new(entries),
        }
    }

    /// Cache at [`default_cache_path`], or in-memory when home is unavailable.
    pub fn open_default() -> Self {
        match default_cache_path() {
            Some(path) => Self::load(path),
            None => Self::in_memory(),
        }
    }

    /// Where this cache persists to, if anywhere.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn len(&self) -> usize {
        self.guard().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, CacheEntry>> {
        // A panicking writer leaves a valid map behind; poisoning the cache for
        // the whole process would be worse than reusing it.
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Entry for `key` if its fingerprint still matches `fingerprint`.
    pub fn lookup(&self, key: &str, fingerprint: &str) -> Option<CacheEntry> {
        let entries = self.guard();
        entries
            .get(key)
            .filter(|entry| entry.fingerprint == fingerprint && !entry.text.is_empty())
            .cloned()
    }

    /// Insert (replacing any stale entry for the same key) and prune to size.
    pub fn store(&self, key: String, entry: CacheEntry) {
        let mut entries = self.guard();
        entries.insert(key, entry);
        if entries.len() > MAX_CACHE_ENTRIES {
            let excess = entries.len() - MAX_CACHE_ENTRIES;
            let mut by_age: Vec<(i64, String)> = entries
                .iter()
                .map(|(k, e)| (e.stored_at, k.clone()))
                .collect();
            by_age.sort();
            for (_, stale) in by_age.into_iter().take(excess) {
                entries.remove(&stale);
            }
        }
    }

    /// Write the map to the cache file. No-op for an in-memory cache.
    pub fn save(&self) -> Result<(), ContextError> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = CacheFile {
            format: CACHE_FORMAT_VERSION,
            entries: self.guard().clone(),
        };
        let raw = serde_json::to_string_pretty(&file)?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    /// Drop every entry, in memory and on disk.
    pub fn clear(&self) -> Result<(), ContextError> {
        self.guard().clear();
        if let Some(path) = self.path.as_ref() {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

/// Outcome of a cache-aware assembly.
#[derive(Debug, Clone)]
pub struct CachedContext {
    /// Payload to deliver to the clipboard.
    pub payload: ContextPayload,
    /// `true` when re-assembly was skipped because nothing changed.
    pub reused: bool,
    /// Which assembly produced (or would have produced) the payload.
    pub kind: ContextKind,
    /// Fingerprint of the inputs the payload corresponds to.
    pub fingerprint: String,
}

/// Assemble the session context, reusing the cached payload when the project
/// state is unchanged.
pub fn assemble_context_cached(
    project: &Project,
    cache: &ContextCache,
) -> Result<CachedContext, ContextError> {
    assemble_cached(project, cache, ContextKind::Session, assemble_context)
}

/// Assemble the close/verification context with the same caching rule as
/// [`assemble_context_cached`].
pub fn assemble_close_context_cached(
    project: &Project,
    cache: &ContextCache,
) -> Result<CachedContext, ContextError> {
    assemble_cached(project, cache, ContextKind::Close, assemble_close_context)
}

fn assemble_cached(
    project: &Project,
    cache: &ContextCache,
    kind: ContextKind,
    assemble: fn(&Project) -> Result<ContextPayload, ContextError>,
) -> Result<CachedContext, ContextError> {
    let signature = ContextSignature::from_project(project, kind)?;
    let fingerprint = signature.fingerprint();
    let key = kind.cache_key(&project.root);

    if let Some(entry) = cache.lookup(&key, &fingerprint) {
        return Ok(CachedContext {
            payload: entry.payload(),
            reused: true,
            kind,
            fingerprint,
        });
    }

    let payload = assemble(project)?;
    if is_cacheable(payload.size_bytes) {
        cache.store(key, CacheEntry::new(signature, fingerprint.clone(), &payload));
        // Best effort: a read-only home directory must not fail a session whose
        // context was assembled successfully.
        let _ = cache.save();
    }
    Ok(CachedContext {
        payload,
        reused: false,
        kind,
        fingerprint,
    })
}

/// Strip governor markers from content (for clean delivery).
pub fn clean_content(content: &str) -> String {
    content
        .lines()
        .filter(|l| {
            !l.contains("<!-- GOVERNOR-HASH:")
                && !l.contains("<!-- End of Governor context -->")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roles_defined() {
        assert!(!roles::ARCHITECT.is_empty());
        assert!(!roles::ENGINEER.is_empty());
        assert_ne!(roles::ARCHITECT, roles::ENGINEER);
    }

    #[test]
    fn test_clean_content_removes_markers() {
        let input = "line1\n<!-- GOVERNOR-HASH:abc123 -->\nline2\n<!-- End of Governor context -->";
        let cleaned = clean_content(input);
        assert!(!cleaned.contains("GOVERNOR-HASH"));
        assert!(!cleaned.contains("End of Governor context"));
        assert!(cleaned.contains("line1"));
        assert!(cleaned.contains("line2"));
    }

    // ── Context-assembly cache (PLAN §5.1) ───────────────────────

    /// Build a project in `dir` with the given file contents.
    fn write_project(dir: &Path, name: &str, state: &str, implementation: Option<&str>) -> Project {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("STATE.md"), state).unwrap();
        let implementation_file = match implementation {
            Some(content) => {
                let path = dir.join("IMPLEMENTATION.md");
                std::fs::write(&path, content).unwrap();
                Some(path)
            }
            None => None,
        };
        Project {
            name: name.to_string(),
            root: dir.to_path_buf(),
            state_file: dir.join("STATE.md"),
            implementation_file,
        }
    }

    /// Payload text with the footer timestamp normalized away, so cached and
    /// freshly assembled payloads can be compared byte-for-byte.
    fn without_timestamp(text: &str) -> String {
        text.lines()
            .map(|l| {
                if l.starts_with(TIMESTAMP_PREFIX) {
                    format!("{TIMESTAMP_PREFIX}<normalized>")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn signature_hashes_content_not_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "state v1", Some("contract v1"));

        let same = ContextSignature::from_contents(&project, ContextKind::Session, "state v1", Some("contract v1"));
        let from_disk = ContextSignature::from_project(&project, ContextKind::Session).unwrap();
        assert_eq!(same, from_disk, "pure and disk halves must agree");
        assert_eq!(same.fingerprint(), from_disk.fingerprint());

        // Any byte change in either input changes the fingerprint.
        let changed_state =
            ContextSignature::from_contents(&project, ContextKind::Session, "state v2", Some("contract v1"));
        assert_ne!(same.fingerprint(), changed_state.fingerprint());

        let changed_contract =
            ContextSignature::from_contents(&project, ContextKind::Session, "state v1", Some("contract v2"));
        assert_ne!(same.fingerprint(), changed_contract.fingerprint());

        // Whitespace-only edits count: the payload embeds them verbatim.
        let trailing =
            ContextSignature::from_contents(&project, ContextKind::Session, "state v1\n", Some("contract v1"));
        assert_ne!(same.fingerprint(), trailing.fingerprint());
    }

    #[test]
    fn signature_separates_missing_and_empty_contract() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "state v1", None);
        let absent = ContextSignature::from_project(&project, ContextKind::Session).unwrap();
        let empty = ContextSignature::from_contents(&project, ContextKind::Session, "state v1", Some(""));
        assert_ne!(absent.fingerprint(), empty.fingerprint(), "None must not alias Some(\"\")");
    }

    #[test]
    fn signature_scopes_by_kind_and_project() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "state v1", Some("contract v1"));

        let session = ContextSignature::from_project(&project, ContextKind::Session).unwrap();
        let close = ContextSignature::from_project(&project, ContextKind::Close).unwrap();
        assert_ne!(session.fingerprint(), close.fingerprint(), "start and close payloads differ");

        let renamed = Project {
            name: "other".to_string(),
            ..project.clone()
        };
        let other = ContextSignature::from_project(&renamed, ContextKind::Session).unwrap();
        assert_ne!(session.fingerprint(), other.fingerprint());
    }

    #[test]
    fn cache_hit_reuses_payload_without_reassembly() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", Some("CONTRACT BODY v1"));
        let cache = ContextCache::in_memory();

        let first = assemble_context_cached(&project, &cache).unwrap();
        assert!(!first.reused, "cold cache must assemble");
        assert_eq!(cache.len(), 1);

        let second = assemble_context_cached(&project, &cache).unwrap();
        assert!(second.reused, "unchanged state must reuse");
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(cache.len(), 1, "a hit must not insert entries");

        assert_eq!(
            without_timestamp(&first.payload.text),
            without_timestamp(&second.payload.text),
            "reused payload must carry the same body"
        );
        let fresh = assemble_context(&project).unwrap();
        assert_eq!(without_timestamp(&fresh.text), without_timestamp(&second.payload.text));

        // The delivered payload stays self-consistent after the refresh.
        assert_eq!(second.payload.size_bytes, second.payload.text.len());
        assert_eq!(second.payload.hash, integrity::compute_hash(&second.payload.text));
    }

    #[test]
    fn cache_invalidates_when_a_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache = ContextCache::in_memory();

        assert!(!assemble_context_cached(&project, &cache).unwrap().reused);
        assert!(assemble_context_cached(&project, &cache).unwrap().reused);

        std::fs::write(&project.state_file, "STATE BODY v2 — task switched").unwrap();
        let after = assemble_context_cached(&project, &cache).unwrap();
        assert!(!after.reused, "STATE.md change must invalidate the cache");
        assert!(after.payload.text.contains("task switched"));
        assert!(!after.payload.text.contains("STATE BODY v1"));

        // And the new payload becomes the cached one.
        let repeat = assemble_context_cached(&project, &cache).unwrap();
        assert!(repeat.reused);
        assert_eq!(repeat.fingerprint, after.fingerprint);
    }

    #[test]
    fn cache_invalidates_when_contract_appears() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache = ContextCache::in_memory();

        assert!(!assemble_context_cached(&project, &cache).unwrap().reused);
        assert!(assemble_context_cached(&project, &cache).unwrap().reused);

        // A contract appearing changes both the payload and the signature.
        let project = write_project(tmp.path(), "p", "STATE BODY v1", Some("# Contract\n"));
        let after = assemble_context_cached(&project, &cache).unwrap();
        assert!(!after.reused, "a newly added IMPLEMENTATION.md must invalidate");
        assert!(after.payload.text.contains("IMPLEMENTATION.md (Active Contract)"));
    }

    #[test]
    fn cache_hit_survives_unchanged_rewrite_of_files() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", Some("CONTRACT v1"));
        let cache = ContextCache::in_memory();

        assemble_context_cached(&project, &cache).unwrap();
        // Same bytes, fresh mtime: content hashing must keep the entry valid.
        write_project(tmp.path(), "p", "STATE BODY v1", Some("CONTRACT v1"));
        assert!(assemble_context_cached(&project, &cache).unwrap().reused);
    }

    #[test]
    fn start_and_close_entries_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", Some("CONTRACT v1"));
        let cache = ContextCache::in_memory();

        assert!(!assemble_context_cached(&project, &cache).unwrap().reused);
        assert!(
            !assemble_close_context_cached(&project, &cache).unwrap().reused,
            "close payload must not be served from the start entry"
        );
        assert_eq!(cache.len(), 2);
        assert!(assemble_close_context_cached(&project, &cache).unwrap().reused);
    }

    #[test]
    fn distinct_projects_have_distinct_keys() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("alpha");
        let b = root.path().join("beta");
        let project_a = write_project(&a, "alpha", "SAME STATE", None);
        let project_b = write_project(&b, "beta", "SAME STATE", None);
        let cache = ContextCache::in_memory();

        assert!(!assemble_context_cached(&project_a, &cache).unwrap().reused);
        assert!(!assemble_context_cached(&project_b, &cache).unwrap().reused);
        assert_eq!(cache.len(), 2, "identical content in two projects is two entries");
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", Some("CONTRACT v1"));
        let cache_file = tmp.path().join("governor").join(CACHE_FILE_NAME);

        let run_one = ContextCache::load(&cache_file);
        assert!(!assemble_context_cached(&project, &run_one).unwrap().reused);
        assert!(cache_file.is_file(), "a miss must persist the cache file");

        // A brand-new process would construct the cache the same way.
        let run_two = ContextCache::load(&cache_file);
        let second = assemble_context_cached(&project, &run_two).unwrap();
        assert!(second.reused, "persisted cache must survive the process");
        assert!(second.payload.text.contains("STATE BODY v1"));
    }

    #[test]
    fn malformed_cache_file_is_treated_as_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache_file = tmp.path().join(CACHE_FILE_NAME);
        std::fs::write(&cache_file, "{ not json at all").unwrap();

        let cache = ContextCache::load(&cache_file);
        assert!(cache.is_empty());
        assert!(!assemble_context_cached(&project, &cache).unwrap().reused);
        assert!(cache_file.is_file(), "the salvageable cache is rewritten, not dropped");
    }

    #[test]
    fn stale_format_version_ignores_old_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache_file = tmp.path().join(CACHE_FILE_NAME);
        let signature = ContextSignature::from_project(&project, ContextKind::Session).unwrap();
        let key = ContextKind::Session.cache_key(&project.root);
        std::fs::write(
            &cache_file,
            r#"{"format":999,"entries":{"session::x":{"signature":{"format":999,"governor_version":"0","kind":"session","project":"p","root":"x","state_hash":"a","implementation_hash":null},"fingerprint":"a","stored_at":0,"text":"old","hash":"a","size_bytes":3}}}"#,
        )
        .unwrap();

        let cache = ContextCache::load(&cache_file);
        assert!(cache.is_empty(), "entries from another schema version must be ignored");
        assert!(cache.lookup(&key, &signature.fingerprint()).is_none());
        assert!(
            !assemble_context_cached(&project, &cache).unwrap().reused,
            "a stale-schema payload must never be served"
        );
    }

    #[test]
    fn fingerprint_mismatch_never_serves_an_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache = ContextCache::in_memory();
        let signature = ContextSignature::from_project(&project, ContextKind::Session).unwrap();
        let key = ContextKind::Session.cache_key(&project.root);

        let payload = assemble_context(&project).unwrap();
        let entry = CacheEntry::new(signature.clone(), signature.fingerprint(), &payload);
        cache.store(key.clone(), entry);

        assert!(cache.lookup(&key, &signature.fingerprint()).is_some());
        assert!(cache.lookup(&key, "deadbeef").is_none(), "changed inputs must miss");
        assert!(cache.lookup("close::other", &signature.fingerprint()).is_none());
    }

    #[test]
    fn oversized_payloads_are_not_cached() {
        assert!(is_cacheable(MAX_CACHED_PAYLOAD_BYTES));
        assert!(!is_cacheable(MAX_CACHED_PAYLOAD_BYTES + 1));

        // A payload over the cap is assembled correctly but never stored.
        let huge_state = "x".repeat(MAX_CACHED_PAYLOAD_BYTES + 1024);
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", &huge_state, None);
        let cache = ContextCache::in_memory();

        let first = assemble_context_cached(&project, &cache).unwrap();
        assert!(!first.reused);
        assert_eq!(first.payload.size_bytes, first.payload.text.len());
        assert!(cache.is_empty(), "oversized payloads must not bloat the cache");
        assert!(!assemble_context_cached(&project, &cache).unwrap().reused);
    }

    #[test]
    fn entry_count_is_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ContextCache::in_memory();
        let payload = ContextPayload {
            text: "ctx".to_string(),
            size_bytes: 3,
            hash: integrity::compute_hash("ctx"),
        };

        let total = MAX_CACHE_ENTRIES + 5;
        for i in 0..total {
            let root = tmp.path().join(format!("p{i}"));
            let entry = CacheEntry {
                signature: ContextSignature {
                    format: CACHE_FORMAT_VERSION,
                    governor_version: "test".to_string(),
                    kind: ContextKind::Session,
                    project: format!("p{i}"),
                    root: normalize_root(&root),
                    state_hash: "h".to_string(),
                    implementation_hash: None,
                },
                fingerprint: format!("fp{i}"),
                stored_at: i as i64,
                text: payload.text.clone(),
                hash: payload.hash.clone(),
                size_bytes: payload.size_bytes,
            };
            cache.store(ContextKind::Session.cache_key(&root), entry);
        }
        assert_eq!(cache.len(), MAX_CACHE_ENTRIES, "oldest entries must be pruned");
        let oldest = ContextKind::Session.cache_key(&tmp.path().join("p0"));
        assert!(cache.lookup(&oldest, "fp0").is_none());
        let newest = ContextKind::Session.cache_key(&tmp.path().join(format!("p{}", total - 1)));
        assert!(cache.lookup(&newest, &format!("fp{}", total - 1)).is_some());
    }

    #[test]
    fn clear_empties_memory_and_removes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache_file = tmp.path().join("nested").join(CACHE_FILE_NAME);

        let cache = ContextCache::load(&cache_file);
        assemble_context_cached(&project, &cache).unwrap();
        assert!(cache_file.is_file());

        cache.clear().unwrap();
        assert!(cache.is_empty());
        assert!(!cache_file.exists());
        assert!(cache.clear().is_ok(), "clearing an already-absent file is not an error");
        assert!(!assemble_context_cached(&project, &cache).unwrap().reused);
    }

    #[test]
    fn in_memory_cache_never_writes_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let project = write_project(tmp.path(), "p", "STATE BODY v1", None);
        let cache = ContextCache::in_memory();
        assemble_context_cached(&project, &cache).unwrap();
        assert_eq!(cache.path(), None);
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1, "only STATE.md");
    }

    #[test]
    fn cache_path_prefers_explicit_override() {
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("cache.json");
        assert_eq!(
            cache_path_for(Some(explicit.to_str().unwrap())),
            Some(explicit.clone())
        );
        // Blank overrides are treated as unset.
        let fallback = cache_path_for(Some("   "));
        assert_eq!(fallback, cache_path_for(None));
        if let Some(path) = fallback {
            assert!(path.ends_with(Path::new(".governor").join(CACHE_FILE_NAME)), "{path:?}");
        }
    }

    #[test]
    fn refresh_touches_only_the_footer_timestamp() {
        let stale = "### STATE.md (Operational Memory)\n- Timestamp: SENTINEL-IN-STATE\n\n### Session Metadata\n- Timestamp: 2000-01-01 00:00:00 +00:00\n- Project: p\n";
        let (refreshed, changed) = refresh_timestamp(stale);
        assert!(changed);
        assert!(refreshed.contains("- Timestamp: SENTINEL-IN-STATE"), "user content must not be rewritten");
        assert!(!refreshed.contains("2000-01-01"));
        assert!(refreshed.ends_with("- Project: p\n"), "trailing newline preserved");

        // No footer block → nothing to refresh, text returned verbatim.
        let (close_text, changed) = refresh_timestamp("### Final STATE.md\nstate\n");
        assert!(!changed);
        assert_eq!(close_text, "### Final STATE.md\nstate\n");
    }

    #[test]
    fn normalized_root_key_is_separator_agnostic() {
        let win = normalize_root(Path::new("C:\\work\\project"));
        let unix_style = normalize_root(Path::new("C:/work/project/"));
        assert_eq!(win, unix_style);
        assert_eq!(
            ContextKind::Session.cache_key(Path::new("C:\\work\\project")),
            ContextKind::Session.cache_key(Path::new("C:/work/project"))
        );
    }
}