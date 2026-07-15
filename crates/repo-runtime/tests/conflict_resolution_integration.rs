use std::{fs, path::Path, process::Command};

use app_domain::{
    ConflictFileDetailRequest, ConflictResolution, RepositoryStatePrecondition,
    ResolveConflictRequest,
};
use repo_runtime::{ConflictResolutionError, RepositoryRuntime};
use tempfile::tempdir;

#[test]
fn loads_three_versions_and_stages_edited_text_resolution() {
    let directory = text_conflict();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    let list = runtime.conflicts(repository).unwrap();
    assert_eq!(list.files.len(), 1);
    let file = list.files[0].clone();
    let detail = runtime
        .conflict_detail(
            repository,
            &ConflictFileDetailRequest {
                path: file.path.clone(),
                expected_base: file.base.clone(),
                expected_ours: file.ours.clone(),
                expected_theirs: file.theirs.clone(),
            },
        )
        .unwrap();
    assert_eq!(detail.base.content.as_deref(), Some("base\n"));
    assert_eq!(detail.ours.content.as_deref(), Some("feature\n"));
    assert_eq!(detail.theirs.content.as_deref(), Some("main\n"));
    assert!(
        detail
            .working_content
            .as_deref()
            .unwrap()
            .contains("<<<<<<<")
    );

    let result = runtime
        .resolve_conflict(
            repository,
            &ResolveConflictRequest {
                path: file.path,
                expected_base: file.base,
                expected_ours: file.ours,
                expected_theirs: file.theirs,
                resolution: ConflictResolution::Content {
                    content: "combined\n".to_owned(),
                },
                precondition: precondition(&list.status),
            },
        )
        .unwrap();
    assert!(result.resolved);
    assert_eq!(
        fs::read_to_string(repository.join("file.txt")).unwrap(),
        "combined\n"
    );
    assert!(git_output(repository, &["diff", "--cached", "--", "file.txt"]).contains("+combined"));
}

#[test]
fn choose_ours_and_theirs_resolve_exact_index_stages() {
    for (resolution, expected) in [
        (ConflictResolution::Ours, "feature\n"),
        (ConflictResolution::Theirs, "main\n"),
    ] {
        let directory = text_conflict();
        let repository = directory.path();
        let runtime = RepositoryRuntime::default();
        let list = runtime.conflicts(repository).unwrap();
        let file = list.files[0].clone();
        let result = runtime
            .resolve_conflict(
                repository,
                &ResolveConflictRequest {
                    path: file.path,
                    expected_base: file.base,
                    expected_ours: file.ours,
                    expected_theirs: file.theirs,
                    resolution,
                    precondition: precondition(&list.status),
                },
            )
            .unwrap();
        assert!(result.resolved);
        assert_eq!(
            fs::read_to_string(repository.join("file.txt")).unwrap(),
            expected
        );
    }
}

#[test]
fn stale_conflict_identity_is_rejected_before_read_or_mutation() {
    let directory = text_conflict();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    let list = runtime.conflicts(repository).unwrap();
    let file = list.files[0].clone();
    let mut stale_ours = file.ours.clone().unwrap();
    stale_ours.oid = "a".repeat(stale_ours.oid.len());

    let detail_error = runtime
        .conflict_detail(
            repository,
            &ConflictFileDetailRequest {
                path: file.path.clone(),
                expected_base: file.base.clone(),
                expected_ours: Some(stale_ours.clone()),
                expected_theirs: file.theirs.clone(),
            },
        )
        .unwrap_err();
    assert!(matches!(
        detail_error,
        ConflictResolutionError::StaleConflict
    ));

    let before = fs::read(repository.join("file.txt")).unwrap();
    let resolve_error = runtime
        .resolve_conflict(
            repository,
            &ResolveConflictRequest {
                path: file.path,
                expected_base: file.base,
                expected_ours: Some(stale_ours),
                expected_theirs: file.theirs,
                resolution: ConflictResolution::Ours,
                precondition: precondition(&list.status),
            },
        )
        .unwrap_err();
    assert!(matches!(
        resolve_error,
        ConflictResolutionError::StaleConflict
    ));
    assert_eq!(fs::read(repository.join("file.txt")).unwrap(), before);
}

