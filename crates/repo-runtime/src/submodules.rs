use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use app_domain::{
    RepositoryStatus, RepositorySubmodule, RepositorySubmodules, StatusCode, StatusEntryKind,
    SubmoduleCommitState, SubmoduleWorktreeState,
};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, GitmodulesEntry,
    SubmoduleParseError, parse_gitlink_index_entries, parse_gitmodules_config,
    sanitize_submodule_url,
};
use thiserror::Error;

use crate::{
    RepositoryRuntime, RepositoryRuntimeError, RepositoryStatusGitExecutor, repository_status,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const INDEX_OUTPUT_LIMIT: usize = 1024 * 1024;
const MODE_PROBE_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const GITMODULES_OUTPUT_LIMIT: usize = 512 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const MAX_SUBMODULES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmoduleQuery {
    GitlinkModes,
    Gitlinks,
    Gitmodules,
}

impl SubmoduleQuery {
    fn output_limit(self) -> usize {
        match self {
            Self::GitlinkModes => MODE_PROBE_OUTPUT_LIMIT,
            Self::Gitlinks => INDEX_OUTPUT_LIMIT,
            Self::Gitmodules => GITMODULES_OUTPUT_LIMIT,
        }
    }

    fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::GitlinkModes => &["ls-files", "-z", "--format=%(objectmode)"],
            Self::Gitlinks => &["ls-files", "--stage", "-z"],
            Self::Gitmodules => &["config", "--null", "--file", ".gitmodules", "--list"],
        }
    }
}

