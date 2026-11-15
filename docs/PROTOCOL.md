# Governor v2 Protocol

The minimal governance protocol for AI-assisted development.

## Core Concepts

| Concept | File | Purpose |
|---------|------|---------|
| **Operational Memory** | `STATE.md` | Current task state, context, progress |
| **Implementation Contract** | `IMPLEMENTATION.md` | What must be built and how |
| **Decisions Log** | `DECISIONS.md` | Archived architectural decisions |
| **Roadmap** | `ROADMAP.md` | Future direction and milestones |

## Session Lifecycle

```
governor start  →  lock → sign contract → assemble context → clipboard
       ↓
   [AI work session]
       ↓
governor close → validate contract → close context → clipboard → unlock
```

## Role Routing

The context header tells the AI which role to assume:

- **Engineer** — implementation tasks: write code, fix bugs, build features
- **Architect** — review tasks: verify correctness, approve changes, plan next steps

## Contract Integrity

Every `IMPLEMENTATION.md` carries a SHA-256 hash marker:

```markdown
<!-- GOVERNOR-HASH:<hex> -->
```

- `governor start` — embeds the hash before the session
- `governor close` — validates the hash against current content
- Mismatch → warning in the verification context

## Lock Mechanism

- One active session per project
- Lock file: `.governor/session.lock` (JSON)
- Records PID, hostname, start time
- Stale locks (dead PID) are auto-cleaned on next acquire

## Clipboard Transport

- Context assembled by Governor → system clipboard
- User pastes into AI chat
- AI produces code/commands → user executes locally
- Relay daemon option: autonomous clipboard-based command execution