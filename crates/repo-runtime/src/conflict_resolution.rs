use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
    time::Duration,
};

use app_domain::{
    ConflictFileDetail, ConflictFileDetailRequest, ConflictFileSummary, ConflictListResult,
    ConflictResolution, ConflictStageIdentity, ConflictVersion, RepositoryStatePrecondition,
    RepositoryStatus, ResolveConflictRequest, ResolveConflictResult, StatusEntryKind,
};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{MutationGitExecutor, RepositoryRuntime, RepositoryRuntimeError, repository_status};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_CONTENT_BYTES: usize = 8 * 1024 * 1024;

pub trait ConflictResolutionGitExecutor: Send + Sync {
    fn query_index_stages(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn read_blob(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn checkout_side(
        &self,
        repository: &Path,
        path: &str,
        ours: bool,
    ) -> Result<GitOutput, GitRunError>;
    fn remove_path(&self, repository: &Path, path: &str) -> Result<GitOutput, GitRunError>;
}

impl ConflictResolutionGitExecutor for GitRunner {
    fn query_index_stages(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, ["ls-files", "--stage", "-z"])
                .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn read_blob(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, ["cat-file", "blob", oid])
                .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn checkout_side(
        &self,
        repository: &Path,
        path: &str,
        ours: bool,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    "checkout",
                    if ours { "--ours" } else { "--theirs" },
                    "--",
                    path,
                ],
            )
            .with_output_limits(STDERR_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn remove_path(&self, repository: &Path, path: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["rm", "--ignore-unmatch", "--", path],
            )
            .with_output_limits(STDERR_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum ConflictResolutionError {
    #[error("the conflict request is invalid")]
    InvalidRequest,
    #[error("the conflict changed; refresh before retrying")]
    StaleConflict,
    #[error("the requested path is not currently conflicted")]
    ConflictNotFound,
    #[error("binary or non-file conflicts require choosing ours, theirs, or delete")]
    ContentResolutionUnavailable,
    #[error("the conflict path is unsafe or escapes the worktree")]
    UnsafePath,
    #[error("conflict data returned by Git is malformed")]
    InvalidOutput,
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ConflictResolutionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::InvalidOutput => "invalidRequest",
            Self::StaleConflict => "staleState",
            Self::ConflictNotFound => "conflictNotFound",
            Self::ContentResolutionUnavailable => "contentResolutionUnavailable",
            Self::UnsafePath => "unsafePath",
            Self::Repository(_) | Self::Git(_) | Self::Io(_) => "gitRejected",
        }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: ConflictResolutionGitExecutor + MutationGitExecutor,
{
    pub fn conflicts(
        &self,
        repository: &Path,
    ) -> Result<ConflictListResult, ConflictResolutionError> {
        let status = repository_status(&self.executor, repository)?;
        let files = conflict_files(&self.executor, repository)?;
        Ok(ConflictListResult { files, status })
    }

    pub fn conflict_detail(
        &self,
        repository: &Path,
        request: &ConflictFileDetailRequest,
    ) -> Result<ConflictFileDetail, ConflictResolutionError> {
        validate_path(&request.path)?;
        let current = exact_conflict(&self.executor, repository, &request.path)?;
        validate_expected(
            &current,
            request.expected_base.as_ref(),
            request.expected_ours.as_ref(),
            request.expected_theirs.as_ref(),
        )?;
        let base = load_version(&self.executor, repository, current.base.clone())?;
        let ours = load_version(&self.executor, repository, current.ours.clone())?;
        let theirs = load_version(&self.executor, repository, current.theirs.clone())?;
        let (working_content, working_binary) = read_working_file(repository, &request.path)?;
        Ok(ConflictFileDetail {
            path: request.path.clone(),
            base,
            ours,
            theirs,
            working_content,
            working_binary,
        })
    }

    pub fn resolve_conflict(
        &self,
        repository: &Path,
        request: &ResolveConflictRequest,
    ) -> Result<ResolveConflictResult, ConflictResolutionError> {
        validate_path(&request.path)?;
        let before = repository_status(&self.executor, repository)?;
        validate_precondition(&before, &request.precondition)?;
        let current = exact_conflict(&self.executor, repository, &request.path)?;
        validate_expected(
            &current,
            request.expected_base.as_ref(),
            request.expected_ours.as_ref(),
            request.expected_theirs.as_ref(),
        )?;

        let mutation = match &request.resolution {
            ConflictResolution::Content { content } => {
                if content.len() > MAX_CONTENT_BYTES
                    || content.contains('\0')
                    || !content_resolution_allowed(&current)
                    || conflict_contains_binary(&self.executor, repository, &current)?
                {
                    return Err(ConflictResolutionError::ContentResolutionUnavailable);
                }
                write_working_file(repository, &request.path, content.as_bytes())?;
                self.executor
                    .stage(repository, false, vec![request.path.clone()])
            }
            ConflictResolution::Ours => {
                if current.ours.is_none() {
                    self.executor.remove_path(repository, &request.path)
                } else {
                    self.executor
                        .checkout_side(repository, &request.path, true)
                        .and_then(|_| {
                            self.executor
                                .stage(repository, false, vec![request.path.clone()])
                        })
                }
            }
            ConflictResolution::Theirs => {
                if current.theirs.is_none() {
                    self.executor.remove_path(repository, &request.path)
                } else {
                    self.executor
                        .checkout_side(repository, &request.path, false)
                        .and_then(|_| {
                            self.executor
                                .stage(repository, false, vec![request.path.clone()])
                        })
                }
            }
            ConflictResolution::Delete => self.executor.remove_path(repository, &request.path),
        };

        let status = repository_status(&self.executor, repository).ok();
        let still_conflicted = status.as_ref().is_some_and(|status| {
            status
                .entries
                .iter()
                .any(|entry| entry.kind == StatusEntryKind::Unmerged && entry.path == request.path)
        });
        match mutation {
            Ok(_) if !still_conflicted && status.is_some() => Ok(ResolveConflictResult {
                resolved: true,
                status,
                error_message: None,
                mutation_may_have_occurred: true,
            }),
            Ok(_) => Ok(ResolveConflictResult {
                resolved: false,
                status,
                error_message: Some(
                    "the path remains conflicted after staging; refresh before retrying".to_owned(),
                ),
                mutation_may_have_occurred: true,
            }),
            Err(error) => Ok(ResolveConflictResult {
                resolved: false,
                status,
                error_message: Some(error.to_string()),
                mutation_may_have_occurred: true,
            }),
        }
    }
}

fn conflict_files<E: ConflictResolutionGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<Vec<ConflictFileSummary>, ConflictResolutionError> {
    let output = executor.query_index_stages(repository)?;
    parse_conflict_stages(&output.stdout)
}

fn parse_conflict_stages(
    input: &[u8],
) -> Result<Vec<ConflictFileSummary>, ConflictResolutionError> {
    let mut grouped: BTreeMap<String, ConflictFileSummary> = BTreeMap::new();
    for record in input
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(ConflictResolutionError::InvalidOutput)?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| ConflictResolutionError::InvalidOutput)?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| ConflictResolutionError::InvalidOutput)?
            .to_owned();
        validate_path(&path)?;
        let fields = metadata.split(' ').collect::<Vec<_>>();
        let [mode, oid, stage] = fields.as_slice() else {
            return Err(ConflictResolutionError::InvalidOutput);
        };
        if !valid_mode(mode) || !valid_oid(oid) {
            return Err(ConflictResolutionError::InvalidOutput);
        }
        let stage: u8 = stage
            .parse()
            .map_err(|_| ConflictResolutionError::InvalidOutput)?;
        if stage == 0 {
            continue;
        }
        let identity = ConflictStageIdentity {
            oid: (*oid).to_owned(),
            mode: (*mode).to_owned(),
        };
        let summary = grouped
            .entry(path.clone())
            .or_insert_with(|| ConflictFileSummary {
                path,
                base: None,
                ours: None,
                theirs: None,
            });
        let slot = match stage {
            1 => &mut summary.base,
            2 => &mut summary.ours,
            3 => &mut summary.theirs,
            _ => return Err(ConflictResolutionError::InvalidOutput),
        };
        if slot.replace(identity).is_some() {
            return Err(ConflictResolutionError::InvalidOutput);
        }
    }
    Ok(grouped.into_values().collect())
}

fn exact_conflict<E: ConflictResolutionGitExecutor>(
    executor: &E,
    repository: &Path,
    path: &str,
) -> Result<ConflictFileSummary, ConflictResolutionError> {
    conflict_files(executor, repository)?
        .into_iter()
        .find(|file| file.path == path)
        .ok_or(ConflictResolutionError::ConflictNotFound)
}

fn validate_expected(
    current: &ConflictFileSummary,
    base: Option<&ConflictStageIdentity>,
    ours: Option<&ConflictStageIdentity>,
    theirs: Option<&ConflictStageIdentity>,
) -> Result<(), ConflictResolutionError> {
    if current.base.as_ref() != base
        || current.ours.as_ref() != ours
        || current.theirs.as_ref() != theirs
    {
        Err(ConflictResolutionError::StaleConflict)
    } else {
        Ok(())
    }
}

fn load_version<E: ConflictResolutionGitExecutor>(
    executor: &E,
    repository: &Path,
    identity: Option<ConflictStageIdentity>,
) -> Result<ConflictVersion, ConflictResolutionError> {
    let Some(identity) = identity else {
        return Ok(ConflictVersion {
            identity: None,
            content: None,
            binary: false,
        });
    };
    if !regular_blob_mode(&identity.mode) {
        return Ok(ConflictVersion {
            identity: Some(identity),
            content: None,
            binary: true,
        });
    }
    let output = executor.read_blob(repository, &identity.oid)?;
    let binary = output.stdout.contains(&0) || std::str::from_utf8(&output.stdout).is_err();
    let content = (!binary).then(|| String::from_utf8(output.stdout).expect("validated UTF-8"));
    Ok(ConflictVersion {
        identity: Some(identity),
        content,
        binary,
    })
}

fn read_working_file(
    repository: &Path,
    path: &str,
) -> Result<(Option<String>, bool), ConflictResolutionError> {
    let target = safe_target(repository, path)?;
    if !target.exists() {
        return Ok((None, false));
    }
    let metadata = fs::symlink_metadata(&target)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_CONTENT_BYTES as u64
    {
        return Ok((None, true));
    }
    let bytes = fs::read(target)?;
    let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
    Ok((
        (!binary).then(|| String::from_utf8(bytes).expect("validated UTF-8")),
        binary,
    ))
}

fn write_working_file(
    repository: &Path,
    path: &str,
    content: &[u8],
) -> Result<(), ConflictResolutionError> {
    let target = safe_target(repository, path)?;
    if target.exists() {
        let metadata = fs::symlink_metadata(&target)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ConflictResolutionError::UnsafePath);
        }
    }
    fs::write(target, content)?;
    Ok(())
}