pub trait SubmoduleGitExecutor: RepositoryStatusGitExecutor + Send + Sync {
    fn execute_submodule_query(
        &self,
        repository: &Path,
        query: SubmoduleQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl SubmoduleGitExecutor for GitRunner {
    fn execute_submodule_query(
        &self,
        repository: &Path,
        query: SubmoduleQuery,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(query.output_limit(), STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum SubmodulesRuntimeError {
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    Parse(#[from] SubmoduleParseError),
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error("the repository root could not be resolved: {0}")]
    RepositoryPath(#[source] std::io::Error),
}

impl SubmodulesRuntimeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Git(_) | Self::Parse(_) | Self::Repository(_) | Self::RepositoryPath(_) => {
                "gitRejected"
            }
        }
    }
}

impl<E: SubmoduleGitExecutor> RepositoryRuntime<E> {
    /// Lists only immediate gitlink entries. A submodule is intentionally a separate repository
    /// context; no recursive discovery or nested child command is performed.
    pub fn submodules(
        &self,
        repository: &Path,
    ) -> Result<RepositorySubmodules, SubmodulesRuntimeError> {
        submodules(&self.executor, repository)
    }
}

pub fn submodules<E: SubmoduleGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<RepositorySubmodules, SubmodulesRuntimeError> {
    let modes = executor.execute_submodule_query(repository, SubmoduleQuery::GitlinkModes)?;
    if !contains_gitlink_mode(&modes.stdout) {
        return Ok(RepositorySubmodules::default());
    }
    let index = executor.execute_submodule_query(repository, SubmoduleQuery::Gitlinks)?;
    let gitlinks = grouped_gitlinks(&index.stdout)?;
    if gitlinks.is_empty() {
        return Ok(RepositorySubmodules::default());
    }

    let metadata = gitmodules_metadata(executor, repository)?;
    let canonical_root =
        fs::canonicalize(repository).map_err(SubmodulesRuntimeError::RepositoryPath)?;
    let submodules = gitlinks
        .into_iter()
        .take(MAX_SUBMODULES)
        .map(|(path, gitlink)| {
            let configured = metadata.get(&path);
            inspect_submodule(
                executor,
                repository,
                &canonical_root,
                path,
                gitlink,
                configured,
            )
        })
        .collect();
    Ok(RepositorySubmodules { submodules })
}

fn contains_gitlink_mode(input: &[u8]) -> bool {
    input.split(|byte| *byte == 0).any(|mode| mode == b"160000")
}

#[derive(Debug, Clone, Default)]
struct GitlinkState {
    expected_oid: Option<String>,
    conflicted: bool,
}

fn grouped_gitlinks(
    input: &[u8],
) -> Result<BTreeMap<String, GitlinkState>, SubmodulesRuntimeError> {
    let mut result = BTreeMap::<String, GitlinkState>::new();
    for entry in parse_gitlink_index_entries(input)? {
        // Do not expose a lexical traversal path to callers. Invalid index records are not a
        // submodule that can be opened, while a valid but unavailable child remains visible.
        if !valid_relative_path(&entry.path) {
            continue;
        }
        let state = result.entry(entry.path).or_default();
        if entry.stage == 0 {
            state.expected_oid = Some(entry.oid);
        } else {
            state.conflicted = true;
        }
    }
    Ok(result)
}

fn gitmodules_metadata<E: SubmoduleGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<BTreeMap<String, GitmodulesEntry>, SubmodulesRuntimeError> {
    let path = repository.join(".gitmodules");
    let regular_file = fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file());
    if !regular_file {
        return Ok(BTreeMap::new());
    }

    let output = match executor.execute_submodule_query(repository, SubmoduleQuery::Gitmodules) {
        Ok(output) => output,
        // `.gitmodules` is descriptive only. A repository can carry gitlinks after the file was
        // deleted or made unreadable, so never hide those index entries because it cannot load.
        Err(GitRunError::Unsuccessful { .. }) => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    let mut metadata = BTreeMap::new();
    for entry in parse_gitmodules_config(&output.stdout).unwrap_or_default() {
        let Some(path) = entry.path.as_ref().filter(|path| valid_relative_path(path)) else {
            continue;
        };
        // Git rejects duplicate paths when adding submodules, but malformed hand-authored files
        // can contain them. Keep the lexicographically first configuration deterministically.
        metadata.entry(path.clone()).or_insert(entry);
    }
    Ok(metadata)
}

fn inspect_submodule<E: SubmoduleGitExecutor>(
    executor: &E,
    repository: &Path,
    canonical_root: &Path,
    path: String,
    gitlink: GitlinkState,
    configured: Option<&GitmodulesEntry>,
) -> RepositorySubmodule {
    let name = configured
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| path.clone());
    let url = configured
        .and_then(|entry| entry.url.as_deref())
        .and_then(sanitize_submodule_url);
    let expected_oid = gitlink.expected_oid;

    let Some(child) = validated_child_path(repository, canonical_root, &path) else {
        let present = repository.join(&path).symlink_metadata().is_ok();
        return unavailable_submodule(name, path, url, expected_oid, gitlink.conflicted, present);
    };

    match repository_status(executor, &child) {
        Ok(status) => {
            inspected_submodule(name, path, url, expected_oid, gitlink.conflicted, status)
        }
        // A missing/broken child is a row-level state. Do not turn a repository refresh into a
        // failure merely because one optional submodule has not been initialized.
        Err(_) => unavailable_submodule(name, path, url, expected_oid, gitlink.conflicted, true),
    }
}

fn unavailable_submodule(
    name: String,
    path: String,
    url: Option<String>,
    expected_oid: Option<String>,
    conflicted: bool,
    present: bool,
) -> RepositorySubmodule {
    RepositorySubmodule {
        name,
        path,
        url,
        expected_oid,
        current_oid: None,
        present,
        initialized: false,
        commit_state: if conflicted {
            SubmoduleCommitState::Conflicted
        } else {
            SubmoduleCommitState::Unavailable
        },
        worktree_state: SubmoduleWorktreeState::Unavailable,
        change_count: 0,
    }
}

fn inspected_submodule(
    name: String,
    path: String,
    url: Option<String>,
    expected_oid: Option<String>,
    conflicted: bool,
    status: RepositoryStatus,
) -> RepositorySubmodule {
    let current_oid = status.branch.oid.clone();
    let change_count = status
        .entries
        .iter()
        .filter(|entry| entry.kind != StatusEntryKind::Ignored)
        .count() as u64;
    let worktree_state = classify_worktree_state(&status);
    let commit_state = if conflicted {
        SubmoduleCommitState::Conflicted
    } else {
        match (expected_oid.as_deref(), current_oid.as_deref()) {
            (Some(expected), Some(current)) if expected == current => {
                SubmoduleCommitState::AtExpected
            }
            (Some(_), Some(_)) => SubmoduleCommitState::Different,
            _ => SubmoduleCommitState::Unavailable,
        }
    };
    RepositorySubmodule {
        name,
        path,
        url,
        expected_oid,
        current_oid,
        present: true,
        initialized: true,
        commit_state,
        worktree_state,
        change_count,
    }
}

fn classify_worktree_state(status: &RepositoryStatus) -> SubmoduleWorktreeState {
    let conflicted = status.entries.iter().any(|entry| {
        entry.kind == StatusEntryKind::Unmerged
            || entry.index_status == StatusCode::Unmerged
            || entry.worktree_status == StatusCode::Unmerged
    });
    if conflicted {
        return SubmoduleWorktreeState::Conflicted;
    }
    let untracked = status
        .entries
        .iter()
        .any(|entry| entry.kind == StatusEntryKind::Untracked);
    let modified = status.entries.iter().any(|entry| {
        entry.kind != StatusEntryKind::Untracked && entry.kind != StatusEntryKind::Ignored
    });
    match (modified, untracked) {
        (false, false) => SubmoduleWorktreeState::Clean,
        (true, false) => SubmoduleWorktreeState::Modified,
        (false, true) => SubmoduleWorktreeState::Untracked,
        (true, true) => SubmoduleWorktreeState::ModifiedAndUntracked,
    }
}

fn valid_relative_path(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn validated_child_path(
    repository: &Path,
    canonical_root: &Path,
    relative: &str,
) -> Option<PathBuf> {
    if !valid_relative_path(relative) {
        return None;
    }
    let child = repository.join(relative);
    let metadata = fs::symlink_metadata(&child).ok()?;
    if !metadata.is_dir() && !metadata.file_type().is_symlink() {
        return None;
    }
    let canonical_child = fs::canonicalize(child).ok()?;
    canonical_child
        .starts_with(canonical_root)
        .then_some(canonical_child)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use git_core::GitExecutor;

    use super::*;

    struct RecordingExecutor {
        queries: Mutex<Vec<(PathBuf, SubmoduleQuery)>>,
        query_outputs: Mutex<VecDeque<Vec<u8>>>,
        status_outputs: Mutex<VecDeque<Vec<u8>>>,
        index_outputs: Mutex<VecDeque<Vec<u8>>>,
    }

    impl GitExecutor for RecordingExecutor {
        fn execute(&self, _: &Path, _: &[&str]) -> Result<GitOutput, GitRunError> {
            panic!("submodule tests use bounded submodule/status APIs")
        }
    }

    impl RepositoryStatusGitExecutor for RecordingExecutor {
        fn execute_repository_status(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: self.status_outputs.lock().unwrap().pop_front().unwrap(),
                stderr: Vec::new(),
            })
        }

        fn execute_index_entries(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: self.index_outputs.lock().unwrap().pop_front().unwrap(),
                stderr: Vec::new(),
            })
        }
    }

    impl SubmoduleGitExecutor for RecordingExecutor {
        fn execute_submodule_query(
            &self,
            repository: &Path,
            query: SubmoduleQuery,
        ) -> Result<GitOutput, GitRunError> {
            self.queries
                .lock()
                .unwrap()
                .push((repository.into(), query));
            Ok(GitOutput {
                stdout: self.query_outputs.lock().unwrap().pop_front().unwrap(),
                stderr: Vec::new(),
            })
        }
    }

    struct UnreadableMetadataExecutor;

    impl GitExecutor for UnreadableMetadataExecutor {
        fn execute(&self, _: &Path, _: &[&str]) -> Result<GitOutput, GitRunError> {
            panic!("submodule tests use bounded submodule/status APIs")
        }
    }

    impl RepositoryStatusGitExecutor for UnreadableMetadataExecutor {
        fn execute_repository_status(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: format!("# branch.oid {}\0# branch.head main\0", "a".repeat(40))
                    .into_bytes(),
                stderr: Vec::new(),
            })
        }

        fn execute_index_entries(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    impl SubmoduleGitExecutor for UnreadableMetadataExecutor {
        fn execute_submodule_query(
            &self,
            _: &Path,
            query: SubmoduleQuery,
        ) -> Result<GitOutput, GitRunError> {
            match query {
                SubmoduleQuery::GitlinkModes => Ok(GitOutput {
                    stdout: b"160000\0".to_vec(),
                    stderr: Vec::new(),
                }),
                SubmoduleQuery::Gitlinks => Ok(GitOutput {
                    stdout: format!("160000 {} 0\tmodules/client\0", "a".repeat(40)).into_bytes(),
                    stderr: Vec::new(),
                }),
                SubmoduleQuery::Gitmodules => Err(GitRunError::Unsuccessful {
                    code: Some(1),
                    stderr: "unreadable".into(),
                }),
            }
        }
    }

    #[test]
    fn large_non_gitlink_mode_probe_skips_the_full_index_query() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir_all(&repository).unwrap();
        let modes = b"100644\0".repeat(200_000);
        assert!(modes.len() > INDEX_OUTPUT_LIMIT);
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            query_outputs: Mutex::new(VecDeque::from([modes])),
            status_outputs: Mutex::new(VecDeque::new()),
            index_outputs: Mutex::new(VecDeque::new()),
        });

        let result = runtime.submodules(&repository).unwrap();

        assert!(result.submodules.is_empty());
        assert_eq!(
            runtime.executor.queries.lock().unwrap().as_slice(),
            &[(repository, SubmoduleQuery::GitlinkModes)]
        );
    }

    #[test]
    fn classifies_child_status_without_recursion() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        let child = repository.join("modules/client");
        fs::create_dir_all(&child).unwrap();
        fs::write(repository.join(".gitmodules"), "[submodule \"client\"]\n").unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            query_outputs: Mutex::new(VecDeque::from([
                b"160000\0".to_vec(),
                format!("160000 {} 0\tmodules/client\0", "a".repeat(40)).into_bytes(),
                b"submodule.client.path\nmodules/client\0submodule.client.url\nhttps://token@example.test/acme/client.git?secret=1\0".to_vec(),
            ])),
            status_outputs: Mutex::new(VecDeque::from([format!(
                "# branch.oid {}\0# branch.head main\01 M. N... 100644 100644 100644 {} {} src/file.ts\0? untracked.ts\0",
                "a".repeat(40), "b".repeat(40), "b".repeat(40)
            ).into_bytes()])),
            index_outputs: Mutex::new(VecDeque::from([Vec::new()])),
        });

        let result = runtime.submodules(&repository).unwrap();
        assert_eq!(result.submodules.len(), 1);
        let submodule = &result.submodules[0];
        assert_eq!(submodule.name, "client");
        assert_eq!(submodule.commit_state, SubmoduleCommitState::AtExpected);
        assert_eq!(
            submodule.worktree_state,
            SubmoduleWorktreeState::ModifiedAndUntracked
        );
        assert_eq!(submodule.change_count, 2);
        assert_eq!(
            submodule.url.as_deref(),
            Some("https://example.test/acme/client.git")
        );
        assert_eq!(
            runtime.executor.queries.lock().unwrap().as_slice(),
            &[
                (repository.clone(), SubmoduleQuery::GitlinkModes),
                (repository.clone(), SubmoduleQuery::Gitlinks),
                (repository, SubmoduleQuery::Gitmodules),
            ]
        );
    }

    #[test]
    fn omits_an_escaping_path_without_running_child_git() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir_all(&repository).unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            query_outputs: Mutex::new(VecDeque::from([
                b"160000\0".to_vec(),
                format!("160000 {} 0\t../outside\0", "a".repeat(40)).into_bytes(),
            ])),
            status_outputs: Mutex::new(VecDeque::new()),
            index_outputs: Mutex::new(VecDeque::new()),
        });

        let result = runtime.submodules(&repository).unwrap();
        assert!(result.submodules.is_empty());
        assert_eq!(runtime.executor.queries.lock().unwrap().len(), 2);
    }

    #[test]
    fn unreadable_gitmodules_never_hides_a_gitlink() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir_all(repository.join("modules/client")).unwrap();
        fs::write(repository.join(".gitmodules"), "[submodule \"client\"]\n").unwrap();

        let result = RepositoryRuntime::new(UnreadableMetadataExecutor)
            .submodules(&repository)
            .unwrap();

        assert_eq!(result.submodules.len(), 1);
        assert_eq!(result.submodules[0].name, "modules/client");
        assert!(result.submodules[0].url.is_none());
    }

    #[test]
    fn malformed_gitmodules_never_hides_a_gitlink() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir_all(repository.join("modules/client")).unwrap();
        fs::write(repository.join(".gitmodules"), "[submodule \"client\"]\n").unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            query_outputs: Mutex::new(VecDeque::from([
                b"160000\0".to_vec(),
                format!("160000 {} 0\tmodules/client\0", "a".repeat(40)).into_bytes(),
                b"not-a-config-record\0".to_vec(),
            ])),
            status_outputs: Mutex::new(VecDeque::from([format!(
                "# branch.oid {}\0# branch.head main\0",
                "a".repeat(40)
            )
            .into_bytes()])),
            index_outputs: Mutex::new(VecDeque::from([Vec::new()])),
        });

        let result = runtime.submodules(&repository).unwrap();
        assert_eq!(result.submodules.len(), 1);
        assert_eq!(result.submodules[0].name, "modules/client");
        assert!(result.submodules[0].url.is_none());
    }

    #[test]
    fn keeps_an_initialized_child_inspectable_when_parent_gitlink_is_conflicted() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir_all(repository.join("modules/client")).unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            query_outputs: Mutex::new(VecDeque::from([
                b"160000\0".to_vec(),
                format!(
                    "160000 {} 0\tmodules/client\0160000 {} 2\tmodules/client\0",
                    "a".repeat(40),
                    "b".repeat(40),
                )
                .into_bytes(),
            ])),
            status_outputs: Mutex::new(VecDeque::from([format!(
                "# branch.oid {}\0# branch.head main\0",
                "a".repeat(40)
            )
            .into_bytes()])),
            index_outputs: Mutex::new(VecDeque::from([Vec::new()])),
        });

        let result = runtime.submodules(&repository).unwrap();
        assert_eq!(result.submodules.len(), 1);
        let client = &result.submodules[0];
        assert_eq!(
            client.expected_oid.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert!(client.present && client.initialized);
        assert_eq!(client.commit_state, SubmoduleCommitState::Conflicted);
        assert_eq!(client.worktree_state, SubmoduleWorktreeState::Clean);
        assert_eq!(runtime.executor.queries.lock().unwrap().len(), 2);
    }
}
