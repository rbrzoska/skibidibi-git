use std::{fs, path::Path, process::Command};

use app_domain::ChangedFileStatus;
use repo_runtime::{RefComparisonError, RepositoryRuntime};
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should be installed for integration tests");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn write(repository: &Path, path: &str, contents: &[u8]) {
    fs::write(repository.join(path), contents).expect("write fixture file");
}

#[test]
fn compares_diverged_exact_refs_and_their_immutable_file_snapshot() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Compare Test"]);
    git(
        repository,
        &["config", "user.email", "compare@example.test"],
    );

    write(repository, "modified.txt", b"base\n");
    write(repository, "deleted.txt", b"delete me\n");
    write(
        repository,
        "rename-old.txt",
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n",
    );
    write(repository, "binary.bin", &[0, 1, 2, 3]);
    git(repository, &["add", "--", "."]);
    git(repository, &["commit", "-m", "base"]);
    let merge_base = git(repository, &["rev-parse", "HEAD"]);
    git(repository, &["branch", "release"]);

    git(repository, &["switch", "-c", "feature/task"]);
    write(repository, "modified.txt", b"base\nsource\n");
    fs::remove_file(repository.join("deleted.txt")).expect("delete fixture file");
    git(repository, &["mv", "rename-old.txt", "rename-new.txt"]);
    write(
        repository,
        "rename-new.txt",
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nrenamed\n",
    );
    write(repository, "added.txt", b"added\n");
    write(repository, "binary.bin", &[0, 9, 8, 7]);
    write(repository, "--output=not-created", b"literal path\n");
    git(repository, &["add", "--", "."]);
    git(repository, &["commit", "-m", "source changes"]);
    write(repository, "second.txt", b"second source commit\n");
    git(repository, &["add", "--", "second.txt"]);
    git(repository, &["commit", "-m", "source follow-up"]);
    let source_oid = git(repository, &["rev-parse", "HEAD"]);
    git(
        repository,
        &[
            "update-ref",
            "refs/remotes/origin/feature/task",
            &source_oid,
        ],
    );

    git(repository, &["switch", "release"]);
    write(repository, "target.txt", b"target only\n");
    git(repository, &["add", "--", "target.txt"]);
    git(repository, &["commit", "-m", "target change"]);
    let target_oid = git(repository, &["rev-parse", "HEAD"]);

    let runtime = RepositoryRuntime::default();
    let comparison = runtime
        .compare_refs(
            repository,
            "refs/remotes/origin/feature/task",
            &source_oid,
            "refs/heads/release",
            &target_oid,
        )
        .expect("ref comparison");

    assert_eq!(comparison.source_oid, source_oid);
    assert_eq!(comparison.target_oid, target_oid);
    assert_eq!(comparison.merge_base_oid, merge_base);
    assert_eq!((comparison.ahead, comparison.behind), (2, 1));
    assert_eq!(comparison.commits.len(), 2);
    assert!(!comparison.commits_truncated);
    assert!(!comparison.files_truncated);
    assert!(
        comparison.files.iter().any(|file| {
            file.path == "modified.txt" && file.status == ChangedFileStatus::Modified
        })
    );
    assert!(
        comparison
            .files
            .iter()
            .any(|file| file.path == "deleted.txt" && file.status == ChangedFileStatus::Deleted)
    );
    assert!(comparison.files.iter().any(|file| {
        file.path == "rename-new.txt"
            && file.old_path.as_deref() == Some("rename-old.txt")
            && file.status == ChangedFileStatus::Renamed
    }));
    assert!(
        comparison
            .files
            .iter()
            .any(|file| file.path == "added.txt" && file.status == ChangedFileStatus::Added)
    );
    assert!(
        comparison
            .files
            .iter()
            .any(|file| file.path == "binary.bin" && file.binary)
    );

    let diff = runtime
        .compare_ref_file_diff(
            repository,
            "refs/remotes/origin/feature/task",
            &source_oid,
            "refs/heads/release",
            &target_oid,
            "--output=not-created",
            None,
        )
        .expect("literal-path ref diff");
    assert!(diff.patch.contains("literal path"));
    assert_eq!(diff.source_oid, source_oid);
    assert_eq!(diff.target_oid, target_oid);
    assert_eq!(diff.old_path, None);
    assert!(!diff.binary);
    assert!(!diff.truncated);

    let renamed_diff = runtime
        .compare_ref_file_diff(
            repository,
            "refs/remotes/origin/feature/task",
            &source_oid,
            "refs/heads/release",
            &target_oid,
            "rename-new.txt",
            Some("rename-old.txt"),
        )
        .expect("renamed ref diff");
    assert_eq!(renamed_diff.path, "rename-new.txt");
    assert_eq!(renamed_diff.old_path.as_deref(), Some("rename-old.txt"));
}

#[test]
fn rejects_stale_or_symbolic_refs_before_returning_a_comparison() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Compare Test"]);
    git(
        repository,
        &["config", "user.email", "compare@example.test"],
    );
    write(repository, "base.txt", b"base\n");
    git(repository, &["add", "--", "base.txt"]);
    git(repository, &["commit", "-m", "base"]);
    let oid = git(repository, &["rev-parse", "HEAD"]);
    git(repository, &["branch", "target"]);
    git(
        repository,
        &["update-ref", "refs/remotes/origin/source", &oid],
    );

    let runtime = RepositoryRuntime::default();
    let stale = runtime
        .compare_refs(
            repository,
            "refs/remotes/origin/source",
            &"f".repeat(40),
            "refs/heads/target",
            &oid,
        )
        .expect_err("stale source must fail");
    assert!(matches!(stale, RefComparisonError::SourceMoved));

    git(
        repository,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/source",
        ],
    );
    let symbolic = runtime
        .compare_refs(
            repository,
            "refs/remotes/origin/HEAD",
            &oid,
            "refs/heads/target",
            &oid,
        )
        .expect_err("symbolic source must fail");
    assert!(matches!(symbolic, RefComparisonError::SourceMoved));
}
