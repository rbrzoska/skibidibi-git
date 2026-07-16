use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU8, Ordering},
    },
};

use app_domain::{
    GitHubAccountState, GitHubAccountSummary, GitHubAuthKind, GitHubDeviceFlowStart,
    GitHubDeviceFlowState, GitHubRateLimit, GitHubRepository, IntegrationHealth,
    IntegrationHealthIssue, IntegrationHealthState, IssueComment, PullRequestDetail,
    PullRequestState, PullRequestSummary, RepositoryHealthUpdate, ReviewComment, ReviewThread,
};
use app_store::{
    GitHubAccount, GitHubAccountStore, UpsertGitHubAccount, UpsertGitHubRepositoryBinding,
};
use github_client::{
    DeviceCode, DeviceFlowError, DeviceFlowErrorCode, DeviceFlowPoll, GitHubClient,
    GitHubClientError, GitHubErrorCode, OAuthRefreshToken, OAuthTokenSet, PersonalAccessToken,
    PullRequestListState,
};
use secret_store::CredentialKey;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::{AppState, CommandError, lock_catalog, unix_timestamp};

const GITHUB_HOST: &str = "github.com";
const GITHUB_CREDENTIAL_SERVICE: &str = "skibidibi-git/github.com";
const DETAIL_PAGE_SIZE: u16 = 50;
const MAX_DETAIL_PAGES: usize = 5;
const OAUTH_CREDENTIAL_VERSION: u8 = 1;
const GITHUB_CLIENT_ID: &str = "Iv23limxphzk5jQdoAhu";
const OAUTH_REFRESH_SKEW_SECONDS: i64 = 60;
const MAX_DEVICE_FLOW_SESSIONS: usize = 8;
const DEVICE_FLOW_ACTIVE: u8 = 0;
const DEVICE_FLOW_AUTHORIZED: u8 = 1;
const DEVICE_FLOW_CANCELLED: u8 = 2;

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct OAuthCredentialEnvelope {
    version: u8,
    access_token: String,
    refresh_token: Option<String>,
    access_token_expires_at: Option<i64>,
    refresh_token_expires_at: Option<i64>,
}

impl OAuthCredentialEnvelope {
    fn encode(&self) -> Result<Zeroizing<String>, CommandError> {
        serde_json::to_string(self)
            .map(Zeroizing::new)
            .map_err(|_| CommandError {
                message: "GitHub credential could not be encoded".to_owned(),
            })
    }

    fn decode(secret: &str) -> Result<Self, CommandError> {
        let credential: Self = serde_json::from_str(secret).map_err(|_| CommandError {
            message: "Stored GitHub OAuth credential is invalid".to_owned(),
        })?;
        if credential.version != OAUTH_CREDENTIAL_VERSION
            || credential.access_token.trim().is_empty()
            || credential
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.trim().is_empty())
        {
            return Err(CommandError {
                message: "Stored GitHub OAuth credential is invalid".to_owned(),
            });
        }
        Ok(credential)
    }
}

pub(crate) struct GitHubDeviceFlowSessions {
    entries: Mutex<HashMap<String, Arc<DeviceFlowSession>>>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl Default for GitHubDeviceFlowSessions {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            permits: Arc::new(tokio::sync::Semaphore::new(MAX_DEVICE_FLOW_SESSIONS)),
        }
    }
}

