# Governor v2 — Architecture Decisions

Key architectural decisions extracted from v1's 42 ADRs.

## ADR-001: Memory Separation

**Decision:** Three separate files for different memory types.

| File | Scope | Lifetime |
|------|-------|----------|
| `STATE.md` | Operational: what's happening now | Per-session |
| `DECISIONS.md` | Architectural: what we decided and why | Project lifetime |
| `ROADMAP.md` | Strategic: where we're going | Project lifetime |

**Rationale:** Mixing operational state with architectural decisions creates
cognitive overload. Separating them lets each file serve its purpose cleanly.

## ADR-002: Single Binary

**Decision:** One Rust binary for all Governor functionality.

**Rationale:** v1 spread across Rust, PowerShell, Python, and Batch.
One binary eliminates launch complexity, cross-language bugs, and
dependency hell.

## ADR-003: SHA-256 Contract Integrity

**Decision:** Implementation contracts carry embedded SHA-256 hashes
for tamper detection.

**Rationale:** When the AI modifies `IMPLEMENTATION.md`, we need to verify
the final state matches what was agreed. An embedded hash provides
cryptographic assurance without external files.

## ADR-004: Role-Based Routing

**Decision:** Context headers signal the AI which role to assume:
Engineer for implementation, Architect for review.

**Rationale:** The same AI model needs different behavioral modes.
Explicit role markers prevent mode confusion.

## ADR-005: Lock-Based Concurrency

**Decision:** File-based exclusive locks with PID validation.

**Rationale:** Multiple concurrent sessions on the same project cause
conflicts. A lock file with PID tracking enables automatic stale-lock
cleanup.

## ADR-006: Clipboard as Transport

**Decision:** System clipboard as the primary human-AI interface.

**Rationale:** Universal, no network setup, works offline.
The relay daemon extends this to autonomous execution when needed.

## ADR-007: YAGNI Exclusions for v2.0

**Decision:** The following v1 features are deferred:

- SDAP anomaly detection — rare use, complex implementation
- Three-layer signals (H/O/I) — premature abstraction
- AUTHORIZED_DELTAS — solves a problem v2 doesn't have yet
- Evidence rotation — flat files are simpler

**Rationale:** Build what's needed now. Add complexity when demand is proven.

## ADR-008: Size-First Release Profile

**Decision:** Release builds optimize for binary size, not runtime speed, and
abort instead of unwinding on panic.

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

**Measured (2026-11-12, rustc 1.98.1, x86_64-pc-windows-msvc):**

| Metric | Before | After |
|--------|--------|-------|
| `governor.exe` | 1,107,456 B (1081.5 KB) | 550,400 B (537.5 KB) |
| Release build time | ~20 s | 42 s |
| Tests | 30/30 | 30/30 (dev *and* `--release`) |

**Size budget:**

Artifact size is a user-visible property (ADR-002), so the release profile is
paired with an explicit budget. The budget is a ceiling, not a target — it
moves only when a feature legitimately needs the headroom:

| Milestone | Budget | Measured then | What consumed the headroom |
|-----------|--------|---------------|----------------------------|
| v2.0 release | 700 KB | 537.5 KB | initial release profile |
| v2.1 (completions) | 1 MB | 771 KB | shell completions, health diagnostics |
| v2.1 (polish, 2026-11-15) | **1.5 MB** | 1017 KB | template system; session history next |

The profile itself is unchanged — only the ceiling moves, and each raise is
backed by a shipped feature rather than by growth on its own.

**Rationale:**

- ADR-002 makes the binary the single deliverable, so artifact size is a
  user-visible property rather than an internal detail.
- Governor is interactive and I/O-bound: clipboard syscalls and filesystem
  walks dominate, with no compute-heavy hot path. Trading codegen speed for
  size therefore costs nothing measurable while halving the artifact.
- Unwind tables are a large share of any Rust binary, and nothing in this
  codebase unwinds: `catch_unwind`, `#[should_panic]`, and `std::panic`
  appear nowhere in `src/` or `tests/`.
- `strip = true` and `panic = "abort"` reinforce ADR-002 — one small binary
  that still does the entire job.

**Alternatives considered:**

| Alternative | Verdict |
|-------------|---------|
| `opt-level = 3` (release default) | Rejected — roughly double the size for an I/O-bound tool. |
| `opt-level = "s"` | Live alternative — similar size, better speed. Revisit if a startup or TUI-redraw benchmark regresses. |
| `lto = "thin"` | Rejected for release artifacts — faster link, larger output. Reasonable for CI iteration if 42 s becomes painful. |
| `panic = "unwind"` | Rejected — pays size for an unwinding path no code uses. |
| `strip = "debuginfo"` | Live alternative — keeps symbol names so panic locations are readable, at a size cost. Adopt if silent crashes become a support problem. |

**Consequences (accepted):**

1. **The TUI stays safe without unwinding.** `ratatui::init()` installs a panic
   hook that calls `restore()` — disabling raw mode and leaving the alternate
   screen — before the original hook runs, and panic hooks *do* run under
   `panic = "abort"` (ratatui-0.29.0, `terminal/init.rs:237-243`). A panic
   inside `governor start` will not strand the user's terminal in raw mode.
2. **Panic exit status changes.** A panic now aborts rather than exiting with
   Rust's usual code 101. Scripts wrapping Governor must not key on 101;
   `main.rs` still exits 1 for handled errors, which is unaffected.
3. **Reduced crash diagnostics.** With `strip = true` the panic message is
   printed but symbolized locations may be unresolvable. Diagnosing a crash
   means rebuilding without `strip`.
4. **Doubled release build time** (20 s → 42 s) from fat LTO plus a single
   codegen unit. Iterate in the dev profile; build release for artifacts.
5. **`catch_unwind` is no longer available** as a design tool. A future
   fault-tolerant relay loop that survives a panicking command must use
   result propagation or process isolation instead of catching unwinds.

**Reversibility:** High. One TOML section; re-measure the artifact and re-run
`cargo test --release` before changing it.