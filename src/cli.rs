//! CLI argument parsing via clap derive.
//!
//! Defines the top-level command structure:
//! ```text
//! governor init               — scaffold STATE.md + IMPLEMENTATION.md
//! governor start [project]   — assemble AI context → clipboard
//! governor close             — validate + close session
//! governor health            — workspace integrity check
//! governor relay             — background relay daemon
//! governor projects          — list discovered projects
//! governor history           — session history table
//! ```

use clap::{Parser, Subcommand};

/// Governor v2 — AI-assisted development governance.
#[derive(Parser, Debug)]
#[command(
    name = "governor",
    version = env!("CARGO_PKG_VERSION"),
    about = "AI-assisted development governance system",
    long_about = "Manages session lifecycle, context assembly, and integrity validation for AI-assisted coding workflows."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start a new session: discover project, assemble AI context → clipboard.
    ///
    /// Without arguments, opens an interactive TUI project selector.
    /// With a project name, starts that project directly.
    Start {
        /// Project name (optional — opens TUI selector if omitted).
        project: Option<String>,
    },

    /// Validate and close the current session.
    Close {
        /// Project name (auto-detected from lock if omitted).
        project: Option<String>,
    },

    /// Run health checks on the workspace and active projects.
    Health {
        /// Project name (checks all projects if omitted).
        project: Option<String>,
        /// Run extended diagnostics.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Start the relay daemon (background clipboard-based command execution).
    Relay {
        /// Preset: minimal, standard, or full. Omit to use config file preset.
        #[arg(short, long)]
        preset: Option<String>,
        /// Pin a status bar to the terminal: clipboard size, last command,
        /// uptime, command/error counters, truncation warning.
        ///
        /// Needs a real terminal on stderr; when redirected, the daemon falls
        /// back to plain log lines.
        #[arg(long, default_value = "false")]
        hud: bool,
    },

    /// Initialise a new Governor project in the current directory.
    ///
    /// Creates STATE.md and IMPLEMENTATION.md templates.
    /// Existing files are not overwritten unless --force is passed.
    Init {
        /// Overwrite existing STATE.md / IMPLEMENTATION.md.
        #[arg(short, long, default_value = "false")]
        force: bool,
        /// Template for STATE.md / IMPLEMENTATION.md: a built-in name
        /// (default, minimal, rust, frontend) or a custom template from
        /// ~/.governor/templates/<name>/. Omit for the default template.
        #[arg(short, long)]
        template: Option<String>,
    },

    /// Show or validate configuration.
    Config {
        /// Show the effective configuration as a TOML template.
        #[arg(long)]
        show: bool,
        /// Validate the configuration file (report errors). If PATH is omitted, validates the default config file.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        validate: Option<String>,
        /// With --validate: also check that every workspace root exists and
        /// is readable. Missing roots are reported as warnings and do not
        /// fail validation.
        #[arg(long, requires = "validate")]
        check_root: bool,
    },

    /// Generate shell completion script.
    ///
    /// Prints the completion script to stdout; redirect to a file to install.
    Completions {
        /// Target shell.
        #[arg(value_parser = ["bash", "zsh", "fish", "powershell", "elvish"])]
        shell: String,
    },

    /// List discovered projects with lock status and metadata.
    Projects {
        /// Show detailed info including lock status and last activity.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show the project's session history.
    ///
    /// Prints the most recent sessions (date, start time, duration, final
    /// status, contract hash) from the project's session history log.
    History {
        /// Project name (current directory by default).
        project: Option<String>,
        /// Number of recent sessions to show.
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn check_root_parses_alongside_validate() {
        let cli = Cli::try_parse_from(["governor", "config", "--validate", "--check-root"])
            .expect("flags must parse");
        match cli.command {
            Command::Config { show, validate, check_root } => {
                assert!(!show);
                assert_eq!(validate.as_deref(), Some(""), "bare --validate means the default file");
                assert!(check_root);
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn check_root_with_explicit_path() {
        let cli = Cli::try_parse_from([
            "governor", "config", "--validate", "C:/tmp/gov.toml", "--check-root",
        ])
        .expect("flags must parse");
        match cli.command {
            Command::Config { validate, check_root, .. } => {
                assert_eq!(validate.as_deref(), Some("C:/tmp/gov.toml"));
                assert!(check_root);
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn check_root_defaults_to_off() {
        let cli = Cli::try_parse_from(["governor", "config", "--validate"]).expect("must parse");
        match cli.command {
            Command::Config { check_root, .. } => assert!(!check_root),
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn check_root_without_validate_is_a_usage_error() {
        // `requires = "validate"` should reject --show --check-root up front,
        // so cmd_config never sees a check_root without something to check.
        let err = Cli::try_parse_from(["governor", "config", "--show", "--check-root"])
            .expect_err("must be rejected");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn init_defaults_to_no_template_and_no_force() {
        let cli = Cli::try_parse_from(["governor", "init"]).expect("must parse");
        match cli.command {
            Command::Init { force, template } => {
                assert!(!force);
                assert_eq!(template, None, "no flag → default template (backward compat)");
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn init_accepts_template_short_and_long() {
        for argv in [
            vec!["governor", "init", "-t", "rust"],
            vec!["governor", "init", "--template", "rust"],
        ] {
            let cli = Cli::try_parse_from(argv).expect("must parse");
            match cli.command {
                Command::Init { force, template } => {
                    assert!(!force);
                    assert_eq!(template.as_deref(), Some("rust"));
                }
                other => panic!("expected Init, got {other:?}"),
            }
        }
    }

    #[test]
    fn init_combines_force_and_template() {
        let cli = Cli::try_parse_from(["governor", "init", "--force", "-t", "minimal"])
            .expect("must parse");
        match cli.command {
            Command::Init { force, template } => {
                assert!(force);
                assert_eq!(template.as_deref(), Some("minimal"));
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn history_defaults_to_limit_ten() {
        let cli = Cli::try_parse_from(["governor", "history"]).expect("must parse");
        match cli.command {
            Command::History { project, limit } => {
                assert_eq!(project, None);
                assert_eq!(limit, 10, "default limit is the last 10 sessions");
            }
            other => panic!("expected History, got {other:?}"),
        }
    }

    #[test]
    fn history_accepts_project_and_custom_limit() {
        for argv in [
            vec!["governor", "history", "my-project", "--limit", "5"],
            vec!["governor", "history", "my-project", "-l", "5"],
        ] {
            let cli = Cli::try_parse_from(argv).expect("must parse");
            match cli.command {
                Command::History { project, limit } => {
                    assert_eq!(project.as_deref(), Some("my-project"));
                    assert_eq!(limit, 5);
                }
                other => panic!("expected History, got {other:?}"),
            }
        }
    }
}