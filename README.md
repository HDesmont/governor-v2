# Governor v2

[![CI](https://github.com/HDesmont/governor-v2/actions/workflows/ci.yml/badge.svg)](https://github.com/HDesmont/governor-v2/actions/workflows/ci.yml)

**AI-assisted development governance** — a single-binary Rust tool that gives
AI coding agents a controlled, verifiable workflow: structured project state,
tamper-evident contracts, and a clipboard-based transport that works with *any*
AI chat, IDE, or agent.

```
governor 2.1.0 — one binary, zero runtime dependencies, < 1.5 MB.
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
governor init                 # default template
governor init -t rust         # built-in template for Rust projects
governor init --template team # custom template from ~/.governor/templates/team/

# 2. Start a session
governor start              # interactive TUI project selector
governor start my-project   # or directly, by name

# 3. ... paste AI context into your AI chat, work, paste back ...

# 4. Close the session (validates state, copies verification context)
governor close
```

### Windows installation

One line in PowerShell — downloads the latest release from GitHub Releases,
installs into `%LOCALAPPDATA%\Governor\bin` and adds it to the user `PATH`:

```powershell
iwr -useb https://raw.githubusercontent.com/HDesmont/governor-v2/main/install.ps1 | iex
```

Open a new terminal and run `governor --version`. The installer is
idempotent — re-running it never duplicates the `PATH` entry. Uninstall:

```powershell
iex "& { $(iwr -useb https://raw.githubusercontent.com/HDesmont/governor-v2/main/install.ps1).Content } -Uninstall"
```

Install from a local build instead of a release download:

```powershell
cargo build --release
.\install.ps1 -Local
```

Build from source (Linux / macOS):

```bash
cargo build --release
# binary: target/release/governor
```

---

## Commands

| Command | Description |
|---|---|
| `governor init [--template <name>] [--force]` | Scaffold `STATE.md` + `IMPLEMENTATION.md` in the current directory from a template (see [Templates](#templates)). Existing files are never overwritten without `--force`. Omitting `--template` uses `default`. |
| `governor start [project]` | Discover the project, assemble AI context, copy it to the clipboard, and hold a session lock. Without a name, opens the interactive TUI selector. |
| `governor close [project]` | Validate and close the active session; copies a verification context to the clipboard. |
| `governor health [project] [--verbose]` | Run workspace health checks (clipboard, filesystem, contract integrity, locks). **Exits 1 if any check fails (❌), 0 on pass/warn only** — safe to use in CI and scripts. |
| `governor relay [--preset minimal\|standard\|full] [--hud]` | Run the relay daemon: watches the clipboard for `GOV-CMD` markers and executes them, optionally with an always-on-top HUD status bar. |
| `governor projects [--verbose]` | List discovered projects with lock status and metadata. |
| `governor history [project] [--limit N]` | Print the project's session history as a table: date, start time, duration, final status (Success / Warning / Fail), contract hash. Shows the last 10 sessions by default; kept in `.governor/history.jsonl`. |
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

## Templates

`governor init` scaffolds `STATE.md` + `IMPLEMENTATION.md` from a template:

```bash
governor init                # default template (backward compatible)
governor init -t minimal     # short form
governor init --template rust
```

### Built-in templates

| Name | Scope |
|---|---|
| `default` | Balanced starter: task, progress, context, notes + full contract. |
| `minimal` | Bare minimum: current task and progress checklist only. |
| `rust` | Rust projects: build/clippy/test checks, Cargo layout, `src/` structure. |
| `frontend` | Frontend projects: npm scripts, components, styling. |

### Custom templates

Any name not built-in is looked up in `~/.governor/templates/<name>/`:

```
~/.governor/templates/
└── team/
    ├── STATE.md            # required
    └── IMPLEMENTATION.md   # required
```

A custom template is valid when the directory contains **both** files; they
become the scaffold content verbatim. Names may contain ASCII letters,
digits, `-` and `_`. The templates root can be relocated with the
`GOVERNOR_TEMPLATES_DIR` environment variable.

An unknown template name fails with the list of everything actually
available (built-ins plus valid custom templates):

```
❌ Error: unknown template 'blog'.
Available templates:
  - default    Balanced starter: task, progress, context, notes + full contract
  - minimal    Bare minimum: current task and progress checklist only
  - rust       Rust projects: build/clippy/test checks, Cargo layout, src/ structure
  - frontend   Frontend projects: npm scripts, components, styling
  - team       (custom, from C:\Users\me\.governor\templates\team)
```

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

Every `start`/`close` pair is also appended to the project's session history
log (`.governor/history.jsonl`), so sessions outlive the lock file:

```bash
governor history             # last 10 sessions
governor history --limit 25  # more of the past
```

The log is append-only JSON Lines — one event per line, written without
rewriting the file. A project with no history prints
`No session history found for this project` instead of failing.

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
