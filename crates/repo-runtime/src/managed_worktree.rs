use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use app_domain::{RepositoryBranchKind, RepositoryNavigation, RepositoryWorktree};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{NavigationGitExecutor, NavigationRuntimeError, RepositoryRuntime};

const WORKTREE_TIMEOUT: Duration = Duration::from_secs(60);
const OUTPUT_LIMIT: usize = 512 * 1024;
const MANAGED_WORKTREE_DIRECTORY: &str = "managed-worktrees";

pub trait ManagedWorktreeGitExecutor: Send + Sync {
    fn add_managed_worktree(
        &self,
        repository: &Path,
        destination: &Path,
        branch_full_name: &str,
    ) -> Result<GitOutput, GitRunError>;

    fn remove_failed_managed_worktree(
        &self,
        repository: &Path,
        destination: &Path,
    ) -> Result<GitOutput, GitRunError>;
}

impl ManagedWorktreeGitExecutor for GitRunner {
    fn add_managed_worktree(
        &self,
        repository: &Path,
        destination: &Path,
        branch_full_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        // `git worktree add <path> refs/heads/name` checks out a detached commit. The short local
        // branch name keeps the worktree attached; callers have already verified the exact full
        // ref and OID immediately before this invocation.
        let branch_name = branch_full_name
            .strip_prefix("refs/heads/")
            .unwrap_or(branch_full_name);
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    OsString::from("worktree"),
                    OsString::from("add"),
                    OsString::from("--no-guess-remote"),
                    OsString::from("--"),
                    destination.as_os_str().to_owned(),
                    OsString::from(branch_name),
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(WORKTREE_TIMEOUT),
        )
    }

    fn remove_failed_managed_worktree(
        &self,
        repository: &Path,
        destination: &Path,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    OsString::from("worktree"),
                    OsString::from("remove"),
                    OsString::from("--"),
                    destination.as_os_str().to_owned(),
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(WORKTREE_TIMEOUT),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBranchWorktree {
    pub path: PathBuf,
    pub created: bool,
}

#[derive(Debug, Error)]
pub enum ManagedWorktreeError {
    #[error("the selected local branch request is invalid")]
    InvalidRequest,
    #[error("the selected local branch does not exist or changed")]
    BranchChanged,
    #[error("the selected branch worktree is unavailable, locked, or changed")]
    WorktreeUnavailable,
    #[error("the application data directory is unavailable or unsafe")]
    UnsafeDataDirectory,
    #[error("the managed worktree destination already exists outside Git's worktree registry")]
    DestinationExists,
    #[error(
        "the created worktree could not be verified and cleanup failed (validation: {validation}; cleanup: {cleanup})"
    )]
    CleanupFailed { validation: String, cleanup: String },
    #[error(transparent)]
    Navigation(#[from] NavigationRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
}

impl ManagedWorktreeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalidRequest",
            Self::BranchChanged => "staleState",
            Self::WorktreeUnavailable => "worktreeUnavailable",
            Self::UnsafeDataDirectory => "unsafeDataDirectory",
            Self::DestinationExists => "destinationExists",
            Self::CleanupFailed { .. } => "manualCleanupRequired",
            Self::Navigation(_) | Self::Git(_) => "gitRejected",
        }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: NavigationGitExecutor + ManagedWorktreeGitExecutor,
{
    pub fn prepare_branch_worktree(
        &self,
        repository: &Path,
        data_root: &Path,
        branch_full_name: &str,
        expected_oid: &str,
    ) -> Result<PreparedBranchWorktree, ManagedWorktreeError> {
        validate_request(branch_full_name, expected_oid)?;
        let repository =
            fs::canonicalize(repository).map_err(|_| ManagedWorktreeError::WorktreeUnavailable)?;
        if !repository.is_dir() {
            return Err(ManagedWorktreeError::WorktreeUnavailable);
        }

        let initial = self.navigation(&repository)?;
        verify_branch(&initial, branch_full_name, expected_oid)?;
        if let Some(worktree) = worktree_for_branch(&initial, branch_full_name) {
            return Ok(PreparedBranchWorktree {
                path: verify_existing_worktree(worktree, expected_oid)?,
                created: false,
            });
        }

        let destination =
            managed_destination(&repository, data_root, branch_full_name, expected_oid)?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => return Err(ManagedWorktreeError::DestinationExists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ManagedWorktreeError::UnsafeDataDirectory),
        }

        self.executor
            .add_managed_worktree(&repository, &destination, branch_full_name)?;

        let validation =
            self.verify_created_worktree(&repository, &destination, branch_full_name, expected_oid);
        match validation {
            Ok(path) => Ok(PreparedBranchWorktree {
                path,
                created: true,
            }),
            Err(validation) => {
                match self
                    .executor
                    .remove_failed_managed_worktree(&repository, &destination)
                {
                    Ok(_) => Err(validation),
                    Err(cleanup) => Err(ManagedWorktreeError::CleanupFailed {
                        validation: validation.to_string(),
                        cleanup: cleanup.to_string(),
                    }),
                }
            }
        }
    }

    fn verify_created_worktree(
        &self,
        repository: &Path,
        destination: &Path,
        branch_full_name: &str,
        expected_oid: &str,
    ) -> Result<PathBuf, ManagedWorktreeError> {
        let refreshed = self.navigation(repository)?;
        verify_branch(&refreshed, branch_full_name, expected_oid)?;
        let worktree = worktree_for_branch(&refreshed, branch_full_name)
            .ok_or(ManagedWorktreeError::WorktreeUnavailable)?;
        let path = verify_existing_worktree(worktree, expected_oid)?;
        let expected_path =
            fs::canonicalize(destination).map_err(|_| ManagedWorktreeError::WorktreeUnavailable)?;
        if path != expected_path {
            return Err(ManagedWorktreeError::WorktreeUnavailable);
        }
        Ok(path)
    }
}