struct DeviceFlowSession {
    device_code: DeviceCode,
    verification_uri: String,
    expires_at: i64,
    terminal_state: AtomicU8,
    schedule: Mutex<DeviceFlowSchedule>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

struct DeviceFlowSchedule {
    interval_seconds: u64,
    next_poll_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitHubDeviceFlowPollResponse {
    state: GitHubDeviceFlowState,
    next_poll_at: Option<i64>,
    account: Option<GitHubAccountSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelGitHubDeviceFlowResponse {
    cancelled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitHubCommandError {
    message: String,
    code: String,
    retryable: bool,
    request_id: Option<String>,
    rate_limit: Option<Box<GitHubRateLimit>>,
}

impl From<CommandError> for GitHubCommandError {
    fn from(error: CommandError) -> Self {
        Self {
            message: error.message,
            code: "internal".to_owned(),
            retryable: false,
            request_id: None,
            rate_limit: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DisconnectGitHubAccountResponse {
    disconnected: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitHubPullRequestListResponse {
    pull_requests: Vec<GitHubPullRequestSummaryResponse>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitHubRepositoryListResponse {
    repositories: Vec<GitHubRepository>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPullRequestSummaryResponse {
    number: u64,
    title: String,
    url: String,
    state: PullRequestState,
    draft: bool,
    author_login: String,
    head_ref_name: String,
    base_ref_name: String,
    updated_at: String,
    authored_by_viewer: bool,
    review_requested_from_viewer: Option<bool>,
    unresolved_thread_count: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitHubPullRequestDetailResponse {
    #[serde(flatten)]
    summary: GitHubPullRequestSummaryResponse,
    body: String,
    additions: u64,
    deletions: u64,
    changed_files: u64,
    mergeability: app_domain::PullRequestMergeability,
    comments: Vec<GitHubCommentResponse>,
    review_threads: Vec<GitHubReviewThreadResponse>,
    conversation_truncated: bool,
    review_threads_truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitHubCommentResponse {
    id: String,
    author_login: String,
    body: String,
    created_at: String,
    updated_at: String,
    url: Option<String>,
    path: Option<String>,
    line: Option<u64>,
    side: Option<app_domain::ReviewCommentSide>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitHubReviewThreadResponse {
    id: String,
    path: String,
    line: Option<u64>,
    resolved: bool,
    outdated: bool,
    comments: Vec<GitHubCommentResponse>,
}

#[tauri::command]
pub(crate) fn github_list_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<GitHubAccountSummary>, GitHubCommandError> {
    Ok(lock_accounts(&state)?
        .list_accounts()
        .map_err(command_error)?
        .iter()
        .map(GitHubAccount::summary)
        .collect())
}

#[tauri::command]
pub(crate) async fn github_start_device_flow(
    state: State<'_, AppState>,
) -> Result<GitHubDeviceFlowStart, GitHubCommandError> {
    let now = unix_timestamp()?;
    lock_device_sessions(&state)?.retain(|_, existing| {
        existing.terminal_state.load(Ordering::Acquire) == DEVICE_FLOW_ACTIVE
            && existing.expires_at > now
    });
    let permit = Arc::clone(&state.github_device_sessions.permits)
        .try_acquire_owned()
        .map_err(|_| GitHubCommandError {
            message: "Too many GitHub sign-in sessions are active; cancel one and retry".to_owned(),
            code: "tooManyDeviceFlows".to_owned(),
            retryable: true,
            request_id: None,
            rate_limit: None,
        })?;
    let client = state.github_device_flow.clone();
    let authorization =
        tauri::async_runtime::spawn_blocking(move || client.start(GITHUB_CLIENT_ID))
            .await
            .map_err(|error| CommandError {
                message: format!("GitHub Device Flow task failed: {error}"),
            })?
            .map_err(device_flow_command_error)?;
    let now = unix_timestamp()?;
    let expires_at = future_timestamp(now, authorization.expires_in_seconds)?;
    let next_poll_at = future_timestamp(now, authorization.interval_seconds)?;
    let flow_id = Uuid::new_v4().to_string();
    let verification_uri = authorization.verification_uri.to_string();
    let response = GitHubDeviceFlowStart {
        flow_id: flow_id.clone(),
        user_code: authorization.user_code,
        verification_uri: verification_uri.clone(),
        expires_at,
        interval_seconds: authorization.interval_seconds,
    };
    let session = Arc::new(DeviceFlowSession {
        device_code: authorization.device_code,
        verification_uri,
        expires_at,
        terminal_state: AtomicU8::new(DEVICE_FLOW_ACTIVE),
        schedule: Mutex::new(DeviceFlowSchedule {
            interval_seconds: authorization.interval_seconds,
            next_poll_at,
        }),
        _permit: permit,
    });
    let mut sessions = lock_device_sessions(&state)?;
    sessions.insert(flow_id, session);
    drop(sessions);
    Ok(response)
}

#[tauri::command]
pub(crate) async fn github_poll_device_flow(
    flow_id: String,
    state: State<'_, AppState>,
) -> Result<GitHubDeviceFlowPollResponse, GitHubCommandError> {
    let session = lock_device_sessions(&state)?
        .get(&flow_id)
        .cloned()
        .ok_or_else(invalid_device_flow_error)?;
    let now = unix_timestamp()?;
    if session.terminal_state.load(Ordering::Acquire) != DEVICE_FLOW_ACTIVE {
        remove_device_session(&state, &flow_id)?;
        return Ok(device_flow_terminal(GitHubDeviceFlowState::Denied));
    }
    if now >= session.expires_at {
        remove_device_session(&state, &flow_id)?;
        return Ok(device_flow_terminal(GitHubDeviceFlowState::Expired));
    }
    {
        let mut schedule = session.schedule.lock().map_err(|_| CommandError {
            message: "GitHub Device Flow schedule is unavailable".to_owned(),
        })?;
        if now < schedule.next_poll_at {
            return Ok(device_flow_pending(schedule.next_poll_at));
        }
        schedule.next_poll_at = future_timestamp(now, schedule.interval_seconds)?;
    }

    let client = state.github_device_flow.clone();
    let session_for_poll = Arc::clone(&session);
    let result = tauri::async_runtime::spawn_blocking(move || {
        client.poll(GITHUB_CLIENT_ID, &session_for_poll.device_code)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("GitHub Device Flow polling task failed: {error}"),
    })?
    .map_err(device_flow_command_error)?;

    if session.terminal_state.load(Ordering::Acquire) != DEVICE_FLOW_ACTIVE {
        remove_device_session(&state, &flow_id)?;
        return Ok(device_flow_terminal(GitHubDeviceFlowState::Denied));
    }
    if unix_timestamp()? >= session.expires_at {
        session
            .terminal_state
            .store(DEVICE_FLOW_CANCELLED, Ordering::Release);
        remove_device_session(&state, &flow_id)?;
        return Ok(device_flow_terminal(GitHubDeviceFlowState::Expired));
    }

    match result {
        DeviceFlowPoll::Pending => {
            let next_poll_at = session
                .schedule
                .lock()
                .map_err(|_| CommandError {
                    message: "GitHub Device Flow schedule is unavailable".to_owned(),
                })?
                .next_poll_at;
            Ok(device_flow_pending(next_poll_at))
        }
        DeviceFlowPoll::SlowDown => {
            let now = unix_timestamp()?;
            let mut schedule = session.schedule.lock().map_err(|_| CommandError {
                message: "GitHub Device Flow schedule is unavailable".to_owned(),
            })?;
            schedule.interval_seconds = schedule
                .interval_seconds
                .checked_add(5)
                .ok_or_else(invalid_device_flow_error)?;
            schedule.next_poll_at = future_timestamp(now, schedule.interval_seconds)?;
            Ok(device_flow_pending(schedule.next_poll_at))
        }
        DeviceFlowPoll::Expired => {
            remove_device_session(&state, &flow_id)?;
            Ok(device_flow_terminal(GitHubDeviceFlowState::Expired))
        }
        DeviceFlowPoll::AccessDenied => {
            remove_device_session(&state, &flow_id)?;
            Ok(device_flow_terminal(GitHubDeviceFlowState::Denied))
        }
        DeviceFlowPoll::Authorized(tokens) => {
            if session
                .terminal_state
                .compare_exchange(
                    DEVICE_FLOW_ACTIVE,
                    DEVICE_FLOW_AUTHORIZED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                return Ok(device_flow_terminal(GitHubDeviceFlowState::Denied));
            }
            remove_device_session(&state, &flow_id)?;
            let account = connect_oauth_tokens(tokens, &state).await?;
            Ok(GitHubDeviceFlowPollResponse {
                state: GitHubDeviceFlowState::Authorized,
                next_poll_at: None,
                account: Some(account),
            })
        }
    }
}

#[tauri::command]
pub(crate) fn github_cancel_device_flow(
    flow_id: String,
    state: State<'_, AppState>,
) -> Result<CancelGitHubDeviceFlowResponse, GitHubCommandError> {
    let removed = lock_device_sessions(&state)?.remove(&flow_id);
    let cancelled = removed.as_ref().is_some_and(|session| {
        session
            .terminal_state
            .compare_exchange(
                DEVICE_FLOW_ACTIVE,
                DEVICE_FLOW_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    });
    Ok(CancelGitHubDeviceFlowResponse { cancelled })
}

#[tauri::command]
pub(crate) fn github_open_device_verification(
    flow_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), GitHubCommandError> {
    let session = lock_device_sessions(&state)?
        .get(&flow_id)
        .cloned()
        .ok_or_else(invalid_device_flow_error)?;
    let now = unix_timestamp()?;
    if session.terminal_state.load(Ordering::Acquire) != DEVICE_FLOW_ACTIVE
        || now >= session.expires_at
    {
        return Err(invalid_device_flow_error().into());
    }
    app.opener()
        .open_url(&session.verification_uri, None::<&str>)
        .map_err(|error| GitHubCommandError {
            message: format!("GitHub sign-in page could not be opened: {error}"),
            code: "openVerificationFailed".to_owned(),
            retryable: true,
            request_id: None,
            rate_limit: None,
        })
}

#[tauri::command]
pub(crate) async fn github_connect_pat(
    token: String,
    state: State<'_, AppState>,
) -> Result<GitHubAccountSummary, GitHubCommandError> {
    let token = Zeroizing::new(token);
    let credential = PersonalAccessToken::new(token.to_string()).map_err(command_error)?;
    let client = GitHubClient::new(
        state.github_config.clone(),
        state.github_transport.clone(),
        credential,
    );
    let validation = tauri::async_runtime::spawn_blocking(move || client.validate_pat())
        .await
        .map_err(|error| CommandError {
            message: format!("GitHub validation task failed: {error}"),
        })?
        .map_err(client_command_error)?;
    let provider_user_id = validation
        .user
        .id
        .ok_or_else(|| CommandError {
            message: "GitHub did not return a stable user identifier".to_owned(),
        })?
        .to_string();
    let account_id = format!("{GITHUB_HOST}:{provider_user_id}");
    let key = credential_key(&provider_user_id)?;
    let _mutation_guard = state.github_mutations.lock().await;
    let now = unix_timestamp()?;
    let credentials = Arc::clone(&state.github_credentials);
    let secret = Zeroizing::new(token.to_string());
    let key_for_write = key.clone();
    let previous_secret = tauri::async_runtime::spawn_blocking(move || {
        let previous = credentials.read(&key_for_write)?.map(Zeroizing::new);
        credentials.write(&key_for_write, secret.as_str())?;
        Ok::<_, secret_store::CredentialStoreError>(previous)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("credential storage task failed: {error}"),
    })?
    .map_err(command_error)?;

    let stored = match lock_accounts(&state) {
        Ok(mut accounts) => accounts
            .upsert_account(&UpsertGitHubAccount {
                id: account_id,
                host: GITHUB_HOST.to_owned(),
                provider_user_id,
                login: validation.user.login,
                display_name: validation.user.display_name,
                avatar_url: validation.user.avatar_url,
                auth_kind: GitHubAuthKind::PersonalAccessToken,
                scopes: validation.scopes,
                state: GitHubAccountState::Connected,
                access_token_expires_at: None,
                last_validated_at: Some(now),
                now,
            })
            .map_err(command_error),
        Err(error) => Err(error),
    };
    match stored {
        Ok(account) => {
            bump_account_generation(&state, &account.id);
            Ok(account.summary())
        }
        Err(error) => {
            let credentials = Arc::clone(&state.github_credentials);
            let rollback = tauri::async_runtime::spawn_blocking(move || match previous_secret {
                Some(previous) => credentials.write(&key, previous.as_str()),
                None => credentials.delete(&key).map(|_| ()),
            })
            .await;
            match rollback {
                Ok(Ok(())) => Err(error.into()),
                _ => Err(GitHubCommandError {
                    message:
                        "GitHub account metadata failed and credential rollback was incomplete"
                            .to_owned(),
                    code: "credentialRollbackFailed".to_owned(),
                    retryable: false,
                    request_id: None,
                    rate_limit: None,
                }),
            }
        }
    }
}

#[tauri::command]
pub(crate) async fn github_disconnect_account(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<DisconnectGitHubAccountResponse, GitHubCommandError> {
    let _mutation_guard = state.github_mutations.lock().await;
    let Some(account) = lock_accounts(&state)?
        .get_account(&account_id)
        .map_err(command_error)?
    else {
        return Ok(DisconnectGitHubAccountResponse {
            disconnected: false,
        });
    };
    let key = credential_key(&account.provider_user_id)?;
    let credentials = Arc::clone(&state.github_credentials);
    let key_for_delete = key.clone();
    let previous_secret = tauri::async_runtime::spawn_blocking(move || {
        let previous = credentials.read(&key_for_delete)?.map(Zeroizing::new);
        credentials.delete(&key_for_delete)?;
        Ok::<_, secret_store::CredentialStoreError>(previous)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("credential deletion task failed: {error}"),
    })?
    .map_err(command_error)?;
    let delete_result = match lock_accounts(&state) {
        Ok(accounts) => accounts.delete_account(&account_id).map_err(command_error),
        Err(error) => Err(error),
    };
    match delete_result {
        Ok(disconnected) => {
            if disconnected {
                bump_account_generation(&state, &account_id);
            }
            Ok(DisconnectGitHubAccountResponse { disconnected })
        }
        Err(error) => {
            let credentials = Arc::clone(&state.github_credentials);
            let rollback = tauri::async_runtime::spawn_blocking(move || match previous_secret {
                Some(previous) => credentials.write(&key, previous.as_str()),
                None => Ok(()),
            })
            .await;
            match rollback {
                Ok(Ok(())) => Err(error.into()),
                _ => Err(GitHubCommandError {
                    message: "GitHub disconnect failed and credential rollback was incomplete"
                        .to_owned(),
                    code: "credentialRollbackFailed".to_owned(),
                    retryable: false,
                    request_id: None,
                    rate_limit: None,
                }),
            }
        }
    }
}

#[tauri::command]
pub(crate) async fn github_list_repositories(
    account_id: String,
    cursor: Option<String>,
    page_size: u16,
    state: State<'_, AppState>,
) -> Result<GitHubRepositoryListResponse, GitHubCommandError> {
    let account = lock_accounts(&state)?
        .get_account(&account_id)
        .map_err(command_error)?
        .ok_or_else(|| CommandError {
            message: "GitHub account is not connected".to_owned(),
        })?;
    let expected_generation = account_generation(&state, &account.id);
    let client = load_client(&state, &account, expected_generation).await?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        client.list_repositories(page_size, cursor.as_deref())
    })
    .await
    .map_err(|error| CommandError {
        message: format!("GitHub repository list task failed: {error}"),
    })?;
    let _finalize_guard = state.github_mutations.lock().await;
    if account_generation(&state, &account.id) != expected_generation {
        return Err(stale_account_error());
    }
    match result {
        Ok(page) => {
            if !update_account_state_locked(
                &state,
                &account,
                expected_generation,
                GitHubAccountState::Connected,
            )? {
                return Err(stale_account_error());
            }
            Ok(GitHubRepositoryListResponse {
                repositories: page.items,
                next_cursor: page.next_cursor,
            })
        }
        Err(error) => {
            if error.code == GitHubErrorCode::AuthenticationRequired
                && !update_account_state_locked(
                    &state,
                    &account,
                    expected_generation,
                    GitHubAccountState::AuthenticationRequired,
                )?
            {
                return Err(stale_account_error());
            }
            Err(client_command_error(error))
        }
    }
}

#[tauri::command]
pub(crate) async fn github_list_pull_requests(
    account_id: String,
    repository_id: String,
    cursor: Option<String>,
    page_size: u16,
    state: State<'_, AppState>,
) -> Result<GitHubPullRequestListResponse, GitHubCommandError> {
    let context = request_context(&state, &account_id, &repository_id)?;
    let viewer_login = context.account.login.clone();
    let owner = context.owner.clone();
    let name = context.name.clone();
    let client = match load_client(&state, &context.account, context.account_generation).await {
        Ok(client) => client,
        Err(error) => {
            if error.code == "authenticationRequired" {
                let _finalize_guard = state.github_mutations.lock().await;
                if account_generation(&state, &context.account.id) != context.account_generation {
                    return Err(stale_account_error());
                }
                mark_repository_github_issue(
                    &state,
                    &repository_id,
                    IntegrationHealthIssue::Authentication,
                )?;
            }
            return Err(error);
        }
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        client.list_pull_requests(
            &owner,
            &name,
            PullRequestListState::Open,
            page_size,
            cursor.as_deref(),
        )
    })
    .await
    .map_err(|error| CommandError {
        message: format!("GitHub pull request task failed: {error}"),
    })?;
    let _finalize_guard = state.github_mutations.lock().await;
    if account_generation(&state, &context.account.id) != context.account_generation {
        return Err(stale_account_error());
    }
    match result {
        Ok(page) => {
            if !update_account_state_locked(
                &state,
                &context.account,
                context.account_generation,
                GitHubAccountState::Connected,
            )? {
                return Err(stale_account_error());
            }
            mark_repository_github_health(&state, &repository_id, None)?;
            bind_repository(&state, &repository_id, &context)?;
            Ok(GitHubPullRequestListResponse {
                pull_requests: page
                    .items
                    .into_iter()
                    .map(|pull| summary_response(pull, &viewer_login, None))
                    .collect(),
                next_cursor: page.next_cursor,
            })
        }
        Err(error) => {
            if error.code == GitHubErrorCode::AuthenticationRequired
                && !update_account_state_locked(
                    &state,
                    &context.account,
                    context.account_generation,
                    GitHubAccountState::AuthenticationRequired,
                )?
            {
                return Err(stale_account_error());
            }
            mark_repository_github_health(&state, &repository_id, Some(&error))?;
            Err(client_command_error(error))
        }
    }
}

#[tauri::command]
pub(crate) async fn github_pull_request_detail(
    account_id: String,
    repository_id: String,
    number: u64,
    state: State<'_, AppState>,
) -> Result<GitHubPullRequestDetailResponse, GitHubCommandError> {
    let context = request_context(&state, &account_id, &repository_id)?;
    let viewer_login = context.account.login.clone();
    let owner = context.owner.clone();
    let name = context.name.clone();
    let client = match load_client(&state, &context.account, context.account_generation).await {
        Ok(client) => client,
        Err(error) => {
            if error.code == "authenticationRequired" {
                let _finalize_guard = state.github_mutations.lock().await;
                if account_generation(&state, &context.account.id) != context.account_generation {
                    return Err(stale_account_error());
                }
                mark_repository_github_issue(
                    &state,
                    &repository_id,
                    IntegrationHealthIssue::Authentication,
                )?;
            }
            return Err(error);
        }
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let detail = client.pull_request_detail(&owner, &name, number)?.value;
        let (comments, conversation_truncated) =
            collect_issue_comments(&client, &owner, &name, number)?;
        let (threads, review_threads_truncated) =
            collect_review_threads(&client, &owner, &name, number)?;
        Ok::<_, GitHubClientError>((
            detail,
            comments,
            threads,
            conversation_truncated,
            review_threads_truncated,
        ))
    })
    .await
    .map_err(|error| CommandError {
        message: format!("GitHub pull request detail task failed: {error}"),
    })?;
    let _finalize_guard = state.github_mutations.lock().await;
    if account_generation(&state, &context.account.id) != context.account_generation {
        return Err(stale_account_error());
    }
    match result {
        Ok((detail, comments, threads, conversation_truncated, review_threads_truncated)) => {
            if !update_account_state_locked(
                &state,
                &context.account,
                context.account_generation,
                GitHubAccountState::Connected,
            )? {
                return Err(stale_account_error());
            }
            mark_repository_github_health(&state, &repository_id, None)?;
            bind_repository(&state, &repository_id, &context)?;
            Ok(detail_response(
                detail,
                comments,
                threads,
                &viewer_login,
                conversation_truncated,
                review_threads_truncated,
            ))
        }
        Err(error) => {
            if error.code == GitHubErrorCode::AuthenticationRequired
                && !update_account_state_locked(
                    &state,
                    &context.account,
                    context.account_generation,
                    GitHubAccountState::AuthenticationRequired,
                )?
            {
                return Err(stale_account_error());
            }
            mark_repository_github_health(&state, &repository_id, Some(&error))?;
            Err(client_command_error(error))
        }
    }
}

struct GitHubRequestContext {
    account: GitHubAccount,
    account_generation: u64,
    owner: String,
    name: String,
}

fn request_context(
    state: &State<'_, AppState>,
    account_id: &str,
    repository_id: &str,
) -> Result<GitHubRequestContext, CommandError> {
    let account = lock_accounts(state)?
        .get_account(account_id)
        .map_err(command_error)?
        .ok_or_else(|| CommandError {
            message: "GitHub account is not connected".to_owned(),
        })?;
    let repository = lock_catalog(state)?
        .get(repository_id)?
        .ok_or_else(|| CommandError {
            message: format!("remembered repository not found: {repository_id}"),
        })?;
    let identity = repository.hosted_identity.ok_or_else(|| CommandError {
        message: "the repository does not have a GitHub remote identity".to_owned(),
    })?;
    if !identity.host.eq_ignore_ascii_case(&account.host)
        || !identity.host.eq_ignore_ascii_case(GITHUB_HOST)
    {
        return Err(CommandError {
            message: "the GitHub account host does not match the repository remote".to_owned(),
        });
    }
    let account_generation = account_generation(state, &account.id);
    Ok(GitHubRequestContext {
        account,
        account_generation,
        owner: identity.owner,
        name: identity.name,
    })
}

fn account_generation(state: &State<'_, AppState>, account_id: &str) -> u64 {
    *state
        .github_account_generations
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(account_id)
        .unwrap_or(&0)
}

fn bump_account_generation(state: &State<'_, AppState>, account_id: &str) {
    let mut generations = state
        .github_account_generations
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let generation = generations.entry(account_id.to_owned()).or_default();
    *generation = generation.wrapping_add(1);
}

async fn connect_oauth_tokens(
    tokens: OAuthTokenSet,
    state: &State<'_, AppState>,
) -> Result<GitHubAccountSummary, GitHubCommandError> {
    let issued_at = unix_timestamp()?;
    let access_token_expires_at =
        optional_future_timestamp(issued_at, tokens.access_token_expires_in_seconds)?;
    let refresh_token_expires_at =
        optional_future_timestamp(issued_at, tokens.refresh_token_expires_in_seconds)?;
    let credential = PersonalAccessToken::new(tokens.access_token.expose_secret().to_owned())
        .map_err(command_error)?;
    let client = GitHubClient::new(
        state.github_config.clone(),
        state.github_transport.clone(),
        credential,
    );
    let validation = tauri::async_runtime::spawn_blocking(move || client.validate_pat())
        .await
        .map_err(|error| CommandError {
            message: format!("GitHub OAuth validation task failed: {error}"),
        })?
        .map_err(client_command_error)?;
    let provider_user_id = validation
        .user
        .id
        .ok_or_else(|| CommandError {
            message: "GitHub did not return a stable user identifier".to_owned(),
        })?
        .to_string();
    let envelope = OAuthCredentialEnvelope {
        version: OAUTH_CREDENTIAL_VERSION,
        access_token: tokens.access_token.expose_secret().to_owned(),
        refresh_token: tokens
            .refresh_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        access_token_expires_at,
        refresh_token_expires_at,
    };
    let encoded = envelope.encode()?;
    let account_id = format!("{GITHUB_HOST}:{provider_user_id}");
    let key = credential_key(&provider_user_id)?;
    let _mutation_guard = state.github_mutations.lock().await;
    let credentials = Arc::clone(&state.github_credentials);
    let key_for_write = key.clone();
    let previous_secret = tauri::async_runtime::spawn_blocking(move || {
        let previous = credentials.read(&key_for_write)?.map(Zeroizing::new);
        credentials.write(&key_for_write, encoded.as_str())?;
        Ok::<_, secret_store::CredentialStoreError>(previous)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("credential storage task failed: {error}"),
    })?
    .map_err(command_error)?;
    let now = unix_timestamp()?;
    let stored = match lock_accounts(state) {
        Ok(mut accounts) => accounts
            .upsert_account(&UpsertGitHubAccount {
                id: account_id,
                host: GITHUB_HOST.to_owned(),
                provider_user_id,
                login: validation.user.login,
                display_name: validation.user.display_name,
                avatar_url: validation.user.avatar_url,
                auth_kind: GitHubAuthKind::OAuthDevice,
                scopes: validation.scopes,
                state: GitHubAccountState::Connected,
                access_token_expires_at,
                last_validated_at: Some(now),
                now,
            })
            .map_err(command_error),
        Err(error) => Err(error),
    };
    match stored {
        Ok(account) => {
            bump_account_generation(state, &account.id);
            Ok(account.summary())
        }
        Err(error) => {
            let credentials = Arc::clone(&state.github_credentials);
            let rollback = tauri::async_runtime::spawn_blocking(move || match previous_secret {
                Some(previous) => credentials.write(&key, previous.as_str()),
                None => credentials.delete(&key).map(|_| ()),
            })
            .await;
            match rollback {
                Ok(Ok(())) => Err(error.into()),
                _ => Err(GitHubCommandError {
                    message:
                        "GitHub account metadata failed and credential rollback was incomplete"
                            .to_owned(),
                    code: "credentialRollbackFailed".to_owned(),
                    retryable: false,
                    request_id: None,
                    rate_limit: None,
                }),
            }
        }
    }
}

async fn load_client(
    state: &State<'_, AppState>,
    account: &GitHubAccount,
    account_generation: u64,
) -> Result<GitHubClient<github_client::ReqwestTransport>, GitHubCommandError> {
    let key = credential_key(&account.provider_user_id)?;
    let credentials = Arc::clone(&state.github_credentials);
    let token = tauri::async_runtime::spawn_blocking(move || credentials.read(&key))
        .await
        .map_err(|error| CommandError {
            message: format!("credential read task failed: {error}"),
        })?
        .map_err(command_error)?;
    let Some(token) = token else {
        if !update_account_state(
            state,
            account,
            account_generation,
            GitHubAccountState::AuthenticationRequired,
        )
        .await?
        {
            return Err(stale_account_error());
        }
        return Err(GitHubCommandError {
            message: "GitHub authentication is required".to_owned(),
            code: "authenticationRequired".to_owned(),
            retryable: false,
            request_id: None,
            rate_limit: None,
        });
    };
    let token = match account.auth_kind {
        GitHubAuthKind::PersonalAccessToken => token,
        GitHubAuthKind::OAuthDevice => {
            load_oauth_access_token(state, account, account_generation, token).await?
        }
    };
    Ok(GitHubClient::new(
        state.github_config.clone(),
        state.github_transport.clone(),
        PersonalAccessToken::new(token).map_err(command_error)?,
    ))
}

async fn load_oauth_access_token(
    state: &State<'_, AppState>,
    account: &GitHubAccount,
    expected_generation: u64,
    stored: String,
) -> Result<String, GitHubCommandError> {
    let stored = Zeroizing::new(stored);
    let credential = OAuthCredentialEnvelope::decode(stored.as_str())?;
    let now = unix_timestamp()?;
    if credential
        .access_token_expires_at
        .is_none_or(|expires_at| expires_at > now + OAUTH_REFRESH_SKEW_SECONDS)
    {
        return Ok(credential.access_token.clone());
    }
    drop(credential);

    let _mutation_guard = state.github_mutations.lock().await;
    if account_generation(state, &account.id) != expected_generation {
        return Err(stale_account_error());
    }
    let key = credential_key(&account.provider_user_id)?;
    let credentials = Arc::clone(&state.github_credentials);
    let key_for_read = key.clone();
    let latest = tauri::async_runtime::spawn_blocking(move || credentials.read(&key_for_read))
        .await
        .map_err(|error| CommandError {
            message: format!("credential read task failed: {error}"),
        })?
        .map_err(command_error)?
        .ok_or_else(|| GitHubCommandError {
            message: "GitHub authentication is required".to_owned(),
            code: "authenticationRequired".to_owned(),
            retryable: false,
            request_id: None,
            rate_limit: None,
        })?;
    let latest = Zeroizing::new(latest);
    let mut credential = OAuthCredentialEnvelope::decode(latest.as_str())?;
    let now = unix_timestamp()?;
    if credential
        .access_token_expires_at
        .is_none_or(|expires_at| expires_at > now + OAUTH_REFRESH_SKEW_SECONDS)
    {
        return Ok(credential.access_token.clone());
    }
    if credential
        .refresh_token_expires_at
        .is_some_and(|expires_at| expires_at <= now + OAUTH_REFRESH_SKEW_SECONDS)
    {
        return Err(authentication_required_error(
            "GitHub OAuth refresh token has expired; reconnect the account",
        ));
    }
    let refresh_token = credential.refresh_token.as_ref().ok_or_else(|| {
        authentication_required_error(
            "GitHub OAuth token cannot be refreshed; reconnect the account",
        )
    })?;
    let refresh_token =
        OAuthRefreshToken::new(refresh_token.clone()).map_err(device_flow_command_error)?;
    let client = state.github_device_flow.clone();
    let refreshed = tauri::async_runtime::spawn_blocking(move || {
        client.refresh(GITHUB_CLIENT_ID, &refresh_token)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("GitHub OAuth refresh task failed: {error}"),
    })?;
    let refreshed = match refreshed {
        Ok(refreshed) => refreshed,
        Err(error) if error.code == DeviceFlowErrorCode::AuthenticationRequired => {
            update_account_state_locked(
                state,
                account,
                expected_generation,
                GitHubAccountState::AuthenticationRequired,
            )?;
            return Err(authentication_required_error(
                "GitHub OAuth authorization expired or was revoked; reconnect the account",
            ));
        }
        Err(error) => return Err(device_flow_command_error(error)),
    };
    let refreshed_at = unix_timestamp()?;
    credential.access_token = refreshed.access_token.expose_secret().to_owned();
    credential.access_token_expires_at =
        optional_future_timestamp(refreshed_at, refreshed.access_token_expires_in_seconds)?;
    if let Some(refresh_token) = &refreshed.refresh_token {
        credential.refresh_token = Some(refresh_token.expose_secret().to_owned());
    }
    if refreshed.refresh_token_expires_in_seconds.is_some() {
        credential.refresh_token_expires_at =
            optional_future_timestamp(refreshed_at, refreshed.refresh_token_expires_in_seconds)?;
    }
    let access_token = credential.access_token.clone();
    let encoded = credential.encode()?;
    let credentials = Arc::clone(&state.github_credentials);
    let key_for_write = key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        credentials.write(&key_for_write, encoded.as_str())
    })
    .await
    .map_err(|error| CommandError {
        message: format!("credential storage task failed: {error}"),
    })?
    .map_err(command_error)?;

    let metadata_result = update_oauth_expiry_locked(
        state,
        account,
        expected_generation,
        credential.access_token_expires_at,
    );
    // GitHub rotates refresh tokens. Once the replacement is safely in the keychain, restoring
    // the old envelope could persist an already-revoked refresh token. SQLite expiry metadata is
    // advisory, so keep the rotated credential if that secondary update fails.
    metadata_result?;
    Ok(access_token)
}

async fn update_account_state(
    state: &State<'_, AppState>,
    account: &GitHubAccount,
    expected_generation: u64,
    account_state: GitHubAccountState,
) -> Result<bool, GitHubCommandError> {
    let _mutation_guard = state.github_mutations.lock().await;
    update_account_state_locked(state, account, expected_generation, account_state)
}

fn update_oauth_expiry_locked(
    state: &State<'_, AppState>,
    account: &GitHubAccount,
    expected_generation: u64,
    access_token_expires_at: Option<i64>,
) -> Result<(), GitHubCommandError> {
    if account_generation(state, &account.id) != expected_generation {
        return Err(stale_account_error());
    }
    let current = lock_accounts(state)?
        .get_account(&account.id)
        .map_err(command_error)?
        .ok_or_else(stale_account_error)?;
    let now = unix_timestamp()?;
    lock_accounts(state)?
        .upsert_account(&UpsertGitHubAccount {
            id: current.id,
            host: current.host,
            provider_user_id: current.provider_user_id,
            login: current.login,
            display_name: current.display_name,
            avatar_url: current.avatar_url,
            auth_kind: current.auth_kind,
            scopes: current.scopes,
            state: GitHubAccountState::Connected,
            access_token_expires_at,
            last_validated_at: current.last_validated_at,
            now,
        })
        .map_err(command_error)?;
    Ok(())
}

fn update_account_state_locked(
    state: &State<'_, AppState>,
    account: &GitHubAccount,
    expected_generation: u64,
    account_state: GitHubAccountState,
) -> Result<bool, GitHubCommandError> {
    if account_generation(state, &account.id) != expected_generation {
        return Ok(false);
    }
    let Some(current) = lock_accounts(state)?
        .get_account(&account.id)
        .map_err(command_error)?
    else {
        return Ok(false);
    };
    let now = unix_timestamp()?;
    lock_accounts(state)?
        .upsert_account(&UpsertGitHubAccount {
            id: current.id,
            host: current.host,
            provider_user_id: current.provider_user_id,
            login: current.login,
            display_name: current.display_name,
            avatar_url: current.avatar_url,
            auth_kind: current.auth_kind,
            scopes: current.scopes,
            state: account_state,
            access_token_expires_at: current.access_token_expires_at,
            last_validated_at: (account_state == GitHubAccountState::Connected).then_some(now),
            now,
        })
        .map_err(command_error)?;
    Ok(true)
}

fn bind_repository(
    state: &State<'_, AppState>,
    repository_id: &str,
    context: &GitHubRequestContext,
) -> Result<(), CommandError> {
    lock_accounts(state)?
        .upsert_repository_binding(&UpsertGitHubRepositoryBinding {
            repository_id: repository_id.to_owned(),
            account_id: context.account.id.clone(),
            provider_repository_id: None,
            owner: context.owner.clone(),
            name: context.name.clone(),
            now: unix_timestamp()?,
        })
        .map_err(command_error)?;
    Ok(())
}

fn mark_repository_github_health(
    state: &State<'_, AppState>,
    repository_id: &str,
    error: Option<&GitHubClientError>,
) -> Result<(), CommandError> {
    let now = unix_timestamp()?;
    let (health_state, issue) = match error {
        None => (IntegrationHealthState::Healthy, None),
        Some(error) => (
            IntegrationHealthState::Degraded,
            Some(match error.code {
                GitHubErrorCode::AuthenticationRequired => IntegrationHealthIssue::Authentication,
                GitHubErrorCode::Forbidden => IntegrationHealthIssue::Authorization,
                GitHubErrorCode::NotFound => IntegrationHealthIssue::NotFound,
                GitHubErrorCode::Network | GitHubErrorCode::TimedOut => {
                    IntegrationHealthIssue::Network
                }
                GitHubErrorCode::InvalidConfiguration | GitHubErrorCode::InvalidRequest => {
                    IntegrationHealthIssue::InvalidConfiguration
                }
                GitHubErrorCode::RateLimited
                | GitHubErrorCode::InvalidResponse
                | GitHubErrorCode::ResponseTooLarge => IntegrationHealthIssue::OperationFailed,
            }),
        ),
    };
    lock_catalog(state)?.update_health(
        repository_id,
        &RepositoryHealthUpdate {
            git: None,
            github: Some(IntegrationHealth {
                state: health_state,
                issue,
                checked_at: Some(now),
            }),
            now,
        },
    )?;
    Ok(())
}

fn mark_repository_github_issue(
    state: &State<'_, AppState>,
    repository_id: &str,
    issue: IntegrationHealthIssue,
) -> Result<(), CommandError> {
    let now = unix_timestamp()?;
    lock_catalog(state)?.update_health(
        repository_id,
        &RepositoryHealthUpdate {
            git: None,
            github: Some(IntegrationHealth {
                state: IntegrationHealthState::Degraded,
                issue: Some(issue),
                checked_at: Some(now),
            }),
            now,
        },
    )?;
    Ok(())
}

fn summary_response(
    pull: PullRequestSummary,
    viewer_login: &str,
    unresolved_thread_count: Option<usize>,
) -> GitHubPullRequestSummaryResponse {
    let author_login = pull
        .author
        .as_ref()
        .map(|author| author.login.clone())
        .unwrap_or_else(|| "ghost".to_owned());
    GitHubPullRequestSummaryResponse {
        number: pull.number,
        title: pull.title,
        url: pull.html_url,
        state: pull.state,
        draft: pull.draft,
        authored_by_viewer: author_login.eq_ignore_ascii_case(viewer_login),
        author_login,
        head_ref_name: pull.head_ref,
        base_ref_name: pull.base_ref,
        updated_at: pull.updated_at,
        review_requested_from_viewer: None,
        unresolved_thread_count,
    }
}

fn detail_response(
    detail: PullRequestDetail,
    comments: Vec<IssueComment>,
    threads: Vec<ReviewThread>,
    viewer_login: &str,
    conversation_truncated: bool,
    review_threads_truncated: bool,
) -> GitHubPullRequestDetailResponse {
    let unresolved_thread_count = threads.iter().filter(|thread| !thread.resolved).count();
    GitHubPullRequestDetailResponse {
        summary: summary_response(detail.summary, viewer_login, Some(unresolved_thread_count)),
        body: detail.body_markdown.unwrap_or_default(),
        additions: detail.additions,
        deletions: detail.deletions,
        changed_files: detail.changed_files,
        mergeability: detail.mergeability,
        comments: comments.into_iter().map(issue_comment_response).collect(),
        review_threads: threads.into_iter().map(review_thread_response).collect(),
        conversation_truncated,
        review_threads_truncated,
    }
}

fn collect_issue_comments(
    client: &GitHubClient<github_client::ReqwestTransport>,
    owner: &str,
    repository: &str,
    number: u64,
) -> Result<(Vec<IssueComment>, bool), GitHubClientError> {
    let mut comments = Vec::new();
    let mut cursor = None;
    for _ in 0..MAX_DETAIL_PAGES {
        let page = client.issue_comments(
            owner,
            repository,
            number,
            DETAIL_PAGE_SIZE,
            cursor.as_deref(),
        )?;
        comments.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok((comments, false));
        }
    }
    Ok((comments, true))
}

fn collect_review_threads(
    client: &GitHubClient<github_client::ReqwestTransport>,
    owner: &str,
    repository: &str,
    number: u64,
) -> Result<(Vec<ReviewThread>, bool), GitHubClientError> {
    let mut threads = Vec::new();
    let mut cursor = None;
    let mut nested_comments_may_be_truncated = false;
    for _ in 0..MAX_DETAIL_PAGES {
        let page = client.review_threads(
            owner,
            repository,
            number,
            DETAIL_PAGE_SIZE,
            cursor.as_deref(),
        )?;
        nested_comments_may_be_truncated |= page
            .items
            .iter()
            .any(|thread| thread.comments.len() >= DETAIL_PAGE_SIZE.into());
        threads.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok((threads, nested_comments_may_be_truncated));
        }
    }
    Ok((threads, true))
}

fn issue_comment_response(comment: IssueComment) -> GitHubCommentResponse {
    GitHubCommentResponse {
        id: comment.id.to_string(),
        author_login: comment
            .author
            .map(|author| author.login)
            .unwrap_or_else(|| "ghost".to_owned()),
        body: comment.body_markdown,
        created_at: comment.created_at,
        updated_at: comment.updated_at,
        url: Some(comment.html_url),
        path: None,
        line: None,
        side: None,
    }
}

fn review_thread_response(thread: ReviewThread) -> GitHubReviewThreadResponse {
    GitHubReviewThreadResponse {
        id: thread.node_id,
        path: thread.path,
        line: thread.line,
        resolved: thread.resolved,
        outdated: thread.outdated,
        comments: thread
            .comments
            .into_iter()
            .map(review_comment_response)
            .collect(),
    }
}

fn review_comment_response(comment: ReviewComment) -> GitHubCommentResponse {
    GitHubCommentResponse {
        id: comment.node_id,
        author_login: comment
            .author
            .map(|author| author.login)
            .unwrap_or_else(|| "ghost".to_owned()),
        body: comment.body_markdown,
        created_at: comment.created_at,
        updated_at: comment.updated_at,
        url: comment.html_url,
        path: comment.path,
        line: comment.line,
        side: comment.side,
    }
}

fn lock_accounts<'a>(
    state: &'a State<'_, AppState>,
) -> Result<MutexGuard<'a, GitHubAccountStore>, CommandError> {
    state.github_accounts.lock().map_err(|_| CommandError {
        message: "GitHub account metadata is unavailable".to_owned(),
    })
}

fn lock_device_sessions<'a>(
    state: &'a State<'_, AppState>,
) -> Result<MutexGuard<'a, HashMap<String, Arc<DeviceFlowSession>>>, CommandError> {
    state
        .github_device_sessions
        .entries
        .lock()
        .map_err(|_| CommandError {
            message: "GitHub Device Flow sessions are unavailable".to_owned(),
        })
}

fn remove_device_session(state: &State<'_, AppState>, flow_id: &str) -> Result<(), CommandError> {
    lock_device_sessions(state)?.remove(flow_id);
    Ok(())
}

fn device_flow_pending(next_poll_at: i64) -> GitHubDeviceFlowPollResponse {
    GitHubDeviceFlowPollResponse {
        state: GitHubDeviceFlowState::Pending,
        next_poll_at: Some(next_poll_at),
        account: None,
    }
}

fn device_flow_terminal(state: GitHubDeviceFlowState) -> GitHubDeviceFlowPollResponse {
    GitHubDeviceFlowPollResponse {
        state,
        next_poll_at: None,
        account: None,
    }
}

fn future_timestamp(now: i64, seconds: u64) -> Result<i64, CommandError> {
    let seconds = i64::try_from(seconds).map_err(|_| invalid_device_flow_error())?;
    now.checked_add(seconds)
        .ok_or_else(invalid_device_flow_error)
}

fn optional_future_timestamp(now: i64, seconds: Option<u64>) -> Result<Option<i64>, CommandError> {
    seconds
        .map(|seconds| future_timestamp(now, seconds))
        .transpose()
}

fn invalid_device_flow_error() -> CommandError {
    CommandError {
        message: "GitHub Device Flow session is invalid or no longer available".to_owned(),
    }
}

fn credential_key(provider_user_id: &str) -> Result<CredentialKey, CommandError> {
    CredentialKey::new(
        GITHUB_CREDENTIAL_SERVICE,
        format!("{provider_user_id}/access"),
    )
    .map_err(command_error)
}

fn command_error(error: impl std::fmt::Display) -> CommandError {
    CommandError {
        message: error.to_string(),
    }
}

fn client_command_error(error: GitHubClientError) -> GitHubCommandError {
    GitHubCommandError {
        message: error.to_string(),
        code: github_error_code(error.code).to_owned(),
        retryable: error.retryable,
        request_id: error.request_id,
        rate_limit: error.rate_limit.map(Box::new),
    }
}

fn device_flow_command_error(error: DeviceFlowError) -> GitHubCommandError {
    let code = match error.code {
        DeviceFlowErrorCode::InvalidConfiguration => "invalidConfiguration",
        DeviceFlowErrorCode::InvalidRequest => "invalidRequest",
        DeviceFlowErrorCode::Network => "network",
        DeviceFlowErrorCode::TimedOut => "timedOut",
        DeviceFlowErrorCode::ResponseTooLarge => "responseTooLarge",
        DeviceFlowErrorCode::InvalidResponse => "invalidResponse",
        DeviceFlowErrorCode::AuthenticationRequired => "authenticationRequired",
        DeviceFlowErrorCode::ProviderRejected => "providerRejected",
    };
    GitHubCommandError {
        message: error.to_string(),
        code: code.to_owned(),
        retryable: error.retryable,
        request_id: None,
        rate_limit: None,
    }
}

fn stale_account_error() -> GitHubCommandError {
    GitHubCommandError {
        message: "The GitHub account changed while the request was running. Retry the request."
            .to_owned(),
        code: "staleAccount".to_owned(),
        retryable: true,
        request_id: None,
        rate_limit: None,
    }
}

fn authentication_required_error(message: &str) -> GitHubCommandError {
    GitHubCommandError {
        message: message.to_owned(),
        code: "authenticationRequired".to_owned(),
        retryable: false,
        request_id: None,
        rate_limit: None,
    }
}

fn github_error_code(code: GitHubErrorCode) -> &'static str {
    match code {
        GitHubErrorCode::InvalidConfiguration => "invalidConfiguration",
        GitHubErrorCode::InvalidRequest => "invalidRequest",
        GitHubErrorCode::AuthenticationRequired => "authenticationRequired",
        GitHubErrorCode::Forbidden => "forbidden",
        GitHubErrorCode::NotFound => "notFound",
        GitHubErrorCode::RateLimited => "rateLimited",
        GitHubErrorCode::Network => "network",
        GitHubErrorCode::TimedOut => "timedOut",
        GitHubErrorCode::InvalidResponse => "invalidResponse",
        GitHubErrorCode::ResponseTooLarge => "responseTooLarge",
    }
}

#[cfg(test)]
mod tests {
    use app_domain::{GitHubUser, PullRequestMergeability, ReviewCommentSide};

