//! Governor v2.0 — AI-assisted development governance.
//!
//! Single-binary governance system for AI-assisted coding workflows.
//! Manages session lifecycle, context assembly, contract integrity,
//! and clipboard-based human-as-transport protocol.
//!
//! ## Quick start
//! ```text
//! governor init               # Scaffold STATE.md + IMPLEMENTATION.md
//! governor start              # TUI project selector → AI context in clipboard
//! governor start my-project   # Direct start for named project
//! governor close              # Validate + close session
//! governor health             # Workspace integrity check
//! governor relay              # Background relay daemon
//! governor projects           # List discovered projects
//! governor history            # Session history table
//! ```

use clap::{Parser, CommandFactory};
use governor::cli;
use governor::{config, health, history, init, integrity, lock, project, relay, session, tui};
use std::path::Path;
use std::process;
use clap_complete::{Shell, generate};

fn main() {
    let cli = cli::Cli::parse();

    let result = match cli.command {
        cli::Command::Start { project } => cmd_start(project.as_deref()),
        cli::Command::Close { project } => cmd_close(project.as_deref()),
        cli::Command::Health { project, verbose } => cmd_health(project.as_deref(), verbose),
        cli::Command::Relay { preset, hud } => cmd_relay(preset.as_deref(), hud),
        cli::Command::Init { force, template } => cmd_init(force, template.as_deref()),
        cli::Command::Config { show, validate, check_root } => cmd_config(show, validate.as_deref(), check_root),
        cli::Command::Completions { shell } => cmd_completions(&shell),
        cli::Command::Projects { verbose } => cmd_projects(verbose),
        cli::Command::History { project, limit } => cmd_history(project.as_deref(), limit),
    };

    if let Err(e) = result {
        eprintln!("❌ Error: {}", e);
        process::exit(1);
    }
}

/// `governor start [project]`
///
/// Without a project name, opens an interactive TUI project selector.
fn cmd_start(project_name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let project = match project_name {
        // Direct start for a named project — no TUI.
        Some(name) => project::resolve_project(Some(name))?,
        // Interactive TUI selection.
        None => match tui::select_project() {
            Ok(p) => p,
            Err(tui::TuiError::Cancelled) => {
                eprintln!("👋 Project selection cancelled. No session started.");
                return Ok(());
            }
            Err(tui::TuiError::Project(project::ProjectError::NoProjectsFound)) => {
                eprintln!("❌ No Governor projects found.");
                eprintln!();
                eprintln!("   A project is any directory containing a STATE.md file.");
                eprintln!("   To fix this, you can:");
                eprintln!("     1. Run `governor init` inside a project directory;");
                eprintln!("     2. Add workspace roots to the governor config");
                eprintln!("        (`governor config --show` to inspect it).");
                process::exit(1);
            }
            Err(e) => return Err(e.into()),
        },
    };

    eprintln!("🚀 Starting session for: {}", project.name);
    eprintln!("   Project root: {}", project.root.display());

    if project.is_locked() {
        eprintln!("⚠️  Project is already locked. Attempting to acquire lock anyway...");
    }

    // Load config to get lock timeout
    let config = config::Config::load().unwrap_or_default();
    let lock_timeout = config.relay_lock_timeout_seconds.unwrap_or(config::DEFAULT_LOCK_TIMEOUT_SECONDS);

    let (result, _lock) = session::start_session(&project, lock_timeout)?;

    // ── Session history (best-effort: never breaks the session) ──
    let contract_hash = project
        .implementation_file
        .as_ref()
        .and_then(|path| integrity::hash_file(path).ok());
    if let Err(e) = history::record_start(&project.root, contract_hash.as_deref()) {
        eprintln!("⚠️  Could not record session history: {}", e);
    }

    eprintln!("✅ Session started");
    eprintln!("   Context: {} bytes", result.context_size);
    eprintln!("   Hash:    {}", result.context_hash);
    if result.context_reused {
        eprintln!("   Cache:   ♻️  reused (project state unchanged)");
    } else if let Some(path) = &result.cache_path {
        eprintln!("   Cache:   assembled → {}", path.display());
    }
    eprintln!("📋 AI context copied to clipboard — paste into your AI chat.");
    eprintln!();
    eprintln!("⚠️  Keep this process running until you close the session.");
    eprintln!("   Run `governor close` when done.");

    // Hold the lock until user presses Enter (or Ctrl+C).
    eprintln!();
    eprintln!("Press Enter to keep session active (Ctrl+C to abort)...");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();

    // Lock stays acquired — user must call `governor close`.
    std::mem::forget(_lock);

    Ok(())
}

