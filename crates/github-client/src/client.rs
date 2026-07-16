use std::collections::BTreeMap;

use app_domain::{
    GitHubApiResult, GitHubPage, GitHubPatValidation, GitHubRateLimit, GitHubRepository,
    GitHubUser, IssueComment, PullRequestDetail, PullRequestMergeability, PullRequestState,
    PullRequestSummary, ReviewComment, ReviewCommentSide, ReviewThread,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    GitHubClientError, GitHubErrorCode, GitHubMethod, GitHubRequest, GitHubResponse,
    GitHubTransport, PersonalAccessToken, TransportError,
};

const MAX_PAGE_SIZE: u16 = 50;
const MAX_PAGE_NUMBER: u32 = 10_000;
const MAX_CURSOR_BYTES: usize = 1_024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const REVIEW_THREADS_QUERY: &str = include_str!("graphql/review_threads.graphql");

#[derive(Debug, Clone)]
pub struct GitHubClientConfig {
    api_base_url: Url,
    graphql_url: Url,
}

impl GitHubClientConfig {
    pub fn new(mut api_base_url: Url, graphql_url: Url) -> Result<Self, GitHubClientError> {
        validate_base_url(&api_base_url)?;
        validate_base_url(&graphql_url)?;
        if !same_origin(&api_base_url, &graphql_url) {
            return Err(GitHubClientError::new(
                GitHubErrorCode::InvalidConfiguration,
                "REST and GraphQL endpoints must use the same GitHub API origin",
            ));
        }
        if !api_base_url.path().ends_with('/') {
            let path = format!("{}/", api_base_url.path());
            api_base_url.set_path(&path);
        }
        Ok(Self {
            api_base_url,
            graphql_url,
        })
    }

