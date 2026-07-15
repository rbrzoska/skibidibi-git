use std::{fs, path::Path, process::Command};

use app_domain::{StashFileDiffRequest, StashFileSource};
use repo_runtime::RepositoryRuntime;
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .env("LC_ALL", "C")
        .output()
        .expect("git should be installed");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn inspects_tracked_rename_binary_and_untracked_files_by_immutable_oid() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Stash Inspector"]);
    git(
        repository,
        &["config", "user.email", "stash-inspector@example.test"],
    );

    let original = (1..=20)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(repository.join("old name.txt"), &original).unwrap();
    fs::write(repository.join("binary.dat"), [0, 1, 2, 3]).unwrap();
    git(repository, &["add", "--", "old name.txt", "binary.dat"]);
    git(repository, &["commit", "-m", "base"]);

    fs::rename(
        repository.join("old name.txt"),
        repository.join("new name.txt"),
    )
    .unwrap();
    fs::write(
        repository.join("new name.txt"),
        format!("{original}stashed line\n"),
    )
    .unwrap();
    fs::write(repository.join("binary.dat"), [0, 255, 2, 3]).unwrap();
    git(
        repository,
        &[
            "add",
            "-A",
            "--",
            "old name.txt",
            "new name.txt",
            "binary.dat",
        ],
    );
    fs::write(repository.join("untracked.txt"), "untracked snapshot\n").unwrap();
    fs::write(repository.join("untracked.bin"), [0, 4, 5, 6]).unwrap();
    git(repository, &["stash", "push", "-u", "-m", "inspect me"]);
    let inspected_oid = git(repository, &["rev-parse", "refs/stash"]);

    fs::write(repository.join("later.txt"), "later\n").unwrap();
    git(repository, &["add", "later.txt"]);
    git(repository, &["stash", "push", "-m", "newer stash"]);
    assert_ne!(inspected_oid, git(repository, &["rev-parse", "refs/stash"]));

    let runtime = RepositoryRuntime::default();
    let details = runtime
        .stash_details(repository, &inspected_oid)
        .expect("inspect older stash by OID");
    let renamed = details
        .files
        .iter()
        .find(|file| file.path == "new name.txt")
        .expect("tracked rename");
    assert_eq!(renamed.source, StashFileSource::Tracked);
    assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
    assert!(details.files.iter().any(|file| {
        file.path == "binary.dat" && file.source == StashFileSource::Tracked && file.binary
    }));
    assert!(
        details.files.iter().any(|file| {
            file.path == "untracked.txt" && file.source == StashFileSource::Untracked
        })
    );
    assert!(details.files.iter().any(|file| {
        file.path == "untracked.bin" && file.source == StashFileSource::Untracked && file.binary
    }));

    let rename_diff = runtime
        .stash_file_diff(
            repository,
            &StashFileDiffRequest {
                oid: inspected_oid.clone(),
                source: StashFileSource::Tracked,
                path: "new name.txt".to_owned(),
                old_path: Some("old name.txt".to_owned()),
            },
        )
        .expect("rename diff");
    assert_eq!(rename_diff.oid, inspected_oid);
    assert!(rename_diff.patch.contains("rename from old name.txt"));
    assert!(rename_diff.patch.contains("rename to new name.txt"));
    assert!(rename_diff.patch.contains("+stashed line"));

    let untracked_diff = runtime
        .stash_file_diff(
            repository,
            &StashFileDiffRequest {
                oid: details.oid,
                source: StashFileSource::Untracked,
                path: "untracked.txt".to_owned(),
                old_path: None,
            },
        )
        .expect("untracked diff");
    assert!(untracked_diff.patch.contains("new file mode"));
    assert!(untracked_diff.patch.contains("+untracked snapshot"));
}

#[test]
fn two_parent_stash_combines_staged_and_unstaged_tracked_snapshot() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Stash Inspector"]);
    git(
        repository,
        &["config", "user.email", "stash-inspector@example.test"],
    );
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    git(repository, &["add", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);

    fs::write(repository.join("tracked.txt"), "base\nstaged\n").unwrap();
    git(repository, &["add", "tracked.txt"]);
    fs::write(repository.join("tracked.txt"), "base\nstaged\nunstaged\n").unwrap();
    git(repository, &["stash", "push", "-m", "two parent"]);
    let oid = git(repository, &["rev-parse", "refs/stash"]);

    let runtime = RepositoryRuntime::default();
    let details = runtime.stash_details(repository, &oid).unwrap();
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].source, StashFileSource::Tracked);
    assert_eq!(details.files[0].additions, Some(2));
    let diff = runtime
        .stash_file_diff(
            repository,
            &StashFileDiffRequest {
                oid,
                source: StashFileSource::Tracked,
                path: "tracked.txt".to_owned(),
                old_path: None,
            },
        )
        .unwrap();
    assert!(diff.patch.contains("+staged"));
    assert!(diff.patch.contains("+unstaged"));
}