/// `governor close [project]`
fn cmd_close(project_name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let project = project::resolve_project(project_name)?;

    eprintln!("🔒 Closing session for: {}", project.name);

    if !project.is_locked() {
        eprintln!("⚠️  No active lock found. Proceeding with close anyway.");
    }

    let lock_path = project.lock_path();
    let mut lock = lock::SessionLock::new(lock_path);
    
    // Load config to get lock timeout (same as start)
    let config = config::Config::load().unwrap_or_default();
    let lock_timeout = config.relay_lock_timeout_seconds.unwrap_or(config::DEFAULT_LOCK_TIMEOUT_SECONDS);
    
    // Try to acquire lock with timeout, but ignore errors (close can proceed without lock)
    lock.acquire_with_timeout(lock_timeout).ok();

    let outcome = session::close_session(&project, &mut lock)?;

    // ── Session history (best-effort: never breaks the session) ──
    let status = match outcome.contract_check {
        session::ContractCheck::Validated => history::SessionStatus::Success,
        session::ContractCheck::SignedNow => history::SessionStatus::Warning,
        session::ContractCheck::Failed => history::SessionStatus::Fail,
    };
    if let Err(e) = history::record_close(&project.root, status) {
        eprintln!("⚠️  Could not record session history: {}", e);
    }

    eprintln!("✅ Session closed");
    eprintln!("📋 Verification context copied to clipboard.");

    Ok(())
}

/// `governor history [project] [--limit N]`
fn cmd_history(project_name: Option<&str>, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let project = project::resolve_project(project_name)?;
    history::print_history(&project.root, limit)?;
    Ok(())
}

/// `governor health [project]`
///
/// Exits with code 1 when at least one check reports a hard failure
/// (`CheckStatus::Fail`); warnings alone still exit 0.
fn cmd_health(project_name: Option<&str>, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Named projects resolve through the same path as start/close: discovery
    // first, then the current directory when the name matches it.
    let target = match project_name {
        Some(name) => Some(project::resolve_project(Some(name))?),
        None => None,
    };

    let results = health::run_health(target.as_ref(), verbose);
    health::print_results(&results);

    if health::has_failures(&results) {
        eprintln!("❌ Health check failed (see ❌ entries above).");
        process::exit(1);
    }

    Ok(())
}

/// `governor relay`
fn cmd_relay(preset: Option<&str>, hud: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Load user configuration (falls back to defaults if missing).
    let user_config = config::Config::load().unwrap_or_default();
    
    // Determine preset name: explicit CLI argument overrides config.
    let preset_name = preset.map(|p| p.to_string()).unwrap_or(user_config.preset);
    let mut config = relay::RelayConfig::from_preset(&preset_name);
    
    // Apply relay‑specific overrides from config file.
    if let Some(poll) = user_config.relay_poll_interval_ms {
        config.poll_interval_ms = poll;
    }
    if let Some(heartbeat) = user_config.relay_heartbeat_secs {
        config.heartbeat_secs = heartbeat;
    }
    if let Some(max_bytes) = user_config.relay_max_output_bytes {
        config.max_output_bytes = max_bytes;
    }

    if hud {
        config.hud = true;
    }

    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    // The handler runs on its own thread while the daemon may be mid-redraw,
    // so it only sets the flag — the daemon logs the shutdown itself, keeping
    // the HUD status line intact.
    ctrlc::set_handler(move || {
        shutdown_clone.store(true, std::sync::atomic::Ordering::Relaxed);
    })
    .ok();

    relay::run_daemon(config, shutdown);

    Ok(())
}

