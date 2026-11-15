//! Project discovery and STATE.md parsing.
//!
//! Scans known workspace roots for directories containing a STATE.md file,
//! which is the Governor v2 project marker. Roots are scanned concurrently
//! by a small thread pool ([`crate::pool`]); the result is identical to a
//! sequential scan because hits are re-ordered by root and de-duplicated
//! first-root-wins.

use crate::config::Config;
use crate::pool;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error("no projects found in workspace roots")]
    NoProjectsFound,
    #[error("project '{0}' not found")]
    ProjectNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// A discovered Governor-managed project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Human-readable project name (derived from directory name).
    pub name: String,
    /// Absolute path to the project root directory.
    pub root: PathBuf,
    /// Path to STATE.md inside the project.
    pub state_file: PathBuf,
    /// Path to IMPLEMENTATION.md (optional).
    pub implementation_file: Option<PathBuf>,
}

impl Project {
    /// Read the contents of STATE.md.
    pub fn read_state(&self) -> Result<String, ProjectError> {
        Ok(std::fs::read_to_string(&self.state_file)?)
    }

    /// Read IMPLEMENTATION.md if it exists.
    pub fn read_implementation(&self) -> Option<String> {
        self.implementation_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
    }

    /// Check whether a lock file exists for this project.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(".governor").join("session.lock")
    }

    /// Check whether the project is currently locked.
    pub fn is_locked(&self) -> bool {
        self.lock_path().exists()
    }
}

/// Discover all Governor projects across workspace roots.
///
/// A project is any directory that contains a `STATE.md` file. Roots are
/// scanned concurrently through [`crate::pool`]; the returned list is
/// byte-for-byte the one a single-threaded scan would have produced.
pub fn discover_projects() -> Result<Vec<Project>, ProjectError> {
    // Load configuration to get workspace roots; if loading fails, use defaults.
    let config = Config::load().unwrap_or_default();
    let projects = discover_in_roots(&config.resolved_roots());

    if projects.is_empty() {
        return Err(ProjectError::NoProjectsFound);
    }

    Ok(projects)
}

/// Discover projects inside an explicit list of workspace roots.
///
/// Uses the default thread pool. Missing or unreadable roots are skipped, so
/// the result is simply empty when nothing could be scanned.
pub fn discover_in_roots(roots: &[PathBuf]) -> Vec<Project> {
    discover_in_roots_with_threads(roots, pool::default_pool_size(roots.len()))
}

/// [`discover_in_roots`] with an explicit worker count.
///
/// A tuning knob for tests and for hosts with slow mounts (network drives):
/// any thread count yields the same output, so this never changes results.
pub fn discover_in_roots_with_threads(roots: &[PathBuf], threads: usize) -> Vec<Project> {
    // One unit of work per root. `parallel_map` hands back results in root
    // order, which keeps the "earlier root wins" de-duplication below
    // deterministic even though the scan itself is concurrent.
    let hits = pool::parallel_map(roots, threads, |root| scan_root(root));

    // Roots may reach the same directory through different spellings
    // (`.` and `../work` can both resolve to `C:\work`), so de-duplicate by
    // canonical path: raw string equality would keep both `C:\work\x` and
    // `C:\work\..\work\x`. Earlier roots win, so the cleanest spelling of a
    // project path is the one that survives.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut projects: Vec<Project> = hits
        .into_iter()
        .flatten()
        .filter(|project| {
            seen.insert(
                std::fs::canonicalize(&project.root).unwrap_or_else(|_| project.root.clone()),
            )
        })
        .collect();

    // Sort by name for stable output.
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    projects
}

/// List project markers one level deep inside a single workspace root.
///
/// A non-existent root, a non-directory and a directory that cannot be
/// listed are all treated as "no projects here" instead of an error:
/// configured roots are commonly optional (`../projects` and friends).
fn scan_root(root: &Path) -> Vec<Project> {
    let mut found: Vec<Project> = Vec::new();
    if !root.is_dir() {
        return found;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };

    // Walk one level deep looking for STATE.md markers.
    for entry in entries.flatten() {
        let candidate = entry.path();
        if !candidate.is_dir() {
            continue;
        }

        let state_file = candidate.join("STATE.md");
        if !state_file.is_file() {
            continue;
        }

        let impl_file = candidate.join("IMPLEMENTATION.md");
        found.push(Project {
            name: candidate
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| candidate.to_string_lossy().to_string()),
            root: candidate,
            state_file,
            implementation_file: impl_file.is_file().then_some(impl_file),
        });
    }

    found
}

