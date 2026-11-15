//! Cross-platform clipboard access via arboard.
//!
//! Uses the native `arboard` crate which wraps the OS clipboard API
//! directly on Windows (OLE), macOS (NSPasteboard), and Linux (wl-clipboard/x11).
//! No external process spawns — fast and reliable.

use std::sync::{Mutex, MutexGuard};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClipboardError {
    #[error("failed to write to clipboard: {0}")]
    WriteFailed(String),
    #[error("failed to read from clipboard: {0}")]
    ReadFailed(String),
}

/// Windows lets only one thread *system-wide* hold the clipboard open, and
/// `arboard` opens/closes it once per operation with no synchronisation of its
/// own (see `Clipboard::open` in `arboard::platform::windows`). Two threads of
/// this process touching the clipboard simultaneously — parallel `cargo test`
/// threads, or a `governor doctor` run while the relay daemon polls — race on
/// the `GlobalAlloc` block behind `CF_UNICODETEXT`: one thread's
/// `EmptyClipboard` destroys the memory object the other one just locked.
/// That is the `STATUS_HEAP_CORRUPTION` (0xC0000374) abort seen in the health
/// tests.
///
/// The system clipboard is a process-wide singleton regardless, so every entry
/// point below serialises on this lock — in tests and in production alike.
static CLIPBOARD_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the process-wide clipboard lock for the current scope.
///
/// An operation that panicked already dropped its `Clipboard`, which closes
/// the OS clipboard, so the guard is safe to recover; letting the mutex poison
/// would turn one failure into a cascade of unrelated ones.
fn lock_clipboard() -> MutexGuard<'static, ()> {
    CLIPBOARD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Write text to the system clipboard.
pub fn write(text: &str) -> Result<(), ClipboardError> {
    let _guard = lock_clipboard();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| ClipboardError::WriteFailed(e.to_string()))?;
    clipboard
        .set_text(text)
        .map_err(|e| ClipboardError::WriteFailed(e.to_string()))
}

/// Read text from the system clipboard.
pub fn read() -> Result<String, ClipboardError> {
    let _guard = lock_clipboard();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| ClipboardError::ReadFailed(e.to_string()))?;
    clipboard
        .get_text()
        .map_err(|e| ClipboardError::ReadFailed(e.to_string()))
}

/// Put `text` on the clipboard and read the clipboard back.
///
/// Both steps share one clipboard lock and one `Clipboard` handle, so the
/// round-trip is atomic. A plain `write` followed by `read` can observe another
/// thread's (or another process's) content in between and report a corruption
/// that never happened.
pub fn round_trip(text: &str) -> Result<String, ClipboardError> {
    let _guard = lock_clipboard();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| ClipboardError::WriteFailed(e.to_string()))?;
    clipboard
        .set_text(text)
        .map_err(|e| ClipboardError::WriteFailed(e.to_string()))?;
    clipboard
        .get_text()
        .map_err(|e| ClipboardError::ReadFailed(e.to_string()))
}

/// Simple base64 encode — kept as a utility (used by relay and integration tests).
pub fn base64_encode(data: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = data.as_bytes();
    let mut result = String::new();

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}