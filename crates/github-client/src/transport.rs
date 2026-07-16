use std::{collections::BTreeMap, fmt, io::Read, time::Duration};

use reqwest::{
    blocking::Client,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT},
    redirect::Policy,
};
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAT_BYTES: usize = 1_024;
const GITHUB_API_VERSION: &str = "2026-03-10";
const GITHUB_API_VERSION_HEADER: &str = "x-github-api-version";

pub struct PersonalAccessToken(Zeroizing<String>);

impl PersonalAccessToken {
    pub fn new(token: String) -> Result<Self, TransportError> {
        if token.trim().is_empty() || token.len() > MAX_PAT_BYTES || token.contains(['\r', '\n']) {
            return Err(TransportError::InvalidCredential);
        }
        Ok(Self(Zeroizing::new(token)))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for PersonalAccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PersonalAccessToken([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRequest {
    pub method: GitHubMethod,
    pub url: Url,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubResponse {
    pub status: u16,
    /// Header names must be lowercase.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("the credential is invalid")]
    InvalidCredential,
    #[error("the request target is outside the configured GitHub API origin")]
    UnsafeTarget,
    #[error("the request timed out")]
    TimedOut,
    #[error("the GitHub API could not be reached")]
    Network,
    #[error("the GitHub API response exceeded the configured limit")]
    ResponseTooLarge,
    #[error("the GitHub API returned an invalid header")]
    InvalidHeader,
}

pub trait GitHubTransport: Send + Sync {
    fn execute(
        &self,
        request: GitHubRequest,
        credential: &PersonalAccessToken,
    ) -> Result<GitHubResponse, TransportError>;
}

/// Blocking transport intended to run on a Tauri blocking worker. Redirects are disabled so an
/// Authorization header can never be forwarded to a different origin.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: Client,
    allowed_origin: Url,
    max_response_bytes: usize,
}

impl ReqwestTransport {
    pub fn new(allowed_origin: Url) -> Result<Self, TransportError> {
        Self::with_limit(allowed_origin, DEFAULT_MAX_RESPONSE_BYTES)
    }

    pub fn with_limit(
        allowed_origin: Url,
        max_response_bytes: usize,
    ) -> Result<Self, TransportError> {
        if max_response_bytes == 0 {
            return Err(TransportError::ResponseTooLarge);
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(Policy::none())
            .build()
            .map_err(|_| TransportError::Network)?;
        Ok(Self {
            client,
            allowed_origin,
            max_response_bytes,
        })
    }

    fn same_origin(&self, target: &Url) -> bool {
        target.scheme() == self.allowed_origin.scheme()
            && target.host_str() == self.allowed_origin.host_str()
            && target.port_or_known_default() == self.allowed_origin.port_or_known_default()
            && target.username().is_empty()
            && target.password().is_none()
    }
}

impl GitHubTransport for ReqwestTransport {
    fn execute(
        &self,
        request: GitHubRequest,
        credential: &PersonalAccessToken,
    ) -> Result<GitHubResponse, TransportError> {
        if !self.same_origin(&request.url) {
            return Err(TransportError::UnsafeTarget);
        }

        let mut builder = match request.method {
            GitHubMethod::Get => self.client.get(request.url),
            GitHubMethod::Post => self.client.post(request.url),
        }
        .headers(build_headers(
            &request.headers,
            credential,
            request.body.is_some(),
        )?);
        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let mut response = builder.send().map_err(|error| {
            if error.is_timeout() {
                TransportError::TimedOut
            } else {
                TransportError::Network
            }
        })?;
        let status = response.status().as_u16();
        let headers = copy_headers(response.headers());
        if response.content_length().is_some_and(|length| {
            length > u64::try_from(self.max_response_bytes).unwrap_or(u64::MAX)
        }) {
            return Err(TransportError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        response
            .by_ref()
            .take((self.max_response_bytes + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| TransportError::Network)?;
        if body.len() > self.max_response_bytes {
            return Err(TransportError::ResponseTooLarge);
        }
        Ok(GitHubResponse {
            status,
            headers,
            body,
        })
    }
}

fn build_headers(
    requested: &BTreeMap<String, String>,
    credential: &PersonalAccessToken,
    has_body: bool,
) -> Result<HeaderMap, TransportError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("skibidibi-git"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        HeaderName::from_static(GITHUB_API_VERSION_HEADER),
        HeaderValue::from_static(GITHUB_API_VERSION),
    );
    let authorization = Zeroizing::new(format!("Bearer {}", credential.expose()));
    let mut authorization = HeaderValue::from_str(authorization.as_str())
        .map_err(|_| TransportError::InvalidCredential)?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    if has_body {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }

    for (name, value) in requested {
        let name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| TransportError::InvalidHeader)?;
        if is_protected_header(&name) {
            continue;
        }
        let value = HeaderValue::from_str(value).map_err(|_| TransportError::InvalidHeader)?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn is_protected_header(name: &HeaderName) -> bool {
    name == AUTHORIZATION
        || name == USER_AGENT
        || name == ACCEPT
        || name == CONTENT_TYPE
        || name.as_str() == GITHUB_API_VERSION_HEADER
}

fn copy_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_debug_output_is_redacted() {
        let token = PersonalAccessToken::new("not-a-real-token".to_owned()).unwrap();
        let output = format!("{token:?}");
        assert!(!output.contains("not-a-real-token"));
    }

    #[test]
    fn token_length_is_bounded() {
        assert!(matches!(
            PersonalAccessToken::new("x".repeat(MAX_PAT_BYTES + 1)),
            Err(TransportError::InvalidCredential)
        ));
        assert!(PersonalAccessToken::new("x".repeat(MAX_PAT_BYTES)).is_ok());
    }

    #[test]
    fn reqwest_transport_rejects_a_different_origin_before_network_io() {
        let transport =
            ReqwestTransport::new(Url::parse("https://api.github.test/").unwrap()).unwrap();
        let request = GitHubRequest {
            method: GitHubMethod::Get,
            url: Url::parse("https://attacker.test/user").unwrap(),
            headers: BTreeMap::new(),
            body: None,
        };
        let token = PersonalAccessToken::new("not-a-real-token".to_owned()).unwrap();
        assert!(matches!(
            transport.execute(request, &token),
            Err(TransportError::UnsafeTarget)
        ));
    }

    #[test]
    fn reqwest_transport_is_cheaply_cloneable() {
        let transport =
            ReqwestTransport::new(Url::parse("https://api.github.test/").unwrap()).unwrap();

        let _shared_pool_clone = transport.clone();
    }

    #[test]
    fn protected_headers_cannot_be_overridden() {
        let token = PersonalAccessToken::new("not-a-real-token".to_owned()).unwrap();
        let requested = BTreeMap::from([
            ("authorization".to_owned(), "attacker-value".to_owned()),
            ("accept".to_owned(), "text/plain".to_owned()),
            ("content-type".to_owned(), "text/plain".to_owned()),
            (
                GITHUB_API_VERSION_HEADER.to_owned(),
                "untrusted-version".to_owned(),
            ),
            ("user-agent".to_owned(), "untrusted-agent".to_owned()),
        ]);

        let headers = build_headers(&requested, &token, true).unwrap();

        assert_eq!(headers[ACCEPT], "application/vnd.github+json");
        assert_eq!(headers[CONTENT_TYPE], "application/json");
        assert_eq!(headers[USER_AGENT], "skibidibi-git");
        assert_eq!(
            headers[HeaderName::from_static(GITHUB_API_VERSION_HEADER)],
            GITHUB_API_VERSION
        );
        assert_ne!(headers[AUTHORIZATION], "attacker-value");
        assert!(headers[AUTHORIZATION].is_sensitive());
        assert!(!format!("{headers:?}").contains("not-a-real-token"));
    }
}
