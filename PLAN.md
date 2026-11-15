# Governor v2.1 — Plan

## Overview
Increment on v2.0 with user‑experience polish, wider ecosystem integration, and performance guardrails.

**Status**: Shell‑completions command implemented (`governor completions <shell>`). Configuration ergonomics (section 4) complete. Performance guardrails (section 5) complete: lock‑acquisition timeout (5.2) and project‑discovery parallelism (5.3).

## Deliverables

### ✅ 1. Shell completions (DONE)
Generate completion scripts for Bash, Zsh, Fish, PowerShell, Elvish using `clap_complete`. Ship as part of the release archive; `governor completions <shell>` command implemented.

**Completed:**
- `clap_complete` dependency added.
- `Command::Completions` subcommand present.
- `cmd_completions` generates scripts to stdout.
- Validates shell name, shows possible values and similarity tip.
- Works for all five shells, syntax correct.

**Remaining:**
- Bundle pre‑generated completions in release archive (future packaging).

### ✅ 2. Health‑check improvements (DONE)
- **Clipboard throughput benchmark:** Measure copy‑paste latency for a realistic‑sized context (~10 KB). Implemented, gated behind `--verbose`.
- **Filesystem saturation detection:** Warn if workspace roots are on a nearly‑full drive. Uses `fs2` and `bytesize` crates, reports drive capacity >95%.
- **Network‑time sync:** Detect whether the system clock is wildly off, which could affect lock freshness. Checks lock‑file timestamps, warns if future >5 minutes.
- **Relay‑daemon sanity test:** Verify that the relay can actually decode a known command from the clipboard. Sends `ping` command round‑trip via clipboard, verifies response.
- Unit tests added for each new check; all 50 tests (37 unit + 13 integration) pass.

### 3. Relay daemon enhancements
- **HUD overlay:** Show status bar (clipboard size, last command, uptime) in a small terminal corner.
- **Preset‑switching at runtime:** Send SIGUSR1 (or a signal file) to reload config and adjust polling.
- **Output truncation warning:** If a command’s stdout exceeds `max_output_bytes`, add a footnote “(truncated…)” before base64 encoding.

### ✅ 4. Configuration ergonomics (DONE)
- **Environment‑variable overrides:** `GOVERNOR_WORKSPACE_ROOTS`, `GOVERNOR_PRESET`, etc. ✅
- **Built‑in config validation:** `governor config --validate` already done; add `--check-root` that also verifies each workspace root exists and is readable. ✅
- **Config migration:** If `~/.governor/config.toml` is missing but `~/.governor/state.json` exists (hypothetical v1 remnant), print a migration notice. ✅

### 5. Performance guardrails
- **✅ Context‑assembly cache:** Hash project state and reuse the assembled context if nothing changed.
- **✅ Lock‑acquisition timeout:** If the lock is held by another PID, wait up to N seconds before giving up.
- **✅ Project‑discovery parallelism:** Scan workspace roots concurrently with a thread pool (`src/pool.rs`, std‑only; results stay deterministic — same order as a sequential scan).

### 6. Packaging
- **Windows installer:** Inno Setup `.exe` that installs binary, completions, and adds `%USERPROFILE%\.governor\config.toml` template.
- **Homebrew formula** (macOS/Linux), **Scoop manifest** (Windows), **AUR PKGBUILD** (Arch Linux).
- **Docker image** for CI environments (scratch‑based, static binary).

### 7. Documentation
- **Man pages** via `clap_mangen`.
- **Configuration examples** for common workflows (multi‑monitor relay, remote projects).
- **Troubleshooting guide:** “Clipboard not working”, “Project not discovered”, “Lock stuck”.

## Non‑goals (v2.1)
- **Plug‑in system:** Keep the binary monolithic.
- **GUI:** Stay terminal‑first.
- **Cloud sync:** Governor remains a local workstation tool.

## Timeline
1. **Week 1:** Shell completions + health‑check improvements.
2. **Week 2:** Relay enhancements + configuration ergonomics.
3. **Week 3:** Performance guardrails + packaging.
4. **Week 4:** Documentation + final integration testing.

## Success metrics
- Binary size stays under 1 MB (771 KB currently).
- All existing tests pass (50).
- Completion scripts work in each target shell (verified).
- Health‑check runs under 2 seconds on a typical workstation.
- Relay daemon memory usage stable (<10 MB resident).