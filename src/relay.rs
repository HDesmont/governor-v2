//! Relay daemon — background clipboard-based command execution.
//!
//! The relay daemon monitors the clipboard for Governor command markers
//! and executes them locally, writing results back to the clipboard.
//! This enables a "human-as-transport" loop where the AI writes commands
//! and the daemon executes them autonomously.
//!
//! ## Protocol
//! - Commands arrive as: `<!-- GOV-CMD:base64 -->` in clipboard
//! - Results are written back as: `exit:<code>|stdout:<b64>|stderr:<b64>`
//!   (payload base64-encoded into the clipboard, see `format_response`)
//! - Output over `max_output_bytes` is cut on a character boundary and
//!   suffixed with `... [truncated]` *before* base64 encoding, so the cut is
//!   visible to whoever decodes the response
//! - Optional `--hud` status bar: clipboard size, last command, uptime
//! - Heartbeat every 30s to detect daemon liveness

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::hud::{Hud, HudSnapshot};

/// Marker appended to command output that exceeded `max_output_bytes`.
///
/// Appended *before* base64 encoding, so the truncation is visible to whoever
/// decodes the response, not just to the local log.
pub const TRUNCATION_NOTE: &str = "... [truncated]";

/// Relay daemon configuration.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Polling interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Heartbeat interval in seconds (0 = disabled).
    pub heartbeat_secs: u64,
    /// Maximum command output size in bytes.
    pub max_output_bytes: usize,
    /// Preset name for logging.
    pub preset: String,
    /// Render the one-line HUD status bar (`--hud`).
    pub hud: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1_000,
            heartbeat_secs: 30,
            max_output_bytes: 64 * 1024, // 64 KiB
            preset: "standard".to_string(),
            hud: false,
        }
    }
}

impl RelayConfig {
    /// Create a config from a preset name.
    pub fn from_preset(preset: &str) -> Self {
        match preset.to_lowercase().as_str() {
            "minimal" => Self {
                poll_interval_ms: 2_000,
                heartbeat_secs: 0,
                max_output_bytes: 8 * 1024,
                preset: "minimal".to_string(),
                hud: false,
            },
            "full" => Self {
                poll_interval_ms: 500,
                heartbeat_secs: 15,
                max_output_bytes: 256 * 1024,
                preset: "full".to_string(),
                hud: false,
            },
            _ => Self::default(),
        }
    }
}

/// Command marker prefix that the daemon watches for.
pub(crate) const CMD_MARKER: &str = "<!-- GOV-CMD:";

/// Run the relay daemon until the shutdown flag is set.
///
/// The daemon:
/// 1. Polls clipboard for command markers
/// 2. Decodes and executes commands via shell
/// 3. Writes results back with a result marker
/// 4. Optionally sends periodic heartbeat markers
///
/// With `config.hud` set, a one-line status bar (clipboard size, last command,
/// uptime) stays pinned to the bottom of the terminal and log messages scroll
/// above it — see [`crate::hud`].
pub fn run_daemon(config: RelayConfig, shutdown: Arc<AtomicBool>) {
    let mut hud = Hud::new(config.hud);

    hud.log(&format!(
        "🔄 Governor relay daemon started (preset: {})",
        config.preset
    ));
    hud.log(&format!("   Poll interval: {}ms", config.poll_interval_ms));
    if config.hud && !hud.enabled() {
        // --hud asked for, but the output is redirected: say so once.
        hud.log("   HUD disabled: stderr is not a terminal (logging plainly).");
    }
    hud.log("   Press Ctrl+C to stop.");

    let mut last_seen: Option<String> = None;
    let started = Instant::now();
    let mut snap = HudSnapshot {
        preset: config.preset.clone(),
        poll_interval_ms: config.poll_interval_ms,
        max_output_bytes: config.max_output_bytes,
        ..Default::default()
    };

    while !shutdown.load(Ordering::Relaxed) {
        snap.uptime = started.elapsed();
        // ── Poll clipboard ───────────────────────────────────────
        match crate::clipboard::read() {
            Ok(content) => {
                snap.clipboard_bytes = content.len();

                // Skip if we've already processed this content.
                let already_seen = last_seen.as_deref() == Some(content.as_str());
                if already_seen {
                    sleep_with_hud(
                        &shutdown,
                        &mut hud,
                        &mut snap,
                        started,
                        config.poll_interval_ms,
                    );
                    continue;
                }

                if let Some(cmd) = extract_command(&content) {
                    hud.log(&format!("📥 Received command ({} bytes)", cmd.len()));
                    snap.last_command = Some(cmd.clone());
                    snap.commands += 1;

                    let result = execute_command(&cmd, config.max_output_bytes);
                    snap.last_exit = Some(result.exit_code);
                    snap.truncated = result.truncated;
                    if result.truncated {
                        hud.log(&format!(
                            "✂️  Output exceeded {} bytes — appended \"{}\" before encoding",
                            config.max_output_bytes, TRUNCATION_NOTE
                        ));
                    }

                    let response = format_response(&result);
                    match crate::clipboard::write(&response) {
                        Ok(()) => {
                            hud.log(&format!(
                                "📤 Result written to clipboard ({} bytes)",
                                response.len()
                            ));
                        }
                        Err(e) => {
                            snap.errors += 1;
                            hud.log(&format!("❌ Failed to write result: {}", e));
                        }
                    }
                }

                last_seen = Some(content);
            }
            Err(e) => {
                snap.errors += 1;
                hud.log(&format!("⚠️  Clipboard read error: {}", e));
            }
        }

        // Bar draws once at the end of each poll; a tick of sleep will refresh
        // the uptime before redraw.
        hud.render(&snap);
        sleep_with_hud(
            &shutdown,
            &mut hud,
            &mut snap,
            started,
            config.poll_interval_ms,
        );
    }

    hud.log("🛑 Relay daemon stopped.");
    hud.finish();
}

