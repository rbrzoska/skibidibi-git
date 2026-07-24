use std::{fs, path::Path, process::Command};

use app_domain::FileBlameState;
use repo_runtime::RepositoryRuntime;
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

fn commit(repository: &Path, summary: &str) {
    git(repository, &["add", "--", "."]);
    git(repository, &["commit", "-m", summary]);
}

#[test]
fn follows_one_renamed_literal_path_from_an_immutable_start_commit() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "File History Test"]);
    git(
        repository,
        &["config", "user.email", "file-history@example.test"],
    );

    fs::write(
        repository.join("before.txt"),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n",
    )
    .expect("write original file");
    commit(repository, "add original");
    git(repository, &["mv", "before.txt", "after.txt"]);
    fs::write(
        repository.join("after.txt"),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\n",
    )
    .expect("update renamed file");
    commit(repository, "rename file");
    let start_oid = git(repository, &["rev-parse", "HEAD"]);

    let response = RepositoryRuntime::default()
        .file_history(repository, &start_oid, "after.txt", None)
        .expect("file history");

    assert_eq!(response.start_oid, start_oid);
    assert_eq!(response.path, "after.txt");
    assert_eq!(response.commits.len(), 2);
    assert_eq!(response.commits[0].summary, "rename file");
    assert_eq!(response.commits[1].summary, "add original");
    assert!(response.next_cursor.is_none());
}

#[test]
fn returns_text_blame_and_explicit_binary_or_oversized_states() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "File History Test"]);
    git(
        repository,
        &["config", "user.email", "file-history@example.test"],
    );

    fs::write(repository.join("text.txt"), "first\nsecond\n").expect("write text fixture");
    fs::write(repository.join("image.bin"), [0, 1, 2, 3]).expect("write binary fixture");
    fs::write(
        repository.join("large.txt"),
        vec![b'x'; 2 * 1024 * 1024 + 1],
    )
    .expect("write large fixture");
    commit(repository, "add blame fixtures");
    let oid = git(repository, &["rev-parse", "HEAD"]);
    let runtime = RepositoryRuntime::default();

    let text = runtime
        .file_blame(repository, &oid, "text.txt")
        .expect("text blame");
    assert_eq!(text.state, FileBlameState::Available);
    assert_eq!(text.lines.len(), 2);
    assert_eq!(text.lines[0].content, "first");
    assert_eq!(text.lines[0].author_name, "File History Test");
    assert!(text.lines[0].authored_at.contains('T'));

    let binary = runtime
        .file_blame(repository, &oid, "image.bin")
        .expect("binary blame state");
    assert_eq!(binary.state, FileBlameState::Binary);
    assert!(binary.lines.is_empty());

    let oversized = runtime
        .file_blame(repository, &oid, "large.txt")
        .expect("oversized blame state");
    assert_eq!(oversized.state, FileBlameState::Oversized);
    assert!(oversized.lines.is_empty());
}
