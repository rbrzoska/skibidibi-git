use std::sync::{Arc, MutexGuard};

use app_domain::{
    GitHubAccountState, GitHubAccountSummary, GitHubAuthKind, GitHubRateLimit, GitHubRepository,
    IntegrationHealth, IntegrationHealthIssue, IntegrationHealthState, IssueComment,
    PullRequestDetail, PullRequestState, PullRequestSummary, RepositoryHealthUpdate, ReviewComment,
    ReviewThread,
};
use app_store::{
    GitHubAccount, GitHubAccountStore, UpsertGitHubAccount, UpsertGitHubRepositoryBinding,
};
use github_client::{
    GitHubClient, GitHubClientError, GitHubErrorCode, PersonalAccessToken, PullRequestListState,
};
use secret_store::CredentialKey;
use serde::Serialize;
use tauri::State;
use zeroize::Zeroizing;

use super::{AppState, CommandError, lock_catalog, unix_timestamp};

const GITHUB_HOST: &str = "github.com";
const GITHUB_CREDENTIAL_SERVICE: &str = "skibidibi-git/github.com";
const DETAIL_PAGE_SIZE: u16 = 50;
const MAX_DETAIL_PAGES: usize = 5;

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
    Ok(GitHubClient::new(
        state.github_config.clone(),
        state.github_transport.clone(),
        PersonalAccessToken::new(token).map_err(command_error)?,
    ))
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
}
