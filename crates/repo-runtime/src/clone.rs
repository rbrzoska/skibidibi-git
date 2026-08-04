use std::{
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use app_domain::{CloneRepositoryRequest, CloneRepositoryResult};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use super::RepositoryRuntime;

const CLONE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CLONE_STDOUT_LIMIT: usize = 2 * 1024 * 1024;
const CLONE_STDERR_LIMIT: usize = 2 * 1024 * 1024;
const MAX_SOURCE_URL_BYTES: usize = 8 * 1024;

pub trait CloneGitExecutor: Send + Sync {
    fn clone_repository(
        &self,
        destination_parent: &Path,
        source_url: &str,
        directory_name: &str,
    ) -> Result<GitOutput, GitRunError>;
}

impl CloneGitExecutor for GitRunner {
    fn clone_repository(
        &self,
        destination_parent: &Path,
        source_url: &str,
        directory_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            destination_parent,
            GitInvocation::new(
                GitInvocationPolicy::Network,
                [
                    OsString::from("clone"),
                    OsString::from("--"),
                    OsString::from(source_url),
                    OsString::from(directory_name),
                ],
            )
            .with_output_limits(CLONE_STDOUT_LIMIT, CLONE_STDERR_LIMIT)
            .with_timeout(CLONE_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum CloneRepositoryError {
    #[error(
        "clone source must be an HTTPS, SSH, or SCP-like remote URL without embedded credentials"
    )]
    InvalidSourceUrl,
    #[error("destination parent must be an existing directory")]
    InvalidDestinationParent,
    #[error("clone directory name must be one normal path component")]
    InvalidDirectoryName,
    #[error("clone destination already exists")]
    DestinationExists,
    #[error("clone destination identity or containment changed while git was running")]
    UnsafeDestination,
    #[error("the validated clone could not be published: {0}")]
    PublishFailed(String),
    #[error(
        "git clone failed and its partial destination could not be removed (git: {git}; cleanup: {cleanup})"
    )]
    CleanupFailed { git: String, cleanup: String },
    #[error(transparent)]
    Git(#[from] GitRunError),
}

impl CloneRepositoryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSourceUrl
            | Self::InvalidDestinationParent
            | Self::InvalidDirectoryName => "invalidRequest",
            Self::DestinationExists => "destinationExists",
            Self::UnsafeDestination => "unsafeDestination",
            Self::PublishFailed(_) => "publishRejected",
            Self::CleanupFailed { .. } => "manualCleanupRequired",
            Self::Git(_) => "gitRejected",
        }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: CloneGitExecutor,
{
    pub fn clone_repository(
        &self,
        request: &CloneRepositoryRequest,
    ) -> Result<CloneRepositoryResult, CloneRepositoryError> {
        validate_source_url(&request.source_url)?;
        self.clone_from_validated_source(request, &request.source_url)
    }

    fn clone_from_validated_source(
        &self,
        request: &CloneRepositoryRequest,
        source: &str,
    ) -> Result<CloneRepositoryResult, CloneRepositoryError> {
        validate_directory_name(&request.directory_name)?;
        let parent = canonical_destination_parent(&request.destination_parent)?;
        let destination = parent.join(&request.directory_name);
        let repository_path = destination
            .to_str()
            .ok_or(CloneRepositoryError::UnsafeDestination)?
            .to_owned();
        ensure_destination_absent(&destination)?;
        let staging = tempfile::Builder::new()
            .prefix(".skibidibi-clone-")
            .tempdir_in(&parent)
            .map_err(|_| CloneRepositoryError::InvalidDestinationParent)?;
        let staging_parent = staging
            .path()
            .canonicalize()
            .map_err(|_| CloneRepositoryError::InvalidDestinationParent)?;
        let staged_repository = staging_parent.join(&request.directory_name);

        if let Err(git) =
            self.executor
                .clone_repository(&staging_parent, source, &request.directory_name)
        {
            return match staging.close() {
                Ok(()) => Err(CloneRepositoryError::Git(git)),
                Err(cleanup) => Err(CloneRepositoryError::CleanupFailed {
                    git: git.to_string(),
                    cleanup: cleanup.to_string(),
                }),
            };
        }

        if let Err(validation) = validate_staged_repository(&staging_parent, &staged_repository) {
            return match staging.close() {
                Ok(()) => Err(validation),
                Err(cleanup) => Err(CloneRepositoryError::CleanupFailed {
                    git: validation.to_string(),
                    cleanup: cleanup.to_string(),
                }),
            };
        }
        if let Err(publish) = publish_no_clobber(&staged_repository, &destination) {
            let error = if publish.kind() == io::ErrorKind::AlreadyExists {
                CloneRepositoryError::DestinationExists
            } else {
                CloneRepositoryError::PublishFailed(publish.to_string())
            };
            return match staging.close() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(CloneRepositoryError::CleanupFailed {
                    git: error.to_string(),
                    cleanup: cleanup.to_string(),
                }),
            };
        }
        // Publication is the commit point. Failure to remove the now-empty random staging
        // directory must not turn a successful clone into an error or encourage a retry.
        let _ = staging.close();
        Ok(CloneRepositoryResult { repository_path })
    }
}