    pub fn api_base_url(&self) -> &Url {
        &self.api_base_url
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestListState {
    Open,
    Closed,
    All,
}

impl PullRequestListState {
    fn as_query(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

pub struct GitHubClient<T> {
    config: GitHubClientConfig,
    transport: T,
    credential: PersonalAccessToken,
}

impl<T: GitHubTransport> GitHubClient<T> {
    pub fn new(config: GitHubClientConfig, transport: T, credential: PersonalAccessToken) -> Self {
        Self {
            config,
            transport,
            credential,
        }
    }

    pub fn validate_pat(&self) -> Result<GitHubPatValidation, GitHubClientError> {
        let response = self.get(self.rest_url(&["user"])?)?;
        let rate_limit = rate_limit(&response.headers);
        let scopes = response
            .headers
            .get("x-oauth-scopes")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|scope| !scope.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let user: RestUser = decode_json(&response)?;
        Ok(GitHubPatValidation {
            user: user.into(),
            scopes,
            rate_limit,
        })
    }

    pub fn list_repositories(
        &self,
        page_size: u16,
        cursor: Option<&str>,
    ) -> Result<GitHubPage<GitHubRepository>, GitHubClientError> {
        validate_page_size(page_size)?;
        let page = decode_page_cursor(cursor)?;
        let expected_path = self.rest_path(&["user", "repos"])?;
        let mut url = self.config.api_base_url.clone();
        url.set_path(&expected_path);
        url.query_pairs_mut()
            .append_pair("per_page", &page_size.to_string())
            .append_pair("page", &page.to_string());
        let response = self.get(url)?;
        let items: Vec<RestRepository> = decode_json(&response)?;
        let next_cursor = next_rest_cursor(
            response.headers.get("link"),
            &self.config.api_base_url,
            &expected_path,
        )?;
        Ok(GitHubPage {
            items: items.into_iter().map(Into::into).collect(),
            next_cursor,
            rate_limit: rate_limit(&response.headers),
        })
    }

    pub fn list_pull_requests(
        &self,
        owner: &str,
        repository: &str,
        state: PullRequestListState,
        page_size: u16,
        cursor: Option<&str>,
    ) -> Result<GitHubPage<PullRequestSummary>, GitHubClientError> {
        validate_page_size(page_size)?;
        let page = decode_page_cursor(cursor)?;
        let expected_path = self.rest_path(&["repos", owner, repository, "pulls"])?;
        let mut url = self.config.api_base_url.clone();
        url.set_path(&expected_path);
        url.query_pairs_mut()
            .append_pair("state", state.as_query())
            .append_pair("per_page", &page_size.to_string())
            .append_pair("page", &page.to_string());
        let response = self.get(url)?;
        let items: Vec<RestPullRequest> = decode_json(&response)?;
        let next_cursor = next_rest_cursor(
            response.headers.get("link"),
            &self.config.api_base_url,
            &expected_path,
        )?;
        Ok(GitHubPage {
            items: items.into_iter().map(Into::into).collect(),
            next_cursor,
            rate_limit: rate_limit(&response.headers),
        })
    }

    pub fn pull_request_detail(
        &self,
        owner: &str,
        repository: &str,
        number: u64,
    ) -> Result<GitHubApiResult<PullRequestDetail>, GitHubClientError> {
        validate_number(number)?;
        let response = self.get(self.rest_url(&[
            "repos",
            owner,
            repository,
            "pulls",
            &number.to_string(),
        ])?)?;
        let rate_limit = rate_limit(&response.headers);
        let pull: RestPullRequestDetail = decode_json(&response)?;
        Ok(GitHubApiResult {
            value: pull.into(),
            rate_limit,
        })
    }

    pub fn issue_comments(
        &self,
        owner: &str,
        repository: &str,
        number: u64,
        page_size: u16,
        cursor: Option<&str>,
    ) -> Result<GitHubPage<IssueComment>, GitHubClientError> {
        validate_number(number)?;
        validate_page_size(page_size)?;
        let page = decode_page_cursor(cursor)?;
        let expected_path = self.rest_path(&[
            "repos",
            owner,
            repository,
            "issues",
            &number.to_string(),
            "comments",
        ])?;
        let mut url = self.config.api_base_url.clone();
        url.set_path(&expected_path);
        url.query_pairs_mut()
            .append_pair("per_page", &page_size.to_string())
            .append_pair("page", &page.to_string());
        let response = self.get(url)?;
        let comments: Vec<RestIssueComment> = decode_json(&response)?;
        let next_cursor = next_rest_cursor(
            response.headers.get("link"),
            &self.config.api_base_url,
            &expected_path,
        )?;
        Ok(GitHubPage {
            items: comments.into_iter().map(Into::into).collect(),
            next_cursor,
            rate_limit: rate_limit(&response.headers),
        })
    }

    pub fn review_threads(
        &self,
        owner: &str,
        repository: &str,
        number: u64,
        page_size: u16,
        cursor: Option<&str>,
    ) -> Result<GitHubPage<ReviewThread>, GitHubClientError> {
        validate_identity(owner, "repository owner")?;
        validate_identity(repository, "repository name")?;
        validate_number(number)?;
        validate_page_size(page_size)?;
        if cursor.is_some_and(|value| value.len() > MAX_CURSOR_BYTES) {
            return Err(invalid_request("the pagination cursor is too long"));
        }
        let body = serde_json::to_vec(&GraphQlRequest {
            query: REVIEW_THREADS_QUERY,
            variables: ReviewThreadVariables {
                owner,
                repository,
                number,
                first: page_size,
                after: cursor,
            },
        })
        .map_err(|_| invalid_response())?;
        let response = self.execute(GitHubRequest {
            method: GitHubMethod::Post,
            url: self.config.graphql_url.clone(),
            headers: BTreeMap::new(),
            body: Some(body),
        })?;
        let rate = rate_limit(&response.headers);
        let envelope: GraphQlEnvelope = decode_json(&response)?;
        if let Some(errors) = envelope.errors.filter(|errors| !errors.is_empty()) {
            if errors.iter().any(GraphQlError::is_rate_limited) {
                let mut error = GitHubClientError::new(
                    GitHubErrorCode::RateLimited,
                    "the GitHub rate limit was reached",
                );
                error.retryable = true;
                error.rate_limit = Some(rate);
                return Err(error);
            }
            return Err(invalid_response());
        }
        let threads = envelope
            .data
            .and_then(|data| data.repository)
            .and_then(|repository| repository.pull_request)
            .map(|pull| pull.review_threads)
            .ok_or_else(invalid_response)?;
        Ok(GitHubPage {
            items: threads.nodes.into_iter().map(Into::into).collect(),
            next_cursor: threads
                .page_info
                .end_cursor
                .filter(|_| threads.page_info.has_next_page),
            rate_limit: rate,
        })
    }

    fn get(&self, url: Url) -> Result<GitHubResponse, GitHubClientError> {
        self.execute(GitHubRequest {
            method: GitHubMethod::Get,
            url,
            headers: BTreeMap::new(),
            body: None,
        })
    }

    fn execute(&self, request: GitHubRequest) -> Result<GitHubResponse, GitHubClientError> {
        if !same_origin(&self.config.api_base_url, &request.url) {
            return Err(GitHubClientError::new(
                GitHubErrorCode::InvalidRequest,
                "the request target is outside the configured GitHub API origin",
            ));
        }
        let response = self
            .transport
            .execute(request, &self.credential)
            .map_err(map_transport_error)?;
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(GitHubClientError::new(
                GitHubErrorCode::ResponseTooLarge,
                "the GitHub API response exceeded the configured limit",
            ));
        }
        if !(200..300).contains(&response.status) {
            return Err(map_status_error(&response));
        }
        Ok(response)
    }

    fn rest_url(&self, segments: &[&str]) -> Result<Url, GitHubClientError> {
        let path = self.rest_path(segments)?;
        let mut url = self.config.api_base_url.clone();
        url.set_path(&path);
        Ok(url)
    }

    fn rest_path(&self, segments: &[&str]) -> Result<String, GitHubClientError> {
        let mut url = self.config.api_base_url.clone();
        {
            let mut path = url.path_segments_mut().map_err(|_| {
                GitHubClientError::new(
                    GitHubErrorCode::InvalidConfiguration,
                    "the GitHub API URL cannot contain path segments",
                )
            })?;
            path.pop_if_empty();
            for segment in segments {
                validate_identity(segment, "GitHub path segment")?;
                path.push(segment);
            }
        }
        Ok(url.path().to_owned())
    }
}

fn validate_base_url(url: &Url) -> Result<(), GitHubClientError> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(GitHubClientError::new(
            GitHubErrorCode::InvalidConfiguration,
            "GitHub API endpoints must be credential-free HTTPS URLs",
        ));
    }
    Ok(())
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn validate_identity(value: &str, field: &'static str) -> Result<(), GitHubClientError> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value.contains(['\0', '\r', '\n', '/', '\\'])
    {
        return Err(invalid_request(field));
    }
    Ok(())
}

fn validate_number(number: u64) -> Result<(), GitHubClientError> {
    if number == 0 || number > i32::MAX as u64 {
        return Err(invalid_request("the pull request number is invalid"));
    }
    Ok(())
}

fn validate_page_size(page_size: u16) -> Result<(), GitHubClientError> {
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(invalid_request("page size must be between 1 and 50"));
    }
    Ok(())
}

fn decode_page_cursor(cursor: Option<&str>) -> Result<u32, GitHubClientError> {
    let Some(cursor) = cursor else { return Ok(1) };
    if cursor.len() > MAX_CURSOR_BYTES {
        return Err(invalid_request("the pagination cursor is too long"));
    }
    let page = cursor
        .strip_prefix("page:")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|page| (1..=MAX_PAGE_NUMBER).contains(page))
        .ok_or_else(|| invalid_request("the pagination cursor is invalid"))?;
    Ok(page)
}