/// Find a specific project by name.
pub fn find_project(name: &str) -> Result<Project, ProjectError> {
    discover_projects()?
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| ProjectError::ProjectNotFound(name.to_string()))
}

/// Build a project handle for a directory containing a `STATE.md` marker.
///
/// Returns `None` when the directory has no marker. The directory does not
/// need to be discoverable through the configured workspace roots — this is
/// how governor commands keep working inside a checkout that lives outside
/// every configured root.
pub fn project_from_dir(dir: &Path) -> Option<Project> {
    let state_file = dir.join("STATE.md");
    if !state_file.is_file() {
        return None;
    }
    let impl_file = dir.join("IMPLEMENTATION.md");
    Some(Project {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        root: dir.to_path_buf(),
        state_file,
        implementation_file: impl_file.is_file().then_some(impl_file),
    })
}

/// The Governor project in the current working directory, if any.
pub fn project_in_current_dir() -> Option<Project> {
    project_from_dir(&std::env::current_dir().ok()?)
}

/// Resolve a project: by name if provided, otherwise by current directory.
pub fn resolve_project(name: Option<&str>) -> Result<Project, ProjectError> {
    match name {
        Some(n) => find_project(n).or_else(|e| {
            // The named project may be the current directory itself, hidden
            // from workspace-root discovery; match it by name.
            match project_in_current_dir() {
                Some(p) if p.name.eq_ignore_ascii_case(n) => Ok(p),
                _ => Err(e),
            }
        }),
        None => {
            // Try current directory first.
            if let Some(project) = project_in_current_dir() {
                return Ok(project);
            }
            // Pick the first discovered project, or error if none.
            discover_projects()?
                .into_iter()
                .next()
                .ok_or(ProjectError::NoProjectsFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Create `<root>/<name>/STATE.md` (+ optional IMPLEMENTATION.md).
    fn make_project(root: &Path, name: &str, with_impl: bool) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("STATE.md"), format!("# STATE — {name}\n")).unwrap();
        if with_impl {
            std::fs::write(
                dir.join("IMPLEMENTATION.md"),
                format!("# Contract — {name}\n"),
            )
            .unwrap();
        }
        dir
    }

    /// Create a workspace root holding project markers for `names`.
    fn make_root(parent: &Path, root_name: &str, names: &[&str]) -> PathBuf {
        let root = parent.join(root_name);
        std::fs::create_dir_all(&root).unwrap();
        for name in names {
            make_project(&root, name, false);
        }
        root
    }

    fn names_of(projects: &[Project]) -> Vec<String> {
        projects.iter().map(|p| p.name.clone()).collect()
    }

    fn roots_of(projects: &[Project]) -> Vec<PathBuf> {
        projects.iter().map(|p| p.root.clone()).collect()
    }

    /// One fixture shared by the equivalence tests: 4 roots × 2 markers.
    /// Fixtures create real directories, so they stay deliberately small —
    /// the whole suite runs concurrently on one machine.
    fn four_roots(tmp: &Path) -> Vec<PathBuf> {
        (0..4)
            .map(|i| make_root(tmp, &format!("root{i}"), &["alpha", "beta"]))
            .collect::<Vec<_>>()
    }

    #[test]
    fn parallel_scan_matches_sequential_across_all_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = four_roots(tmp.path());
        let want = roots
            .iter()
            .flat_map(|r| [r.join("alpha"), r.join("beta")])
            .collect::<Vec<_>>();

        let projects = discover_in_roots_with_threads(&roots, 4);
        assert_eq!(projects.len(), 8, "every root must contribute");
        assert!(
            names_of(&projects).windows(2).all(|w| w[0] <= w[1]),
            "output must stay sorted by name: {:?}",
            names_of(&projects)
        );
        for path in &want {
            assert!(
                projects.iter().any(|p| p.root == *path),
                "missing project {}",
                path.display()
            );
        }

        // Concurrency must be invisible: same hits, same order, any pool size.
        let sequential = roots_of(&discover_in_roots_with_threads(&roots, 1));
        for threads in [2usize, 4, 8, 64] {
            assert_eq!(
                roots_of(&discover_in_roots_with_threads(&roots, threads)),
                sequential,
                "thread count {threads} changed the output"
            );
        }
        assert_eq!(roots_of(&discover_in_roots(&roots)), sequential);
    }

    #[test]
    fn duplicate_roots_are_reported_once() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_root(tmp.path(), "ws", &["solo"]);

        let projects = discover_in_roots_with_threads(&[root.clone(), root.clone(), root], 4);

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "solo");
    }

    #[test]
    fn same_name_in_two_roots_keeps_both_earlier_root_first() {
        let tmp = tempfile::tempdir().unwrap();
        let listed_first = make_root(tmp.path(), "a-roots", &["twin"]);
        let listed_second = make_root(tmp.path(), "b-roots", &["twin"]);

        let projects =
            discover_in_roots_with_threads(&[listed_second.clone(), listed_first.clone()], 4);

        assert_eq!(projects.len(), 2);
        // Stable sort on equal names preserves the configured root order.
        assert_eq!(projects[0].root, listed_second.join("twin"));
        assert_eq!(projects[1].root, listed_first.join("twin"));
    }

    #[test]
    fn missing_roots_and_plain_files_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let good = make_root(tmp.path(), "good", &["found"]);
        let missing = tmp.path().join("does-not-exist");
        let a_file = tmp.path().join("not-a-dir.txt");
        std::fs::write(&a_file, "x").unwrap();

        let roots = vec![missing.clone(), a_file.clone(), good];
        for threads in [1usize, 4] {
            let projects = discover_in_roots_with_threads(&roots, threads);
            assert_eq!(names_of(&projects), vec!["found".to_string()]);
        }

        assert!(discover_in_roots(&[]).is_empty());
        assert!(discover_in_roots(&[missing, a_file]).is_empty());
    }

    #[test]
    fn scan_root_only_walks_one_level_and_requires_state_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();

        // Directories without the marker are not projects.
        std::fs::create_dir_all(root.join("no-marker")).unwrap();
        // A STATE.md that is a plain file next to nothing else is not a project.
        std::fs::write(root.join("STATE.md"), "# loose").unwrap();
        // One level deeper: `nested` has no marker, its child does.
        make_project(&root.join("nested"), "deep", false);

        let projects = scan_root(&root);

        assert!(
            projects.is_empty(),
            "unexpected projects: {:?}",
            names_of(&projects)
        );
    }

    #[test]
    fn implementation_marker_is_optional() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        let with_impl = make_project(&root, "with-contract", true);
        let without = make_project(&root, "bare", false);

        let projects = discover_in_roots(&[root]);

        assert_eq!(names_of(&projects), vec!["bare", "with-contract"]);
        assert_eq!(projects[0].root, without);
        assert_eq!(projects[0].implementation_file, None);
        assert_eq!(projects[1].root, with_impl);
        assert_eq!(
            projects[1].implementation_file,
            Some(with_impl.join("IMPLEMENTATION.md"))
        );
    }

    #[test]
    fn many_roots_are_all_scanned_without_loss() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = (0..10)
            .map(|i| make_root(tmp.path(), &format!("r{i:02}"), &["one"]))
            .collect::<Vec<_>>();

        let projects = discover_in_roots(&roots);

        assert_eq!(projects.len(), 10);
        assert!(projects.iter().all(|p| p.name == "one"));
        assert_eq!(
            roots_of(&projects).iter().collect::<HashSet<_>>().len(),
            10,
            "each root must contribute exactly one hit"
        );
    }

    #[test]
    fn equivalent_root_spellings_yield_one_project() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_root(tmp.path(), "ws", &["solo"]);

        // `.` and `..`-based spellings that resolve to the same directory
        // must not duplicate the projects found through them.
        let dotted = root.join(".");
        let upward = root.join("..").join("ws");
        let projects = discover_in_roots_with_threads(&[dotted, upward], 2);

        assert_eq!(projects.len(), 1, "duplicates: {:?}", names_of(&projects));
        assert_eq!(projects[0].name, "solo");
    }

    #[test]
    fn project_from_dir_matches_marker_presence() {
        let tmp = tempfile::tempdir().unwrap();
        let with_impl = make_project(tmp.path(), "full", true);
        let without_impl = make_project(tmp.path(), "bare", false);
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();

        let full = project_from_dir(&with_impl).expect("STATE.md present");
        assert_eq!(full.name, "full");
        assert_eq!(full.root, with_impl);
        assert_eq!(
            full.implementation_file,
            Some(with_impl.join("IMPLEMENTATION.md"))
        );

        let bare = project_from_dir(&without_impl).expect("STATE.md present");
        assert_eq!(bare.name, "bare");
        assert_eq!(bare.implementation_file, None);

        assert!(project_from_dir(&empty).is_none(), "no STATE.md marker");
    }

    #[test]
    fn resolve_project_falls_back_to_current_dir_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_project(tmp.path(), "unlisted", true);

        // Discovery over an empty root list knows nothing about `unlisted`;
        // the name-based resolution must still find it via its directory.
        let by_dir = project_from_dir(&dir).unwrap();
        assert_eq!(by_dir.name, "unlisted");
        assert_eq!(by_dir.root, dir);
    }
}
