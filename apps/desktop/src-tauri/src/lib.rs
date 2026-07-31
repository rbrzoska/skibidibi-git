use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

use app_domain::{
    AmendCommitRequest, AmendCommitResult, ApplyIndexChangeRequest, ApplyIndexChangeResult,
    ApplyStashRequest, ApplyStashResult, CloneRepositoryRequest, CommitDetails, CommitHistoryPage,
    CommitOperationRequest, CommitOperationResult, ConflictFileDetail, ConflictFileDetailRequest,
    ConflictListResult, CreateBranchRequest, CreateBranchResult, CreateCommitRequest,
    CreateCommitResult, DeleteBranchRequest, DeleteBranchResult, DiscardWorkingTreeChangesRequest,
    DiscardWorkingTreeChangesResult, DiscardWorkingTreeHunkRequest, DropStashRequest,
    DropStashResult, FetchRepositoryResult, FileBlame, FileDiff, FileHistoryPage,
    IntegrationHealth, IntegrationHealthIssue, IntegrationHealthState, MergeBranchRequest,
    MergeBranchResult, PopStashRequest, PopStashResult, PullInactiveBranchRequest,
    PullInactiveBranchResult, PullRequest, PullResult, PushAnalysis, PushRequest, PushResult,
    PushStashRequest, PushStashResult, RefComparison, RefComparisonFileDiff,
    RememberRepositoryInput, RememberedRepository, RemoveWorktreeRequest, RemoveWorktreeResult,
    RepositoryAvailability, RepositoryGitIdentity, RepositoryGroupRelation, RepositoryHealthUpdate,
    RepositoryNavigation, RepositoryProvider, RepositoryStatus, RepositorySubmodules,
    RepositoryTransport, RepositoryWorktreeRole, ResetCommitRequest, ResolveConflictRequest,
    ResolveConflictResult, SetUpstreamRequest, SetUpstreamResult, StashDetails, StashFileDiff,
    StashFileDiffRequest, StashFileSource, SubmoduleCommitState, SubmoduleWorktreeState,
    SwitchBranchRequest, SwitchBranchResult, WorkingTreeFileDiff, WorktreeDirtyState,
    WorktreeRemovalMode,
};
use app_store::{CatalogError, RepositoryCatalog, RepositoryCatalogReconciliation};
use repo_runtime::{
    BranchCreationError, BranchOperationError, BranchPreviewError, BranchSwitchError,
    CloneRepositoryError, CommitOperationError, ConflictResolutionError, FileDiffRuntimeError,
    FileHistoryRuntimeError, HistoryRuntimeError, MaintenanceError, MutationRuntimeError,
    NavigationRuntimeError, NetworkOperationError, RefComparisonError, RepositoryRuntime,
    RepositoryRuntimeError, StashActionError, StashInspectionError, SubmodulesRuntimeError,
    WorkingTreeDiffRuntimeError,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

mod ai;
mod diagnostics;
mod external_workspace;
mod github;
mod maintenance_stats;
mod updater;

const MIN_APPLICATION_ZOOM: f64 = 1.0;
const MAX_APPLICATION_ZOOM: f64 = 1.5;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationZoomResponse {
    scale: f64,
}

fn validated_application_zoom(scale: f64) -> Result<f64, CommandError> {
    if scale.is_finite() && (MIN_APPLICATION_ZOOM..=MAX_APPLICATION_ZOOM).contains(&scale) {
        Ok(scale)
    } else {
        Err(CommandError {
            message: "application zoom must be between 100% and 150%".to_owned(),
        })
    }
}

#[tauri::command]
fn set_application_zoom(
    window: tauri::WebviewWindow,
    scale: f64,
) -> Result<ApplicationZoomResponse, CommandError> {
    let scale = validated_application_zoom(scale)?;
    window.set_zoom(scale).map_err(|_| CommandError {
        message: "application zoom could not be changed".to_owned(),
    })?;
    Ok(ApplicationZoomResponse { scale })
}

struct AppState {
    repositories: RepositoryRuntime,
    catalog: Mutex<RepositoryCatalog>,
    github_accounts: Mutex<app_store::GitHubAccountStore>,
    github_credentials: Arc<dyn secret_store::CredentialStore>,
    github_config: github_client::GitHubClientConfig,
    github_transport: github_client::ReqwestTransport,
    github_device_flow: github_client::GitHubDeviceFlowClient,
    github_cli: github_client::GitHubCliTransport,
    github_device_sessions: github::GitHubDeviceFlowSessions,
    github_mutations: tokio::sync::Mutex<()>,
    github_cli_operations: tokio::sync::Mutex<()>,
    github_account_generations: Mutex<HashMap<String, u64>>,
    mutations: MutationLockRegistry,
    clones: tokio::sync::Mutex<()>,
    diagnostics: diagnostics::DiagnosticsState,
}

impl AppState {
    fn new(
        catalog: RepositoryCatalog,
        github_accounts: app_store::GitHubAccountStore,
        diagnostics: diagnostics::DiagnosticsState,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let api_base_url = url::Url::parse("https://api.github.com/")?;
        let github_config = github_client::GitHubClientConfig::new(
            api_base_url.clone(),
            url::Url::parse("https://api.github.com/graphql")?,
        )?;
        let github_transport = github_client::ReqwestTransport::new(api_base_url.clone())?;
        let github_device_flow = github_client::GitHubDeviceFlowClient::new()?;
        Ok(Self {
            repositories: RepositoryRuntime::default(),
            catalog: Mutex::new(catalog),
            github_accounts: Mutex::new(github_accounts),
            github_credentials: Arc::new(secret_store::OsCredentialStore::new()),
            github_config,
            github_transport,
            github_device_flow,
            github_cli: github_client::GitHubCliTransport::discover(api_base_url),
            github_device_sessions: github::GitHubDeviceFlowSessions::default(),
            github_mutations: tokio::sync::Mutex::new(()),
            github_cli_operations: tokio::sync::Mutex::new(()),
            github_account_generations: Mutex::new(HashMap::new()),
            mutations: MutationLockRegistry::default(),
            clones: tokio::sync::Mutex::new(()),
            diagnostics,
        })
    }
}

#[derive(Default)]
struct MutationLockRegistry {
    locks: Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>,
}

impl MutationLockRegistry {
    fn for_repository(&self, repository: &Path) -> Result<Arc<Mutex<()>>, CommandError> {
        let mut locks = self.locks.lock().map_err(|_| CommandError {
            message: "repository mutation registry is unavailable".to_owned(),
        })?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(repository).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(repository.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandError {
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeDirtyStatesResponse {
    states: Vec<WorktreeDirtyState>,
}

#[derive(Debug, Serialize)]
struct SelectRepositoryDirectoryResponse {
    path: Option<String>,
}

impl From<RepositoryRuntimeError> for CommandError {
    fn from(error: RepositoryRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<CatalogError> for CommandError {
    fn from(error: CatalogError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<HistoryRuntimeError> for CommandError {
    fn from(error: HistoryRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<BranchPreviewError> for CommandError {
    fn from(error: BranchPreviewError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<NavigationRuntimeError> for CommandError {
    fn from(error: NavigationRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<BranchSwitchError> for CommandError {
    fn from(error: BranchSwitchError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<BranchCreationError> for CommandError {
    fn from(error: BranchCreationError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<BranchOperationError> for CommandError {
    fn from(error: BranchOperationError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<StashActionError> for CommandError {
    fn from(error: StashActionError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<NetworkOperationError> for CommandError {
    fn from(error: NetworkOperationError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<ConflictResolutionError> for CommandError {
    fn from(error: ConflictResolutionError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<CloneRepositoryError> for CommandError {
    fn from(error: CloneRepositoryError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<StashInspectionError> for CommandError {
    fn from(error: StashInspectionError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<MaintenanceError> for CommandError {
    fn from(error: MaintenanceError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<FileDiffRuntimeError> for CommandError {
    fn from(error: FileDiffRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<RefComparisonError> for CommandError {
    fn from(error: RefComparisonError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<FileHistoryRuntimeError> for CommandError {
    fn from(error: FileHistoryRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<CommitOperationError> for CommandError {
    fn from(error: CommitOperationError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<WorkingTreeDiffRuntimeError> for CommandError {
    fn from(error: WorkingTreeDiffRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<MutationRuntimeError> for CommandError {
    fn from(error: MutationRuntimeError) -> Self {
        Self {
            message: format!("{}: {}", error.code(), error),
        }
    }
}

impl From<SubmodulesRuntimeError> for CommandError {
    fn from(error: SubmodulesRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

fn unix_timestamp() -> Result<i64, CommandError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CommandError {
            message: format!("system clock is before the Unix epoch: {error}"),
        })?
        .as_secs();
    i64::try_from(seconds).map_err(|_| CommandError {
        message: "system clock value is out of range".to_owned(),
    })
}

fn lock_catalog<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, RepositoryCatalog>, CommandError> {
    state.catalog.lock().map_err(|_| CommandError {
        message: "repository catalog is unavailable".to_owned(),
    })
}

fn resolve_repository_path(
    repository_id: &str,
    state: &State<'_, AppState>,
) -> Result<PathBuf, CommandError> {
    lock_catalog(state)?
        .get(repository_id)?
        .map(|repository| PathBuf::from(repository.canonical_path))
        .ok_or_else(|| CommandError {
            message: format!("remembered repository not found: {repository_id}"),
        })
}

fn read_small_git_pointer(path: &Path) -> Result<String, CommandError> {
    let metadata = std::fs::metadata(path).map_err(|error| CommandError {
        message: format!("Git metadata pointer is unavailable: {error}"),
    })?;
    if metadata.len() > 64 * 1024 {
        return Err(CommandError {
            message: "Git metadata pointer is unexpectedly large".to_owned(),
        });
    }
    std::fs::read_to_string(path).map_err(|error| CommandError {
        message: format!("Git metadata pointer is invalid: {error}"),
    })
}

#[derive(Debug)]
struct ResolvedGitIdentity {
    common_dir: PathBuf,
    worktree_role: RepositoryWorktreeRole,
}

fn resolve_git_identity(repository: &Path) -> Result<ResolvedGitIdentity, CommandError> {
    let repository = std::fs::canonicalize(repository).map_err(|error| CommandError {
        message: format!("repository path is unavailable: {error}"),
    })?;
    let dot_git = repository.join(".git");
    let (git_dir_result, worktree_role, pointer_git_dir) = if dot_git.is_dir() {
        (
            std::fs::canonicalize(dot_git),
            RepositoryWorktreeRole::Main,
            false,
        )
    } else if dot_git.is_file() {
        let pointer = read_small_git_pointer(&dot_git)?;
        let value = pointer
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("gitdir: "))
            .ok_or_else(|| CommandError {
                message: "linked-worktree Git directory pointer is invalid".to_owned(),
            })?;
        let candidate = Path::new(value);
        (
            std::fs::canonicalize(if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                repository.join(candidate)
            }),
            RepositoryWorktreeRole::Main,
            true,
        )
    } else if repository.join("HEAD").is_file() && repository.join("objects").is_dir() {
        (Ok(repository), RepositoryWorktreeRole::Bare, false)
    } else {
        (
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Git directory not found",
            )),
            RepositoryWorktreeRole::Unknown,
            false,
        )
    };
    let git_dir = git_dir_result.map_err(|error| CommandError {
        message: format!("Git directory is unavailable: {error}"),
    })?;

    let common_pointer = git_dir.join("commondir");
    if !common_pointer.is_file() {
        return Ok(ResolvedGitIdentity {
            common_dir: git_dir,
            worktree_role,
        });
    }
    let value = read_small_git_pointer(&common_pointer)?;
    let candidate = Path::new(value.trim());
    let common_dir = std::fs::canonicalize(if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        git_dir.join(candidate)
    })
    .map_err(|error| CommandError {
        message: format!("Git common directory is unavailable: {error}"),
    })?;
    Ok(ResolvedGitIdentity {
        common_dir,
        worktree_role: if pointer_git_dir {
            RepositoryWorktreeRole::Linked
        } else {
            worktree_role
        },
    })
}

fn resolve_git_common_dir(repository: &Path) -> Result<PathBuf, CommandError> {
    Ok(resolve_git_identity(repository)?.common_dir)
}

fn catalog_git_identity(repository: &Path) -> Result<RepositoryGitIdentity, CommandError> {
    let identity = resolve_git_identity(repository)?;
    let canonical_common_dir =
        identity
            .common_dir
            .into_os_string()
            .into_string()
            .map_err(|_| CommandError {
                message: "Git common directory is not valid UTF-8".to_owned(),
            })?;
    Ok(RepositoryGitIdentity {
        repository_group_id: Uuid::new_v4().to_string(),
        canonical_common_dir,
        worktree_role: identity.worktree_role,
    })
}

fn mutation_lock(
    state: &State<'_, AppState>,
    repository_path: &Path,
) -> Result<Arc<Mutex<()>>, CommandError> {
    state
        .mutations
        .for_repository(&resolve_git_common_dir(repository_path)?)
}

fn repository_availability(path: &str) -> RepositoryAvailability {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => RepositoryAvailability::Available,
        Ok(_) => RepositoryAvailability::Missing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            RepositoryAvailability::Missing
        }
        Err(_) => RepositoryAvailability::Inaccessible,
    }
}

#[tauri::command]
async fn repository_status(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositoryStatus, CommandError> {
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .status(Path::new(&repository_path))
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository status task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_history(
    repository_id: String,
    cursor: Option<String>,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<CommitHistoryPage, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .history_page(&repository_path, limit, cursor.as_deref())
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository history task failed: {error}"),
    })?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn repository_branch_history(
    repository_id: String,
    branch_full_name: String,
    expected_branch_oid: String,
    target_full_name: String,
    expected_target_oid: String,
    cursor: Option<String>,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<CommitHistoryPage, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .branch_history_page(
                &repository_path,
                &branch_full_name,
                &expected_branch_oid,
                &target_full_name,
                &expected_target_oid,
                limit,
                cursor.as_deref(),
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("branch preview history task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_commit_detail(
    repository_id: String,
    oid: String,
    state: State<'_, AppState>,
) -> Result<CommitDetails, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .commit_details(&repository_path, &oid)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("commit detail task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_file_diff(
    repository_id: String,
    oid: String,
    path: String,
    old_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<FileDiff, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .file_diff(&repository_path, &oid, &path, old_path.as_deref())
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("file diff task failed: {error}"),
    })?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn repository_compare_refs(
    repository_id: String,
    source_full_name: String,
    expected_source_oid: String,
    target_full_name: String,
    expected_target_oid: String,
    state: State<'_, AppState>,
) -> Result<RefComparison, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .compare_refs(
                &repository_path,
                &source_full_name,
                &expected_source_oid,
                &target_full_name,
                &expected_target_oid,
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("ref comparison task failed: {error}"),
    })?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn repository_compare_ref_file_diff(
    repository_id: String,
    source_full_name: String,
    expected_source_oid: String,
    target_full_name: String,
    expected_target_oid: String,
    path: String,
    old_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<RefComparisonFileDiff, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .compare_ref_file_diff(
                &repository_path,
                &source_full_name,
                &expected_source_oid,
                &target_full_name,
                &expected_target_oid,
                &path,
                old_path.as_deref(),
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("ref comparison file diff task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_file_history(
    repository_id: String,
    start_oid: String,
    path: String,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<FileHistoryPage, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .file_history(&repository_path, &start_oid, &path, cursor.as_deref())
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("file history task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_file_blame(
    repository_id: String,
    oid: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<FileBlame, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .file_blame(&repository_path, &oid, &path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("file blame task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_stash_detail(
    repository_id: String,
    oid: String,
    state: State<'_, AppState>,
) -> Result<StashDetails, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .stash_details(&repository_path, &oid)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("stash detail task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_stash_file_diff(
    repository_id: String,
    oid: String,
    source: StashFileSource,
    path: String,
    old_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<StashFileDiff, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .stash_file_diff(
                &repository_path,
                &StashFileDiffRequest {
                    oid,
                    source,
                    path,
                    old_path,
                },
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("stash file diff task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_push_analysis(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<PushAnalysis, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .push_analysis(&repository_path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("push analysis task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_pull(
    repository_id: String,
    operation: PullRequest,
    state: State<'_, AppState>,
) -> Result<PullResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .pull_repository(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("pull task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_merge_branch(
    repository_id: String,
    operation: MergeBranchRequest,
    state: State<'_, AppState>,
) -> Result<MergeBranchResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .merge_branch(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("branch merge task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_pull_inactive_branch(
    repository_id: String,
    operation: PullInactiveBranchRequest,
    state: State<'_, AppState>,
) -> Result<PullInactiveBranchResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .pull_inactive_branch(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("background branch pull task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_worktree_dirty_states(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<WorktreeDirtyStatesResponse, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    let states = tauri::async_runtime::spawn_blocking(move || {
        repositories
            .worktree_dirty_states(&repository_path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("worktree dirty-state task failed: {error}"),
    })??;
    Ok(WorktreeDirtyStatesResponse { states })
}

#[tauri::command]
async fn repository_push(
    repository_id: String,
    operation: PushRequest,
    state: State<'_, AppState>,
) -> Result<PushResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .push_repository(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("push task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_set_upstream(
    repository_id: String,
    operation: SetUpstreamRequest,
    state: State<'_, AppState>,
) -> Result<SetUpstreamResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .set_repository_upstream(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("set-upstream task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_conflicts(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<ConflictListResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .conflicts(&repository_path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("conflict list task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_conflict_detail(
    repository_id: String,
    operation: ConflictFileDetailRequest,
    state: State<'_, AppState>,
) -> Result<ConflictFileDetail, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .conflict_detail(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("conflict detail task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_resolve_conflict(
    repository_id: String,
    operation: ResolveConflictRequest,
    state: State<'_, AppState>,
) -> Result<ResolveConflictResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .resolve_conflict(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("conflict resolution task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_working_tree_file_diff(
    repository_id: String,
    path: String,
    old_path: Option<String>,
    entry_kind: app_domain::StatusEntryKind,
    state: State<'_, AppState>,
) -> Result<WorkingTreeFileDiff, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .working_tree_file_diff(&repository_path, &path, old_path.as_deref(), entry_kind)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("working-tree file diff task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_apply_index_change(
    repository_id: String,
    operation: ApplyIndexChangeRequest,
    state: State<'_, AppState>,
) -> Result<ApplyIndexChangeResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .apply_index_change(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("index mutation task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_discard_worktree_changes(
    repository_id: String,
    operation: DiscardWorkingTreeChangesRequest,
    state: State<'_, AppState>,
) -> Result<DiscardWorkingTreeChangesResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .discard_worktree_changes(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("working-tree discard task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_discard_worktree_hunk(
    repository_id: String,
    operation: DiscardWorkingTreeHunkRequest,
    state: State<'_, AppState>,
) -> Result<DiscardWorkingTreeChangesResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .discard_worktree_hunk(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("working-tree hunk discard task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_create_commit(
    repository_id: String,
    operation: CreateCommitRequest,
    state: State<'_, AppState>,
) -> Result<CreateCommitResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .create_commit(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("commit task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_amend_commit(
    repository_id: String,
    operation: AmendCommitRequest,
    state: State<'_, AppState>,
) -> Result<AmendCommitResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .amend_commit(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("amend task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_cherry_pick_commit(
    repository_id: String,
    operation: CommitOperationRequest,
    state: State<'_, AppState>,
) -> Result<CommitOperationResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .cherry_pick_commit(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("cherry-pick task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_revert_commit(
    repository_id: String,
    operation: CommitOperationRequest,
    state: State<'_, AppState>,
) -> Result<CommitOperationResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .revert_commit(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("revert task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_reset_commit(
    repository_id: String,
    operation: ResetCommitRequest,
    state: State<'_, AppState>,
) -> Result<CommitOperationResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .reset_commit(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("reset task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_navigation(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<RepositoryNavigation, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .navigation(&repository_path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository navigation task failed: {error}"),
    })?
}

#[tauri::command]
async fn create_repository_branch(
    repository_id: String,
    operation: CreateBranchRequest,
    state: State<'_, AppState>,
) -> Result<CreateBranchResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .create_branch(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("branch creation task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_push_stash(
    repository_id: String,
    operation: PushStashRequest,
    state: State<'_, AppState>,
) -> Result<PushStashResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .push_stash(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("stash push task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_apply_stash(
    repository_id: String,
    operation: ApplyStashRequest,
    state: State<'_, AppState>,
) -> Result<ApplyStashResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .apply_stash(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("stash apply task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_pop_stash(
    repository_id: String,
    operation: PopStashRequest,
    state: State<'_, AppState>,
) -> Result<PopStashResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .pop_stash(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("stash pop task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_drop_stash(
    repository_id: String,
    operation: DropStashRequest,
    state: State<'_, AppState>,
) -> Result<DropStashResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let mutation = mutation_lock(&state, &repository_path)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .drop_stash(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("stash drop task failed: {error}"),
    })?
}

#[tauri::command]
async fn switch_repository_branch(
    repository_id: String,
    operation: SwitchBranchRequest,
    state: State<'_, AppState>,
) -> Result<SwitchBranchResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    let mutation = mutation_lock(&state, &repository_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _mutation_guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .switch_branch(&repository_path, &operation)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("branch switch task failed: {error}"),
    })?
}

#[tauri::command]
async fn delete_repository_branch(
    repository_id: String,
    full_name: String,
    expected_oid: String,
    state: State<'_, AppState>,
) -> Result<DeleteBranchResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    let mutation = mutation_lock(&state, &repository_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .delete_branch(
                &repository_path,
                &DeleteBranchRequest {
                    full_name,
                    expected_oid,
                },
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("branch deletion task failed: {error}"),
    })?
}

#[tauri::command]
async fn remove_repository_worktree(
    repository_id: String,
    path: String,
    expected_head: Option<String>,
    branch_full_name: Option<String>,
    mode: WorktreeRemovalMode,
    stash_message: Option<String>,
    state: State<'_, AppState>,
) -> Result<RemoveWorktreeResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    let mutation = mutation_lock(&state, &repository_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .remove_worktree_and_branch(
                &repository_path,
                &RemoveWorktreeRequest {
                    path,
                    expected_head,
                    branch_full_name,
                    mode,
                    stash_message,
                },
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("worktree removal task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_fetch(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<FetchRepositoryResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    let mutation = mutation_lock(&state, &repository_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .fetch_repository(&repository_path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("fetch task failed: {error}"),
    })?
}

#[tauri::command]
async fn repository_maintenance_stats(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<maintenance_stats::RepositoryMaintenanceStatisticsResponse, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let last_opened_by_path = lock_catalog(&state)?
        .list()?
        .into_iter()
        .filter_map(|repository| {
            let last_opened = repository.last_opened_at?;
            std::fs::canonicalize(repository.canonical_path)
                .ok()
                .map(|path| (path, last_opened))
        })
        .collect::<HashMap<_, _>>();
    let repositories = state.repositories.clone();
    let scanned_at = unix_timestamp()?;
    tauri::async_runtime::spawn_blocking(move || {
        let navigation = repositories
            .navigation(&repository_path)
            .map_err(CommandError::from)?;
        let repository_last_commit_at = repositories
            .history_page(&repository_path, 1, None)
            .ok()
            .and_then(|page| page.commits.into_iter().next())
            .map(|commit| commit.author.authored_at);
        let worktrees = navigation.worktrees.into_iter().map(|worktree| {
            let last_commit_at = repositories
                .history_page(Path::new(&worktree.path), 1, None)
                .ok()
                .and_then(|page| page.commits.into_iter().next())
                .map(|commit| commit.author.authored_at);
            (worktree.path, worktree.branch, last_commit_at)
        });
        maintenance_stats::scan_repository_maintenance(
            &repository_path,
            repository_last_commit_at,
            worktrees,
            &last_opened_by_path,
            scanned_at,
        )
        .map_err(|message| CommandError { message })
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository maintenance scan task failed: {error}"),
    })?
}

#[tauri::command]
async fn list_remembered_repositories(
    state: State<'_, AppState>,
) -> Result<Vec<RememberedRepository>, CommandError> {
    let repositories = lock_catalog(&state)?.list()?;
    let refreshes = tauri::async_runtime::spawn_blocking(move || {
        repositories
            .into_iter()
            .map(|repository| {
                let availability = repository_availability(&repository.canonical_path);
                let identity = (availability == RepositoryAvailability::Available)
                    .then(|| catalog_git_identity(Path::new(&repository.canonical_path)).ok())
                    .flatten();
                RepositoryCatalogReconciliation {
                    repository_id: repository.id,
                    availability,
                    git_identity: identity,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository catalog refresh task failed: {error}"),
    })?;
    let mut catalog = lock_catalog(&state)?;
    let now = unix_timestamp()?;
    catalog.reconcile(&refreshes, now)?;
    catalog.list().map_err(CommandError::from)
}

#[tauri::command]
async fn repository_submodules(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<RepositorySubmodules, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .submodules(&repository_path)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository submodule task failed: {error}"),
    })?
}

#[tauri::command]
fn list_repository_relations(
    state: State<'_, AppState>,
) -> Result<Vec<RepositoryGroupRelation>, CommandError> {
    lock_catalog(&state)?
        .list_relations()
        .map_err(CommandError::from)
}

fn safe_submodule_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

async fn remember_repository_internal(
    repository_path: String,
    state: &State<'_, AppState>,
) -> Result<RememberedRepository, CommandError> {
    let repositories = state.repositories.clone();
    let path_for_validation = repository_path.clone();
    let (metadata, git_identity) = tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&path_for_validation);
        repositories.status(path).map_err(CommandError::from)?;
        Ok::<_, CommandError>((
            repositories.metadata(path).ok(),
            catalog_git_identity(path)?,
        ))
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository validation task failed: {error}"),
    })??;

    let display_name = Path::new(&repository_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Repository")
        .to_owned();
    let now = unix_timestamp()?;
    let input = RememberRepositoryInput {
        id: Uuid::new_v4().to_string(),
        path: repository_path,
        display_name,
        provider: metadata
            .as_ref()
            .map_or(RepositoryProvider::Local, |metadata| metadata.provider),
        transport: metadata
            .as_ref()
            .map_or(RepositoryTransport::Local, |metadata| metadata.transport),
        hosted_identity: metadata
            .as_ref()
            .and_then(|metadata| metadata.hosted_identity.clone()),
        git_identity: Some(git_identity),
        now,
    };
    let mut catalog = lock_catalog(state)?;
    let remembered = catalog.upsert(&input)?;
    catalog.touch_opened(&remembered.id, now)?;
    if metadata.is_none() {
        catalog.update_health(
            &remembered.id,
            &RepositoryHealthUpdate {
                git: Some(IntegrationHealth {
                    state: IntegrationHealthState::Degraded,
                    issue: Some(IntegrationHealthIssue::InvalidConfiguration),
                    checked_at: Some(now),
                }),
                github: None,
                now,
            },
        )?;
    }
    catalog
        .list()?
        .into_iter()
        .find(|repository| repository.id == remembered.id)
        .ok_or_else(|| CommandError {
            message: "remembered repository could not be reloaded".to_owned(),
        })
}

#[tauri::command]
async fn remember_repository(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RememberedRepository, CommandError> {
    remember_repository_internal(repository_path, &state).await
}

#[tauri::command]
async fn open_submodule_repository(
    parent_repository_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<RememberedRepository, CommandError> {
    if !safe_submodule_relative_path(&path) {
        return Err(CommandError {
            message: "submodule path must be a safe repository-relative path".to_owned(),
        });
    }
    let mut parent = lock_catalog(&state)?
        .get(&parent_repository_id)?
        .ok_or_else(|| CommandError {
            message: format!("remembered repository not found: {parent_repository_id}"),
        })?;
    if parent.repository_group_id.is_none() {
        parent = remember_repository_internal(parent.canonical_path, &state).await?;
    }
    let parent_group_id = parent
        .repository_group_id
        .clone()
        .ok_or_else(|| CommandError {
            message: "parent repository identity is unavailable".to_owned(),
        })?;
    let canonical_parent = PathBuf::from(parent.canonical_path);
    let repositories = state.repositories.clone();
    let requested_path = path.clone();
    let (submodule, child_path) =
        tauri::async_runtime::spawn_blocking(move || {
            let submodules = repositories
                .submodules(&canonical_parent)
                .map_err(CommandError::from)?;
            let submodule = submodules
                .submodules
                .into_iter()
                .find(|submodule| submodule.path == requested_path)
                .ok_or_else(|| CommandError {
                    message: "submodule is not configured in the parent repository".to_owned(),
                })?;
            if !submodule.present || !submodule.initialized {
                return Err(CommandError {
                    message: "submodule must be present and initialized before it can be opened"
                        .to_owned(),
                });
            }
            if submodule.commit_state == SubmoduleCommitState::Unavailable
                || submodule.worktree_state == SubmoduleWorktreeState::Unavailable
            {
                return Err(CommandError {
                    message: "submodule repository state is unavailable".to_owned(),
                });
            }
            let canonical_parent =
                std::fs::canonicalize(&canonical_parent).map_err(|error| CommandError {
                    message: format!("parent repository path is unavailable: {error}"),
                })?;
            let child_path = std::fs::canonicalize(canonical_parent.join(&submodule.path))
                .map_err(|error| CommandError {
                    message: format!("submodule repository path is unavailable: {error}"),
                })?;
            if !child_path.starts_with(&canonical_parent) || child_path == canonical_parent {
                return Err(CommandError {
                    message: "submodule repository path escapes the parent repository".to_owned(),
                });
            }
            Ok::<_, CommandError>((submodule, child_path))
        })
        .await
        .map_err(|error| CommandError {
            message: format!("submodule validation task failed: {error}"),
        })??;

    let child_path = child_path
        .into_os_string()
        .into_string()
        .map_err(|_| CommandError {
            message: "submodule repository path is not valid UTF-8".to_owned(),
        })?;
    let remembered = remember_repository_internal(child_path, &state).await?;
    let child_group_id = remembered
        .repository_group_id
        .as_deref()
        .ok_or_else(|| CommandError {
            message: "submodule repository identity is unavailable".to_owned(),
        })?;
    lock_catalog(&state)?.link_submodule(
        &parent_group_id,
        child_group_id,
        &submodule.path,
        unix_timestamp()?,
    )?;
    Ok(remembered)
}

#[tauri::command]
async fn clone_repository(
    source_url: String,
    destination_parent: String,
    directory_name: String,
    state: State<'_, AppState>,
) -> Result<RememberedRepository, CommandError> {
    let request = CloneRepositoryRequest {
        source_url,
        destination_parent,
        directory_name,
    };
    let repositories = state.repositories.clone();
    let clone_result = {
        let _guard = state.clones.lock().await;
        tauri::async_runtime::spawn_blocking(move || {
            repositories
                .clone_repository(&request)
                .map_err(CommandError::from)
        })
        .await
        .map_err(|error| CommandError {
            message: format!("repository clone task failed: {error}"),
        })??
    };
    remember_repository_internal(clone_result.repository_path, &state).await
}

#[tauri::command]
fn set_repository_pinned(
    repository_id: String,
    pinned: bool,
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    lock_catalog(&state)?
        .set_pinned(&repository_id, pinned, unix_timestamp()?)
        .map_err(CommandError::from)
}

#[tauri::command]
fn forget_repository(
    repository_id: String,
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    lock_catalog(&state)?
        .forget(&repository_id)
        .map_err(CommandError::from)
}

#[tauri::command]
async fn select_repository_directory(
    app: AppHandle,
    initial_path: Option<String>,
) -> Result<SelectRepositoryDirectoryResponse, CommandError> {
    let mut dialog = app.dialog().file().set_title("Select a Git repository");

    if let Some(initial_path) = initial_path.filter(|path| Path::new(path).is_dir()) {
        dialog = dialog.set_directory(initial_path);
    }

    let path = dialog
        .blocking_pick_folder()
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| CommandError {
                    message: format!("selected directory path is invalid: {error}"),
                })
        })
        .transpose()?;

    Ok(SelectRepositoryDirectoryResponse { path })
}

#[tauri::command]
async fn select_clone_parent_directory(
    app: AppHandle,
    initial_path: Option<String>,
) -> Result<SelectRepositoryDirectoryResponse, CommandError> {
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Select the parent folder for the clone");

    if let Some(initial_path) = initial_path.filter(|path| Path::new(path).is_dir()) {
        dialog = dialog.set_directory(initial_path);
    }

    let path = dialog
        .blocking_pick_folder()
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| CommandError {
                    message: format!("selected directory path is invalid: {error}"),
                })
        })
        .transpose()?;

    Ok(SelectRepositoryDirectoryResponse { path })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;
            let diagnostics = diagnostics::DiagnosticsState::open(
                app.path().home_dir()?.join(".skibidibi-git"),
                app.path().app_config_dir()?.join("data-root.json"),
            )?;
            let catalog = RepositoryCatalog::open(data_directory.join("repositories.sqlite3"))?;
            let github_accounts =
                app_store::GitHubAccountStore::open(data_directory.join("github.sqlite3"))?;
            app.manage(AppState::new(catalog, github_accounts, diagnostics)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ai::ai_cli_status,
            ai::ai_commander_turn,
            ai::ai_generate_commit_message,
            ai::ai_task_review_preflight,
            ai::ai_generate_task_review,
            ai::code_review_list,
            ai::code_review_read,
            diagnostics::diagnostics_settings,
            diagnostics::diagnostics_update_settings,
            diagnostics::diagnostics_read,
            diagnostics::diagnostics_clear,
            diagnostics::select_diagnostics_directory,
            external_workspace::open_branch_workspace,
            updater::application_update_check,
            updater::application_update_install,
            updater::application_release_notes,
            updater::application_update_restart,
            set_application_zoom,
            repository_status,
            repository_history,
            repository_branch_history,
            repository_commit_detail,
            repository_file_diff,
            repository_compare_refs,
            repository_compare_ref_file_diff,
            repository_file_history,
            repository_file_blame,
            repository_stash_detail,
            repository_stash_file_diff,
            repository_push_analysis,
            repository_pull,
            repository_merge_branch,
            repository_pull_inactive_branch,
            repository_worktree_dirty_states,
            repository_push,
            repository_set_upstream,
            repository_conflicts,
            repository_conflict_detail,
            repository_resolve_conflict,
            repository_working_tree_file_diff,
            repository_apply_index_change,
            repository_discard_worktree_changes,
            repository_discard_worktree_hunk,
            repository_create_commit,
            repository_amend_commit,
            repository_cherry_pick_commit,
            repository_revert_commit,
            repository_reset_commit,
            repository_navigation,
            create_repository_branch,
            repository_push_stash,
            repository_apply_stash,
            repository_pop_stash,
            repository_drop_stash,
            switch_repository_branch,
            delete_repository_branch,
            remove_repository_worktree,
            repository_fetch,
            repository_maintenance_stats,
            select_repository_directory,
            select_clone_parent_directory,
            list_remembered_repositories,
            repository_submodules,
            list_repository_relations,
            remember_repository,
            open_submodule_repository,
            clone_repository,
            set_repository_pinned,
            forget_repository,
            github::github_list_accounts,
            github::github_start_device_flow,
            github::github_poll_device_flow,
            github::github_cancel_device_flow,
            github::github_open_device_verification,
            github::github_connect_pat,
            github::github_connect_cli,
            github::github_disconnect_account,
            github::github_list_repositories,
            github::github_list_pull_requests,
            github::github_pull_request_detail,
            github::github_pull_request_files,
            github::github_approve_pull_request
        ])
        .run(tauri::generate_context!())
        .expect("error while running the desktop application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn git(args: &[&str], current_dir: &Path) {
        let output = Command::new("git")
            .args(args)
            .current_dir(current_dir)
            .output()
            .expect("git should run in the identity fixture");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn application_zoom_accepts_only_the_supported_finite_range() {
        assert_eq!(validated_application_zoom(1.0).unwrap(), 1.0);
        assert_eq!(validated_application_zoom(1.5).unwrap(), 1.5);
        assert!(validated_application_zoom(0.99).is_err());
        assert!(validated_application_zoom(1.51).is_err());
        assert!(validated_application_zoom(f64::NAN).is_err());
        assert!(validated_application_zoom(f64::INFINITY).is_err());
    }

    #[test]
    fn main_and_linked_worktrees_share_the_common_directory() {
        let fixture = tempdir().unwrap();
        let main = fixture.path().join("main");
        let linked = fixture.path().join("linked");
        std::fs::create_dir(&main).unwrap();
        git(&["init"], &main);
        std::fs::write(main.join("tracked.txt"), "fixture").unwrap();
        git(&["add", "tracked.txt"], &main);
        git(
            &[
                "-c",
                "user.name=Skibidibi Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
            &main,
        );
        git(
            &[
                "worktree",
                "add",
                "-b",
                "codex/test-linked",
                linked.to_str().unwrap(),
            ],
            &main,
        );

        let main_identity = resolve_git_identity(&main).unwrap();
        let linked_identity = resolve_git_identity(&linked).unwrap();

        assert_eq!(main_identity.worktree_role, RepositoryWorktreeRole::Main);
        assert_eq!(
            linked_identity.worktree_role,
            RepositoryWorktreeRole::Linked
        );
        assert_eq!(main_identity.common_dir, linked_identity.common_dir);
    }

    #[test]
    fn submodule_opening_accepts_only_normal_relative_paths() {
        assert!(safe_submodule_relative_path("modules/client"));
        assert!(!safe_submodule_relative_path(""));
        assert!(!safe_submodule_relative_path("../outside"));
        assert!(!safe_submodule_relative_path("modules/../outside"));
        assert!(!safe_submodule_relative_path("/absolute/client"));
    }

    #[test]
    fn bare_repository_is_its_own_common_directory() {
        let fixture = tempdir().unwrap();
        let bare = fixture.path().join("repository.git");
        git(&["init", "--bare", bare.to_str().unwrap()], fixture.path());

        let identity = resolve_git_identity(&bare).unwrap();

        assert_eq!(identity.worktree_role, RepositoryWorktreeRole::Bare);
        assert_eq!(identity.common_dir, std::fs::canonicalize(bare).unwrap());
    }

    #[test]
    fn separate_git_directory_is_a_main_worktree() {
        let fixture = tempdir().unwrap();
        let worktree = fixture.path().join("worktree");
        let git_dir = fixture.path().join("metadata.git");
        git(
            &[
                "init",
                "--separate-git-dir",
                git_dir.to_str().unwrap(),
                worktree.to_str().unwrap(),
            ],
            fixture.path(),
        );

        let identity = resolve_git_identity(&worktree).unwrap();

        assert_eq!(identity.worktree_role, RepositoryWorktreeRole::Main);
        assert_eq!(identity.common_dir, std::fs::canonicalize(git_dir).unwrap());
    }
}