fn ensure_destination_absent(destination: &Path) -> Result<(), CloneRepositoryError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(CloneRepositoryError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CloneRepositoryError::InvalidDestinationParent),
    }
}

fn canonical_destination_parent(value: &str) -> Result<PathBuf, CloneRepositoryError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(CloneRepositoryError::InvalidDestinationParent);
    }
    let parent =
        fs::canonicalize(value).map_err(|_| CloneRepositoryError::InvalidDestinationParent)?;
    if !parent.is_dir() {
        return Err(CloneRepositoryError::InvalidDestinationParent);
    }
    Ok(parent)
}

fn validate_directory_name(value: &str) -> Result<(), CloneRepositoryError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.starts_with('-')
        || value.ends_with([' ', '.'])
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(CloneRepositoryError::InvalidDirectoryName);
    }
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(CloneRepositoryError::InvalidDirectoryName);
    }
    Ok(())
}

fn validate_source_url(value: &str) -> Result<(), CloneRepositoryError> {
    if value.is_empty()
        || value.len() > MAX_SOURCE_URL_BYTES
        || value.trim() != value
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || value.contains([' ', '\\', '?', '#'])
    {
        return Err(CloneRepositoryError::InvalidSourceUrl);
    }

    if let Some(rest) = value.strip_prefix("https://") {
        return validate_absolute_remote(rest, false);
    }
    if let Some(rest) = value.strip_prefix("ssh://") {
        return validate_absolute_remote(rest, true);
    }
    if value.contains("://") {
        return Err(CloneRepositoryError::InvalidSourceUrl);
    }
    validate_scp_remote(value)
}

fn validate_absolute_remote(rest: &str, ssh: bool) -> Result<(), CloneRepositoryError> {
    let (authority, path) = rest
        .split_once('/')
        .ok_or(CloneRepositoryError::InvalidSourceUrl)?;
    if authority.is_empty() || path.is_empty() {
        return Err(CloneRepositoryError::InvalidSourceUrl);
    }

    let host = if ssh {
        match authority.split_once('@') {
            Some((user, host))
                if valid_ssh_user(user) && !host.is_empty() && !host.contains('@') =>
            {
                host
            }
            Some(_) => return Err(CloneRepositoryError::InvalidSourceUrl),
            None => authority,
        }
    } else {
        if authority.contains('@') {
            return Err(CloneRepositoryError::InvalidSourceUrl);
        }
        authority
    };

    validate_host(host)
}

fn validate_scp_remote(value: &str) -> Result<(), CloneRepositoryError> {
    let (user, remote) = value
        .split_once('@')
        .ok_or(CloneRepositoryError::InvalidSourceUrl)?;
    let (host, path) = remote
        .split_once(':')
        .ok_or(CloneRepositoryError::InvalidSourceUrl)?;
    if !valid_ssh_user(user) || remote.contains('@') || path.is_empty() || path.starts_with('-') {
        return Err(CloneRepositoryError::InvalidSourceUrl);
    }
    validate_host(host)
}

