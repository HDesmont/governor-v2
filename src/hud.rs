//! Relay HUD — a one-line status bar for the relay daemon.
//!
//! The daemon runs in the foreground and owns the terminal, so the HUD does
//! *not* take over the alternate screen: a full-screen `ratatui` viewport
//! would hide the log stream, fight with `Ctrl+C` feedback, and leave the
//! terminal damaged if the process is `SIGKILL`ed (no panic hook can run
//! there — see ADR-008, `panic = "abort"`).
//!
//! Instead the status line is pinned to the bottom of the viewport and every
//! daemon log message is routed through [`Hud::log`], which erases the line,
//! scrolls the message, and redraws the line. Plain text, no colours: escape
//! sequences would make the width arithmetic wrong on narrow terminals, and
//! the line must never wrap (a wrapped status line corrupts the redraw).
//!
//! When stderr is not a terminal (piped logs, CI, `governor relay >> file`),
//! the HUD degrades to ordinary log lines with no escape sequences at all.

use std::io::Write; // for `writeln!`/`queue!` into stderr
use std::time::Duration;

use crossterm::cursor::MoveToColumn;
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::tty::IsTty;
use crossterm::queue;

/// Fallback terminal width when the size cannot be queried
/// (non-tty stderr, or a console reporting zeros).
pub const DEFAULT_WIDTH: usize = 80;

/// Everything the status bar displays. Collected by the daemon loop once per
/// poll tick — the HUD never touches the clipboard or the clock itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HudSnapshot {
    /// Preset the daemon is running under (`minimal` / `standard` / `full`).
    pub preset: String,
    /// Poll interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Hard ceiling on captured output, in bytes (`max_output_bytes`).
    pub max_output_bytes: usize,
    /// Wall-clock uptime of the daemon.
    pub uptime: Duration,
    /// Size of the most recent clipboard read, in bytes.
    pub clipboard_bytes: usize,
    /// The last command received (before execution).
    pub last_command: Option<String>,
    /// Exit code of the last executed command.
    pub last_exit: Option<i32>,
    
    /// Whether the last command's output hit `max_output_bytes`.
    pub truncated: bool,
    /// Commands executed since start.
    pub commands: u64,
    /// Clipboard or execution errors since start.
    pub errors: u64,
}

impl HudSnapshot {
    /// Preset with a safe placeholder when the daemon started without one.
    pub fn preset_or_default(&self) -> &str {
        if self.preset.is_empty() {
            "standard"
        } else {
            &self.preset
        }
    }
}

/// Field separator between status segments.
const SEP: &str = " | ";

/// Renders the status line by adding segments in priority order and dropping
/// whatever does not fit — so the bar never wraps and never loses the fields
/// the plan requires (clipboard size, last command, uptime).
///
/// Priority, highest first: preset, uptime, clipboard size, last command
/// (clipped to the space that is left), truncation warning, error count,
/// command count. A 120-column terminal sees all of it; an 80-column one
/// loses the tail counters before it loses the last command.
///
/// Example (wide):
/// ```text
/// relay[standard] | up 00:04:12 | clip 1.2 KiB | last "ping" exit 0 | cmds 3
/// ```
pub fn status_line(snap: &HudSnapshot, width: usize) -> String {
    // Core: always present, always short.
    let mut line = format!("relay[{}]", snap.preset_or_default());
    try_push(&mut line, &format!("up {}", format_uptime(snap.uptime)), width);
    try_push(
        &mut line,
        &format!("clip {}", human_bytes(snap.clipboard_bytes)),
        width,
    );

    // Last command: sized to the space that is actually left.
    if let Some(cmd) = &snap.last_command {
        let suffix = format!(" exit {}", format_exit(snap.last_exit));
        // line + SEP + `last "` + `" closing the quote + suffix + the command
        let overhead = line.chars().count()
            + SEP.chars().count()
            + "last ".chars().count()
            + 2
            + suffix.chars().count();
        let room = width.saturating_sub(overhead);
        if room >= 4 {
            let seg = format!("last {:?}{}", one_line(cmd, room), suffix);
            try_push(&mut line, &seg, width);
        }
    } else {
        try_push(&mut line, "last -", width);
    }

    // Tail: dropped first on a narrow terminal.
    if snap.truncated {
        let seg = if snap.max_output_bytes > 0 {
            format!("TRUNCATED@{}", human_bytes(snap.max_output_bytes))
        } else {
            "TRUNCATED".to_string()
        };
        try_push(&mut line, &seg, width);
    }
    if snap.errors > 0 {
        try_push(&mut line, &format!("err {}", snap.errors), width);
    }
    if snap.commands > 0 {
        try_push(&mut line, &format!("cmds {}", snap.commands), width);
    }

    clip_chars(&line, width)
}

