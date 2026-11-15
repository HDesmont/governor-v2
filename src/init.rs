//! Project initialisation — scaffold STATE.md and IMPLEMENTATION.md.
//!
//! `governor init` creates template files in the current directory,
//! turning it into a Governor-managed project. Existing files are
//! never overwritten without explicit confirmation.
//!
//! ## Templates
//!
//! `governor init --template <name>` picks the scaffold from one of the
//! built-in templates (`default`, `minimal`, `rust`, `frontend`). When the
//! name is not built-in, Governor looks for a custom template under
//! `~/.governor/templates/<name>/` containing `STATE.md` and
//! `IMPLEMENTATION.md`. An unknown name produces an error that lists every
//! template actually available.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Name of the template used when no `--template` flag is given.
pub const DEFAULT_TEMPLATE_NAME: &str = "default";

/// Environment variable overriding the custom-template root
/// (defaults to `~/.governor/templates`).
pub const ENV_TEMPLATES_DIR: &str = "GOVERNOR_TEMPLATES_DIR";

#[derive(Error, Debug)]
pub enum InitError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("current directory is not accessible")]
    NoCurrentDir,
    #[error("unknown template '{name}'.\nAvailable templates:\n{available}")]
    UnknownTemplate { name: String, available: String },
    #[error("template '{name}' is incomplete: {detail}")]
    IncompleteTemplate { name: String, detail: String },
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
    /// Name of the template the scaffolding came from.
    pub template: String,
}

/// A scaffold template: the pair of files `governor init` writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub name: String,
    pub description: String,
    /// Content for `STATE.md`.
    pub state: String,
    /// Content for `IMPLEMENTATION.md`.
    pub implementation: String,
}

/// Template for STATE.md — operational memory (`default`).
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

/// Template for IMPLEMENTATION.md — active contract (`default`).
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

const DEFAULT_TEMPLATE_DESCRIPTION: &str =
    "Balanced starter: task, progress, context, notes + full contract";

const MINIMAL_DESCRIPTION: &str = "Bare minimum: current task and progress checklist only";

const MINIMAL_STATE: &str = r#"# STATE — Operational Memory

## Current Task
<!-- What are you working on right now? -->

## Progress
- [ ] Task not yet started
"#;

const MINIMAL_IMPL: &str = r#"# Implementation Contract

## Goal
<!-- One sentence: what must be achieved. -->

## Acceptance Criteria
- [ ] Criterion 1
"#;

const RUST_DESCRIPTION: &str = "Rust projects: build/clippy/test checks, Cargo layout, src/ structure";

const RUST_STATE: &str = r#"# STATE — Operational Memory

## Current Task
<!-- Describe the active task: feature, fix, refactor. -->

## Progress
- [ ] Task not yet started

## Build & Checks
- [ ] `cargo build` — compiles cleanly
- [ ] `cargo clippy --all-targets -- -D warnings` — no warnings
- [ ] `cargo test` — all tests pass

## Context
<!-- Crate layout, module map under src/, constraints. -->

## Notes
<!-- Scratchpad: toolchain quirks, dependency notes, gotchas. -->
"#;

const RUST_IMPL: &str = r#"# Implementation Contract

## Goal
<!-- One sentence: what must be achieved. -->

## Acceptance Criteria
- [ ] `cargo build` succeeds
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] New logic is covered by tests

## Constraints
<!-- MSRV, dependency budget, error-handling policy, performance targets. -->

## Approach
<!-- Crate/module layout under src/, key types, risks. -->
"#;

const FRONTEND_DESCRIPTION: &str = "Frontend projects: npm scripts, components, styling";

const FRONTEND_STATE: &str = r#"# STATE — Operational Memory

## Current Task
<!-- Describe the active task: component, page, refactor. -->

## Progress
- [ ] Task not yet started

## Build & Checks
- [ ] `npm install` — dependencies up to date
- [ ] `npm run build` — production build succeeds
- [ ] `npm run lint` — no lint errors

## Context
<!-- Framework, component map, styling conventions. -->

## Notes
<!-- Scratchpad: browser quirks, TODOs, design links. -->
"#;

const FRONTEND_IMPL: &str = r#"# Implementation Contract

## Goal
<!-- One sentence: what must be achieved. -->

## Acceptance Criteria
- [ ] `npm run build` succeeds
- [ ] `npm run lint` passes
- [ ] UI renders correctly at mobile and desktop breakpoints

## Constraints
<!-- Framework, browser support, accessibility requirements. -->

## Approach
<!-- Component structure, state management, styling approach. -->
"#;

/// A built-in template table entry: `(name, description, STATE.md, IMPLEMENTATION.md)`.
type BuiltinEntry = (&'static str, &'static str, &'static str, &'static str);