fn valid_ssh_user(user: &str) -> bool {
    !user.is_empty()
        && !user.starts_with('-')
        && user.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '+')
        })
}

fn validate_host(host: &str) -> Result<(), CloneRepositoryError> {
    if host.is_empty()
        || host.starts_with('-')
        || host.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':' | '[' | ']'))
        })
    {
        Err(CloneRepositoryError::InvalidSourceUrl)
    } else {
        Ok(())
    }
}

fn validate_staged_repository(
    parent: &Path,
    destination: &Path,
) -> Result<(), CloneRepositoryError> {
    let metadata =
        fs::symlink_metadata(destination).map_err(|_| CloneRepositoryError::UnsafeDestination)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CloneRepositoryError::UnsafeDestination);
    }
    let canonical =
        fs::canonicalize(destination).map_err(|_| CloneRepositoryError::UnsafeDestination)?;
    if canonical.parent() != Some(parent) {
        return Err(CloneRepositoryError::UnsafeDestination);
    }
    Ok(())
}

#[cfg(unix)]
fn publish_no_clobber(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(windows)]
fn publish_no_clobber(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both pointers refer to live, NUL-terminated UTF-16 buffers for the duration of the
    // call. Flags are zero intentionally: MoveFileExW then atomically refuses an existing target.
    let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{process::Command, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct RecordingExecutor {
        calls: Mutex<Vec<(PathBuf, String, String)>>,
        failure: bool,
        competing_destination: Option<PathBuf>,
        #[cfg(unix)]
        symlink_output: bool,
    }

    impl CloneGitExecutor for RecordingExecutor {
        fn clone_repository(
            &self,
            destination_parent: &Path,
            source_url: &str,
            directory_name: &str,
        ) -> Result<GitOutput, GitRunError> {
            self.calls.lock().unwrap().push((
                destination_parent.to_owned(),
                source_url.to_owned(),
                directory_name.to_owned(),
            ));
            let staged = destination_parent.join(directory_name);
            #[cfg(unix)]
            if self.symlink_output {
                std::os::unix::fs::symlink(destination_parent, &staged).unwrap();
            } else {
                fs::create_dir(&staged).unwrap();
                fs::write(staged.join("cloned.txt"), "cloned").unwrap();
            }
            #[cfg(not(unix))]
            {
                fs::create_dir(&staged).unwrap();
                fs::write(staged.join("cloned.txt"), "cloned").unwrap();
            }
            if let Some(competing) = &self.competing_destination {
                fs::create_dir(competing).unwrap();
                fs::write(competing.join("foreign.txt"), "foreign").unwrap();
            }
            if self.failure {
                Err(GitRunError::Unsuccessful {
                    code: Some(128),
                    stderr: "clone failed".to_owned(),
                })
            } else {
                Ok(GitOutput {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                })
            }
        }
    }

    fn request(parent: &Path, source: &str, name: &str) -> CloneRepositoryRequest {
        CloneRepositoryRequest {
            source_url: source.to_owned(),
            destination_parent: parent.display().to_string(),
            directory_name: name.to_owned(),
        }
    }

    #[test]
    fn accepts_supported_remote_url_shapes() {
        for source in [
            "https://github.com/acme/project.git",
            "ssh://git@github.com/acme/project.git",
            "git@github.com:acme/project.git",
        ] {
            assert!(validate_source_url(source).is_ok(), "{source}");
        }
    }

    #[test]
    fn rejects_credentials_local_protocols_controls_and_flags() {
        for source in [
            "https://token@github.com/acme/project.git",
            "https://user:token@github.com/acme/project.git",
            "ssh://user:password@github.com/acme/project.git",
            "ssh://-oProxyCommand@github.com/acme/project.git",
            "-oProxyCommand@github.com:acme/project.git",
            "file:///tmp/project.git",
            "ext::helper project",
            "git://github.com/acme/project.git",
            "/tmp/project.git",
            "../project.git",
            "-uploader",
            "https://github.com/acme/project.git\n--upload-pack=evil",
        ] {
            assert!(
                matches!(
                    validate_source_url(source),
                    Err(CloneRepositoryError::InvalidSourceUrl)
                ),
                "accepted unsafe source: {source:?}"
            );
        }
    }

    #[test]
    fn validates_destination_before_invoking_git() {
        let parent = tempfile::tempdir().unwrap();
        fs::create_dir(parent.path().join("existing")).unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor::default());

        for name in ["", ".", "..", "nested/repo", "nested\\repo", "-config"] {
            let error = runtime
                .clone_repository(&request(
                    parent.path(),
                    "https://github.com/acme/project.git",
                    name,
                ))
                .unwrap_err();
            assert!(matches!(error, CloneRepositoryError::InvalidDirectoryName));
        }
        let error = runtime
            .clone_repository(&request(
                parent.path(),
                "https://github.com/acme/project.git",
                "existing",
            ))
            .unwrap_err();
        assert!(matches!(error, CloneRepositoryError::DestinationExists));
        assert!(runtime.executor.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn removes_only_the_expected_partial_child_after_git_failure() {
        let parent = tempfile::tempdir().unwrap();
        fs::write(parent.path().join("keep.txt"), "keep").unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            failure: true,
            ..RecordingExecutor::default()
        });

        let error = runtime
            .clone_repository(&request(
                parent.path(),
                "https://github.com/acme/project.git",
                "partial",
            ))
            .unwrap_err();

        assert!(matches!(error, CloneRepositoryError::Git(_)));
        assert!(!parent.path().join("partial").exists());
        assert_eq!(
            fs::read_to_string(parent.path().join("keep.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn atomic_publish_preserves_a_competing_destination() {
        let parent = tempfile::tempdir().unwrap();
        let competing = parent.path().join("raced");
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            competing_destination: Some(competing.clone()),
            ..RecordingExecutor::default()
        });

        let error = runtime
            .clone_repository(&request(
                parent.path(),
                "https://github.com/acme/project.git",
                "raced",
            ))
            .unwrap_err();

        assert!(matches!(error, CloneRepositoryError::DestinationExists));
        assert_eq!(
            fs::read_to_string(competing.join("foreign.txt")).unwrap(),
            "foreign"
        );
        assert_eq!(
            fs::read_dir(parent.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".skibidibi-clone-"))
                .count(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_replacement_after_reported_git_success() {
        let parent = tempfile::tempdir().unwrap();
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            symlink_output: true,
            ..RecordingExecutor::default()
        });

        let error = runtime
            .clone_repository(&request(
                parent.path(),
                "https://github.com/acme/project.git",
                "swapped",
            ))
            .unwrap_err();

        assert!(matches!(error, CloneRepositoryError::UnsafeDestination));
        assert!(!parent.path().join("swapped").exists());
    }

    #[test]
    fn git_runner_clones_a_local_fixture_only_through_the_test_bypass() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination_parent = root.path().join("destinations");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination_parent).unwrap();
        let git = |repository: &Path, arguments: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(repository)
                .args(arguments)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&source, &["init", "-q", "-b", "main"]);
        git(&source, &["config", "user.name", "Clone Test"]);
        git(&source, &["config", "user.email", "clone@example.test"]);
        fs::write(source.join("README.md"), "fixture\n").unwrap();
        git(&source, &["add", "README.md"]);
        git(&source, &["commit", "-qm", "fixture"]);

        let request = request(&destination_parent, "test-only-local", "cloned");
        let result = RepositoryRuntime::default()
            .clone_from_validated_source(&request, source.to_str().unwrap())
            .unwrap();

        let readme =
            fs::read_to_string(Path::new(&result.repository_path).join("README.md")).unwrap();
        assert!(matches!(readme.as_str(), "fixture\n" | "fixture\r\n"));
        assert!(matches!(
            RepositoryRuntime::default().clone_repository(&request),
            Err(CloneRepositoryError::InvalidSourceUrl)
        ));
    }
}