fn next_rest_cursor(
    link: Option<&String>,
    base: &Url,
    expected_path: &str,
) -> Result<Option<String>, GitHubClientError> {
    let Some(link) = link else { return Ok(None) };
    for part in link.split(',') {
        let Some((target, relations)) = part.trim().split_once(';') else {
            continue;
        };
        if !relations
            .split(';')
            .any(|relation| relation.trim() == "rel=\"next\"")
        {
            continue;
        }
        let target = target
            .trim()
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .ok_or_else(invalid_response)?;
        let target = Url::parse(target).map_err(|_| invalid_response())?;
        if !same_origin(base, &target)
            || target.path() != expected_path
            || !target.username().is_empty()
            || target.password().is_some()
        {
            return Err(invalid_response());
        }
        let page = target
            .query_pairs()
            .find_map(|(key, value)| (key == "page").then_some(value))
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|page| (1..=MAX_PAGE_NUMBER).contains(page))
            .ok_or_else(invalid_response)?;
        return Ok(Some(format!("page:{page}")));
    }
    Ok(None)
}

fn rate_limit(headers: &BTreeMap<String, String>) -> GitHubRateLimit {
    GitHubRateLimit {
        limit: parse_header(headers, "x-ratelimit-limit"),
        remaining: parse_header(headers, "x-ratelimit-remaining"),
        reset_at: parse_header(headers, "x-ratelimit-reset"),
        retry_after_seconds: parse_header(headers, "retry-after"),
    }
}