/// Every built-in template, in display order.
const BUILT_IN: &[BuiltinEntry] = &[
    (
        DEFAULT_TEMPLATE_NAME,
        DEFAULT_TEMPLATE_DESCRIPTION,
        STATE_TEMPLATE,
        IMPLEMENTATION_TEMPLATE,
    ),
    ("minimal", MINIMAL_DESCRIPTION, MINIMAL_STATE, MINIMAL_IMPL),
    ("rust", RUST_DESCRIPTION, RUST_STATE, RUST_IMPL),
    ("frontend", FRONTEND_DESCRIPTION, FRONTEND_STATE, FRONTEND_IMPL),
];

/// The default template, constructed directly from its constants —
/// cannot fail to resolve.
pub fn default_template() -> Template {
    Template {
        name: DEFAULT_TEMPLATE_NAME.to_string(),
        description: DEFAULT_TEMPLATE_DESCRIPTION.to_string(),
        state: STATE_TEMPLATE.to_string(),
        implementation: IMPLEMENTATION_TEMPLATE.to_string(),
    }
}

/// Names of every built-in template, in display order.
pub fn built_in_names() -> Vec<String> {
    BUILT_IN.iter().map(|(name, ..)| (*name).to_string()).collect()
}

/// Look up a built-in template by name.
fn built_in(name: &str) -> Option<Template> {
    BUILT_IN.iter().find(|(n, ..)| *n == name).map(|(name, description, state, implementation)| Template {
        name: (*name).to_string(),
        description: (*description).to_string(),
        state: (*state).to_string(),
        implementation: (*implementation).to_string(),
    })
}

/// Root directory for custom templates.
///
/// `GOVERNOR_TEMPLATES_DIR` wins when set; otherwise
/// `~/.governor/templates`. `None` when there is no home directory.
pub fn templates_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(ENV_TEMPLATES_DIR) {
        let dir = dir.trim();
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|home| home.join(".governor").join("templates"))
}

/// Template names are directory names; restrict them to a safe charset so
/// they can never escape the templates root (no `/`, `\`, `..`, unicode tricks).
fn is_valid_template_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Outcome of a custom-template lookup.
enum CustomLookup {
    Loaded(Template),
    /// The directory exists but the file pair is incomplete/unreadable.
    Incomplete(String),
    /// No such custom template — fall through to the unknown-template error.
    NotFound,
}

/// Load a custom template from `<root>/<name>/{STATE.md, IMPLEMENTATION.md}`.
fn load_custom_template(name: &str) -> CustomLookup {
    if !is_valid_template_name(name) {
        return CustomLookup::NotFound;
    }
    let Some(root) = templates_root() else {
        return CustomLookup::NotFound;
    };
    let dir = root.join(name);
    if !dir.is_dir() {
        return CustomLookup::NotFound;
    }

    let state_path = dir.join("STATE.md");
    let impl_path = dir.join("IMPLEMENTATION.md");
    if !state_path.is_file() {
        return CustomLookup::Incomplete(format!("missing STATE.md in {}", dir.display()));
    }
    if !impl_path.is_file() {
        return CustomLookup::Incomplete(format!("missing IMPLEMENTATION.md in {}", dir.display()));
    }

    let state = match std::fs::read_to_string(&state_path) {
        Ok(s) => s,
        Err(e) => {
            return CustomLookup::Incomplete(format!(
                "{} is not readable: {}",
                state_path.display(),
                e
            ))
        }
    };
    let implementation = match std::fs::read_to_string(&impl_path) {
        Ok(s) => s,
        Err(e) => {
            return CustomLookup::Incomplete(format!(
                "{} is not readable: {}",
                impl_path.display(),
                e
            ))
        }
    };

    CustomLookup::Loaded(Template {
        name: name.to_string(),
        description: format!("custom template from {}", dir.display()),
        state,
        implementation,
    })
}

/// Custom template names present in `root` — complete directories only,
/// valid names only, sorted. Used for the available-list in errors.
fn custom_template_names(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| path.join("STATE.md").is_file() && path.join("IMPLEMENTATION.md").is_file())
        .filter_map(|path| path.file_name().map(|f| f.to_string_lossy().to_string()))
        .filter(|name| is_valid_template_name(name))
        .collect();
    names.sort();
    names
}

/// Human-readable list of every template available right now:
/// built-ins first, then complete custom templates.
fn format_available() -> String {
    let mut lines: Vec<String> = BUILT_IN
        .iter()
        .map(|(name, description, ..)| format!("  - {:<10} {}", name, description))
        .collect();

    if let Some(root) = templates_root() {
        for name in custom_template_names(&root) {
            let dir = root.join(&name);
            lines.push(format!("  - {:<10} (custom, from {})", name, dir.display()));
        }
    }

    lines.join("\n")
}