    use super::*;

    fn pull_request() -> PullRequestSummary {
        PullRequestSummary {
            number: 42,
            title: "Keep credentials local".to_owned(),
            state: PullRequestState::Open,
            draft: false,
            author: Some(GitHubUser {
                id: Some(7),
                login: "octocat".to_owned(),
                display_name: None,
                avatar_url: None,
            }),
            head_ref: "secure/github".to_owned(),
            base_ref: "main".to_owned(),
            html_url: "https://github.com/example/repo/pull/42".to_owned(),
            updated_at: "2026-07-15T12:00:00Z".to_owned(),
            comment_count: 1,
        }
    }

    #[test]
    fn summary_mapping_marks_the_authenticated_author_without_inventing_review_state() {
        let summary = summary_response(pull_request(), "OctoCat", Some(2));

        assert!(summary.authored_by_viewer);
        assert_eq!(summary.review_requested_from_viewer, None);
        assert_eq!(summary.unresolved_thread_count, Some(2));
        assert_eq!(summary.head_ref_name, "secure/github");
    }

    #[test]
    fn detail_mapping_keeps_conversation_and_review_threads_separate() {
        let detail = PullRequestDetail {
            summary: pull_request(),
            body_markdown: Some("A safe read-only slice".to_owned()),
            additions: 10,
            deletions: 2,
            changed_files: 3,
            mergeability: PullRequestMergeability::Mergeable,
        };
        let comments = vec![IssueComment {
            id: 5,
            author: None,
            body_markdown: "General comment".to_owned(),
            html_url: "https://github.com/example/repo/pull/42#issuecomment-5".to_owned(),
            created_at: "2026-07-15T12:01:00Z".to_owned(),
            updated_at: "2026-07-15T12:01:00Z".to_owned(),
        }];
        let threads = vec![ReviewThread {
            node_id: "thread-1".to_owned(),
            resolved: false,
            outdated: false,
            path: "src/lib.rs".to_owned(),
            line: Some(12),
            side: Some(ReviewCommentSide::Right),
            comments: vec![ReviewComment {
                node_id: "comment-1".to_owned(),
                author: None,
                body_markdown: "Inline comment".to_owned(),
                path: Some("src/lib.rs".to_owned()),
                line: Some(12),
                side: Some(ReviewCommentSide::Right),
                created_at: "2026-07-15T12:02:00Z".to_owned(),
                updated_at: "2026-07-15T12:02:00Z".to_owned(),
                html_url: None,
            }],
        }];

        let response = detail_response(detail, comments, threads, "octocat", false, false);

        assert_eq!(response.comments[0].body, "General comment");
        assert_eq!(
            response.review_threads[0].comments[0].body,
            "Inline comment"
        );
        assert_eq!(response.summary.unresolved_thread_count, Some(1));
    }

