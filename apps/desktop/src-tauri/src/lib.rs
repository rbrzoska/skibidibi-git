use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

use app_domain::{
    ApplyIndexChangeRequest, ApplyIndexChangeResult, CommitDetails, CommitHistoryPage,
    CreateCommitRequest, CreateCommitResult, DeleteBranchRequest, DeleteBranchResult,
    FetchRepositoryResult, FileDiff, IntegrationHealth, IntegrationHealthIssue,
    IntegrationHealthState, RememberRepositoryInput, RememberedRepository, RemoveWorktreeRequest,
    RemoveWorktreeResult, RepositoryAvailability, RepositoryHealthUpdate, RepositoryNavigation,
    RepositoryProvider, RepositoryStatus, RepositoryTransport, SwitchBranchRequest,
    SwitchBranchResult, WorkingTreeFileDiff,
};
use app_store::{CatalogError, RepositoryCatalog};
use repo_runtime::{
    BranchSwitchError, FileDiffRuntimeError, HistoryRuntimeError, MaintenanceError,
    MutationRuntimeError, NavigationRuntimeError, RepositoryRuntime, RepositoryRuntimeError,
    WorkingTreeDiffRuntimeError,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

struct AppState {
    repositories: RepositoryRuntime,
    catalog: Mutex<RepositoryCatalog>,
    mutations: MutationLockRegistry,
}

impl AppState {
    fn new(catalog: RepositoryCatalog) -> Self {
        Self {
            repositories: RepositoryRuntime::default(),
            catalog: Mutex::new(catalog),
            mutations: MutationLockRegistry::default(),
        }
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
struct CommandError {
    message: String,
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

fn resolve_git_common_dir(repository: &Path) -> Result<PathBuf, CommandError> {
    let repository = std::fs::canonicalize(repository).map_err(|error| CommandError {
        message: format!("repository path is unavailable: {error}"),
    })?;
    let dot_git = repository.join(".git");
    let git_dir = if dot_git.is_dir() {
        std::fs::canonicalize(dot_git)
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
        std::fs::canonicalize(if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            repository.join(candidate)
        })
    } else if repository.join("HEAD").is_file() && repository.join("objects").is_dir() {
        Ok(repository)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Git directory not found",
        ))
    }
    .map_err(|error| CommandError {
        message: format!("Git directory is unavailable: {error}"),
    })?;

    let common_pointer = git_dir.join("commondir");
    if !common_pointer.is_file() {
        return Ok(git_dir);
    }
    let value = read_small_git_pointer(&common_pointer)?;
    let candidate = Path::new(value.trim());
    std::fs::canonicalize(if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        git_dir.join(candidate)
    })
    .map_err(|error| CommandError {
        message: format!("Git common directory is unavailable: {error}"),
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
fn list_remembered_repositories(
    state: State<'_, AppState>,
) -> Result<Vec<RememberedRepository>, CommandError> {
    let repositories = lock_catalog(&state)?.list()?;
    let availability = repositories
        .iter()
        .map(|repository| {
            (
                repository.id.clone(),
                repository_availability(&repository.canonical_path),
            )
        })
        .collect::<Vec<_>>();
    let catalog = lock_catalog(&state)?;
    for (repository_id, availability) in availability {
        catalog.set_availability(&repository_id, availability, unix_timestamp()?)?;
    }
    catalog.list().map_err(CommandError::from)
}

#[tauri::command]
async fn remember_repository(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RememberedRepository, CommandError> {
    let repositories = state.repositories.clone();
    let path_for_validation = repository_path.clone();
    let metadata = tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&path_for_validation);
        repositories.status(path).map_err(CommandError::from)?;
        Ok::<_, CommandError>(repositories.metadata(path).ok())
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
        now,
    };
    let mut catalog = lock_catalog(&state)?;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_directory)?;
            let catalog = RepositoryCatalog::open(data_directory.join("repositories.sqlite3"))?;
            app.manage(AppState::new(catalog));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            repository_status,
            repository_history,
            repository_commit_detail,
            repository_file_diff,
            repository_working_tree_file_diff,
            repository_apply_index_change,
            repository_create_commit,
            repository_navigation,
            switch_repository_branch,
            delete_repository_branch,
            remove_repository_worktree,
            repository_fetch,
            select_repository_directory,
            list_remembered_repositories,
            remember_repository,
            set_repository_pinned,
            forget_repository
        ])
        .run(tauri::generate_context!())
        .expect("error while running the desktop application");
}
