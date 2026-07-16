use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

const MAX_SCAN_ENTRIES: usize = 2_000_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepositoryMaintenanceStatisticsResponse {
    repository_bytes: u64,
    git_bytes: u64,
    worktree_bytes: u64,
    scanned_at: i64,
    repository_last_commit_at: Option<String>,
    worktrees: Vec<RepositoryWorktreeStatisticsResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryWorktreeStatisticsResponse {
    path: String,
    branch: Option<String>,
    bytes: u64,
    last_commit_at: Option<String>,
    last_opened_at: Option<i64>,
}

pub(crate) fn scan_repository_maintenance(
    repository: &Path,
    repository_last_commit_at: Option<String>,
    worktrees: impl IntoIterator<Item = (String, Option<String>, Option<String>)>,
    last_opened_by_path: &HashMap<PathBuf, i64>,
    scanned_at: i64,
) -> Result<RepositoryMaintenanceStatisticsResponse, String> {
    let git_directory = common_git_directory(repository)?;
    let mut remaining_entries = MAX_SCAN_ENTRIES;
    let git_bytes = directory_size(&git_directory, &mut remaining_entries, false)?;
    let mut seen = HashSet::new();
    let mut worktree_responses = Vec::new();
    let mut worktree_bytes = 0_u64;

    for (path, branch, last_commit_at) in worktrees {
        let canonical = fs::canonicalize(&path)
            .map_err(|error| format!("worktree path could not be read: {error}"))?;
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let bytes = directory_size(&canonical, &mut remaining_entries, true)?;
        worktree_bytes = worktree_bytes
            .checked_add(bytes)
            .ok_or_else(|| "repository size exceeded the supported range".to_owned())?;
        worktree_responses.push(RepositoryWorktreeStatisticsResponse {
            path,
            branch,
            bytes,
            last_commit_at,
            last_opened_at: last_opened_by_path.get(&canonical).copied(),
        });
    }

    let repository_bytes = git_bytes
        .checked_add(worktree_bytes)
        .ok_or_else(|| "repository size exceeded the supported range".to_owned())?;
    Ok(RepositoryMaintenanceStatisticsResponse {
        repository_bytes,
        git_bytes,
        worktree_bytes,
        scanned_at,
        repository_last_commit_at,
        worktrees: worktree_responses,
    })
}

fn directory_size(
    root: &Path,
    remaining_entries: &mut usize,
    skip_root_git: bool,
) -> Result<u64, String> {
    let mut size = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("directory could not be scanned: {error}"))?
        {
            if *remaining_entries == 0 {
                return Err("repository scan exceeded the file-count safety limit".to_owned());
            }
            *remaining_entries -= 1;
            let entry =
                entry.map_err(|error| format!("directory entry could not be read: {error}"))?;
            if skip_root_git && directory == root && entry.file_name() == ".git" {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("file metadata could not be read: {error}"))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                size = size
                    .checked_add(metadata.len())
                    .ok_or_else(|| "repository size exceeded the supported range".to_owned())?;
            }
        }
    }
    Ok(size)
}

fn common_git_directory(repository: &Path) -> Result<PathBuf, String> {
    let dot_git = repository.join(".git");
    if dot_git.is_dir() {
        return fs::canonicalize(dot_git)
            .map_err(|error| format!("Git directory could not be resolved: {error}"));
    }
    let marker = fs::read_to_string(&dot_git)
        .map_err(|error| format!("Git directory marker could not be read: {error}"))?;
    let git_dir_value = marker
        .strip_prefix("gitdir:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Git directory marker is invalid".to_owned())?;
    let git_dir = resolve_relative(repository, git_dir_value);
    let common_marker = git_dir.join("commondir");
    let common = match fs::read_to_string(&common_marker) {
        Ok(value) => resolve_relative(&git_dir, value.trim()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => git_dir,
        Err(error) => {
            return Err(format!(
                "Git common directory marker could not be read: {error}"
            ));
        }
    };
    fs::canonicalize(common)
        .map_err(|error| format!("Git common directory could not be resolved: {error}"))
}

fn resolve_relative(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_worktree_files_without_following_symlinks_or_counting_git_twice() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();
        fs::write(root.path().join(".git/object"), b"git").unwrap();
        fs::write(root.path().join("file"), b"worktree").unwrap();
        let stats = scan_repository_maintenance(
            root.path(),
            Some("2026-01-01T00:00:00+00:00".to_owned()),
            [(
                root.path().to_string_lossy().into_owned(),
                Some("refs/heads/main".to_owned()),
                Some("2026-01-01T00:00:00+00:00".to_owned()),
            )],
            &HashMap::new(),
            10,
        )
        .unwrap();

        assert_eq!(stats.git_bytes, 3);
        assert_eq!(stats.worktree_bytes, 8);
        assert_eq!(stats.repository_bytes, 11);
        assert_eq!(stats.worktrees.len(), 1);
    }
}
