use std::{fs, path::Path, process::Command};

use app_domain::{BranchCreationSource, CreateBranchRequest};
use repo_runtime::{BranchCreationError, RepositoryRuntime};

fn git_output(repository: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .expect("run git")
}

fn git(repository: &Path, arguments: &[&str]) {
    let output = git_output(repository, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn output_text(repository: &Path, arguments: &[&str]) -> String {
    String::from_utf8(git_output(repository, arguments).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q", "-b", "main"]);
    git(directory.path(), &["config", "user.name", "Branch Test"]);
    git(
        directory.path(),
        &["config", "user.email", "branch@example.test"],
    );
    fs::write(directory.path().join("tracked.txt"), "base\n").unwrap();
    git(directory.path(), &["add", "tracked.txt"]);
    git(directory.path(), &["commit", "-qm", "base"]);
    directory
}

#[test]
fn creates_a_local_branch_from_the_exact_current_head_without_switching() {
    let repository = repository();
    let head = output_text(repository.path(), &["rev-parse", "HEAD"]);

    let result = RepositoryRuntime::default()
        .create_branch(
            repository.path(),
            &CreateBranchRequest {
                name: "feature/current".to_owned(),
                source: BranchCreationSource::Current {
                    expected_oid: head.clone(),
                },
            },
        )
        .unwrap();

    assert_eq!(result.full_name, "refs/heads/feature/current");
    assert_eq!(result.head, head);
    assert_eq!(result.upstream, None);
    assert_eq!(
        output_text(repository.path(), &["branch", "--show-current"]),
        "main"
    );
    assert_eq!(
        output_text(repository.path(), &["rev-parse", "feature/current"]),
        result.head
    );
}

#[test]
fn creates_from_an_exact_historical_commit_and_rejects_non_commit_objects() {
    let repository = repository();
    let base = output_text(repository.path(), &["rev-parse", "HEAD"]);
    fs::write(repository.path().join("tracked.txt"), "second\n").unwrap();
    git(repository.path(), &["commit", "-qam", "second"]);

    RepositoryRuntime::default()
        .create_branch(
            repository.path(),
            &CreateBranchRequest {
                name: "from/base".to_owned(),
                source: BranchCreationSource::Commit { oid: base.clone() },
            },
        )
        .unwrap();
    assert_eq!(
        output_text(repository.path(), &["rev-parse", "from/base"]),
        base
    );

    let blob = output_text(repository.path(), &["hash-object", "tracked.txt"]);
    let error = RepositoryRuntime::default()
        .create_branch(
            repository.path(),
            &CreateBranchRequest {
                name: "from/blob".to_owned(),
                source: BranchCreationSource::Commit { oid: blob },
            },
        )
        .unwrap_err();
    assert!(matches!(error, BranchCreationError::SourceNotCommit));
    assert!(
        !git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/from/blob"]
        )
        .status
        .success()
    );
}

#[test]
fn creates_an_exact_remote_tracking_branch() {
    let remote = repository();
    let clone_directory = tempfile::tempdir().unwrap();
    git(
        clone_directory.path(),
        &["clone", "-q", remote.path().to_str().unwrap(), "."],
    );
    let repository = clone_directory.path();
    let oid = output_text(repository, &["rev-parse", "refs/remotes/origin/main"]);

    let result = RepositoryRuntime::default()
        .create_branch(
            repository,
            &CreateBranchRequest {
                name: "tracking/main".to_owned(),
                source: BranchCreationSource::RemoteTracking {
                    full_name: "refs/remotes/origin/main".to_owned(),
                    expected_oid: oid.clone(),
                },
            },
        )
        .unwrap();

    assert_eq!(result.head, oid);
    assert_eq!(result.upstream.as_deref(), Some("origin/main"));
    assert_eq!(
        output_text(
            repository,
            &[
                "rev-parse",
                "--symbolic-full-name",
                "tracking/main@{upstream}"
            ]
        ),
        "refs/remotes/origin/main"
    );
    assert_eq!(
        output_text(repository, &["branch", "--show-current"]),
        "main"
    );
}

#[test]
fn rejects_stale_sources_duplicates_and_symbolic_remote_refs() {
    let remote = repository();
    let clone_directory = tempfile::tempdir().unwrap();
    git(
        clone_directory.path(),
        &["clone", "-q", remote.path().to_str().unwrap(), "."],
    );
    let repository = clone_directory.path();
    let head = output_text(repository, &["rev-parse", "HEAD"]);
    let stale = "0".repeat(head.len());
    let runtime = RepositoryRuntime::default();

    let current_error = runtime
        .create_branch(
            repository,
            &CreateBranchRequest {
                name: "stale/current".to_owned(),
                source: BranchCreationSource::Current {
                    expected_oid: stale.clone(),
                },
            },
        )
        .unwrap_err();
    assert!(matches!(
        current_error,
        BranchCreationError::CurrentHeadChanged
    ));

    let remote_error = runtime
        .create_branch(
            repository,
            &CreateBranchRequest {
                name: "stale/remote".to_owned(),
                source: BranchCreationSource::RemoteTracking {
                    full_name: "refs/remotes/origin/main".to_owned(),
                    expected_oid: stale,
                },
            },
        )
        .unwrap_err();
    assert!(matches!(
        remote_error,
        BranchCreationError::RemoteBranchChanged
    ));

    git(repository, &["branch", "duplicate"]);
    let duplicate_error = runtime
        .create_branch(
            repository,
            &CreateBranchRequest {
                name: "duplicate".to_owned(),
                source: BranchCreationSource::Commit { oid: head.clone() },
            },
        )
        .unwrap_err();
    assert!(matches!(
        duplicate_error,
        BranchCreationError::BranchAlreadyExists
    ));

    let symbolic_error = runtime
        .create_branch(
            repository,
            &CreateBranchRequest {
                name: "from/origin-head".to_owned(),
                source: BranchCreationSource::RemoteTracking {
                    full_name: "refs/remotes/origin/HEAD".to_owned(),
                    expected_oid: head,
                },
            },
        )
        .unwrap_err();
    assert!(matches!(
        symbolic_error,
        BranchCreationError::SymbolicRemoteNotAllowed
    ));
}