/// `governor config`
fn cmd_config(show: bool, validate: Option<&str>, check_root: bool) -> Result<(), Box<dyn std::error::Error>> {
    use config::Config;
    use config::render_template;
    
    if show {
        let config = Config::load()?;
        println!("# Effective Governor configuration");
        println!("# Loaded from: {}", config.loaded_from
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "built‑in defaults".to_string()));
        println!();
        
        // Print as TOML. We could reuse render_template but need to inject actual values.
        // For simplicity, we can show the template with values commented.
        println!("{}", render_template());
        println!("# Effective values:");
        println!("# workspace.roots = {:?}", config.workspace_roots);
        println!("# session.preset = {:?}", config.preset);
        println!("# clipboard.backend = {:?}", config.clipboard_backend);
        if let Some(poll) = config.relay_poll_interval_ms {
            println!("# relay.poll_interval_ms = {}", poll);
        }
        if let Some(heartbeat) = config.relay_heartbeat_secs {
            println!("# relay.heartbeat_secs = {}", heartbeat);
        }
        if let Some(max_bytes) = config.relay_max_output_bytes {
            println!("# relay.max_output_bytes = {}", max_bytes);
        }
        if let Some(timeout) = config.relay_lock_timeout_seconds {
            println!("# relay.lock_timeout_seconds = {}", timeout);
        } else {
            println!("# relay.lock_timeout_seconds = {}", config::DEFAULT_LOCK_TIMEOUT_SECONDS);
        }
        Ok(())
    } else if let Some(path) = validate {
        let config = match path {
            "" => Config::load(),
            p => Config::load_from(Path::new(p)),
        };
        match config {
            Ok(config) => {
                println!("✅ Configuration is valid.");
                if let Some(ref path) = config.loaded_from {
                    println!("   Loaded from: {}", path.display());
                } else {
                    println!("   Using built‑in defaults (no configuration file).");
                }
                println!("   Workspace roots: {}", config.workspace_roots.len());
                println!("   Relay preset: {}", config.preset);
                if check_root {
                    println!();
                    println!("📁 Workspace root check:");
                    let checks = config.check_roots();
                    for check in &checks {
                        match check.status {
                            config::RootStatus::Readable => {
                                println!("   ✅ {}", check.describe());
                            }
                            config::RootStatus::Missing
                            | config::RootStatus::NotDirectory
                            | config::RootStatus::Unreadable(_) => {
                                println!("   ⚠️  {}", check.describe());
                            }
                        }
                    }
                    if checks.iter().all(|c| c.is_readable()) {
                        println!("   All workspace roots are readable.");
                    }
                }
                Ok(())
            }
            Err(e) => {
                eprintln!("❌ Configuration error:");
                eprintln!("   {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Neither flag: show help?
        eprintln!("Use either --show or --validate");
        std::process::exit(1);
    }
}

/// `governor completions <shell>`
fn cmd_completions(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    let shell = match shell.to_lowercase().as_str() {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        "powershell" => Shell::PowerShell,
        "elvish" => Shell::Elvish,
        _ => {
            eprintln!("❌ Unknown shell: {}", shell);
            std::process::exit(1);
        }
    };
    let mut cmd = cli::Cli::command();
    let bin_name = cmd.get_name().to_string();
    generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

/// `governor init [--template <name>]`
///
/// No `--template` means the default template (backward compatible).
fn cmd_init(force: bool, template: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let template_name = template.unwrap_or(init::DEFAULT_TEMPLATE_NAME);
    let result = init::init_project_with(force, template_name)?;
    init::print_result(&result);
    Ok(())
}

/// `governor projects`
fn cmd_projects(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    let projects = project::discover_projects()?;

    println!();
    println!("  Discovered {} project(s):", projects.len());
    println!();

    for p in &projects {
        let lock_icon = if p.is_locked() { "🔒" } else { "  " };
        let impl_icon = if p.implementation_file.is_some() {
            "📄"
        } else {
            "  "
        };
        println!("  {} {}  {}", lock_icon, impl_icon, p.name);

        if verbose {
            println!("       Root:  {}", p.root.display());
            println!("       State: {}", p.state_file.display());
            if let Some(ref imp) = p.implementation_file {
                println!("       Impl:  {}", imp.display());
            }
            println!();
        }
    }

    println!();
    Ok(())
}