/// Sleep between polls in short slices so the HUD keeps its uptime current and
/// `Ctrl+C` is not held up for a whole poll interval.
fn sleep_with_hud(
    shutdown: &AtomicBool,
    hud: &mut Hud,
    snap: &mut HudSnapshot,
    started: Instant,
    poll_interval_ms: u64,
) {
    // One redraw per ~250 ms; the redraw is a no-op unless the line changed.
    const TICK_MS: u64 = 250;

    let mut remaining = poll_interval_ms;
    while remaining > 0 {
        let step = remaining.min(TICK_MS);
        thread::sleep(Duration::from_millis(step));
        remaining -= step;
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        snap.uptime = started.elapsed();
        hud.render(snap);
    }
}

/// Extract a Governor command from clipboard content.
pub(crate) fn extract_command(content: &str) -> Option<String> {
    let start = content.find(CMD_MARKER)?;
    let payload_start = start + CMD_MARKER.len();
    let end = content[payload_start..].find("-->")?;
    let b64 = &content[payload_start..payload_start + end];

    base64_decode(b64).ok()
}

/// Format a result marker for clipboard delivery.
fn format_response(result: &CommandResult) -> String {
    let payload = format!(
        "exit:{}|stdout:{}|stderr:{}",
        result.exit_code,
        base64_encode(&result.stdout),
        base64_encode(&result.stderr),
    );
    // The relay protocol places the marker in clipboard for AI to read.
    // Full marker wrapping: <!-- GOV-RES:<b64> -->
    base64_encode(&payload)
}

/// Result of executing a shell command.
#[derive(Debug)]
struct CommandResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    /// True when either stream exceeded `max_output_bytes` and was suffixed
    /// with [`TRUNCATION_NOTE`].
    truncated: bool,
}

/// Execute a shell command and capture output.
///
/// Both streams are capped at `max_output` bytes *including* the truncation
/// note, so a response never grows past the configured ceiling.
fn execute_command(cmd: &str, max_output: usize) -> CommandResult {
    let shell = shell_command();
    let output = std::process::Command::new(&shell.0)
        .arg(&shell.1)
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let stdout = truncate_utf8(&stdout, max_output);
            let stderr = truncate_utf8(&stderr, max_output);
            CommandResult {
                exit_code: out.status.code().unwrap_or(-1),
                truncated: stdout.was_truncated || stderr.was_truncated,
                stdout: stdout.text,
                stderr: stderr.text,
            }
        }
        Err(e) => CommandResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("execution error: {}", e),
            truncated: false,
        },
    }
}

/// Output of [`truncate_utf8`]: the text plus whether a note was appended.
#[derive(Debug)]
pub(crate) struct TruncatedOutput {
    pub text: String,
    pub was_truncated: bool,
}

/// Truncate a UTF-8 string to at most `max` bytes on a character boundary.
///
/// When `s` is over the limit the result is `head + TRUNCATION_NOTE`, and room
/// for the note is carved out of the head so the *result* still fits in `max`
/// bytes — callers can treat `max` as a hard ceiling. If `max` is smaller than
/// the note itself, the note wins: a cut the reader cannot recognise as a cut
/// is worse than a payload a few bytes over budget.
pub(crate) fn truncate_utf8(s: &str, max: usize) -> TruncatedOutput {
    if s.len() <= max {
        return TruncatedOutput {
            text: s.to_string(),
            was_truncated: false,
        };
    }

    let mut end = max.saturating_sub(TRUNCATION_NOTE.len()).min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }

    TruncatedOutput {
        text: format!("{}{}", &s[..end], TRUNCATION_NOTE),
        was_truncated: true,
    }
}

/// Get the platform-specific shell invocation.
fn shell_command() -> (String, String) {
    #[cfg(windows)]
    {
        ("powershell".to_string(), "-Command".to_string())
    }
    #[cfg(not(windows))]
    {
        ("sh".to_string(), "-c".to_string())
    }
}