fn parse_header<T: std::str::FromStr>(headers: &BTreeMap<String, String>, name: &str) -> Option<T> {
    headers.get(name).and_then(|value| value.parse().ok())
}

fn decode_json<T: for<'de> Deserialize<'de>>(
    response: &GitHubResponse,
) -> Result<T, GitHubClientError> {
    serde_json::from_slice(&response.body).map_err(|_| invalid_response())
}

fn map_transport_error(error: TransportError) -> GitHubClientError {
    match error {
        TransportError::TimedOut => {
            let mut error = GitHubClientError::new(GitHubErrorCode::TimedOut, "GitHub timed out");
            error.retryable = true;
            error
        }
        TransportError::ResponseTooLarge => GitHubClientError::new(
            GitHubErrorCode::ResponseTooLarge,
            "the GitHub API response exceeded the configured limit",
        ),
        TransportError::UnsafeTarget
        | TransportError::InvalidCredential
        | TransportError::InvalidHeader => GitHubClientError::new(
            GitHubErrorCode::InvalidRequest,
            "the GitHub request is invalid",
        ),
        TransportError::Network => {
            let mut error =
                GitHubClientError::new(GitHubErrorCode::Network, "GitHub is unavailable");
            error.retryable = true;
            error
        }
    }
}

fn map_status_error(response: &GitHubResponse) -> GitHubClientError {
    let rate = rate_limit(&response.headers);
    let code = match response.status {
        401 => GitHubErrorCode::AuthenticationRequired,
        403 if rate.remaining == Some(0) || rate.retry_after_seconds.is_some() => {
            GitHubErrorCode::RateLimited
        }
        403 => GitHubErrorCode::Forbidden,
        404 => GitHubErrorCode::NotFound,
        429 => GitHubErrorCode::RateLimited,
        500..=599 => GitHubErrorCode::Network,
        _ => GitHubErrorCode::InvalidResponse,
    };
    let message = match code {
        GitHubErrorCode::AuthenticationRequired => "GitHub authentication is required",
        GitHubErrorCode::Forbidden => "the GitHub account does not have access",
        GitHubErrorCode::NotFound => "the GitHub resource was not found",
        GitHubErrorCode::RateLimited => "the GitHub rate limit was reached",
        GitHubErrorCode::Network => "GitHub is temporarily unavailable",
        _ => "GitHub returned an unexpected response",
    };
    let mut error = GitHubClientError::new(code, message);
    error.retryable = matches!(
        code,
        GitHubErrorCode::Network | GitHubErrorCode::RateLimited
    );
    error.request_id = response.headers.get("x-github-request-id").cloned();
    error.rate_limit = Some(rate);
    error
}

fn invalid_request(message: &'static str) -> GitHubClientError {
    GitHubClientError::new(GitHubErrorCode::InvalidRequest, message)
}

fn invalid_response() -> GitHubClientError {
    GitHubClientError::new(
        GitHubErrorCode::InvalidResponse,
        "GitHub returned an invalid response",
    )
}

