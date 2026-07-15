use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::{
    ChangedFileStatus, StashChangedFile, StashDetails, StashFileDiff, StashFileDiffRequest,
    StashFileSource,
};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, HistoryParseError,
    parse_changed_files,
};
use thiserror::Error;

use crate::RepositoryRuntime;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const METADATA_OUTPUT_LIMIT: usize = 4 * 1024;
const FILE_LIST_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const PATCH_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const LITERAL_PATHSPEC_PREFIX: &str = ":(literal)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashInspectionQuery {
    Metadata {
        oid: String,
    },
    Names {
        snapshot_oid: String,
    },
    Numbers {
        snapshot_oid: String,
    },
    FileDiff {
        snapshot_oid: String,
        path: String,
        old_path: Option<String>,
    },
}

impl StashInspectionQuery {
    fn arguments(&self) -> Vec<OsString> {
        let values = match self {
            Self::Metadata { oid } => vec![
                "show".to_owned(),
                "-s".to_owned(),
                "-z".to_owned(),
                "--format=%H%x00%P%x00%T".to_owned(),
                oid.clone(),
                "--".to_owned(),
            ],
            Self::Names { snapshot_oid } => file_list_arguments(snapshot_oid, "--name-status"),
            Self::Numbers { snapshot_oid } => file_list_arguments(snapshot_oid, "--numstat"),
            Self::FileDiff {
                snapshot_oid,
                path,
                old_path,
            } => {
                let mut values = vec![
                    "show".to_owned(),
                    "--format=".to_owned(),
                    "--first-parent".to_owned(),
                    "--no-ext-diff".to_owned(),
                    "--no-textconv".to_owned(),
                    "--no-color".to_owned(),
                    "-M".to_owned(),
                    "--unified=2147483647".to_owned(),
                    snapshot_oid.clone(),
                    "--".to_owned(),
                ];
                if let Some(old_path) = old_path {
                    values.push(format!("{LITERAL_PATHSPEC_PREFIX}{old_path}"));
                }
                values.push(format!("{LITERAL_PATHSPEC_PREFIX}{path}"));
                values
            }
        };
        values.into_iter().map(OsString::from).collect()
    }

    fn stdout_limit(&self) -> usize {
        match self {
            Self::Metadata { .. } => METADATA_OUTPUT_LIMIT,
            Self::Names { .. } | Self::Numbers { .. } => FILE_LIST_OUTPUT_LIMIT,
            Self::FileDiff { .. } => PATCH_OUTPUT_LIMIT,
        }
    }
}

fn file_list_arguments(snapshot_oid: &str, mode: &str) -> Vec<String> {
    vec![
        "show".to_owned(),
        "--format=".to_owned(),
        "--first-parent".to_owned(),
        "--no-ext-diff".to_owned(),
        "--no-textconv".to_owned(),
        mode.to_owned(),
        "-z".to_owned(),
        "-M".to_owned(),
        snapshot_oid.to_owned(),
        "--".to_owned(),
    ]
}

