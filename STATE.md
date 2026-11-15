# STATE — Operational Memory

## Current Task
Release v2.1.0: bump версии (Cargo.toml/README/Cargo.lock), коммит `chore: bump version to 2.1.0, update contract`, тег `v2.1.0` + push, проверка GitHub Release от CI. v2.1 Polish (template system + session history log) полностью завершён.

## Just Done
- [x] Dogfooding (v2.1): контракт IMPLEMENTATION.md обновлён руками (`1 MB` → `1.5 MB` в 3 местах, старый хэш-маркер удалён); `governor close` переподписал контракт → статус **Warning** («подписан при закрытии»); затем полный цикл `start`+`close` → **Success** с новым хэшем контракта (b99a5f80…); `governor history` показывает обе записи (Warning + Success). Нюанс: повторная подпись подписанного файла даёт хэш контента без хвостового `\n` — контракт самосогласован, validation OK
- [x] Session history log (v2.1): модуль `src/history.rs` — append-only JSONL `.governor/history.jsonl`; `governor start` пишет событие start (ts, PID, host, SHA-256 контракта), `governor close` — close со статусом Success (контракт валиден/отсутствует) / Warning (подписан при закрытии) / Fail (hash mismatch); `session::close_session` возвращает `CloseOutcome{ContractCheck}`; новая команда `governor history [project] [--limit N]` (default 10) — таблица Дата/Start/Длительность/Статус/Hash (сокращён до 8 hex); открытая сессия → Active, orphan close сохраняется; нет истории → "No session history found for this project", битая строка → ошибка с путём и номером строки; chrono + feature "serde"; 14 unit-тестов history + 2 CLI-теста (163 unit + 25 integration = 188); README: таблица команд + Session lifecycle; бинарник 1068 KB (< 1.5 MB); запись истории best-effort (ошибка не валит сессию)
- [x] Бюджет бинарника повышен до 1.5 MB: ADR-008 (docs/DECISIONS.md) — секция Size budget с историей повышений (700 KB → 1 MB → 1.5 MB), README `< 1.5 MB`; коммит `docs: raise binary size budget to 1.5 MB`
- [x] Template system (v2.1): `governor init --template <name>` / `-t`; 4 built-in (default/minimal/rust/frontend); custom из `~/.governor/templates/<name>/{STATE.md,IMPLEMENTATION.md}`; unknown → ошибка со списком доступных (built-in + валидные custom); имя шаблона — только [A-Za-z0-9_-] (path traversal отсечён); env-хук `GOVERNOR_TEMPLATES_DIR` для тестов/перенос корня; без флага — default (обратная совместимость, старые `init_project`/`init_in_dir` сохранены); 14 новых unit + 3 CLI-теста; README: секция Templates; бинарник 1017 KB (+8.7 KB от шаблонов)
- [x] MVP released: тег `v2.0.0` → GitHub Release с `governor-x86_64-pc-windows-msvc.zip` + `.sha256` (прогон 43721042465 — все 3 джобы зелёные); E2E-тест one-liner `iwr -useb .../install.ps1 | iex` на чистой машине (Windows 11, PowerShell) пройден: SHA-256 verified, PATH обновлён, в новом терминале `governor --version` → 2.0.0 и `--help` показывает все команды
- [x] CI зелёный: https://github.com/HDesmont/governor-v2/actions — `clippy + test (windows)` ✅ и stretch-джоба `test (ubuntu)` ✅ (прогон 43717616662, b0f41f8)
- [x] Фикс гонки lock (упал на CI): удаление lock-файла между `exists()` и чтением отдавало `IoError(NotFound)` и валило `acquire_with_timeout` → `remove_if_exists` трактует NotFound как «лок свободен»
- [x] Портабельные config-фикстуры (упали на ubuntu): `env::split_paths` на Unix режет пути по `:` → Windows-пути `C:/x` в env-тестах заменены на бесколонные
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
- [x] CI (GitHub Actions): push/PR в main, windows-latest (clippy -D warnings, cargo test, release build) + ubuntu-latest stretch (cargo test) — обе джобы зелёные; бейдж в README
- [x] Packaging: `install.ps1` — установка в `%LOCALAPPDATA%\Governor\bin`, идемпотентный user-PATH (raw-чтение, REG_EXPAND_SZ), `-Uninstall`, `-Local`, `-Version`; CI-джоба `release` на тегах `v*` публикует zip + `.sha256`; README: Windows installation one-liner
- [x] Template system: `init --template/-t` — 4 built-in + custom из `~/.governor/templates/<name>/`; понятная ошибка unknown со списком; 172 теста (147 unit + 25 integration); бинарник 1017 KB (< 1 MB, запас 7 KB)
- [x] Session history log + команда `governor history` (v2.1): JSONL `.governor/history.jsonl`, статусы Success/Warning/Fail из валидации контракта при close; 188 тестов; бинарник 1068 KB (< 1.5 MB)

