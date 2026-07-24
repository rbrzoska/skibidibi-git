use std::collections::BTreeMap;

use app_domain::{
    GitHubApiResult, GitHubPage, GitHubPatValidation, GitHubRateLimit, GitHubRepository,
    GitHubUser, IssueComment, PullRequestDetail, PullRequestFile, PullRequestMergeability,
    PullRequestState, PullRequestSummary, ReviewComment, ReviewCommentSide, ReviewThread,
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
const PULL_REQUESTS_QUERY: &str = include_str!("graphql/pull_requests.graphql");
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
pub enum PullRequestListScope {
    AssignedToViewer,
    AuthoredByViewer,
}

impl PullRequestListScope {
    fn qualifier(self) -> &'static str {
        match self {
            Self::AssignedToViewer => "review-requested",
            Self::AuthoredByViewer => "author",
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
        viewer_login: &str,
        scope: PullRequestListScope,
        page_size: u16,
        cursor: Option<&str>,
    ) -> Result<GitHubPage<PullRequestSummary>, GitHubClientError> {
        validate_search_owner_or_login(owner, "repository owner")?;
        validate_search_repository(repository)?;
        validate_search_owner_or_login(viewer_login, "viewer login")?;
        validate_page_size(page_size)?;
        validate_graphql_cursor(cursor)?;
        let query = format!(
            "repo:{owner}/{repository} is:pr is:open {}:{viewer_login}",
            scope.qualifier()
        );
        let body = serde_json::to_vec(&GraphQlRequest {
            query: PULL_REQUESTS_QUERY,
            variables: PullRequestListVariables {
                query: &query,
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
        let envelope: PullRequestListEnvelope = decode_json(&response)?;
        reject_graphql_errors(envelope.errors, &rate)?;
        let pulls = envelope
            .data
            .map(|data| data.search)
            .ok_or_else(invalid_response)?;
        let next_cursor = graphql_next_cursor(&pulls.page_info)?;
        Ok(GitHubPage {
            items: pulls.nodes.into_iter().map(Into::into).collect(),
            next_cursor,
            rate_limit: rate,
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

    pub fn pull_request_files(
        &self,
        owner: &str,
        repository: &str,
        number: u64,
        page_size: u16,
        cursor: Option<&str>,
    ) -> Result<GitHubPage<PullRequestFile>, GitHubClientError> {
        validate_number(number)?;
        validate_page_size(page_size)?;
        let page = decode_page_cursor(cursor)?;
        let expected_path = self.rest_path(&[
            "repos",
            owner,
            repository,
            "pulls",
            &number.to_string(),
            "files",
        ])?;
        let mut url = self.config.api_base_url.clone();
        url.set_path(&expected_path);
        url.query_pairs_mut()
            .append_pair("per_page", &page_size.to_string())
            .append_pair("page", &page.to_string());
        let response = self.get(url)?;
        let files: Vec<RestPullRequestFile> = decode_json(&response)?;
        let next_cursor = next_rest_cursor(
            response.headers.get("link"),
            &self.config.api_base_url,
            &expected_path,
        )?;
        Ok(GitHubPage {
            items: files.into_iter().map(Into::into).collect(),
            next_cursor,
            rate_limit: rate_limit(&response.headers),
        })
    }

    pub fn approve_pull_request(
        &self,
        owner: &str,
        repository: &str,
        number: u64,
    ) -> Result<GitHubApiResult<()>, GitHubClientError> {
        validate_number(number)?;
        let body = serde_json::to_vec(&serde_json::json!({ "event": "APPROVE" }))
            .map_err(|_| invalid_response())?;
        let response = self.execute(GitHubRequest {
            method: GitHubMethod::Post,
            url: self.rest_url(&[
                "repos",
                owner,
                repository,
                "pulls",
                &number.to_string(),
                "reviews",
            ])?,
            headers: BTreeMap::new(),
            body: Some(body),
        })?;
        Ok(GitHubApiResult {
            value: (),
            rate_limit: rate_limit(&response.headers),
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
        validate_graphql_cursor(cursor)?;
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
        reject_graphql_errors(envelope.errors, &rate)?;
        let threads = envelope
            .data
            .and_then(|data| data.repository)
            .and_then(|repository| repository.pull_request)
            .map(|pull| pull.review_threads)
            .ok_or_else(invalid_response)?;
        Ok(GitHubPage {
            items: threads.nodes.into_iter().map(Into::into).collect(),
            next_cursor: graphql_next_cursor(&threads.page_info)?,
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

fn validate_search_owner_or_login(
    value: &str,
    field: &'static str,
) -> Result<(), GitHubClientError> {
    validate_identity(value, field)?;
    if value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(invalid_request(field));
    }
    Ok(())
}

fn validate_search_repository(value: &str) -> Result<(), GitHubClientError> {
    validate_identity(value, "repository name")?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid_request("repository name"));
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

fn validate_graphql_cursor(cursor: Option<&str>) -> Result<(), GitHubClientError> {
    if cursor.is_some_and(|value| value.is_empty() || value.len() > MAX_CURSOR_BYTES) {
        return Err(invalid_request("the pagination cursor is invalid"));
    }
    Ok(())
}

fn graphql_next_cursor(page_info: &GraphQlPageInfo) -> Result<Option<String>, GitHubClientError> {
    if !page_info.has_next_page {
        return Ok(None);
    }
    page_info
        .end_cursor
        .as_ref()
        .filter(|cursor| !cursor.is_empty() && cursor.len() <= MAX_CURSOR_BYTES)
        .cloned()
        .map(Some)
        .ok_or_else(invalid_response)
}

fn reject_graphql_errors(
    errors: Option<Vec<GraphQlError>>,
    rate: &GitHubRateLimit,
) -> Result<(), GitHubClientError> {
    let Some(errors) = errors.filter(|errors| !errors.is_empty()) else {
        return Ok(());
    };
    if errors.iter().any(GraphQlError::is_rate_limited) {
        let mut error = GitHubClientError::new(
            GitHubErrorCode::RateLimited,
            "the GitHub rate limit was reached",
        );
        error.retryable = true;
        error.rate_limit = Some(rate.clone());
        return Err(error);
    }
    Err(invalid_response())
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
            || !is_allowed_pagination_path(target.path(), expected_path)
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

fn is_allowed_pagination_path(target: &str, expected: &str) -> bool {
    if target == expected {
        return true;
    }
    let Some((prefix, named_repository)) = expected.rsplit_once("/repos/") else {
        return false;
    };
    let named_segments = named_repository.split('/').collect::<Vec<_>>();
    if named_segments.len() != 3
        || named_segments[0].is_empty()
        || named_segments[1].is_empty()
        || named_segments[2] != "pulls"
    {
        return false;
    }
    let canonical_prefix = format!("{prefix}/repositories/");
    let Some(canonical_repository) = target.strip_prefix(&canonical_prefix) else {
        return false;
    };
    let canonical_segments = canonical_repository.split('/').collect::<Vec<_>>();
    canonical_segments.len() == 2
        && !canonical_segments[0].is_empty()
        && canonical_segments[0]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        && canonical_segments[1] == "pulls"
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
            approval_count: 0,
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

#[derive(Deserialize)]
struct RestPullRequestFile {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    additions: u64,
    deletions: u64,
    changes: u64,
    patch: Option<String>,
}

impl From<RestPullRequestFile> for PullRequestFile {
    fn from(file: RestPullRequestFile) -> Self {
        Self {
            filename: file.filename,
            previous_filename: file.previous_filename,
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            changes: file.changes,
            patch: file.patch,
        }
    }
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
struct GraphQlRequest<V> {
    query: &'static str,
    variables: V,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestListVariables<'a> {
    query: &'a str,
    first: u16,
    after: Option<&'a str>,
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
struct PullRequestListEnvelope {
    data: Option<PullRequestListData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct PullRequestListData {
    search: GraphQlPullRequestConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPullRequestConnection {
    nodes: Vec<GraphQlPullRequestSummary>,
    page_info: GraphQlPageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPullRequestSummary {
    number: u64,
    title: String,
    state: String,
    is_draft: bool,
    author: Option<GraphQlUser>,
    head_ref_name: String,
    base_ref_name: String,
    url: String,
    updated_at: String,
    comments: GraphQlTotalCount,
    latest_reviews: GraphQlReviewConnection,
}

#[derive(Deserialize)]
struct GraphQlReviewConnection {
    nodes: Vec<GraphQlReview>,
}

#[derive(Deserialize)]
struct GraphQlReview {
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlTotalCount {
    total_count: u64,
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
    diff_hunk: Option<String>,
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

impl From<GraphQlPullRequestSummary> for PullRequestSummary {
    fn from(pull: GraphQlPullRequestSummary) -> Self {
        Self {
            number: pull.number,
            title: pull.title,
            state: match pull.state.as_str() {
                "OPEN" => PullRequestState::Open,
                "MERGED" => PullRequestState::Merged,
                _ => PullRequestState::Closed,
            },
            draft: pull.is_draft,
            author: pull.author.map(Into::into),
            head_ref: pull.head_ref_name,
            base_ref: pull.base_ref_name,
            html_url: pull.url,
            updated_at: pull.updated_at,
            comment_count: pull.comments.total_count,
            approval_count: pull
                .latest_reviews
                .nodes
                .iter()
                .filter(|review| review.state == "APPROVED")
                .count() as u64,
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
            diff_hunk: comment.diff_hunk,
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
    fn lists_pull_requests_with_accurate_conversation_comment_counts_in_one_request() {
        let response = response(
            200,
            r#"{"data":{"search":{
              "nodes":[{
                "number":17,"title":"Accurate comments","state":"OPEN","isDraft":false,
                "author":{"databaseId":42,"login":"octo","name":"Octo","avatarUrl":null},
                "headRefName":"feature/comments","baseRefName":"main",
                "url":"https://github.test/acme/widget/pull/17",
                "updatedAt":"2026-07-17T08:30:00Z",
                "comments":{"totalCount":7},
                "latestReviews":{"nodes":[{"state":"APPROVED"},{"state":"CHANGES_REQUESTED"},{"state":"APPROVED"}]}
              }],
              "pageInfo":{"hasNextPage":true,"endCursor":"cursor-2"}
            }}}"#,
        );
        let client = client(vec![response]);

        let page = client
            .list_pull_requests(
                "acme",
                "widget",
                "octo",
                PullRequestListScope::AssignedToViewer,
                20,
                None,
            )
            .unwrap();

        assert_eq!(page.items[0].comment_count, 7);
        assert_eq!(page.items[0].approval_count, 2);
        assert_eq!(page.items[0].number, 17);
        assert_eq!(page.items[0].title, "Accurate comments");
        assert_eq!(page.items[0].state, PullRequestState::Open);
        assert!(!page.items[0].draft);
        assert_eq!(page.items[0].author.as_ref().unwrap().login, "octo");
        assert_eq!(page.items[0].head_ref, "feature/comments");
        assert_eq!(page.items[0].base_ref, "main");
        assert_eq!(
            page.items[0].html_url,
            "https://github.test/acme/widget/pull/17"
        );
        assert_eq!(page.items[0].updated_at, "2026-07-17T08:30:00Z");
        assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));
        let requests = client.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, GitHubMethod::Post);
        assert_eq!(requests[0].url.path(), "/api/graphql");
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["query"], PULL_REQUESTS_QUERY);
        assert_eq!(
            body["variables"]["query"],
            "repo:acme/widget is:pr is:open review-requested:octo"
        );
        assert_eq!(body["variables"]["first"], 20);
        assert!(body["variables"]["after"].is_null());
    }

    #[test]
    fn pull_request_graphql_pagination_forwards_cursor_and_authored_scope() {
        let response = response(
            200,
            r#"{"data":{"search":{
              "nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}
            }}}"#,
        );
        let client = client(vec![response]);

        let page = client
            .list_pull_requests(
                "acme",
                "widget",
                "octo",
                PullRequestListScope::AuthoredByViewer,
                50,
                Some("cursor-1"),
            )
            .unwrap();

        assert!(page.next_cursor.is_none());
        let requests = client.transport.requests.lock().unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(
            body["variables"]["query"],
            "repo:acme/widget is:pr is:open author:octo"
        );
        assert_eq!(body["variables"]["after"], "cursor-1");
    }

    #[test]
    fn pull_request_files_returns_patches_and_file_metadata() {
        let client = client(vec![response(
            200,
            r#"[{"filename":"src/new.ts","previous_filename":"src/old.ts","status":"renamed","additions":3,"deletions":1,"changes":4,"patch":"@@ -1 +1 @@\n-old\n+new"}]"#,
        )]);

        let page = client
            .pull_request_files("acme", "widget", 17, 50, None)
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].filename, "src/new.ts");
        assert_eq!(
            page.items[0].previous_filename.as_deref(),
            Some("src/old.ts")
        );
        assert_eq!(page.items[0].status, "renamed");
        assert_eq!(
            page.items[0].patch.as_deref(),
            Some("@@ -1 +1 @@\n-old\n+new")
        );
        let requests = client.transport.requests.lock().unwrap();
        assert_eq!(requests[0].method, GitHubMethod::Get);
        assert_eq!(
            requests[0].url.path(),
            "/api/v3/repos/acme/widget/pulls/17/files"
        );
        assert_eq!(requests[0].url.query(), Some("per_page=50&page=1"));
    }

    #[test]
    fn approve_pull_request_posts_an_approve_review() {
        let client = client(vec![response(200, r#"{}"#)]);

        client.approve_pull_request("acme", "widget", 17).unwrap();

        let requests = client.transport.requests.lock().unwrap();
        assert_eq!(requests[0].method, GitHubMethod::Post);
        assert_eq!(
            requests[0].url.path(),
            "/api/v3/repos/acme/widget/pulls/17/reviews"
        );
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["event"], "APPROVE");
    }

    #[test]
    fn rejects_unrelated_or_malformed_canonical_pagination_paths() {
        assert!(!is_allowed_pagination_path(
            "/api/v3/repositories/108991260/issues",
            "/api/v3/repos/rspective/voucherify-mono/pulls"
        ));
        assert!(!is_allowed_pagination_path(
            "/api/v3/repositories/not-a-number/pulls",
            "/api/v3/repos/rspective/voucherify-mono/pulls"
        ));
    }

    #[test]
    fn pull_request_listing_rejects_invalid_cursors_before_transport() {
        let client = client(Vec::new());
        for cursor in [String::new(), "x".repeat(MAX_CURSOR_BYTES + 1)] {
            let error = client
                .list_pull_requests(
                    "acme",
                    "widget",
                    "octo",
                    PullRequestListScope::AssignedToViewer,
                    20,
                    Some(&cursor),
                )
                .unwrap_err();
            assert_eq!(error.code, GitHubErrorCode::InvalidRequest);
        }
        assert!(client.transport.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn pull_request_listing_rejects_a_missing_or_oversized_next_cursor() {
        for cursor in [None, Some("x".repeat(MAX_CURSOR_BYTES + 1))] {
            let body = serde_json::json!({
                "data": {"search": {
                    "nodes": [],
                    "pageInfo": {"hasNextPage": true, "endCursor": cursor}
                }}
            })
            .to_string();
            let client = client(vec![response(200, &body)]);
            let error = client
                .list_pull_requests(
                    "acme",
                    "widget",
                    "octo",
                    PullRequestListScope::AssignedToViewer,
                    20,
                    None,
                )
                .unwrap_err();
            assert_eq!(error.code, GitHubErrorCode::InvalidResponse);
        }
    }

    #[test]
    fn pull_request_scope_query_rejects_qualifier_injection_before_transport() {
        let client = client(Vec::new());
        for (owner, repository, viewer) in [
            ("acme is:public", "widget", "octo"),
            ("acme", "widget author:attacker", "octo"),
            ("acme", "widget", "octo assignee:attacker"),
        ] {
            let error = client
                .list_pull_requests(
                    owner,
                    repository,
                    viewer,
                    PullRequestListScope::AssignedToViewer,
                    20,
                    None,
                )
                .unwrap_err();
            assert_eq!(error.code, GitHubErrorCode::InvalidRequest);
        }
        assert!(client.transport.requests.lock().unwrap().is_empty());
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
                  "comments": {"nodes": [{
                    "id": "comment-1", "author": null, "body": "Inline note",
                    "path": "src/lib.rs", "line": 12, "diffSide": "RIGHT",
                    "diffHunk": "@@ -10,2 +10,3 @@\n old\n+new",
                    "createdAt": "2026-07-15T12:00:00Z",
                    "updatedAt": "2026-07-15T12:00:00Z",
                    "url": "https://github.com/acme/widget/pull/7#discussion_r1"
                  }]}
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
        assert_eq!(
            page.items[0].comments[0].diff_hunk.as_deref(),
            Some("@@ -10,2 +10,3 @@\n old\n+new")
        );
        assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));
        let requests = client.transport.requests.lock().unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["query"], REVIEW_THREADS_QUERY);
        assert_eq!(REVIEW_THREADS_QUERY.matches("diffSide").count(), 1);
        assert!(REVIEW_THREADS_QUERY.contains("diffHunk"));
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