fn verify_branch(
    navigation: &RepositoryNavigation,
    branch_full_name: &str,
    expected_oid: &str,
) -> Result<(), ManagedWorktreeError> {
    let branch = navigation
        .branches
        .iter()
        .find(|branch| branch.full_name == branch_full_name)
        .ok_or(ManagedWorktreeError::BranchChanged)?;
    if branch.kind != RepositoryBranchKind::Local
        || branch.oid != expected_oid
        || branch.symbolic_target.is_some()
    {
        return Err(ManagedWorktreeError::BranchChanged);
    }
    Ok(())
}

fn worktree_for_branch<'a>(
    navigation: &'a RepositoryNavigation,
    branch_full_name: &str,
) -> Option<&'a RepositoryWorktree> {
    navigation
        .worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_full_name))
}

fn verify_existing_worktree(
    worktree: &RepositoryWorktree,
    expected_oid: &str,
) -> Result<PathBuf, ManagedWorktreeError> {
    if worktree.bare
        || worktree.detached
        || worktree.locked
        || worktree.prunable
        || worktree.head.as_deref() != Some(expected_oid)
    {
        return Err(ManagedWorktreeError::WorktreeUnavailable);
    }
    let path =
        fs::canonicalize(&worktree.path).map_err(|_| ManagedWorktreeError::WorktreeUnavailable)?;
    if !path.is_dir() {
        return Err(ManagedWorktreeError::WorktreeUnavailable);
    }
    Ok(path)
}

fn validate_request(
    branch_full_name: &str,
    expected_oid: &str,
) -> Result<(), ManagedWorktreeError> {
    let Some(branch_name) = branch_full_name.strip_prefix("refs/heads/") else {
        return Err(ManagedWorktreeError::InvalidRequest);
    };
    if branch_name.is_empty()
        || branch_name.starts_with('-')
        || branch_name.chars().any(char::is_control)
        || branch_name.contains("..")
        || branch_name.contains("@{")
        || branch_name.ends_with('/')
        || !matches!(expected_oid.len(), 40 | 64)
        || !expected_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ManagedWorktreeError::InvalidRequest);
    }
    Ok(())
}