/// Simple base64 decode.
pub(crate) fn base64_decode(input: &str) -> Result<String, String> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [0u8; 128];
    for (i, &c) in CHARS.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }

    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'=' && b != b'\n' && b != b'\r' && b != b' ')
        .map(|b| lookup.get(b as usize).copied().unwrap_or(0))
        .collect();

    let mut result = Vec::new();
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let b3 = chunk.get(3).copied().unwrap_or(0) as u32;

        let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
        result.push((triple >> 16) as u8);
        if chunk.len() > 2 {
            result.push(((triple >> 8) & 0xFF) as u8);
        }
        if chunk.len() > 3 {
            result.push((triple & 0xFF) as u8);
        }
    }

    String::from_utf8(result).map_err(|e| format!("invalid UTF-8 in decoded payload: {}", e))
}

/// Simple base64 encode (used for relay responses).
pub(crate) fn base64_encode(data: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_roundtrip() {
        let original = "Hello, Governor!";
        let encoded = base64_encode(original);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_truncate_utf8() {
        let s = "hello world";

        // Under the limit: byte-for-byte identical, no note, no flag.
        let out = truncate_utf8(s, 100);
        assert!(!out.was_truncated);
        assert_eq!(out.text, s);

        // Exactly at the limit is not truncation.
        let out = truncate_utf8(s, s.len());
        assert!(!out.was_truncated);
        assert_eq!(out.text, s);

        // A budget too small to hold the note: the note is the payload.
        let out = truncate_utf8(s, 5);
        assert!(out.was_truncated);
        assert_eq!(out.text, TRUNCATION_NOTE);
    }

    #[test]
    fn truncation_note_text_and_length() {
        assert_eq!(TRUNCATION_NOTE, "... [truncated]");
        assert_eq!(TRUNCATION_NOTE.len(), 15);

        let s = "hello world, this is a long line of output";
        let out = truncate_utf8(s, 30);
        assert!(out.was_truncated);
        assert_eq!(out.text, "hello world, th... [truncated]");
        assert_eq!(out.text.len(), 30, "note must fit inside the budget");
    }

    #[test]
    fn truncation_never_exceeds_the_budget() {
        let ascii = "x".repeat(4096);
        // Multi-byte output: 5 bytes per repeat, so every budget below is
        // actually exceeded and the char-boundary walk is exercised.
        let unicode = "πз ".repeat(2000);
        assert!(unicode.len() > 4095, "fixture must exceed the largest budget");

        for s in [&ascii, &unicode] {
            for max in [TRUNCATION_NOTE.len(), 16, 64, 100, 777, 4095] {
                let out = truncate_utf8(s, max);
                assert!(out.was_truncated, "len {} should exceed {max}", s.len());
                assert!(
                    out.text.len() <= max,
                    "max {max} violated: got {} bytes",
                    out.text.len()
                );
                assert!(out.text.ends_with(TRUNCATION_NOTE));
                // The head must still be valid UTF-8 (no split code points).
                let head = &out.text[..out.text.len() - TRUNCATION_NOTE.len()];
                assert!(std::str::from_utf8(head.as_bytes()).is_ok(), "{head:?}");
            }
        }
    }

    #[test]
    fn truncation_note_survives_the_base64_roundtrip() {
        // The requirement: the note is part of the text *before* encoding, so
        // whoever decodes the response sees that the output was cut.
        let result = CommandResult {
            exit_code: 0,
            stdout: truncate_utf8(&"y".repeat(500), 64).text,
            stderr: String::new(),
            truncated: true,
        };

        let response = format_response(&result);
        let payload = base64_decode(&response).expect("response payload decodes");
        let field = payload
            .split('|')
            .find(|f| f.starts_with("stdout:"))
            .expect("stdout field present");

        let stdout = base64_decode(&field["stdout:".len()..]).expect("stdout b64 decodes");
        assert!(stdout.ends_with(TRUNCATION_NOTE), "note lost: {stdout:?}");
        assert_eq!(stdout.len(), 64);
    }

    #[test]
    fn test_extract_command() {
        let b64 = base64_encode("echo test");
        let content = format!("some text {}--> more text", CMD_MARKER.to_string() + &b64);
        let extracted = extract_command(&content);
        assert_eq!(extracted, Some("echo test".to_string()));
    }

    #[test]
    fn test_config_presets() {
        let minimal = RelayConfig::from_preset("minimal");
        assert_eq!(minimal.poll_interval_ms, 2000);

        let full = RelayConfig::from_preset("full");
        assert_eq!(full.poll_interval_ms, 500);

        let standard = RelayConfig::from_preset("standard");
        assert_eq!(standard.poll_interval_ms, 1000);
    }

    #[test]
    fn presets_do_not_enable_the_hud() {
        // --hud stays an explicit opt-in, never a preset side effect.
        assert!(!RelayConfig::default().hud);
        for preset in ["minimal", "standard", "full", "unknown"] {
            assert!(!RelayConfig::from_preset(preset).hud, "{preset}");
        }
    }
}