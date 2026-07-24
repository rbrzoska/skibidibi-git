use std::{path::Path, process::Command};

use repo_runtime::RepositoryRuntime;
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .expect("git should start in disposable repository");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("fixture output is UTF-8")
        .trim()
        .to_owned()
}

#[test]
fn creates_and_then_reuses_a_managed_worktree_without_touching_the_active_worktree() {
    let fixture = tempdir().unwrap();
    let repository = fixture.path().join("repository");
    let data_root = fixture.path().join("application-data");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&data_root).unwrap();
    git(&repository, &["init", "-b", "main"]);
    std::fs::write(repository.join("tracked.txt"), "fixture\n").unwrap();
    git(&repository, &["add", "tracked.txt"]);
    git(
        &repository,
        &[
            "-c",
            "user.name=Skibidibi Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    git(&repository, &["branch", "feature/safe"]);
    let expected_oid = git(&repository, &["rev-parse", "refs/heads/feature/safe"]);
    std::fs::write(repository.join("untracked-local.txt"), "must remain\n").unwrap();

    let runtime = RepositoryRuntime::default();
    let created = runtime
        .prepare_branch_worktree(
            &repository,
            &data_root,
            "refs/heads/feature/safe",
            &expected_oid,
        )
        .unwrap();

    assert!(created.created);
    assert!(created.path.starts_with(data_root.canonicalize().unwrap()));
    assert_eq!(git(&repository, &["branch", "--show-current"]), "main");
    assert!(repository.join("untracked-local.txt").is_file());
    assert_eq!(
        git(&created.path, &["branch", "--show-current"]),
        "feature/safe"
    );
    assert_eq!(git(&created.path, &["rev-parse", "HEAD"]), expected_oid);

    let reused = runtime
        .prepare_branch_worktree(
            &repository,
            &data_root,
            "refs/heads/feature/safe",
            &expected_oid,
        )
        .unwrap();
    assert!(!reused.created);
    assert_eq!(reused.path, created.path);
}
