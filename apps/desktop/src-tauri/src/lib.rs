use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use app_domain::{
    CommitDetails, CommitHistoryPage, FileDiff, IntegrationHealth, IntegrationHealthIssue,
    IntegrationHealthState, RememberRepositoryInput, RememberedRepository, RepositoryAvailability,
    RepositoryHealthUpdate, RepositoryNavigation, RepositoryProvider, RepositoryStatus,
    RepositoryTransport, SwitchBranchRequest, SwitchBranchResult,
};
use app_store::{CatalogError, RepositoryCatalog};
use repo_runtime::{
    BranchSwitchError, FileDiffRuntimeError, HistoryRuntimeError, NavigationRuntimeError,
    RepositoryRuntime, RepositoryRuntimeError,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

struct AppState {
    repositories: RepositoryRuntime,
    catalog: Mutex<RepositoryCatalog>,
    mutations: Arc<Mutex<()>>,
}

impl AppState {
    fn new(catalog: RepositoryCatalog) -> Self {
        Self {
            repositories: RepositoryRuntime::default(),
            catalog: Mutex::new(catalog),
            mutations: Arc::new(Mutex::new(())),
        }
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
            message: error.to_string(),
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
    full_name: String,
    state: State<'_, AppState>,
) -> Result<SwitchBranchResult, CommandError> {
    let repository_path = resolve_repository_path(&repository_id, &state)?;
    let repositories = state.repositories.clone();
    let mutations = Arc::clone(&state.mutations);
    tauri::async_runtime::spawn_blocking(move || {
        let _mutation_guard = mutations.lock().map_err(|_| CommandError {
            message: "repository mutation queue is unavailable".to_owned(),
        })?;
        repositories
            .switch_branch(&repository_path, &SwitchBranchRequest { full_name })
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("branch switch task failed: {error}"),
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
            repository_navigation,
            switch_repository_branch,
            select_repository_directory,
            list_remembered_repositories,
            remember_repository,
            set_repository_pinned,
            forget_repository
        ])
        .run(tauri::generate_context!())
        .expect("error while running the desktop application");
}
