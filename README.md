# Governor v2

**AI-assisted development governance** — a single-binary Rust tool that gives
AI coding agents a controlled, verifiable workflow: structured project state,
tamper-evident contracts, and a clipboard-based transport that works with *any*
AI chat, IDE, or agent.

```
governor 2.0.0 — one binary, zero runtime dependencies, < 1 MB.
```

---

## Philosophy

### Human-as-Transport

The AI never touches your filesystem directly. Instead, the human moves
information between the machine and the AI by hand — via the clipboard:

1. `governor start` assembles the **AI context** (project state, contract,
   decisions) and copies it to the clipboard. You paste it into your AI chat.
2. The AI produces updates. You paste results back; the relay daemon and
   clipboard markers (`<!-- GOV-CMD:... -->`) carry structured commands.
3. `governor close` validates everything and ends the session.

This makes the **human the arbiter**: nothing reaches the repository without
passing through a person. It also makes Governor universally compatible —
if a system can read a clipboard, it can participate.

### AI-assisted governance, not AI autonomy

Governor assumes the AI is a brilliant but unreliable collaborator. So:

- **Contracts over vibes.** `IMPLEMENTATION.md` is the binding statement of
  what is being built. It is signed with a SHA-256 hash; `governor health`
  detects any silent edit to a signed contract (tamper evidence).
- **Memory that survives the chat.** `STATE.md` is the project's operational
  memory — current task, progress, decisions, notes. It outlives every
  conversation.
- **Small, verifiable surface.** One binary, no daemons required, no network
  services. Everything it does is inspectable as plain Markdown on disk.

---

## Quick start

```bash
# 1. Scaffold a project (creates STATE.md + IMPLEMENTATION.md templates)
governor init

# 2. Start a session
governor start              # interactive TUI project selector
governor start my-project   # or directly, by name

# 3. ... paste AI context into your AI chat, work, paste back ...

# 4. Close the session (validates state, copies verification context)
governor close
```

Build from source:

```bash
cargo build --release
# binary: target/release/governor
```

---

## Commands

| Command | Description |
|---|---|
| `governor init [--force]` | Scaffold `STATE.md` + `IMPLEMENTATION.md` in the current directory. Existing files are never overwritten without `--force`. |
| `governor start [project]` | Discover the project, assemble AI context, copy it to the clipboard, and hold a session lock. Without a name, opens the interactive TUI selector. |
| `governor close [project]` | Validate and close the active session; copies a verification context to the clipboard. |
| `governor health [project] [--verbose]` | Run workspace health checks (clipboard, filesystem, contract integrity, locks). **Exits 1 if any check fails (❌), 0 on pass/warn only** — safe to use in CI and scripts. |
| `governor relay [--preset minimal\|standard\|full] [--hud]` | Run the relay daemon: watches the clipboard for `GOV-CMD` markers and executes them, optionally with an always-on-top HUD status bar. |
| `governor projects [--verbose]` | List discovered projects with lock status and metadata. |
| `governor config --show` / `--validate [path] [--check-root]` | Show the effective configuration or validate a config file (`~/.governor/config.toml`). |
| `governor completions <shell>` | Print shell completions for bash, zsh, fish, PowerShell, or Elvish. |

### Exit codes

`governor health` follows a strict exit-code contract:

- `0` — all checks pass, or only warnings (⚠️) were reported;
- `1` — at least one check failed (❌), or a command-level error occurred.

### Configuration

`~/.governor/config.toml`:

```toml
[workspace]
roots = ["C:/work", "D:/projects"]   # where discovery looks for STATE.md

[session]
preset = "standard"                  # context assembly preset

[clipboard]
backend = "auto"                     # arboard-backed, no external processes
```

See `governor config --show` for the full annotated template.

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                      governor (bin)                      │
│  cli ── start / close / health / relay / init / config   │
└──────┬───────────────┬───────────────┬───────────────────┘
       │               │               │
   project.rs      session.rs      relay.rs (daemon)
   discovery,      lifecycle,      clipboard command
   locks           lock mgmt       transport (+ HUD)
       │               │               │
       └───────┬───────┴───────┬───────┘
           context.rs      integrity.rs
           assembly,       SHA-256 signing
           cache, hash     & validation
       ┌───────┴───────────────┴───────┐
       │  clipboard.rs (arboard)       │
       │  tui.rs (ratatui selector)    │
       │  config.rs, health.rs, pool.rs│
       └───────────────────────────────┘
```

### The three pillars

**`STATE.md` — memory.**
The project's operational memory, human-readable and version-controllable.
It records the current task, progress checklist, context, and notes.
Discovery finds projects by locating `STATE.md` files under configured
workspace roots (scanned in parallel by a small std-only thread pool).

**`IMPLEMENTATION.md` — contract.**
The binding definition of what is being built. `governor start` signs it by
embedding a `GOVERNOR-HASH:` marker (SHA-256 of the stripped content).
`governor health` re-validates the signature: any accidental or silent edit
produces a `hash mismatch` failure. An unsigned contract is reported as a
warning, not an error.

**Integrity hash — tamper evidence.**
Hashes live inside the file itself (`<!-- GOVERNOR-HASH:... -->`), so the
contract is self-verifying everywhere: in git history, on another machine,
in an AI chat paste. `context.rs` strips hash markers before assembling the
AI context, keeping prompts clean.

**Relay daemon — the transport layer.**
`governor relay` polls the clipboard for `<!-- GOV-CMD:<base64> -->` markers
and executes them as commands, with heartbeat, output caps, and truncation
warnings. `--hud` pins a status bar (clipboard size, last command, counters)
to the terminal. Presets (`minimal` / `standard` / `full`) trade capability
for footprint.

### Session lifecycle

```
init → start (context → clipboard, lock acquired)
     → [ human pastes context into AI, works, pastes back ]
     → close (validation, verification context → clipboard, lock released)
```

Locks (`session.lock`, JSON with PID + host + timestamp) prevent concurrent
sessions; stale locks are detected by PID liveness and reported by
`governor health`.

---

## Development

```bash
cargo clippy --all-targets -- -D warnings   # lint gate
cargo test                                  # unit + integration tests
```

Further documentation:

- [`docs/PROTOCOL.md`](docs/PROTOCOL.md) — clipboard transport protocol
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — architecture decision records
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — what's next

## License

MIT