fn safe_target(
    repository: &Path,
    path: &str,
) -> Result<std::path::PathBuf, ConflictResolutionError> {
    validate_path(path)?;
    let relative = Path::new(path);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(ConflictResolutionError::UnsafePath);
    }
    let root = repository.canonicalize()?;
    let target = root.join(relative);
    let parent = target
        .parent()
        .ok_or(ConflictResolutionError::UnsafePath)?
        .canonicalize()?;
    if !parent.starts_with(&root) {
        return Err(ConflictResolutionError::UnsafePath);
    }
    Ok(target)
}

fn validate_precondition(
    status: &RepositoryStatus,
    expected: &RepositoryStatePrecondition,
) -> Result<(), ConflictResolutionError> {
    if expected.expected_index_fingerprint.is_empty()
        || expected.expected_worktree_fingerprint.is_empty()
        || status.branch.oid != expected.expected_head
        || status.branch.head != expected.expected_head_name
        || status.branch.detached != expected.expected_detached
        || status.branch.unborn != expected.expected_unborn
        || status.index_fingerprint != expected.expected_index_fingerprint
        || status.worktree_fingerprint != expected.expected_worktree_fingerprint
    {
        Err(ConflictResolutionError::StaleConflict)
    } else {
        Ok(())
    }
}

