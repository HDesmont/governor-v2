//! Project initialisation — scaffold STATE.md and IMPLEMENTATION.md.
//!
//! `governor init` creates template files in the current directory,
//! turning it into a Governor-managed project. Existing files are
//! never overwritten without explicit confirmation.

use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum InitError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("current directory is not accessible")]
    NoCurrentDir,
}

/// Outcome of a single file initialisation step.
#[derive(Debug, PartialEq)]
pub enum FileOutcome {
    /// File was created from the template.
    Created,
    /// File already exists — skipped to avoid overwrite.
    Skipped,
}

/// Summary of an init run.
#[derive(Debug)]
pub struct InitResult {
    pub state: FileOutcome,
    pub implementation: FileOutcome,
}

/// Template for STATE.md — operational memory.
pub const STATE_TEMPLATE: &str = r#"# STATE — Operational Memory

## Current Task
<!-- Describe the active task: what is being built / fixed / changed. -->

## Progress
- [ ] Task not yet started

## Context
<!-- Relevant background, constraints, links to decisions. -->

## Notes
<!-- Free-form scratchpad for the current session. -->
"#;

/// Template for IMPLEMENTATION.md — active contract.
pub const IMPLEMENTATION_TEMPLATE: &str = r#"# Implementation Contract

## Goal
<!-- One sentence: what must be achieved. -->

## Acceptance Criteria
- [ ] Criterion 1
- [ ] Criterion 2

## Constraints
<!-- Non-functional requirements, tech stack, deadlines. -->

## Approach
<!-- High-level plan: architecture, key decisions, risks. -->
"#;

/// Run `governor init` in the current directory.
///
/// Creates STATE.md and IMPLEMENTATION.md if they don't already exist.
/// Existing files are left untouched.
pub fn init_project(force: bool) -> Result<InitResult, InitError> {
    let cwd = std::env::current_dir().map_err(|_| InitError::NoCurrentDir)?;
    Ok(init_in_dir(&cwd, force))
}

/// Run initialisation in a specific directory (usable from tests).
pub fn init_in_dir(dir: &Path, force: bool) -> InitResult {
    let state_path = dir.join("STATE.md");
    let impl_path = dir.join("IMPLEMENTATION.md");

    let state = create_if_missing(&state_path, STATE_TEMPLATE, force);
    let implementation = create_if_missing(&impl_path, IMPLEMENTATION_TEMPLATE, force);

    InitResult {
        state,
        implementation,
    }
}

/// Print the init result in a human-friendly format.
pub fn print_result(result: &InitResult) {
    println!();
    println!("  Governor v{} — Project Initialised", env!("CARGO_PKG_VERSION"));
    println!();

    print_file_status("STATE.md", &result.state);
    print_file_status("IMPLEMENTATION.md", &result.implementation);

    println!();
    println!("  Next: review the templates, then run `governor start`.");
    println!();
}

fn print_file_status(name: &str, outcome: &FileOutcome) {
    match outcome {
        FileOutcome::Created => println!("  ✅ Created {}", name),
        FileOutcome::Skipped => println!("  ⏭️  Skipped {} (already exists)", name),
    }
}

/// Create a file with the given content if it doesn't exist.
///
/// If `force` is true, an existing file is overwritten.
fn create_if_missing(path: &Path, content: &str, force: bool) -> FileOutcome {
    if path.exists() && !force {
        return FileOutcome::Skipped;
    }
    // Best-effort: don't fail the whole init if one file fails.
    match std::fs::write(path, content) {
        Ok(()) => FileOutcome::Created,
        Err(_) => FileOutcome::Skipped, // Treat write failure as skip.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_templates_non_empty() {
        assert!(!STATE_TEMPLATE.is_empty());
        assert!(!IMPLEMENTATION_TEMPLATE.is_empty());
        assert!(STATE_TEMPLATE.contains("STATE"));
        assert!(IMPLEMENTATION_TEMPLATE.contains("Contract"));
    }

    #[test]
    fn test_create_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let result = init_in_dir(dir.path(), false);

        assert_eq!(result.state, FileOutcome::Created);
        assert_eq!(result.implementation, FileOutcome::Created);

        let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
        let impl_ = std::fs::read_to_string(dir.path().join("IMPLEMENTATION.md")).unwrap();

        assert!(state.contains("STATE"));
        assert!(impl_.contains("Contract"));
    }

    #[test]
    fn test_skip_existing_files() {
        let dir = tempfile::tempdir().unwrap();

        // Pre-create STATE.md with custom content.
        std::fs::write(dir.path().join("STATE.md"), "custom state").unwrap();

        let result = init_in_dir(dir.path(), false);

        assert_eq!(result.state, FileOutcome::Skipped);
        assert_eq!(result.implementation, FileOutcome::Created);

        // Content should be preserved.
        let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
        assert_eq!(state, "custom state");
    }

    #[test]
    fn test_force_overwrite() {
        let dir = tempfile::tempdir().unwrap();

        // Pre-create STATE.md with custom content.
        std::fs::write(dir.path().join("STATE.md"), "custom state").unwrap();

        let result = init_in_dir(dir.path(), true);

        assert_eq!(result.state, FileOutcome::Created);
        assert_eq!(result.implementation, FileOutcome::Created);

        // Content should be overwritten with template.
        let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
        assert!(state.contains("STATE"));
        assert!(!state.contains("custom state"));
    }

    #[test]
    fn test_init_result_debug() {
        let result = InitResult {
            state: FileOutcome::Created,
            implementation: FileOutcome::Skipped,
        };
        // Just verify the struct is constructable.
        assert_eq!(result.state, FileOutcome::Created);
        assert_eq!(result.implementation, FileOutcome::Skipped);
    }
}