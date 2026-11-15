# STATE — Operational Memory

## Current Task
Настроить CI (GitHub Actions, `.github/workflows/ci.yml`: clippy -D warnings + cargo test на windows-latest), затем — Packaging (install.ps1 / .msi).

## Just Done
- [x] Exit-code фикс: `governor health` → Fail = exit 1, Warn/Pass = exit 0 (`has_failures` + `process::exit(1)` в `cmd_health`); unit- и integration-тесты на контракт
- [x] README.md добавлен (+ бейдж CI)
- [x] CI: `.github/workflows/ci.yml` — windows-latest, dtolnay/rust-toolchain@1.98.1 (пин = локальный тулчейн, rust-toolchain.toml не добавлен), clippy -D warnings + cargo test + release build; fmt-гейт не добавлен (rustfmt не установлен в кастомном тулчейне — осмысленный diff проверить негде)

## Progress
- [x] Ядро v2.0: 10 модулей на Rust (cli, project, lock, integrity, context, clipboard, session, tui, health, relay)
- [x] Шесть CLI-команд: init, start, close, health, relay, projects
- [x] Кроссплатформенный clipboard через arboard (без PowerShell)
- [x] SHA-256 контрактная валидация (embed/extract/validate/sign)
- [x] Lock-менеджмент с PID-валидацией и автоочисткой stale-локов
- [x] TUI-селектор проектов на ratatui (j/k-навигация, детали, Enter-выбор)
- [x] Relay-демон с base64-командами через clipboard
- [x] Health-check: clipboard, FS, контракты, lock-статус
- [x] Project discovery по STATE.md в workspace-roots
- [x] 50 тестов (37 unit + 13 integration) — все проходят
- [x] Релизный бинарник: 1081 KB, сборка за 20 секунд
- [x] Стриппинг бинарника: opt-level="z", lto=true, codegen-units=1, panic="abort", strip=true → 537 KB (−50.3%), 42/42 тестов
- [x] Конфиг-файл (~/.governor/config.toml) — модуль config.rs, проект использует workspace_roots из конфига или defaults
- [x] Shell-автодополнения (bash, zsh, fish, powershell, elvish) — команда `governor completions <shell>` реализована и работает (234 KB added)
- [x] Бинарный размер: 771 KB (< 1 MB, бюджет увеличен для v2.1)
- [x] Health‑check improvements: clipboard throughput, FS saturation, clock skew, relay sanity test — реализованы, gated behind `--verbose`, unit‑тесты добавлены
- [x] Project‑discovery parallelism (5.3): `src/pool.rs` — пул потоков на std (без новых зависимостей, ≤8 воркеров), roots сканируются конкурентно; вывод побайтово совпадает с последовательным сканированием
- [x] 158 тестов (133 unit + 25 integration) — проходят; релизный бинарник 851 KB (< 1 MB)
- [x] Health exit-code контракт: `has_failures()` (Fail → 1, Warn/Pass → 0) + интеграционные тесты на tampered/unsigned контракт через реальный бинарник
- [x] README.md: описание, установка, команды, архитектура

## Context
- Проект: Governor v2.0 — AI-ассистированное управление разработкой
- Стек: Rust 1.98, без PowerShell/Python/Batch — один бинарник
- Архитектура: 12 файлов src, 1 бинарная цель
- Ключевые решения: docs/DECISIONS.md (8 ADR, ADR-008 = size-first release profile), протокол: PROTOCOL.md
- Принцип: «одна программа = один бинарник»
- Буфер обмена: arboard (нативный, без внешних процессов)
- Сборка: `cargo build --release` из `C:\work\governor-v2`

## Notes
- Тулиент: `C:\work\.toolchains\rust\bin` (rustc 1.98.1)
- ВНИМАНИЕ: `RUSTUP_HOME` и `CARGO_HOME` не унаследуются в новой сессии — сборка падает
  с «could not create home directory: ~/.rustup». Перед сборкой экспортировать:
  `$env:RUSTUP_HOME="C:\work\.toolchains\rustup"; $env:CARGO_HOME="C:\work\.toolchains\rust"`
- Для сборки нужен `danger-full-access` (сеть для crates.io)
- Цель < 700 KB после LTO — ДОСТИГНУТО: 550 400 байт (537.5 KB), было 1 107 456 (1081.5 KB). Бюджет v2.1 повышен до 1 MB (771 KB достигнуто).
- Сборка release с fat LTO + codegen-units=1: 42 с (было 20 с) — приемлемо для релиза
- `panic="abort"` не ломает тесты: cargo форсирует unwind для test-harness,
  `cargo test` и `cargo test --release` оба дают 30/30
- Проверено на optimized-бинарнике: --version, --help, projects (16), health,
  health --verbose (36 passed / 10 warnings / 0 failures), SHA-256-валидация контрактов
- Конфиг: workspace-roots, preset по умолчанию, clipboard-бэкенд; интеграция с проектом: `discover_projects` использует `Config::load().workspace_roots`
- Автодополнения: clap генерирует через `clap_complete`
- Project discovery (5.3): `discover_projects()` делегирует в `discover_in_roots*` → `pool::parallel_map`
  (по одному заданию на root, индексы через атомарный курсор, порядок результатов = порядок roots)
- Известная нестабильность clipboard-тестов в процессе (`cargo test --lib health::`): arboard на Windows
  падает с `STATUS_HEAP_CORRUPTION` (0xc0000374) примерно в 1 случае из 2 — это in-process arboard,
  на бинарнике и в integration-тестах (отдельные процессы) не воспроизводится
- Гонка acquire/wait найдена на CI (`lock::tests::acquire_with_timeout_succeeds_when_lock_released`):
  удаление lock-файла между `exists()` и `read_to_string` отдавало `IoError(NotFound)` и валило ожидание.
  Исправлено: `remove_if_exists` (NotFound = лок свободен), тайминговые границы теста ослаблены для нагруженных раннеров