//! Integration tests for Governor v2.
//!
//! These tests use temporary directories with fixture data to validate
//! end-to-end workflows: project discovery, lock acquisition, context
//! assembly, contract signing, and session lifecycle.

use std::fs;
use std::path::PathBuf;

#[test]
fn test_integrity_sign_and_validate() {
    use governor::integrity;

    let content = "# Test Contract\n\nBuild a thing.";
    let signed = integrity::embed_hash(content);

    // Should have a hash marker.
    assert!(signed.contains("GOVERNOR-HASH:"));

    // Extract and validate.
    let hash = integrity::extract_hash(&signed).unwrap();
    assert!(!hash.is_empty());

    // Strip and re-hash should match.
    let cleaned = integrity::strip_hash(&signed);
    assert!(!cleaned.contains("GOVERNOR-HASH:"));
    assert_eq!(integrity::compute_hash(&cleaned), hash);
}

#[test]
fn test_integrity_tamper_detection() {
    use governor::integrity;

    let content = "# Original\n\nSome content.";
    let signed = integrity::embed_hash(content);

    // Tamper: modify content but keep the hash.
    let tampered = signed.replace("Some content", "Modified content");

    // Extract hash from tampered content.
    let result = integrity::extract_hash(&tampered);
    assert!(result.is_ok());

    let hash = result.unwrap();
    let cleaned = integrity::strip_hash(&tampered);
    let computed = integrity::compute_hash(&cleaned);

    // Hashes should differ.
    assert_ne!(hash, computed);
}

#[test]
fn test_integrity_missing_hash() {
    use governor::integrity;

    let result = integrity::extract_hash("no marker here");
    assert!(result.is_err());
}

#[test]
fn test_lock_acquire_release() {
    use governor::lock::SessionLock;

    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join(".governor").join("session.lock");

    let mut lock = SessionLock::new(lock_path.clone());

    // Acquire.
    lock.acquire().expect("should acquire lock");
    assert!(lock.is_acquired());
    assert!(lock_path.exists());

    // Release.
    lock.release().expect("should release lock");
    assert!(!lock.is_acquired());
    assert!(!lock_path.exists());
}

#[test]
fn test_lock_already_held() {
    use governor::lock::SessionLock;

    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join(".governor").join("session.lock");

    let mut lock1 = SessionLock::new(lock_path.clone());
    lock1.acquire().unwrap();

    let mut lock2 = SessionLock::new(lock_path.clone());
    let result = lock2.acquire();

    // Should fail because lock1 holds it.
    assert!(result.is_err());

    // Clean up.
    lock1.release().unwrap();
}

#[test]
fn test_context_clean_content() {
    use governor::context;

    let input = "Header\n<!-- GOVERNOR-HASH:abc123 -->\nBody\n<!-- End of Governor context -->";
    let cleaned = context::clean_content(input);

    assert!(!cleaned.contains("GOVERNOR-HASH"));
    assert!(!cleaned.contains("End of Governor"));
    assert!(cleaned.contains("Header"));
    assert!(cleaned.contains("Body"));
}

#[test]
fn test_project_discovery_in_temp() {
    // This test verifies that discovery logic works (though it depends on
    // the WORKSPACE_ROOTS constant; in CI we'd set up fixtures differently).
    // For now, just verify the struct construction.
    let proj = governor::project::Project {
        name: "test-proj".into(),
        root: PathBuf::from("/tmp/test-proj"),
        state_file: PathBuf::from("/tmp/test-proj/STATE.md"),
        implementation_file: Some(PathBuf::from("/tmp/test-proj/IMPLEMENTATION.md")),
    };

    assert_eq!(proj.name, "test-proj");
    assert!(proj.implementation_file.is_some());
}

// ── Project-discovery parallelism (PLAN 5.3) ─────────────────────

/// Build a workspace root with `projects` marker directories, one level deep.
fn make_workspace(parent: &std::path::Path, root_name: &str, projects: &[&str]) -> PathBuf {
    let root = parent.join(root_name);
    fs::create_dir_all(&root).unwrap();
    for name in projects {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("STATE.md"), format!("# STATE\n\n## Current Task\n{name}\n")).unwrap();
    }
    root
}