fn content_resolution_allowed(file: &ConflictFileSummary) -> bool {
    [&file.base, &file.ours, &file.theirs]
        .into_iter()
        .flatten()
        .all(|identity| regular_blob_mode(&identity.mode))
}

fn conflict_contains_binary<E: ConflictResolutionGitExecutor>(
    executor: &E,
    repository: &Path,
    file: &ConflictFileSummary,
) -> Result<bool, ConflictResolutionError> {
    for identity in [&file.base, &file.ours, &file.theirs].into_iter().flatten() {
        if load_version(executor, repository, Some(identity.clone()))?.binary {
            return Ok(true);
        }
    }
    Ok(false)
}

fn regular_blob_mode(mode: &str) -> bool {
    matches!(mode, "100644" | "100755")
}
fn valid_mode(mode: &str) -> bool {
    matches!(mode, "100644" | "100755" | "120000" | "160000")
}
fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn validate_path(path: &str) -> Result<(), ConflictResolutionError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains(['\0', '\u{fffd}']) {
        Err(ConflictResolutionError::InvalidRequest)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_index_stages_and_deleted_side() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        let input = format!(
            "100644 {a} 1\tfile.txt\0100644 {b} 2\tfile.txt\0100644 {c} 3\tfile.txt\0100644 {a} 0\tclean.txt\0"
        );
        let files = parse_conflict_stages(input.as_bytes()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].ours.as_ref().unwrap().oid, b);
        assert_eq!(files[0].theirs.as_ref().unwrap().oid, c);
    }

    #[test]
    fn rejects_duplicate_stage_and_unsafe_path() {
        let oid = "a".repeat(40);
        let duplicate = format!("100644 {oid} 2\tfile\0100644 {oid} 2\tfile\0");
        assert!(parse_conflict_stages(duplicate.as_bytes()).is_err());
        assert!(validate_path("../escape").is_ok());
        assert!(matches!(
            safe_target(Path::new("/tmp"), "../escape"),
            Err(ConflictResolutionError::UnsafePath)
        ));
    }
}
