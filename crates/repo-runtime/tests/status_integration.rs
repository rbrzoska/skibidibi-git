use std::{fs, path::Path, process::Command};

use app_domain::{StatusCode, StatusEntryKind};
use repo_runtime::RepositoryRuntime;
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reads_real_repository_status_with_nul_sensitive_paths() {
    let temporary = tempdir().expect("temporary directory");
    let repository = temporary.path().join("repository;not-a-shell-command");
    fs::create_dir(&repository).expect("repository directory");
    git(&repository, &["init", "-q"]);
    git(&repository, &["config", "user.email", "test@example.com"]);
    git(&repository, &["config", "user.name", "Test User"]);

    fs::write(repository.join("tracked file.txt"), "before\n").expect("tracked file");
    fs::write(repository.join("old name.txt"), "rename me\n").expect("rename source");
    git(
        &repository,
        &["add", "--", "tracked file.txt", "old name.txt"],
    );
    git(&repository, &["commit", "-qm", "initial"]);

    fs::write(repository.join("tracked file.txt"), "after\n").expect("modified file");
    fs::write(repository.join("untracked ;$() [file].txt"), "new\n").expect("untracked file");
    git(&repository, &["mv", "--", "old name.txt", "new name.txt"]);

    let status = RepositoryRuntime::default()
        .status(&repository)
        .expect("repository status");

    assert!(status.branch.oid.is_some());
    assert!(status.branch.head.is_some());
    assert!(status.entries.iter().any(|entry| {
        entry.path == "tracked file.txt"
            && entry.worktree_status == StatusCode::Modified
            && entry.kind == StatusEntryKind::Ordinary
    }));
    assert!(status.entries.iter().any(|entry| {
        entry.path == "untracked ;$() [file].txt" && entry.kind == StatusEntryKind::Untracked
    }));
    assert!(status.entries.iter().any(|entry| {
        entry.path == "new name.txt"
            && entry.original_path.as_deref() == Some("old name.txt")
            && entry.kind == StatusEntryKind::RenamedOrCopied
            && entry.index_status == StatusCode::Renamed
    }));
}