## Context
- Проект: Governor v2.0 — AI-ассистированное управление разработкой
- Стек: Rust 1.98, без PowerShell/Python/Batch — один бинарник
- Архитектура: 12 файлов src, 1 бинарная цель
- Ключевые решения: docs/DECISIONS.md (8 ADR, ADR-008 = size-first release profile), протокол: PROTOCOL.md
- Принцип: «одна программа = один бинарник»
- Буфер обмена: arboard (нативный, без внешних процессов)
- Сборка: `cargo build --release` из `C:\work\governor-v2`

## Notes
- install.ps1: user-PATH читается RAW из HKCU\Environment (DoNotExpandEnvironmentNames), иначе `%VAR%`-записи раскрылись бы и перезаписались литералами; запись через `[Environment]::SetEnvironmentVariable` (broadcast WM_SETTINGCHANGE) + восстановление kind=REG_EXPAND_SZ (она пишет REG_SZ). Сравнение записей — case-insensitive + TrimEnd('\'). Piped-режим (`iwr | iex`) определяется по пустому `$PSCommandPath`: ошибки делают `return`, а не `exit`, чтобы не убивать шелл пользователя
- E2E-тест install.ps1 на реальной машине (5.1): `-Local` install → PATH +1 запись, kind сохранён; повторный запуск → «nothing to change», дублей нет; свежий процесс → `governor --version` ок; `-Version`/latest без релизов → понятный 404 и exit 1; `-Uninstall` → PATH побайтно восстановлен, директории удалены. Нюанс: при записи сбрасываются пустые записи (в т.ч. хвостовой `;`) — безвредно
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
- Бинарник после template system: 1 041 408 байт (1017.0 KB) — на HEAD (без правок) было 1 032 704 (1008.5 KB);
  цифра 851 KB в старых записях устарела. Запас до бюджета 1 MB всего 7 KB — следующая фича может его пробить
- Template system: корень кастомных — `dirs::home_dir()/.governor/templates`, переопределяется env `GOVERNOR_TEMPLATES_DIR`
  (тот же паттерн, что `GOVERNOR_CONFIG` в config.rs; в тестах — мьютекс-фикстура, env снимается в Drop);
  имена кастомных валидируются `[A-Za-z0-9_-]` до любого обращения к FS; в списке available кастомные
  показываются только если директория содержит ОБА файла (для вывода ошибки шаблон не читается, только is_file-проверки)
- Session history: append-only JSONL (события start/close, сопоставление LIFO — close закрывает последнюю
  открытую сессию); `governor start` БЕЗ аргумента всегда открывает TUI (для скриптов — `start <имя>`);
  ts — RFC 3339 UTC (chrono serde), таблица рендерится в локальной зоне; duration < 0 клампится в "0s";
  контрактный хэш в истории — `integrity::hash_file(IMPLEMENTATION.md)` на момент события, в таблице
  сокращается до 8 hex; запись истории в main.rs — best-effort (⚠️ в stderr, сессия не падает);
  запись в cargo test-фикстурах через Set-Content — указывать `-Encoding utf8` (default pwsh тут UTF-16)