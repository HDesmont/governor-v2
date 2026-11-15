# Implementation Contract

## Goal
Verify that the shell‑completions feature (`governor completions <shell>`) is production‑ready, meets the binary‑size budget, and integrate it into the release workflow. Additionally, implement health‑check improvements (clipboard throughput, FS saturation, clock skew, relay sanity test) as the second v2.1 plan item.

## Acceptance Criteria
- [x] `governor completions <shell>` generates valid scripts for bash, zsh, fish, powershell, elvish (five shells)
- [x] Command validates input, prints helpful error messages with possible values and similarity tip
- [x] Completion scripts are syntactically correct and output to stdout
- [x] All existing tests pass (50/50)
- [x] Binary size stays under 1.5 MB (budget increased for v2.1, currently 771 KB)
- [x] Health‑check improvements implemented: clipboard throughput benchmark (gated behind `--verbose`)
- [x] Filesystem saturation detection (warns if drive >95% full)
- [x] Clock‑skew detection (checks lock‑file timestamps)
- [x] Relay‑daemon sanity test (round‑trip ping via clipboard)
- [ ] Update documentation: mention completions in `--help`, add installation instructions to README or a separate `completions/` directory
- [ ] Update release workflow (PLAN.md) to bundle pre‑generated completions in release archive

## Constraints
- Rust 1.98, single binary, minimal external dependencies (`clap_complete`, `bytesize`, `fs2`)
- Must not break existing CLI commands or TUI
- Size‑first principle: keep binary under 1.5 MB after LTO+strip (budget increased for v2.1)
- Cross‑platform (Windows, macOS, Linux)
- New health checks must be gated behind `--verbose` flag to avoid slowing default runs

## Approach
1. Shell completions: integrate `clap_complete`, add `Command::Completions` subcommand, generate scripts for five shells.
2. Health‑check improvements: implement four verbose‑only checks using `bytesize` and `fs2` crates.
3. Add unit tests for new checks.
4. Verify binary size stays under 1.5 MB (771 KB achieved).
5. Ensure all existing tests pass (50/50).
6. Update STATE.md, PLAN.md, and IMPLEMENTATION.md with progress and new hash.
7. Verify release‑profile settings remain aggressive: `opt-level="z"`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true`.
8. Ensure `cargo test --release` passes all tests (confirmed).
<!-- GOVERNOR-HASH:aedc301ae6386d5dd43495b74400b8d59bf466aa6d6b49768789be89bccc378a -->