#[test]
fn binary_conflict_rejects_text_but_allows_choose_theirs() {
    let directory = tempdir().unwrap();
    let repository = directory.path();
    init(repository);
    fs::write(repository.join("asset.bin"), [0, 1, 2]).unwrap();
    commit_all(repository, "base");
    git(repository, &["branch", "feature"]);
    fs::write(repository.join("asset.bin"), [0, 3, 2]).unwrap();
    commit_all(repository, "main");
    git(repository, &["switch", "feature"]);
    fs::write(repository.join("asset.bin"), [0, 4, 2]).unwrap();
    commit_all(repository, "feature");
    git_fails(repository, &["merge", "main"]);

    let runtime = RepositoryRuntime::default();
    let list = runtime.conflicts(repository).unwrap();
    let file = list.files[0].clone();
    let detail = runtime
        .conflict_detail(
            repository,
            &ConflictFileDetailRequest {
                path: file.path.clone(),
                expected_base: file.base.clone(),
                expected_ours: file.ours.clone(),
                expected_theirs: file.theirs.clone(),
            },
        )
        .unwrap();
    assert!(detail.ours.binary && detail.theirs.binary);
    let error = runtime
        .resolve_conflict(
            repository,
            &ResolveConflictRequest {
                path: file.path.clone(),
                expected_base: file.base.clone(),
                expected_ours: file.ours.clone(),
                expected_theirs: file.theirs.clone(),
                resolution: ConflictResolution::Content {
                    content: "unsafe".to_owned(),
                },
                precondition: precondition(&list.status),
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ConflictResolutionError::ContentResolutionUnavailable
    ));
    let refreshed = runtime.status(repository).unwrap();
    let result = runtime
        .resolve_conflict(
            repository,
            &ResolveConflictRequest {
                path: file.path,
                expected_base: file.base,
                expected_ours: file.ours,
                expected_theirs: file.theirs,
                resolution: ConflictResolution::Theirs,
                precondition: precondition(&refreshed),
            },
        )
        .unwrap();
    assert!(result.resolved);
    assert_eq!(fs::read(repository.join("asset.bin")).unwrap(), [0, 3, 2]);
}

#[test]
fn deleted_side_can_resolve_by_deleting_the_path() {
    let directory = tempdir().unwrap();
    let repository = directory.path();
    init(repository);
    fs::write(repository.join("file.txt"), "base\n").unwrap();
    commit_all(repository, "base");
    git(repository, &["branch", "feature"]);
    git(repository, &["rm", "--", "file.txt"]);
    git(repository, &["commit", "-m", "delete on main"]);
    git(repository, &["switch", "feature"]);
    fs::write(repository.join("file.txt"), "feature\n").unwrap();
    commit_all(repository, "modify on feature");
    git_fails(repository, &["merge", "main"]);
    let runtime = RepositoryRuntime::default();
    let list = runtime.conflicts(repository).unwrap();
    let file = list.files[0].clone();
    assert!(file.theirs.is_none());
    let result = runtime
        .resolve_conflict(
            repository,
            &ResolveConflictRequest {
                path: file.path,
                expected_base: file.base,
                expected_ours: file.ours,
                expected_theirs: file.theirs,
                resolution: ConflictResolution::Theirs,
                precondition: precondition(&list.status),
            },
        )
        .unwrap();
    assert!(result.resolved);
    assert!(!repository.join("file.txt").exists());
}

fn text_conflict() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    let repository = directory.path();
    init(repository);
    fs::write(repository.join("file.txt"), "base\n").unwrap();
    commit_all(repository, "base");
    git(repository, &["branch", "feature"]);
    fs::write(repository.join("file.txt"), "main\n").unwrap();
    commit_all(repository, "main");
    git(repository, &["switch", "feature"]);
    fs::write(repository.join("file.txt"), "feature\n").unwrap();
    commit_all(repository, "feature");
    git_fails(repository, &["merge", "main"]);
    directory
}

fn init(repository: &Path) {
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Conflict Test"]);
    git(
        repository,
        &["config", "user.email", "conflict@example.test"],
    );
}

fn commit_all(repository: &Path, message: &str) {
    git(repository, &["add", "--all"]);
    git(repository, &["commit", "-m", message]);
}

fn precondition(status: &app_domain::RepositoryStatus) -> RepositoryStatePrecondition {
    RepositoryStatePrecondition {
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
}

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_fails(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "git {args:?} unexpectedly succeeded"
    );
}

fn git_output(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