/// Unknown-template error pre-filled with the list of available templates.
fn unknown_template(name: &str) -> InitError {
    InitError::UnknownTemplate {
        name: name.to_string(),
        available: format_available(),
    }
}

/// Resolve a template by name: built-ins first, then custom templates
/// from the templates root.
pub fn resolve_template(name: &str) -> Result<Template, InitError> {
    if let Some(template) = built_in(name) {
        return Ok(template);
    }
    match load_custom_template(name) {
        CustomLookup::Loaded(template) => Ok(template),
        CustomLookup::Incomplete(detail) => Err(InitError::IncompleteTemplate {
            name: name.to_string(),
            detail,
        }),
        CustomLookup::NotFound => Err(unknown_template(name)),
    }
}

/// Run `governor init` in the current directory with the default template.
///
/// Creates STATE.md and IMPLEMENTATION.md if they don't already exist.
/// Existing files are left untouched.
pub fn init_project(force: bool) -> Result<InitResult, InitError> {
    init_project_with(force, DEFAULT_TEMPLATE_NAME)
}

/// Run `governor init` in the current directory with a named template.
pub fn init_project_with(force: bool, template_name: &str) -> Result<InitResult, InitError> {
    let cwd = std::env::current_dir().map_err(|_| InitError::NoCurrentDir)?;
    init_in_dir_with(&cwd, force, template_name)
}

/// Run initialisation in a specific directory with the default template
/// (usable from tests).
pub fn init_in_dir(dir: &Path, force: bool) -> InitResult {
    scaffold(dir, force, &default_template())
}

/// Run initialisation in a specific directory with a named template.
pub fn init_in_dir_with(dir: &Path, force: bool, template_name: &str) -> Result<InitResult, InitError> {
    let template = resolve_template(template_name)?;
    Ok(scaffold(dir, force, &template))
}

/// Write both scaffold files from `template` (existing files respected).
fn scaffold(dir: &Path, force: bool, template: &Template) -> InitResult {
    let state = create_if_missing(&dir.join("STATE.md"), &template.state, force);
    let implementation = create_if_missing(&dir.join("IMPLEMENTATION.md"), &template.implementation, force);

    InitResult {
        state,
        implementation,
        template: template.name.clone(),
    }
}

