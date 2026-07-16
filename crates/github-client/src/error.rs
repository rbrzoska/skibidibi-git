use app_domain::GitHubRateLimit;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubErrorCode {
    InvalidConfiguration,
    InvalidRequest,
    AuthenticationRequired,
    Forbidden,
    NotFound,
    RateLimited,
    Network,
    TimedOut,
    InvalidResponse,
    ResponseTooLarge,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct GitHubClientError {
    pub code: GitHubErrorCode,
    pub message: &'static str,
    pub retryable: bool,
    pub request_id: Option<String>,
    pub rate_limit: Option<GitHubRateLimit>,
}

impl GitHubClientError {
    pub(crate) fn new(code: GitHubErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
            request_id: None,
            rate_limit: None,
        }
    }
}
