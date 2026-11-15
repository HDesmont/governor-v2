//! Contract integrity validation via SHA-256.
//!
//! Governor uses content hashing to detect tampering with implementation
//! contracts (IMPLEMENTATION.md) between session start and close.
//! The hash is embedded as an HTML comment in the contract file.

use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IntegrityError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no hash found in contract file")]
    MissingHash,
    #[error("hash mismatch: expected {expected}, computed {computed}")]
    HashMismatch { expected: String, computed: String },
    #[error("multiple hash markers found — ambiguous")]
    AmbiguousHash,
}

/// Hash marker embedded in contract files.
const HASH_MARKER: &str = "<!-- GOVERNOR-HASH:";

/// Compute SHA-256 hash of content, returned as hex string.
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute hash of a file's contents.
pub fn hash_file(path: &Path) -> Result<String, IntegrityError> {
    let content = std::fs::read_to_string(path)?;
    Ok(compute_hash(&content))
}

/// Embed a hash marker into content.
///
/// Appends `<!-- GOVERNOR-HASH:<hex> -->` at the end of the content.
/// Existing hash markers are stripped before re-embedding.
pub fn embed_hash(content: &str) -> String {
    let cleaned = strip_hash(content);
    let hash = compute_hash(&cleaned);
    format!("{}\n<!-- GOVERNOR-HASH:{} -->\n", cleaned.trim_end(), hash)
}

/// Extract the hash from content that contains a marker.
pub fn extract_hash(content: &str) -> Result<String, IntegrityError> {
    let markers: Vec<&str> = content
        .lines()
        .filter(|l| l.contains(HASH_MARKER))
        .collect();

    match markers.len() {
        0 => Err(IntegrityError::MissingHash),
        1 => {
            let line = markers[0];
            let start = line.find(HASH_MARKER).unwrap() + HASH_MARKER.len();
            let end = line.rfind("-->").ok_or(IntegrityError::MissingHash)?;
            Ok(line[start..end].trim().to_string())
        }
        _ => Err(IntegrityError::AmbiguousHash),
    }
}

/// Strip all hash markers from content.
pub fn strip_hash(content: &str) -> String {
    content
        .lines()
        .filter(|l| !l.contains(HASH_MARKER))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Validate a contract file: compare embedded hash against computed hash
/// of the content (excluding the hash line itself).
pub fn validate_contract(path: &Path) -> Result<String, IntegrityError> {
    let content = std::fs::read_to_string(path)?;
    let expected = extract_hash(&content)?;
    let cleaned = strip_hash(&content);
    let computed = compute_hash(&cleaned);

    if expected != computed {
        return Err(IntegrityError::HashMismatch { expected, computed });
    }

    Ok(computed)
}

/// Sign a contract file by embedding its hash.
///
/// Reads the file, strips any existing hash, embeds a fresh one, and writes back.
pub fn sign_contract(path: &Path) -> Result<String, IntegrityError> {
    let content = std::fs::read_to_string(path)?;
    let signed = embed_hash(&content);
    std::fs::write(path, &signed)?;
    Ok(compute_hash(&strip_hash(&signed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash_consistent() {
        let h1 = compute_hash("hello");
        let h2 = compute_hash("hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_compute_hash_different() {
        let h1 = compute_hash("hello");
        let h2 = compute_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_embed_and_extract() {
        let content = "# Title\n\nSome content.";
        let signed = embed_hash(content);
        assert!(signed.contains(HASH_MARKER));

        let extracted = extract_hash(&signed).unwrap();
        let cleaned = strip_hash(&signed);
        assert_eq!(extracted, compute_hash(&cleaned));
    }

    #[test]
    fn test_strip_hash_idempotent() {
        let content = "line1\nline2";
        assert_eq!(strip_hash(content), content);
    }

    #[test]
    fn test_missing_hash() {
        assert!(matches!(
            extract_hash("no hash here"),
            Err(IntegrityError::MissingHash)
        ));
    }
}