#[derive(Deserialize)]
struct RestUser {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct RestRepositoryOwner {
    login: String,
}

#[derive(Deserialize)]
struct RestRepository {
    id: u64,
    owner: RestRepositoryOwner,
    name: String,
    full_name: String,
    private: bool,
    updated_at: String,
    clone_url: String,
    ssh_url: String,
}

impl From<RestRepository> for GitHubRepository {
    fn from(repository: RestRepository) -> Self {
        Self {
            id: repository.id.to_string(),
            owner: repository.owner.login,
            name: repository.name,
            full_name: repository.full_name,
            private: repository.private,
            updated_at: repository.updated_at,
            https_clone_url: repository.clone_url,
            ssh_clone_url: repository.ssh_url,
        }
    }
}

impl From<RestUser> for GitHubUser {
    fn from(user: RestUser) -> Self {
        Self {
            id: Some(user.id),
            login: user.login,
            display_name: user.name,
            avatar_url: user.avatar_url,
        }
    }
}

#[derive(Deserialize)]
struct RestRef {
    #[serde(rename = "ref")]
    name: String,
}

#[derive(Deserialize)]
struct RestPullRequest {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    user: Option<RestUser>,
    head: RestRef,
    base: RestRef,
    html_url: String,
    updated_at: String,
    #[serde(default)]
    comments: u64,
    #[serde(default)]
    review_comments: u64,
    merged_at: Option<String>,
}

impl From<RestPullRequest> for PullRequestSummary {
    fn from(pull: RestPullRequest) -> Self {
        Self {
            number: pull.number,
            title: pull.title,
            state: if pull.merged_at.is_some() {
                PullRequestState::Merged
            } else if pull.state == "open" {
                PullRequestState::Open
            } else {
                PullRequestState::Closed
            },
            draft: pull.draft,
            author: pull.user.map(Into::into),
            head_ref: pull.head.name,
            base_ref: pull.base.name,
            html_url: pull.html_url,
            updated_at: pull.updated_at,
            comment_count: pull.comments.saturating_add(pull.review_comments),
        }
    }
}

#[derive(Deserialize)]
struct RestPullRequestDetail {
    #[serde(flatten)]
    summary: RestPullRequest,
    body: Option<String>,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    changed_files: u64,
    mergeable: Option<bool>,
}

impl From<RestPullRequestDetail> for PullRequestDetail {
    fn from(pull: RestPullRequestDetail) -> Self {
        Self {
            summary: pull.summary.into(),
            body_markdown: pull.body,
            additions: pull.additions,
            deletions: pull.deletions,
            changed_files: pull.changed_files,
            mergeability: match pull.mergeable {
                Some(true) => PullRequestMergeability::Mergeable,
                Some(false) => PullRequestMergeability::Conflicting,
                None => PullRequestMergeability::Unknown,
            },
        }
    }
}

#[derive(Deserialize)]
struct RestIssueComment {
    id: u64,
    user: Option<RestUser>,
    #[serde(default)]
    body: String,
    html_url: String,
    created_at: String,
    updated_at: String,
}

impl From<RestIssueComment> for IssueComment {
    fn from(comment: RestIssueComment) -> Self {
        Self {
            id: comment.id,
            author: comment.user.map(Into::into),
            body_markdown: comment.body,
            html_url: comment.html_url,
            created_at: comment.created_at,
            updated_at: comment.updated_at,
        }
    }
}

#[derive(Serialize)]
struct GraphQlRequest<'a> {
    query: &'static str,
    variables: ReviewThreadVariables<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadVariables<'a> {
    owner: &'a str,
    repository: &'a str,
    number: u64,
    first: u16,
    after: Option<&'a str>,
}