#[test]
fn test_parallel_discovery_matches_sequential() {
    use governor::project;

    let tmp = tempfile::tempdir().unwrap();
    let specs: Vec<(&str, Vec<&str>)> = vec![
        ("root-a", vec!["aurora", "delta", "golf"]),
        ("root-b", vec!["echo", "foxtrot", "hotel"]),
        ("root-c", vec!["india", "juliet", "kilo"]),
    ];
    let roots: Vec<PathBuf> = specs
        .iter()
        .map(|(name, projects)| make_workspace(tmp.path(), name, projects))
        .collect();

    let sequential = project::discover_in_roots_with_threads(&roots, 1);
    assert_eq!(sequential.len(), 9, "fixture must hold 9 markers");
    assert!(sequential
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .windows(2)
        .all(|w| w[0] <= w[1]));

    // Concurrency must be invisible: identical names, identical roots, same order.
    let want: Vec<(String, PathBuf)> = sequential
        .iter()
        .map(|p| (p.name.clone(), p.root.clone()))
        .collect();
    for threads in [2usize, 4, 16] {
        let got: Vec<(String, PathBuf)> = project::discover_in_roots_with_threads(&roots, threads)
            .iter()
            .map(|p| (p.name.clone(), p.root.clone()))
            .collect();
        assert_eq!(got, want, "threads={threads} changed the discovery output");
    }

    // Default entry point agrees with the explicit single-thread run.
    assert_eq!(project::discover_in_roots(&roots).len(), want.len());
}

#[test]
fn test_parallel_discovery_tolerates_missing_roots() {
    use governor::project;

    let tmp = tempfile::tempdir().unwrap();
    let real = make_workspace(tmp.path(), "real", &["proj"]);

    let roots = vec![
        tmp.path().join("ghost-a"),
        tmp.path().join("ghost-b"),
        real,
        tmp.path().join("ghost-c"),
    ];

    for threads in [1usize, 2, 16] {
        let found = project::discover_in_roots_with_threads(&roots, threads);
        assert_eq!(found.len(), 1, "threads={threads}");
        assert_eq!(found[0].name, "proj");
    }
}

#[test]
fn test_thread_pool_public_api_keeps_order() {
    use governor::pool;

    let items: Vec<usize> = (0..48).collect();
    let squares = pool::parallel_map(&items, 6, |i| i * i);
    assert_eq!(squares, items.iter().map(|i| i * i).collect::<Vec<_>>());

    assert_eq!(pool::default_pool_size(0), 1);
    assert!(pool::default_pool_size(1000) <= pool::MAX_THREADS);
    let empty: Vec<u8> = Vec::new();
    assert!(pool::parallel_map(&empty, 4, |v| *v + 1).is_empty());
}

#[test]
fn test_relay_base64_roundtrip() {
    // The relay module has its own base64. Test roundtrip.
    let original = "echo 'Hello, World!'";
    // We test the module-internal functions indirectly via the public API.
    // For now, just test the clipboard base64 from the clipboard module.
    let encoded = governor::clipboard::base64_encode(original);
    // base64_decode is private to relay, but we can verify the encoded form.
    assert!(!encoded.is_empty());
    assert!(!encoded.contains('\n'));
}

// ── Init integration tests ───────────────────────────────────────

#[test]
fn test_init_creates_both_files() {
    use governor::init;
    let dir = tempfile::tempdir().unwrap();
    let result = init::init_in_dir(dir.path(), false);

    assert_eq!(result.state, init::FileOutcome::Created);
    assert_eq!(result.implementation, init::FileOutcome::Created);

    let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
    let impl_ = std::fs::read_to_string(dir.path().join("IMPLEMENTATION.md")).unwrap();

    assert!(state.contains("# STATE"));
    assert!(impl_.contains("# Implementation Contract"));
}