    #[test]
    fn oauth_credential_envelope_round_trips_without_changing_expiry_metadata() {
        let credential = OAuthCredentialEnvelope {
            version: OAUTH_CREDENTIAL_VERSION,
            access_token: "ghu_access".to_owned(),
            refresh_token: Some("ghr_refresh".to_owned()),
            access_token_expires_at: Some(1234),
            refresh_token_expires_at: Some(5678),
        };

        let encoded = credential.encode().unwrap();
        let decoded = OAuthCredentialEnvelope::decode(encoded.as_str()).unwrap();

        assert_eq!(decoded.version, OAUTH_CREDENTIAL_VERSION);
        assert_eq!(decoded.access_token, "ghu_access");
        assert_eq!(decoded.refresh_token.as_deref(), Some("ghr_refresh"));
        assert_eq!(decoded.access_token_expires_at, Some(1234));
        assert_eq!(decoded.refresh_token_expires_at, Some(5678));
    }

    #[test]
    fn oauth_credential_envelope_rejects_unknown_versions_and_empty_tokens() {
        assert!(
            OAuthCredentialEnvelope::decode(
                r#"{"version":2,"accessToken":"ghu_access","refreshToken":null,"accessTokenExpiresAt":null,"refreshTokenExpiresAt":null}"#,
            )
            .is_err()
        );
        assert!(
            OAuthCredentialEnvelope::decode(
                r#"{"version":1,"accessToken":"","refreshToken":null,"accessTokenExpiresAt":null,"refreshTokenExpiresAt":null}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn device_flow_timestamp_conversion_rejects_overflow() {
        assert_eq!(future_timestamp(100, 5).unwrap(), 105);
        assert!(future_timestamp(i64::MAX, 1).is_err());
        assert!(future_timestamp(0, u64::MAX).is_err());
    }
}