#[derive(Deserialize)]
struct GraphQlEnvelope {
    data: Option<GraphQlData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct GraphQlError {
    #[serde(default)]
    extensions: GraphQlErrorExtensions,
}

impl GraphQlError {
    fn is_rate_limited(&self) -> bool {
        self.extensions.kind.as_deref() == Some("RATE_LIMITED")
    }
}

#[derive(Default, Deserialize)]
struct GraphQlErrorExtensions {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct GraphQlData {
    repository: Option<GraphQlRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlRepository {
    pull_request: Option<GraphQlPullRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPullRequest {
    review_threads: GraphQlThreadConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlThreadConnection {
    nodes: Vec<GraphQlThread>,
    page_info: GraphQlPageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlThread {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    path: String,
    line: Option<u64>,
    diff_side: Option<String>,
    comments: GraphQlCommentConnection,
}

#[derive(Deserialize)]
struct GraphQlCommentConnection {
    nodes: Vec<GraphQlComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlComment {
    id: String,
    author: Option<GraphQlUser>,
    #[serde(default)]
    body: String,
    path: Option<String>,
    line: Option<u64>,
    diff_side: Option<String>,
    created_at: String,
    updated_at: String,
    url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlUser {
    database_id: Option<u64>,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

impl From<GraphQlUser> for GitHubUser {
    fn from(user: GraphQlUser) -> Self {
        Self {
            id: user.database_id,
            login: user.login,
            display_name: user.name,
            avatar_url: user.avatar_url,
        }
    }
}

impl From<GraphQlThread> for ReviewThread {
    fn from(thread: GraphQlThread) -> Self {
        Self {
            node_id: thread.id,
            resolved: thread.is_resolved,
            outdated: thread.is_outdated,
            path: thread.path,
            line: thread.line,
            side: parse_side(thread.diff_side.as_deref()),
            comments: thread.comments.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<GraphQlComment> for ReviewComment {
    fn from(comment: GraphQlComment) -> Self {
        Self {
            node_id: comment.id,
            author: comment.author.map(Into::into),
            body_markdown: comment.body,
            path: comment.path,
            line: comment.line,
            side: parse_side(comment.diff_side.as_deref()),
            created_at: comment.created_at,
            updated_at: comment.updated_at,
            html_url: comment.url,
        }
    }
}

fn parse_side(side: Option<&str>) -> Option<ReviewCommentSide> {
    match side {
        Some("LEFT") => Some(ReviewCommentSide::Left),
        Some("RIGHT") => Some(ReviewCommentSide::Right),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    struct FakeTransport {
        responses: Mutex<VecDeque<Result<GitHubResponse, TransportError>>>,
        requests: Mutex<Vec<GitHubRequest>>,
    }

    impl FakeTransport {
        fn returning(responses: Vec<GitHubResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(Ok).collect()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl GitHubTransport for FakeTransport {
        fn execute(
            &self,
            request: GitHubRequest,
            _credential: &PersonalAccessToken,
        ) -> Result<GitHubResponse, TransportError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("missing fake response")
        }
    }

    fn client(responses: Vec<GitHubResponse>) -> GitHubClient<FakeTransport> {
        let config = GitHubClientConfig::new(
            Url::parse("https://api.github.test/api/v3/").unwrap(),
            Url::parse("https://api.github.test/api/graphql").unwrap(),
        )
        .unwrap();
        GitHubClient::new(
            config,
            FakeTransport::returning(responses),
            PersonalAccessToken::new("test-token-not-real".to_owned()).unwrap(),
        )
    }

    fn response(status: u16, body: &str) -> GitHubResponse {
        GitHubResponse {
            status,
            headers: BTreeMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn validates_pat_and_extracts_scopes_without_exposing_the_token() {
        let mut response = response(
            200,
            r#"{"id":42,"login":"octo","name":"Octo","avatar_url":null}"#,
        );
        response
            .headers
            .insert("x-oauth-scopes".into(), "repo, read:user".into());
        let client = client(vec![response]);

        let validation = client.validate_pat().unwrap();

        assert_eq!(validation.user.login, "octo");
        assert_eq!(validation.scopes, ["repo", "read:user"]);
        let requests = client.transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url.as_str(),
            "https://api.github.test/api/v3/user"
        );
        assert!(!format!("{:?}", requests[0]).contains("test-token-not-real"));
    }

    #[test]
    fn lists_authenticated_repositories_with_clone_urls_and_a_safe_cursor() {
        let mut response = response(
            200,
            r#"[{
              "id":9007199254740993,
              "owner":{"login":"octo"},
              "name":"widget",
              "full_name":"octo/widget",
              "private":true,
              "updated_at":"2026-07-16T08:30:00Z",
              "clone_url":"https://github.com/octo/widget.git",
              "ssh_url":"git@github.com:octo/widget.git"
            }]"#,
        );
        response.headers.insert(
            "link".into(),
            "<https://api.github.test/api/v3/user/repos?page=2&per_page=25>; rel=\"next\"".into(),
        );
        let client = client(vec![response]);

        let page = client.list_repositories(25, None).unwrap();

        assert_eq!(page.items[0].id, "9007199254740993");
        assert_eq!(page.items[0].full_name, "octo/widget");
        assert_eq!(
            page.items[0].https_clone_url,
            "https://github.com/octo/widget.git"
        );
        assert_eq!(
            page.items[0].ssh_clone_url,
            "git@github.com:octo/widget.git"
        );
        assert_eq!(page.next_cursor.as_deref(), Some("page:2"));
        let requests = client.transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url.as_str(),
            "https://api.github.test/api/v3/user/repos?per_page=25&page=1"
        );
    }

    #[test]
    fn repository_listing_rejects_invalid_pagination_before_transport() {
        let client = client(Vec::new());

        assert_eq!(
            client.list_repositories(0, None).unwrap_err().code,
            GitHubErrorCode::InvalidRequest
        );
        assert_eq!(
            client
                .list_repositories(20, Some("page:10001"))
                .unwrap_err()
                .code,
            GitHubErrorCode::InvalidRequest
        );
        let oversized_cursor = "x".repeat(MAX_CURSOR_BYTES + 1);
        assert_eq!(
            client
                .list_repositories(20, Some(&oversized_cursor))
                .unwrap_err()
                .code,
            GitHubErrorCode::InvalidRequest
        );
        assert!(client.transport.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn repository_listing_rejects_a_cross_origin_next_link() {
        let mut response = response(200, "[]");
        response.headers.insert(
            "link".into(),
            "<https://attacker.test/api/v3/user/repos?page=2>; rel=\"next\"".into(),
        );
        let client = client(vec![response]);

        let error = client.list_repositories(20, None).unwrap_err();

        assert_eq!(error.code, GitHubErrorCode::InvalidResponse);
    }

    #[test]
    fn returns_a_bounded_opaque_cursor_from_a_same_origin_link() {
        let mut response = response(200, "[]");
        response.headers.insert(
            "link".into(),
            "<https://api.github.test/api/v3/repos/acme/widget/pulls?page=2&per_page=20>; rel=\"next\"".into(),
        );
        let client = client(vec![response]);

        let page = client
            .list_pull_requests("acme", "widget", PullRequestListState::Open, 20, None)
            .unwrap();

        assert_eq!(page.next_cursor.as_deref(), Some("page:2"));
    }

    #[test]
    fn rejects_a_cross_origin_pagination_link() {
        let mut response = response(200, "[]");
        response.headers.insert(
            "link".into(),
            "<https://attacker.test/api/v3/repos/acme/widget/pulls?page=2>; rel=\"next\"".into(),
        );
        let client = client(vec![response]);

        let error = client
            .list_pull_requests("acme", "widget", PullRequestListState::Open, 20, None)
            .unwrap_err();

        assert_eq!(error.code, GitHubErrorCode::InvalidResponse);
    }

    #[test]
    fn maps_rate_limited_forbidden_responses() {
        let mut response = response(403, r#"{"message":"API rate limit exceeded"}"#);
        response
            .headers
            .insert("x-ratelimit-remaining".into(), "0".into());
        response
            .headers
            .insert("x-ratelimit-reset".into(), "2000000000".into());
        let client = client(vec![response]);

        let error = client.validate_pat().unwrap_err();

        assert_eq!(error.code, GitHubErrorCode::RateLimited);
        assert_eq!(error.rate_limit.unwrap().reset_at, Some(2_000_000_000));
    }

    #[test]
    fn review_threads_uses_the_fixed_query_and_variables() {
        let response = response(
            200,
            r#"{
              "data": {"repository": {"pullRequest": {"reviewThreads": {
                "nodes": [{
                  "id": "thread-1", "isResolved": false, "isOutdated": false,
                  "path": "src/lib.rs", "line": 12, "diffSide": "RIGHT",
                  "comments": {"nodes": []}
                }],
                "pageInfo": {"hasNextPage": true, "endCursor": "cursor-2"}
              }}}}
            }"#,
        );
        let client = client(vec![response]);

        let page = client
            .review_threads("acme", "widget", 7, 25, None)
            .unwrap();

        assert_eq!(page.items[0].path, "src/lib.rs");
        assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));
        let requests = client.transport.requests.lock().unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["query"], REVIEW_THREADS_QUERY);
        assert_eq!(body["variables"]["number"], 7);
        assert_eq!(body["variables"]["first"], 25);
    }

    #[test]
    fn rejects_unbounded_page_sizes_and_malformed_cursors() {
        let client = client(Vec::new());
        assert_eq!(
            client
                .issue_comments("acme", "widget", 1, 51, None)
                .unwrap_err()
                .code,
            GitHubErrorCode::InvalidRequest
        );
        assert_eq!(
            client
                .issue_comments("acme", "widget", 1, 20, Some("page:999999"))
                .unwrap_err()
                .code,
            GitHubErrorCode::InvalidRequest
        );
    }
}