/// Append `seg` to `line` (with [`SEP`]) only when the result still fits.
fn try_push(line: &mut String, seg: &str, width: usize) {
    let candidate_len = line.chars().count() + SEP.chars().count() + seg.chars().count();
    if candidate_len <= width {
        line.push_str(SEP);
        line.push_str(seg);
    }
}

/// Renders the status line with the core fields only — used when the full
/// line does not fit the terminal. Keeps the three fields the plan calls out
/// (clipboard size, last command, uptime) instead of wrapping the line.
fn minimal_line(snap: &HudSnapshot, width: usize) -> String {
    let last = match &snap.last_command {
        Some(cmd) => one_line(cmd, 16),
        None => "-".to_string(),
    };
    let line = format!(
        "relay | up {} | clip {} | {}",
        format_uptime(snap.uptime),
        human_bytes(snap.clipboard_bytes),
        last
    );
    clip_chars(&line, width)
}

/// Collapse embedded newlines/tabs so one log message is one screen line.
///
/// Runs of whitespace become a single space: `\r\n` would otherwise leave a
/// gap, and a status bar padded with blank columns looks like missing data.
pub fn one_line(s: &str, max_chars: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let mut out: String = flat.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `HH:MM:SS`, where hours may exceed 24 (a daemon left running for days).
pub fn format_uptime(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Compact binary-unit byte count: `512 B`, `1.2 KiB`, `64.0 KiB`, `2.0 MiB`.
pub fn human_bytes(n: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    if n < 1024 {
        return format!("{} {}", n, UNITS[0]);
    }
    let mut value = n as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value as usize, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Exit code as it should appear in the bar.
fn format_exit(code: Option<i32>) -> String {
    match code {
        Some(c) => c.to_string(),
        None => "?".to_string(),
    }
}

/// Clip to `max` *characters* (not bytes), marking the cut with `…`.
fn clip_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The HUD handle, owned by the daemon loop.
///
/// Not thread-safe by design: only the daemon thread renders, so no lock is
/// needed and a `Ctrl+C` from another thread can never interleave a partial
/// escape sequence with a status redraw.
///
/// The writer is a parameter, not a hardcoded `stderr`, so tests can assert
/// the exact redraw sequence without a terminal. The HUD owns no clock either:
/// uptime arrives in the snapshot, which keeps rendering deterministic.
#[derive(Debug)]
pub struct Hud<W: Write = std::io::Stderr> {
    enabled: bool,
    /// True while a status line is sitting on the last row of the terminal.
    shown: bool,
    width: usize,
    last_drawn: String,
    out: W,
}

impl Hud {
    /// Create a HUD writing to stderr. `want` comes from `--hud`; the HUD
    /// additionally requires a real terminal on stderr.
    pub fn new(want: bool) -> Self {
        let enabled = want && std::io::stderr().is_tty();
        let width = terminal::size()
            .map(|(cols, _)| cols as usize)
            .unwrap_or(DEFAULT_WIDTH)
            .max(1);
        Self {
            enabled,
            shown: false,
            width,
            last_drawn: String::new(),
            out: std::io::stderr(),
        }
    }
}

impl<W: Write> Hud<W> {
    /// Build a HUD over an arbitrary writer. Used by tests; `enabled` is
    /// forced so a test can drive the tty code path without a tty.
    pub fn with_writer(enabled: bool, width: usize, out: W) -> Self {
        Self {
            enabled,
            shown: false,
            width: width.max(1),
            last_drawn: String::new(),
            out,
        }
    }

    /// Whether the status bar is active (i.e. `--hud` *and* a tty).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Compose the status line for this HUD's width, falling back to the
    /// compact core line whenever the full line cannot carry the fields the
    /// plan requires.
    pub fn compose(&self, snap: &HudSnapshot) -> String {
        let full = status_line(snap, self.width);

        // `status_line` fills the row greedily, so a short optional segment
        // (`err 2`) can squeeze out a longer required one (`clip 64.0 KiB`),
        // and `last` is dropped as soon as the command does not fit. Both
        // would leave the bar lying about the session, so fall back to the
        // core line (uptime + clipboard size + last command) instead.
        let lost_clip = !full.contains("clip ");
        let lost_last = snap.last_command.is_some() && !full.contains("last ");
        if lost_clip || lost_last || full.chars().count() >= self.width {
            minimal_line(snap, self.width.saturating_sub(1))
        } else {
            full
        }
    }

    /// Erase the row the cursor is on and park the cursor at column 0.
    fn erase_line(&mut self) {
        let _ = queue!(self.out, MoveToColumn(0), Clear(ClearType::CurrentLine));
    }

    /// Print a daemon log message, keeping the pinned status line intact.
    ///
    /// The status line is written without a trailing newline, so a bare
    /// `writeln!` would append the message to it. Instead the row is erased
    /// first, the message is scrolled past, and the bar reappears on the next
    /// [`Hud::render`] — which the daemon calls every tick.
    pub fn log(&mut self, msg: &str) {
        if self.enabled && self.shown {
            self.erase_line();
            self.shown = false;
            self.last_drawn.clear();
        }
        let _ = writeln!(self.out, "{}", msg);
        let _ = self.out.flush();
    }

    /// Redraw the status bar with the latest snapshot.
    ///
    /// Cheap when the rendered line is identical to what is already on
    /// screen — the daemon calls this every poll tick.
    pub fn render(&mut self, snap: &HudSnapshot) {
        if !self.enabled {
            return;
        }

        let line = self.compose(snap);
        if self.shown && line == self.last_drawn {
            return;
        }

        if self.shown {
            self.erase_line();
        }
        let _ = write!(self.out, "{}", line);
        let _ = self.out.flush();

        self.shown = true;
        self.last_drawn = line;
    }

    /// Erase the pinned line so the shell prompt lands on a clean row.
    pub fn finish(&mut self) {
        if !self.enabled || !self.shown {
            return;
        }
        self.erase_line();
        let _ = self.out.flush();
        self.shown = false;
        self.last_drawn.clear();
    }
}

impl<W: Write> Drop for Hud<W> {
    fn drop(&mut self) {
        // Normal exits go through `Hud::finish`; this covers early returns
        // and unwound panics so the bar never outlives the daemon.
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> HudSnapshot {
        HudSnapshot {
            preset: "standard".to_string(),
            poll_interval_ms: 1000,
            max_output_bytes: 64 * 1024,
            uptime: Duration::from_secs(252),
            clipboard_bytes: 1224,
            last_command: Some("ping".to_string()),
            last_exit: Some(0),
            truncated: false,
            commands: 3,
            errors: 0,
        }
    }

    #[test]
    fn status_line_contains_core_fields() {
        let line = status_line(&snap(), 200);
        assert!(line.contains("relay[standard]"), "{line}");
        assert!(line.contains("up 00:04:12"), "{line}");
        assert!(line.contains("clip 1.2 KiB"), "{line}");
        assert!(line.contains("\"ping\" exit 0"), "{line}");
        assert!(line.contains("cmds 3"), "{line}");
    }

    #[test]
    fn status_line_truncation_and_errors_appear_in_wide_bar() {
        let s = HudSnapshot {
            truncated: true,
            errors: 2,
            max_output_bytes: 64 * 1024,
            ..snap()
        };
        let line = status_line(&s, 120);
        assert!(line.contains("TRUNCATED@64.0 KiB"), "{line}");
        assert!(line.contains("err 2"), "{line}");
    }

    #[test]
    fn status_line_without_commands_shows_placeholder() {
        let line = status_line(&HudSnapshot::default(), 200);
        assert!(line.contains("relay[standard]"), "{line}");
        assert!(line.contains("up 00:00:00"), "{line}");
        assert!(line.contains("clip 0 B"), "{line}");
        assert!(line.contains("last -"), "{line}");
    }

    #[test]
    fn status_line_never_exceeds_width() {
        let s = HudSnapshot {
            last_command: Some("a".repeat(400)),
            ..snap()
        };
        for width in [1usize, 8, 20, 40, 79, 80, 81, 120] {
            let line = status_line(&s, width);
            assert!(
                line.chars().count() <= width,
                "width {width} produced {} chars",
                line.chars().count()
            );
        }
    }

#[test]
    fn wide_bars_keep_every_segment() {
        let snap = HudSnapshot {
            truncated: true,
            errors: 5,
            commands: 42,
            max_output_bytes: 8192,
            ..snap()
        };

        let line = status_line(&snap, 160);
        assert!(line.contains("relay[standard]"), "{line}");
        assert!(line.contains("up 00:04:12"), "{line}");
        assert!(line.contains("clip 1.2 KiB"), "{line}");
        assert!(line.contains("\"ping\" exit 0"), "{line}");
        assert!(line.contains("TRUNCATED@8.0 KiB"), "{line}");
        assert!(line.contains("err 5"), "{line}");
        assert!(line.contains("cmds 42"), "{line}");
    }

    #[test]
    fn status_line_drops_segments_that_do_not_fit() {
        // status_line alone trades whole segments for width: at 40 columns
        // `clip` is longer than the space left after preset + uptime, so it
        // goes. Keeping the required fields is `Hud::compose`'s job.
        let snap = HudSnapshot {
            truncated: true,
            errors: 5,
            commands: 42,
            max_output_bytes: 8192,
            ..snap()
        };

        let line = status_line(&snap, 40);
        assert!(line.chars().count() <= 40, "{line}");
        assert!(line.contains("up 00:04:12"), "{line}");
        assert!(!line.contains("clip"), "segment that does not fit is dropped: {line}");
        assert!(!line.contains("cmds"), "{line}");
    }

    #[test]
    fn compose_keeps_the_planned_fields_at_any_width() {
        let snap = HudSnapshot {
            truncated: true,
            errors: 5,
            commands: 42,
            max_output_bytes: 8192,
            ..snap()
        };

        for width in [40usize, 50, 60, 80, 100, 120, 160] {
            let hud = Hud::with_writer(true, width, Vec::new());
            let line = hud.compose(&snap);
            assert!(line.chars().count() < width, "width {width}: {line}");
            assert!(line.contains("up 00:04:12"), "width {width}: {line}");
            assert!(line.contains("clip"), "width {width}: {line}");
            if width >= 42 {
                // Below that the command head is the thing sacrificed; the
                // sizes and uptime — the fields that cannot be re-derived —
                // stay.
                assert!(line.contains("ping"), "width {width}: {line}");
            }
        }
    }

    #[test]
    fn compose_returns_the_full_line_on_a_wide_terminal() {
        let hud = Hud::with_writer(true, 200, Vec::new());
        assert_eq!(hud.compose(&snap()), status_line(&snap(), 200));
    }

    #[test]
    fn format_uptime_rolls_past_24_hours() {
        assert_eq!(format_uptime(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_uptime(Duration::from_secs(61)), "00:01:01");
        assert_eq!(format_uptime(Duration::from_secs(3661)), "01:01:01");
        assert_eq!(
            format_uptime(Duration::from_secs(90_061)),
            "25:01:01",
            "hours must keep counting instead of wrapping"
        );
    }

    #[test]
    fn human_bytes_uses_binary_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1224), "1.2 KiB");
        assert_eq!(human_bytes(64 * 1024), "64.0 KiB");
        assert_eq!(human_bytes(2 * 1024 * 1024), "2.0 MiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn one_line_collapses_newlines_and_tabs() {
        assert_eq!(one_line("a\nb\tc\r\nd", 40), "a b c d");
        assert_eq!(one_line("   padded   ", 40), "padded");
    }

    #[test]
    fn one_line_clips_on_char_boundary() {
        let clipped = one_line("прив мир", 5);
        assert_eq!(clipped.chars().count(), 5, "{clipped}");
        assert!(clipped.ends_with('…'), "{clipped}");
    }

    #[test]
    fn clip_chars_handles_zero_and_exact_widths() {
        assert_eq!(clip_chars("abc", 0), "");
        assert_eq!(clip_chars("abc", 3), "abc");
        assert_eq!(clip_chars("abcd", 3), "ab…");
    }

    #[test]
    fn hud_degrades_when_stderr_is_not_a_terminal() {
        let hud = Hud::new(true);
        if !std::io::stderr().is_tty() {
            // Cargo captures test output, so the HUD must switch itself off.
            assert!(!hud.enabled(), "HUD enabled without a tty");
        }
    }

    #[test]
    fn hud_without_flag_is_disabled() {
        assert!(!Hud::new(false).enabled());
    }

    #[test]
    fn disabled_hud_emits_plain_lines_only() {
        let mut hud = Hud::with_writer(false, 120, Vec::new());
        hud.render(&snap());
        hud.log("daemon started");
        hud.finish();
        let out = String::from_utf8(hud.out.clone()).unwrap();
        assert_eq!(out, "daemon started\n", "render/finish must be silent: {out:?}");
        assert!(!out.contains('\x1b'), "escape codes leaked into plain mode");
    }

    #[test]
    fn render_writes_the_bar_without_a_trailing_newline() {
        let mut hud = Hud::with_writer(true, 120, Vec::new());
        hud.render(&snap());

        let out = String::from_utf8(hud.out.clone()).unwrap();
        assert!(out.starts_with("relay[standard]"), "{out:?}");
        assert!(!out.ends_with('\n'), "the bar must stay on its own row: {out:?}");
        assert!(!out.contains("\x1b["), "first draw needs no erase: {out:?}");
    }

    #[test]
    fn identical_redraw_is_a_no_op() {
        let mut hud = Hud::with_writer(true, 120, Vec::new());
        hud.render(&snap());
        let after_first = hud.out.len();
        hud.render(&snap());
        assert_eq!(hud.out.len(), after_first, "unchanged bar must not be rewritten");
        hud.render(&HudSnapshot {
            uptime: Duration::from_secs(253),
            ..snap()
        });
        assert!(hud.out.len() > after_first, "uptime change must trigger a redraw");
    }

    #[test]
    fn log_erasures_scroll_the_message_above_the_bar() {
        let mut hud = Hud::with_writer(true, 120, Vec::new());
        hud.render(&snap());
        hud.out.clear();

        hud.log("📥 Received command (9 bytes)");
        let out = String::from_utf8(hud.out.clone()).unwrap();

        // MoveToColumn(0) → `"\x1b[1G"`, Clear(ClearType::CurrentLine) → `"\x1b[2K"`.
        assert!(out.starts_with("\x1b[1G\x1b[2K"), "{out:?}");
        assert!(out.contains("📥 Received command (9 bytes)\n"), "{out:?}");
        assert!(!out.contains("relay["), "bar drawn by log(): {out:?}");

        hud.out.clear();
        hud.render(&snap());
        let out = String::from_utf8(hud.out.clone()).unwrap();
        assert!(out.starts_with("relay["), "{out:?}");
        assert!(!out.contains("\x1b[2K"), "no stale row to erase: {out:?}");
    }

    #[test]
    fn finish_erases_the_pinned_row() {
        let mut hud = Hud::with_writer(true, 120, Vec::new());
        hud.render(&snap());
        hud.out.clear();
        hud.finish();
        assert_eq!(String::from_utf8(hud.out.clone()).unwrap(), "\x1b[1G\x1b[2K");

        // Second finish has nothing to erase.
        let mut quiet = Hud::with_writer(true, 120, Vec::new());
        quiet.finish();
        assert!(quiet.out.is_empty(), "finishing an unshown bar must be silent");
    }

    #[test]
    fn compose_falls_back_to_minimal_when_the_bar_would_wrap() {
        let snap = HudSnapshot {
            last_command: Some("cargo build --release --all-targets".to_string()),
            ..snap()
        };

        let hud = Hud::with_writer(true, 120, Vec::new());
        let line = hud.compose(&snap);
        assert!(line.contains("cmds"), "wide terminal keeps detail: {line}");

        // Narrow (but still realistic) terminal triggers fallback.
        let hud = Hud::with_writer(true, 50, Vec::new());
        let line = hud.compose(&snap);
        assert!(line.chars().count() < 50, "must leave a column: {line}");
        assert!(line.contains("up 00:04:12"), "{line}");
        assert!(line.contains("clip"), "{line}");
        // minimal_line prints the clipped command, not "ping".
        assert!(line.contains("cargo build"), "{line}");
    }

    #[test]
    fn snapshot_preset_falls_back_to_standard() {
        assert_eq!(HudSnapshot::default().preset_or_default(), "standard");
        let s = HudSnapshot {
            preset: "full".to_string(),
            ..Default::default()
        };
        assert_eq!(s.preset_or_default(), "full");
    }
}