/// Print the init result in a human-friendly format.
pub fn print_result(result: &InitResult) {
    println!();
    println!("  Governor v{} — Project Initialised", env!("CARGO_PKG_VERSION"));
    println!("  Template: {}", result.template);
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
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Serialises env manipulation and points `GOVERNOR_TEMPLATES_DIR`
    /// at a temp directory; cleanup runs on `Drop` even when a test panics.
    struct TemplatesFixture {
        _lock: MutexGuard<'static, ()>,
        _tmp: tempfile::TempDir,
    }

    impl TemplatesFixture {
        fn new() -> Self {
            // A panicking test poisons the lock; later tests must still run.
            let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_var(ENV_TEMPLATES_DIR, tmp.path());
            TemplatesFixture { _lock: lock, _tmp: tmp }
        }

        fn root(&self) -> &Path {
            self._tmp.path()
        }

        fn write_template(&self, name: &str, state: &str, implementation: Option<&str>) {
            let dir = self._tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("STATE.md"), state).unwrap();
            if let Some(implementation) = implementation {
                std::fs::write(dir.join("IMPLEMENTATION.md"), implementation).unwrap();
            }
        }
    }

    impl Drop for TemplatesFixture {
        fn drop(&mut self) {
            std::env::remove_var(ENV_TEMPLATES_DIR);
        }
    }

    #[test]
    fn test_templates_non_empty() {
        assert!(!STATE_TEMPLATE.is_empty());
        assert!(!IMPLEMENTATION_TEMPLATE.is_empty());
        assert!(STATE_TEMPLATE.contains("STATE"));
        assert!(IMPLEMENTATION_TEMPLATE.contains("Contract"));
    }

    #[test]
    fn test_builtin_catalog_is_complete_and_distinct() {
        let names = built_in_names();
        assert_eq!(names, vec!["default", "minimal", "rust", "frontend"]);

        // Every built-in resolves and has distinct, non-empty content.
        let mut state_contents = Vec::new();
        for name in &names {
            let template = resolve_template(name).unwrap();
            assert_eq!(template.name, *name);
            assert!(!template.description.is_empty());
            assert!(!template.state.is_empty());
            assert!(!template.implementation.is_empty());
            state_contents.push(template.state);
        }
        let unique = state_contents.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), names.len(), "built-in STATE contents must differ");
    }

    #[test]
    fn test_resolve_built_in_content_markers() {
        let minimal = resolve_template("minimal").unwrap();
        assert!(minimal.state.contains("Current Task"));
        assert!(!minimal.state.contains("## Notes"), "minimal omits Notes");

        let rust = resolve_template("rust").unwrap();
        assert!(rust.state.contains("cargo clippy"));
        assert!(rust.implementation.contains("src/"));

        let frontend = resolve_template("frontend").unwrap();
        assert!(frontend.state.contains("npm install"));
        assert!(frontend.implementation.contains("Component structure"));

        let default = resolve_template(DEFAULT_TEMPLATE_NAME).unwrap();
        assert_eq!(default.state, STATE_TEMPLATE);
        assert_eq!(default.implementation, IMPLEMENTATION_TEMPLATE);
    }

    #[test]
    fn test_resolve_unknown_lists_available() {
        let err = resolve_template("no-such-template").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("no-such-template"));
        for name in built_in_names() {
            assert!(message.contains(&name), "missing {name} in: {message}");
        }
    }

    #[test]
    fn test_resolve_custom_template_from_env_dir() {
        let fx = TemplatesFixture::new();
        fx.write_template(
            "mytpl",
            "# custom state\n",
            Some("# custom contract\n"),
        );

        let template = resolve_template("mytpl").unwrap();
        assert_eq!(template.name, "mytpl");
        assert_eq!(template.state, "# custom state\n");
        assert_eq!(template.implementation, "# custom contract\n");
        assert!(template.description.contains(fx.root().join("mytpl").display().to_string().as_str()));
    }

    #[test]
    fn test_custom_template_missing_implementation_is_incomplete() {
        let fx = TemplatesFixture::new();
        fx.write_template("half", "# state only\n", None);

        let err = resolve_template("half").unwrap_err();
        assert!(matches!(err, InitError::IncompleteTemplate { .. }));
        assert!(err.to_string().contains("IMPLEMENTATION.md"));
        // The error must not claim the template is simply unknown.
        assert!(!err.to_string().contains("Available templates"));
    }

    #[test]
    fn test_invalid_template_names_are_rejected_safely() {
        let fx = TemplatesFixture::new();
        // A directory with an invalid name exists on disk, but invalid
        // names are filtered before any filesystem lookup happens.
        fx.write_template("sub dir", "# s\n", Some("# i\n"));

        for bad in ["sub dir", "a/b", "../escape", "", "..", "."] {
            let err = resolve_template(bad).unwrap_err();
            assert!(matches!(err, InitError::UnknownTemplate { .. }), "name {bad:?}: {err}");
        }
    }

    #[test]
    fn test_unknown_template_error_lists_custom_templates_too() {
        let fx = TemplatesFixture::new();
        fx.write_template("teamtpl", "# s\n", Some("# i\n"));

        let err = resolve_template("missing").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("teamtpl"), "custom templates must be listed: {message}");
    }

    #[test]
    fn test_custom_template_names_only_complete_dirs() {
        let fx = TemplatesFixture::new();
        fx.write_template("good", "# s\n", Some("# i\n"));
        fx.write_template("bad", "# s\n", None); // missing IMPLEMENTATION.md
        fx.write_template("bad name", "# s\n", Some("# i\n")); // invalid name
        std::fs::write(fx.root().join("stray-file.md"), "not a dir").unwrap();

        assert_eq!(custom_template_names(fx.root()), vec!["good".to_string()]);
    }

    #[test]
    fn test_init_in_dir_with_named_template() {
        let dir = tempfile::tempdir().unwrap();
        let result = init_in_dir_with(dir.path(), false, "minimal").unwrap();

        assert_eq!(result.template, "minimal");
        let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
        assert!(state.contains("Current Task"));
        assert!(!state.contains("## Notes"));
    }

    #[test]
    fn test_init_in_dir_unknown_template_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let err = init_in_dir_with(dir.path(), false, "no-such-template").unwrap_err();

        assert!(matches!(err, InitError::UnknownTemplate { .. }));
        assert!(!dir.path().join("STATE.md").exists());
        assert!(!dir.path().join("IMPLEMENTATION.md").exists());
    }

    #[test]
    fn test_backward_compat_init_in_dir_matches_default() {
        let dir = tempfile::tempdir().unwrap();
        let result = init_in_dir(dir.path(), false);

        assert_eq!(result.template, DEFAULT_TEMPLATE_NAME);
        let expected = resolve_template(DEFAULT_TEMPLATE_NAME).unwrap();
        let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
        assert_eq!(state, expected.state);
        let implementation =
            std::fs::read_to_string(dir.path().join("IMPLEMENTATION.md")).unwrap();
        assert_eq!(implementation, expected.implementation);
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
            template: DEFAULT_TEMPLATE_NAME.to_string(),
        };
        // Just verify the struct is constructable.
        assert_eq!(result.state, FileOutcome::Created);
        assert_eq!(result.implementation, FileOutcome::Skipped);
    }
}