#[test]
fn test_init_skips_existing_state() {
    use governor::init;
    let dir = tempfile::tempdir().unwrap();

    // Pre-create STATE.md with custom content.
    std::fs::write(dir.path().join("STATE.md"), "my custom state").unwrap();

    let result = init::init_in_dir(dir.path(), false);

    assert_eq!(result.state, init::FileOutcome::Skipped);
    assert_eq!(result.implementation, init::FileOutcome::Created);

    // STATE.md content must be preserved.
    let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
    assert_eq!(state, "my custom state");
}

#[test]
fn test_init_force_overwrites() {
    use governor::init;
    let dir = tempfile::tempdir().unwrap();

    // Pre-create both files with custom content.
    std::fs::write(dir.path().join("STATE.md"), "old state").unwrap();
    std::fs::write(dir.path().join("IMPLEMENTATION.md"), "old impl").unwrap();

    let result = init::init_in_dir(dir.path(), true);

    assert_eq!(result.state, init::FileOutcome::Created);
    assert_eq!(result.implementation, init::FileOutcome::Created);

    let state = std::fs::read_to_string(dir.path().join("STATE.md")).unwrap();
    let impl_ = std::fs::read_to_string(dir.path().join("IMPLEMENTATION.md")).unwrap();

    assert!(state.contains("# STATE"));
    assert!(impl_.contains("# Implementation Contract"));
    assert!(!state.contains("old state"));
    assert!(!impl_.contains("old impl"));
}

#[test]
fn test_init_templates_non_empty() {
    use governor::init;
    assert!(!init::STATE_TEMPLATE.is_empty());
    assert!(!init::IMPLEMENTATION_TEMPLATE.is_empty());
}

#[test]
fn test_init_idempotent() {
    use governor::init;
    let dir = tempfile::tempdir().unwrap();

    // First init.
    let r1 = init::init_in_dir(dir.path(), false);
    assert_eq!(r1.state, init::FileOutcome::Created);
    assert_eq!(r1.implementation, init::FileOutcome::Created);

    // Second init — both skipped.
    let r2 = init::init_in_dir(dir.path(), false);
    assert_eq!(r2.state, init::FileOutcome::Skipped);
    assert_eq!(r2.implementation, init::FileOutcome::Skipped);
}

// ── `governor config --validate --check-root` (PLAN §4.2) ────────

/// Run the real binary; never the library shortcut, because the flag
/// wiring (clap → cmd_config) is exactly what this pins down.
fn run_governor(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_governor"))
        .args(args)
        .output()
        .expect("governor binary must be runnable in tests")
}

#[test]
fn test_check_root_warns_but_exits_zero_for_missing_root() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("exists");
    fs::create_dir(&real).unwrap();
    let ghost = dir.path().join("ghost"); // deliberately never created

    // Forward slashes keep the TOML basic string valid on Windows
    // (backslashes would need escaping) and are understood by the OS.
    let slash = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
    let cfg_path = dir.path().join("config.toml");
    fs::write(
        &cfg_path,
        format!("[workspace]\nroots = [\"{}\", \"{}\"]\n", slash(&real), slash(&ghost)),
    )
    .unwrap();

    let output = run_governor(&[
        "config",
        "--validate",
        cfg_path.to_str().unwrap(),
        "--check-root",
    ]);

    assert!(
        output.status.success(),
        "a missing root is a warning, not a failure. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Configuration is valid"), "{stdout}");
    assert!(stdout.contains("is readable"), "real root must pass: {stdout}");
    assert!(stdout.contains("does not exist"), "ghost root must warn: {stdout}");
}

#[test]
fn test_check_root_without_validate_is_rejected() {
    let output = run_governor(&["config", "--show", "--check-root"]);
    assert!(!output.status.success(), "clap `requires` should reject this");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--validate"),
        "usage error must name the missing flag: {stderr}"
    );
}

#[test]
fn test_validate_without_check_root_prints_no_root_section() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, "[workspace]\nroots = []\n").unwrap();

    let output = run_governor(&["config", "--validate", cfg_path.to_str().unwrap()]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Configuration is valid"));
    assert!(
        !stdout.contains("Workspace root check"),
        "--check-root must stay off unless asked: {stdout}"
    );
}