fn managed_destination(
    repository: &Path,
    data_root: &Path,
    branch_full_name: &str,
    expected_oid: &str,
) -> Result<PathBuf, ManagedWorktreeError> {
    let data_root =
        fs::canonicalize(data_root).map_err(|_| ManagedWorktreeError::UnsafeDataDirectory)?;
    if !data_root.is_dir() {
        return Err(ManagedWorktreeError::UnsafeDataDirectory);
    }
    let managed_root = data_root.join(MANAGED_WORKTREE_DIRECTORY);
    fs::create_dir_all(&managed_root).map_err(|_| ManagedWorktreeError::UnsafeDataDirectory)?;
    let managed_root =
        fs::canonicalize(&managed_root).map_err(|_| ManagedWorktreeError::UnsafeDataDirectory)?;
    if !managed_root.starts_with(&data_root) {
        return Err(ManagedWorktreeError::UnsafeDataDirectory);
    }

    let repository_root = managed_root.join(format!("repo-{:016x}", stable_hash(repository)));
    fs::create_dir_all(&repository_root).map_err(|_| ManagedWorktreeError::UnsafeDataDirectory)?;
    let repository_root = fs::canonicalize(&repository_root)
        .map_err(|_| ManagedWorktreeError::UnsafeDataDirectory)?;
    if !repository_root.starts_with(&managed_root) {
        return Err(ManagedWorktreeError::UnsafeDataDirectory);
    }

    let branch_name = branch_full_name
        .strip_prefix("refs/heads/")
        .ok_or(ManagedWorktreeError::InvalidRequest)?;
    let component = managed_component(branch_name, expected_oid);
    Ok(repository_root.join(component))
}

fn managed_component(branch_name: &str, expected_oid: &str) -> String {
    let mut readable = String::with_capacity(48);
    let mut previous_separator = false;
    for character in branch_name.chars() {
        let normalized = if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            character.to_ascii_lowercase()
        } else {
            '-'
        };
        if normalized == '-' && previous_separator {
            continue;
        }
        readable.push(normalized);
        previous_separator = normalized == '-';
        if readable.len() >= 48 {
            break;
        }
    }
    let readable = readable.trim_matches('-');
    let readable = if readable.is_empty() {
        "branch"
    } else {
        readable
    };
    format!(
        "{readable}-{:016x}-{}",
        stable_hash(Path::new(branch_name)),
        &expected_oid[..7]
    )
}

