# Governor v2 — Roadmap

## v2.0 — Core (Current)

- [x] Single Rust binary with clap CLI
- [x] Project discovery (STATE.md markers)
- [x] Session lifecycle (start → close)
- [x] SHA-256 contract integrity
- [x] AI context assembly
- [x] Cross-platform clipboard
- [x] Lock manager with PID validation
- [x] TUI project selector (ratatui)
- [x] Health checks / doctor
- [x] Relay daemon (optional background mode)

## v2.1 — Polish

- [ ] Config file support (~/.governor/config.toml)
- [ ] Custom workspace roots configuration
- [ ] Template system for STATE.md / IMPLEMENTATION.md scaffolding
- [ ] Session history log
- [ ] Shell completions (bash, zsh, fish, powershell)

## v2.2 — Collaboration

- [ ] Multi-project workspaces
- [ ] Cross-project context references
- [ ] Team lock awareness (networked lock files)
- [ ] Session handoff between team members

## v3.0 — Intelligence

- [ ] AI-assisted context diffing (what changed since last session?)
- [ ] Automatic STATE.md updates from git history
- [ ] Smart project suggestions based on activity
- [ ] Integrated relay with structured command parsing

## Non-Goals (for now)

- ❌ GUI application — TUI is sufficient
- ❌ IDE plugin — clipboard transport is IDE-agnostic
- ❌ Cloud sync — local-first by design
- ❌ Python/Node.js integration — pure Rust