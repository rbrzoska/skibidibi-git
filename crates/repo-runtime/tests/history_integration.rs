use std::{fs, path::Path, process::Command};

use repo_runtime::RepositoryRuntime;
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) {
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
}

#[test]
fn reads_paginated_history_and_commit_file_details_from_git() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init"]);
    git(repository, &["config", "user.name", "History Test"]);
    git(
        repository,
        &["config", "user.email", "history@example.test"],
    );

    let original = (1..=10)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(repository.join("before.txt"), &original).expect("write first file");
    git(repository, &["add", "--", "before.txt"]);
    git(repository, &["commit", "-m", "first commit"]);

    fs::rename(repository.join("before.txt"), repository.join("after.txt"))
        .expect("rename tracked file");
    fs::write(
        repository.join("after.txt"),
        format!("{original}one more line\n"),
    )
    .expect("update renamed file");
    git(repository, &["add", "-A"]);
    git(repository, &["commit", "-m", "second commit"]);

    let runtime = RepositoryRuntime::default();
    let first_page = runtime
        .history_page(repository, 1, None)
        .expect("first history page");
    assert_eq!(first_page.commits.len(), 1);
    assert_eq!(first_page.commits[0].summary, "second commit");
    let selected_oid = first_page.commits[0].oid.clone();
    let cursor = first_page.next_cursor.expect("continuation cursor");

    let second_page = runtime
        .history_page(repository, 1, Some(&cursor))
        .expect("second history page");
    assert_eq!(second_page.commits.len(), 1);
    assert_eq!(second_page.commits[0].summary, "first commit");
    assert!(second_page.next_cursor.is_none());

    let details = runtime
        .commit_details(repository, &selected_oid)
        .expect("commit details");
    assert_eq!(details.summary, "second commit");
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].path, "after.txt");
    assert_eq!(details.files[0].old_path.as_deref(), Some("before.txt"));
    assert_eq!(details.files[0].additions, Some(1));
}