fn stable_hash(path: &Path) -> u64 {
    // FNV-1a is deliberately fixed rather than relying on DefaultHasher's unspecified algorithm.
    path.to_string_lossy()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use app_domain::{RepositoryBranch, RepositoryBranchKind};
    use git_core::GitExecutor;
    use tempfile::tempdir;

    use super::*;

    struct RecordingExecutor {
        navigations: Mutex<VecDeque<RepositoryNavigation>>,
        added: Mutex<Vec<(PathBuf, String)>>,
    }

    impl GitExecutor for RecordingExecutor {
        fn execute(
            &self,
            _repository: &Path,
            _arguments: &[&str],
        ) -> Result<GitOutput, GitRunError> {
            panic!("managed-worktree tests use dedicated executor methods")
        }
    }

    impl NavigationGitExecutor for RecordingExecutor {
        fn execute_navigation(
            &self,
            _repository: &Path,
            query: crate::NavigationQuery,
        ) -> Result<GitOutput, GitRunError> {
            let mut navigations = self.navigations.lock().unwrap();
            let navigation = navigations.front().expect("recorded navigation");
            let output = match query {
                crate::NavigationQuery::Branches => branch_output(&navigation.branches),
                crate::NavigationQuery::Worktrees => worktree_output(&navigation.worktrees),
                crate::NavigationQuery::StashPresence => Vec::new(),
                crate::NavigationQuery::Stashes => unreachable!(),
            };
            if query == crate::NavigationQuery::StashPresence {
                navigations.pop_front();
            }
            Ok(GitOutput {
                stdout: output,
                stderr: Vec::new(),
            })
        }
    }

    impl ManagedWorktreeGitExecutor for RecordingExecutor {
        fn add_managed_worktree(
            &self,
            _repository: &Path,
            destination: &Path,
            branch_full_name: &str,
        ) -> Result<GitOutput, GitRunError> {
            fs::create_dir_all(destination).unwrap();
            self.added
                .lock()
                .unwrap()
                .push((destination.to_path_buf(), branch_full_name.to_owned()));
            Ok(GitOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }

        fn remove_failed_managed_worktree(
            &self,
            _repository: &Path,
            destination: &Path,
        ) -> Result<GitOutput, GitRunError> {
            let _ = fs::remove_dir_all(destination);
            Ok(GitOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    fn branch() -> RepositoryBranch {
        RepositoryBranch {
            kind: RepositoryBranchKind::Local,
            full_name: "refs/heads/feature/safe".to_owned(),
            name: "feature/safe".to_owned(),
            oid: "a".repeat(40),
            current: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            symbolic_target: None,
        }
    }

    fn navigation(worktree: Option<RepositoryWorktree>) -> RepositoryNavigation {
        RepositoryNavigation {
            branches: vec![branch()],
            worktrees: worktree.into_iter().collect(),
            stashes: Vec::new(),
        }
    }

    fn branch_output(branches: &[RepositoryBranch]) -> Vec<u8> {
        branches
            .iter()
            .flat_map(|branch| {
                format!(
                    "{}\0{}\0{}\0 \0\0\0\0\n",
                    branch.full_name, branch.name, branch.oid
                )
                .into_bytes()
            })
            .collect()
    }

    fn worktree_output(worktrees: &[RepositoryWorktree]) -> Vec<u8> {
        worktrees
            .iter()
            .flat_map(|worktree| {
                format!(
                    "worktree {}\0HEAD {}\0branch {}\0\0",
                    worktree.path,
                    worktree.head.as_deref().unwrap_or_default(),
                    worktree.branch.as_deref().unwrap_or_default()
                )
                .into_bytes()
            })
            .collect()
    }

    #[test]
    fn reuses_an_existing_exact_worktree_without_mutation() {
        let fixture = tempdir().unwrap();
        let repository = fixture.path().join("repository");
        let existing = fixture.path().join("existing");
        let data = fixture.path().join("data");
        for directory in [&repository, &existing, &data] {
            fs::create_dir(directory).unwrap();
        }
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            navigations: Mutex::new(VecDeque::from([navigation(Some(RepositoryWorktree {
                path: existing.to_string_lossy().into_owned(),
                head: Some("a".repeat(40)),
                branch: Some("refs/heads/feature/safe".to_owned()),
                detached: false,
                bare: false,
                locked: false,
                lock_reason: None,
                prunable: false,
                prunable_reason: None,
            }))])),
            added: Mutex::new(Vec::new()),
        });

        let result = runtime
            .prepare_branch_worktree(
                &repository,
                &data,
                "refs/heads/feature/safe",
                &"a".repeat(40),
            )
            .unwrap();

        assert!(!result.created);
        assert_eq!(result.path, existing.canonicalize().unwrap());
        assert!(runtime.executor.added.lock().unwrap().is_empty());
    }

    #[test]
    fn creates_only_inside_the_managed_data_directory_and_revalidates() {
        let fixture = tempdir().unwrap();
        let repository = fixture.path().join("repository");
        let data = fixture.path().join("data");
        fs::create_dir(&repository).unwrap();
        fs::create_dir(&data).unwrap();
        let expected_destination = managed_destination(
            &repository.canonicalize().unwrap(),
            &data,
            "refs/heads/feature/safe",
            &"a".repeat(40),
        )
        .unwrap();
        let created_worktree = RepositoryWorktree {
            path: expected_destination.to_string_lossy().into_owned(),
            head: Some("a".repeat(40)),
            branch: Some("refs/heads/feature/safe".to_owned()),
            detached: false,
            bare: false,
            locked: false,
            lock_reason: None,
            prunable: false,
            prunable_reason: None,
        };
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            navigations: Mutex::new(VecDeque::from([
                navigation(None),
                navigation(Some(created_worktree)),
            ])),
            added: Mutex::new(Vec::new()),
        });

        let result = runtime
            .prepare_branch_worktree(
                &repository,
                &data,
                "refs/heads/feature/safe",
                &"a".repeat(40),
            )
            .unwrap();

        assert!(result.created);
        assert!(result.path.starts_with(data.canonicalize().unwrap()));
        assert_eq!(runtime.executor.added.lock().unwrap().len(), 1);
    }

    #[test]
    fn rejects_remote_refs_and_stale_oids_before_mutating() {
        for (branch_full_name, oid) in [
            ("refs/remotes/origin/main", "a".repeat(40)),
            ("refs/heads/feature/safe", "b".repeat(40)),
        ] {
            let fixture = tempdir().unwrap();
            let repository = fixture.path().join("repository");
            let data = fixture.path().join("data");
            fs::create_dir(&repository).unwrap();
            fs::create_dir(&data).unwrap();
            let runtime = RepositoryRuntime::new(RecordingExecutor {
                navigations: Mutex::new(VecDeque::from([navigation(None)])),
                added: Mutex::new(Vec::new()),
            });

            assert!(
                runtime
                    .prepare_branch_worktree(&repository, &data, branch_full_name, &oid)
                    .is_err()
            );
            assert!(runtime.executor.added.lock().unwrap().is_empty());
        }
    }
}