pub trait StashInspectionGitExecutor: Send + Sync {
    fn execute_stash_inspection(
        &self,
        repository: &Path,
        query: StashInspectionQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl StashInspectionGitExecutor for GitRunner {
    fn execute_stash_inspection(
        &self,
        repository: &Path,
        query: StashInspectionQuery,
    ) -> Result<GitOutput, GitRunError> {
        let stdout_limit = query.stdout_limit();
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(stdout_limit, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum StashInspectionError {
    #[error("invalid stash object id")]
    InvalidObjectId,
    #[error("file path must be non-empty, bounded, and losslessly representable as UTF-8")]
    InvalidPath,
    #[error("the selected object is not a supported two- or three-parent stash commit")]
    InvalidStashStructure,
    #[error("the stash's untracked snapshot is not a root commit")]
    InvalidUntrackedSnapshot,
    #[error("untracked stash files cannot have an old path")]
    InvalidUntrackedPath,
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    Parse(#[from] HistoryParseError),
}

impl<E: StashInspectionGitExecutor> RepositoryRuntime<E> {
    pub fn stash_details(
        &self,
        repository: &Path,
        oid: &str,
    ) -> Result<StashDetails, StashInspectionError> {
        stash_details(&self.executor, repository, oid)
    }

    pub fn stash_file_diff(
        &self,
        repository: &Path,
        request: &StashFileDiffRequest,
    ) -> Result<StashFileDiff, StashInspectionError> {
        stash_file_diff(&self.executor, repository, request)
    }
}

pub fn stash_details<E: StashInspectionGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> Result<StashDetails, StashInspectionError> {
    let oid = validated_oid(oid)?;
    let stash = resolve_stash(executor, repository, &oid)?;
    let mut files = changed_files(executor, repository, &stash.oid, StashFileSource::Tracked)?;
    if let Some(untracked_oid) = stash.untracked_oid {
        files.extend(changed_files(
            executor,
            repository,
            &untracked_oid,
            StashFileSource::Untracked,
        )?);
    }
    Ok(StashDetails { oid, files })
}

pub fn stash_file_diff<E: StashInspectionGitExecutor>(
    executor: &E,
    repository: &Path,
    request: &StashFileDiffRequest,
) -> Result<StashFileDiff, StashInspectionError> {
    let oid = validated_oid(&request.oid)?;
    validate_path(&request.path)?;
    if let Some(old_path) = &request.old_path {
        validate_path(old_path)?;
    }
    if request.source == StashFileSource::Untracked && request.old_path.is_some() {
        return Err(StashInspectionError::InvalidUntrackedPath);
    }

    let stash = resolve_stash(executor, repository, &oid)?;
    let snapshot_oid = match request.source {
        StashFileSource::Tracked => stash.oid,
        StashFileSource::Untracked => stash
            .untracked_oid
            .ok_or(StashInspectionError::InvalidUntrackedSnapshot)?,
    };
    let output = executor.execute_stash_inspection(
        repository,
        StashInspectionQuery::FileDiff {
            snapshot_oid,
            path: request.path.clone(),
            old_path: request
                .old_path
                .as_deref()
                .filter(|old_path| *old_path != request.path)
                .map(str::to_owned),
        },
    )?;
    let patch = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(StashFileDiff {
        oid,
        source: request.source,
        path: request.path.clone(),
        binary: is_binary_patch(&patch),
        patch,
        truncated: false,
    })
}

struct ResolvedStash {
    oid: String,
    untracked_oid: Option<String>,
}

fn resolve_stash<E: StashInspectionGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> Result<ResolvedStash, StashInspectionError> {
    let metadata = query_metadata(executor, repository, oid)?;
    if metadata.oid != oid || !matches!(metadata.parents.len(), 2 | 3) {
        return Err(StashInspectionError::InvalidStashStructure);
    }
    let untracked_oid = metadata.parents.get(2).cloned();
    if let Some(untracked_oid) = &untracked_oid {
        let untracked = query_metadata(executor, repository, untracked_oid)?;
        if untracked.oid != *untracked_oid || !untracked.parents.is_empty() {
            return Err(StashInspectionError::InvalidUntrackedSnapshot);
        }
    }
    Ok(ResolvedStash {
        oid: metadata.oid,
        untracked_oid,
    })
}

fn changed_files<E: StashInspectionGitExecutor>(
    executor: &E,
    repository: &Path,
    snapshot_oid: &str,
    source: StashFileSource,
) -> Result<Vec<StashChangedFile>, StashInspectionError> {
    let names = executor.execute_stash_inspection(
        repository,
        StashInspectionQuery::Names {
            snapshot_oid: snapshot_oid.to_owned(),
        },
    )?;
    let numbers = executor.execute_stash_inspection(
        repository,
        StashInspectionQuery::Numbers {
            snapshot_oid: snapshot_oid.to_owned(),
        },
    )?;
    parse_changed_files(&names.stdout, &numbers.stdout)?
        .into_iter()
        .map(|file| {
            if source == StashFileSource::Untracked
                && (file.status != ChangedFileStatus::Added || file.old_path.is_some())
            {
                return Err(StashInspectionError::InvalidUntrackedSnapshot);
            }
            Ok(StashChangedFile {
                source,
                status: file.status,
                path: file.path,
                old_path: file.old_path,
                additions: file.additions,
                deletions: file.deletions,
                binary: file.binary,
            })
        })
        .collect()
}

struct CommitMetadata {
    oid: String,
    parents: Vec<String>,
    _tree: String,
}

fn query_metadata<E: StashInspectionGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> Result<CommitMetadata, StashInspectionError> {
    let output = executor.execute_stash_inspection(
        repository,
        StashInspectionQuery::Metadata {
            oid: oid.to_owned(),
        },
    )?;
    parse_metadata(&output.stdout)
}

fn parse_metadata(output: &[u8]) -> Result<CommitMetadata, StashInspectionError> {
    if !output.ends_with(&[0]) {
        return Err(StashInspectionError::InvalidStashStructure);
    }
    let fields = output[..output.len() - 1]
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    if fields.len() != 3 {
        return Err(StashInspectionError::InvalidStashStructure);
    }
    let oid = validated_oid(text(fields[0])?)?;
    let parents = text(fields[1])?
        .split_whitespace()
        .map(validated_oid)
        .collect::<Result<Vec<_>, _>>()?;
    let tree = validated_oid(text(fields[2])?)?;
    Ok(CommitMetadata {
        oid,
        parents,
        _tree: tree,
    })
}

fn text(value: &[u8]) -> Result<&str, StashInspectionError> {
    std::str::from_utf8(value).map_err(|_| StashInspectionError::InvalidStashStructure)
}

fn validated_oid(value: &str) -> Result<String, StashInspectionError> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(StashInspectionError::InvalidObjectId)
    }
}

fn validate_path(path: &str) -> Result<(), StashInspectionError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains(['\0', '\u{fffd}']) {
        Err(StashInspectionError::InvalidPath)
    } else {
        Ok(())
    }
}

fn is_binary_patch(patch: &str) -> bool {
    patch.lines().any(|line| {
        line == "GIT binary patch"
            || (line.starts_with("Binary files ") && line.ends_with(" differ"))
            || line.starts_with("Binary file ")
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    const OID: &str = "0123456789012345678901234567890123456789";
    const BASE: &str = "1111111111111111111111111111111111111111";
    const INDEX: &str = "2222222222222222222222222222222222222222";
    const TREE: &str = "3333333333333333333333333333333333333333";

    struct RecordingExecutor {
        queries: Mutex<Vec<StashInspectionQuery>>,
        outputs: Mutex<VecDeque<Vec<u8>>>,
    }

    impl StashInspectionGitExecutor for RecordingExecutor {
        fn execute_stash_inspection(
            &self,
            _repository: &Path,
            query: StashInspectionQuery,
        ) -> Result<GitOutput, GitRunError> {
            self.queries.lock().unwrap().push(query);
            Ok(GitOutput {
                stdout: self.outputs.lock().unwrap().pop_front().unwrap(),
                stderr: Vec::new(),
            })
        }
    }

    fn metadata(oid: &str, parents: &str) -> Vec<u8> {
        format!("{oid}\0{parents}\0{TREE}\0").into_bytes()
    }

    #[test]
    fn details_use_first_parent_file_queries_for_a_two_parent_stash() {
        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(VecDeque::from([
                metadata(OID, &format!("{BASE} {INDEX}")),
                b"R100\0old name\0new name\0".to_vec(),
                b"2\t1\t\0old name\0new name\0".to_vec(),
            ])),
        };

        let details = stash_details(&executor, Path::new("/repo"), OID).unwrap();

        assert_eq!(details.files[0].source, StashFileSource::Tracked);
        assert_eq!(details.files[0].old_path.as_deref(), Some("old name"));
        assert_eq!(
            executor.queries.lock().unwrap().as_slice(),
            &[
                StashInspectionQuery::Metadata {
                    oid: OID.to_owned()
                },
                StashInspectionQuery::Names {
                    snapshot_oid: OID.to_owned()
                },
                StashInspectionQuery::Numbers {
                    snapshot_oid: OID.to_owned()
                }
            ]
        );
    }

    #[test]
    fn file_diff_uses_literal_old_and_new_paths_and_rejects_untracked_rename() {
        let query = StashInspectionQuery::FileDiff {
            snapshot_oid: OID.to_owned(),
            path: "--output=/tmp/pwn :(glob)*".to_owned(),
            old_path: Some("old name".to_owned()),
        };
        let arguments = query
            .arguments()
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "show",
                "--format=",
                "--first-parent",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                OID,
                "--",
                ":(literal)old name",
                ":(literal)--output=/tmp/pwn :(glob)*",
            ]
        );

        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(VecDeque::new()),
        };
        let error = stash_file_diff(
            &executor,
            Path::new("/repo"),
            &StashFileDiffRequest {
                oid: OID.to_owned(),
                source: StashFileSource::Untracked,
                path: "new".to_owned(),
                old_path: Some("old".to_owned()),
            },
        )
        .unwrap_err();
        assert!(matches!(error, StashInspectionError::InvalidUntrackedPath));
        assert!(executor.queries.lock().unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_oids_paths_and_non_root_third_parent_before_file_queries() {
        let third = "4444444444444444444444444444444444444444";
        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(VecDeque::from([
                metadata(OID, &format!("{BASE} {INDEX} {third}")),
                metadata(third, BASE),
            ])),
        };
        assert!(matches!(
            stash_details(&executor, Path::new("/repo"), OID),
            Err(StashInspectionError::InvalidUntrackedSnapshot)
        ));
        assert!(matches!(
            stash_details(&executor, Path::new("/repo"), "short"),
            Err(StashInspectionError::InvalidObjectId)
        ));

        for path in ["", "bad\0path", "bad\u{fffd}path"] {
            let empty = RecordingExecutor {
                queries: Mutex::new(Vec::new()),
                outputs: Mutex::new(VecDeque::new()),
            };
            assert!(matches!(
                stash_file_diff(
                    &empty,
                    Path::new("/repo"),
                    &StashFileDiffRequest {
                        oid: OID.to_owned(),
                        source: StashFileSource::Tracked,
                        path: path.to_owned(),
                        old_path: None,
                    }
                ),
                Err(StashInspectionError::InvalidPath)
            ));
        }
    }
}
