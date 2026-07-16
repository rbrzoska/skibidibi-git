use std::{fmt, io::Read, time::Duration};

use reqwest::{
    blocking::Client,
    header::{ACCEPT, CONTENT_TYPE, USER_AGENT},
    redirect::Policy,
};
use serde::Deserialize;
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

const GITHUB_WEB_ORIGIN: &str = "https://github.com/";
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_CLIENT_ID_BYTES: usize = 256;
const MAX_SECRET_BYTES: usize = 4 * 1024;
const MAX_PUBLIC_FIELD_BYTES: usize = 4 * 1024;

pub struct DeviceCode(Zeroizing<String>);

impl DeviceCode {
    fn new(value: String) -> Result<Self, DeviceFlowError> {
        validate_secret(&value)?;
        Ok(Self(Zeroizing::new(value)))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceCode([REDACTED])")
    }
}

pub struct OAuthAccessToken(Zeroizing<String>);

impl OAuthAccessToken {
    fn new(value: String) -> Result<Self, DeviceFlowError> {
        validate_secret(&value)?;
        Ok(Self(Zeroizing::new(value)))
    }

    /// Exposes the token only for transfer into the platform credential store.
    pub fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for OAuthAccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuthAccessToken([REDACTED])")
    }
}

pub struct OAuthRefreshToken(Zeroizing<String>);

impl OAuthRefreshToken {
    /// Reconstructs a refresh credential read from the platform credential store.
    pub fn new(value: String) -> Result<Self, DeviceFlowError> {
        validate_secret(&value)?;
        Ok(Self(Zeroizing::new(value)))
    }

    /// Exposes the token only for transfer into the platform credential store or a refresh call.
    pub fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for OAuthRefreshToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuthRefreshToken([REDACTED])")
    }
}

#[derive(Debug)]
pub struct DeviceAuthorization {
    pub device_code: DeviceCode,
    pub user_code: String,
    pub verification_uri: Url,
    pub expires_in_seconds: u64,
    pub interval_seconds: u64,
}

pub struct OAuthTokenSet {
    pub access_token: OAuthAccessToken,
    pub refresh_token: Option<OAuthRefreshToken>,
    pub access_token_expires_in_seconds: Option<u64>,
    pub refresh_token_expires_in_seconds: Option<u64>,
    pub token_type: String,
}

impl fmt::Debug for OAuthTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthTokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "access_token_expires_in_seconds",
                &self.access_token_expires_in_seconds,
            )
            .field(
                "refresh_token_expires_in_seconds",
                &self.refresh_token_expires_in_seconds,
            )
            .field("token_type", &self.token_type)
            .finish()
    }
}

#[derive(Debug)]
pub enum DeviceFlowPoll {
    Pending,
    SlowDown,
    Expired,
    AccessDenied,
    Authorized(OAuthTokenSet),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFlowErrorCode {
    InvalidConfiguration,
    InvalidRequest,
    Network,
    TimedOut,
    ResponseTooLarge,
    InvalidResponse,
    AuthenticationRequired,
    ProviderRejected,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct DeviceFlowError {
    pub code: DeviceFlowErrorCode,
    pub message: &'static str,
    pub retryable: bool,
}

impl DeviceFlowError {
    fn new(code: DeviceFlowErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
        }
    }

    fn retryable(code: DeviceFlowErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: true,
        }
    }
}

/// Blocking GitHub OAuth client intended to run on a Tauri blocking worker.
///
/// The production constructor is pinned to `https://github.com`; redirects are disabled and the
/// response body is bounded. It deliberately does not accept a client secret because GitHub App
/// Device Flow and refresh calls authenticate with the public client id.
#[derive(Clone)]
pub struct GitHubDeviceFlowClient {
    client: Client,
    origin: Url,
    max_response_bytes: usize,
}

impl GitHubDeviceFlowClient {
    pub fn new() -> Result<Self, DeviceFlowError> {
        let origin = Url::parse(GITHUB_WEB_ORIGIN).map_err(|_| {
            DeviceFlowError::new(
                DeviceFlowErrorCode::InvalidConfiguration,
                "the GitHub OAuth origin is invalid",
            )
        })?;
        Self::build(origin, DEFAULT_MAX_RESPONSE_BYTES)
    }

    fn build(origin: Url, max_response_bytes: usize) -> Result<Self, DeviceFlowError> {
        if max_response_bytes == 0 {
            return Err(DeviceFlowError::new(
                DeviceFlowErrorCode::InvalidConfiguration,
                "the OAuth response limit is invalid",
            ));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(Policy::none())
            .build()
            .map_err(|_| {
                DeviceFlowError::new(
                    DeviceFlowErrorCode::InvalidConfiguration,
                    "the GitHub OAuth client could not be configured",
                )
            })?;
        Ok(Self {
            client,
            origin,
            max_response_bytes,
        })
    }

    pub fn start(&self, client_id: &str) -> Result<DeviceAuthorization, DeviceFlowError> {
        validate_client_id(client_id)?;
        let body = Zeroizing::new(form_body(&[("client_id", client_id)]));
        let response = self.post("login/device/code", body.as_bytes())?;
        let wire: DeviceAuthorizationWire = decode_success(response)?;
        validate_public_field(&wire.user_code)?;
        validate_public_field(&wire.verification_uri)?;
        if wire.expires_in == 0 || wire.interval == 0 {
            return Err(invalid_response());
        }
        let verification_uri =
            Url::parse(&wire.verification_uri).map_err(|_| invalid_response())?;
        if !self.same_origin(&verification_uri)
            || verification_uri.query().is_some()
            || verification_uri.fragment().is_some()
        {
            return Err(invalid_response());
        }
        Ok(DeviceAuthorization {
            device_code: DeviceCode::new(wire.device_code)?,
            user_code: wire.user_code,
            verification_uri,
            expires_in_seconds: wire.expires_in,
            interval_seconds: wire.interval,
        })
    }

    pub fn poll(
        &self,
        client_id: &str,
        device_code: &DeviceCode,
    ) -> Result<DeviceFlowPoll, DeviceFlowError> {
        validate_client_id(client_id)?;
        let body = Zeroizing::new(form_body(&[
            ("client_id", client_id),
            ("device_code", device_code.expose()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ]));
        let response = self.post("login/oauth/access_token", body.as_bytes())?;
        decode_poll(response)
    }

    pub fn refresh(
        &self,
        client_id: &str,
        refresh_token: &OAuthRefreshToken,
    ) -> Result<OAuthTokenSet, DeviceFlowError> {
        validate_client_id(client_id)?;
        let body = Zeroizing::new(form_body(&[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose_secret()),
        ]));
        let response = self.post("login/oauth/access_token", body.as_bytes())?;
        decode_token_response(response)
    }

    fn post(&self, path: &str, body: &[u8]) -> Result<HttpResponse, DeviceFlowError> {
        let target = self.origin.join(path).map_err(|_| {
            DeviceFlowError::new(
                DeviceFlowErrorCode::InvalidConfiguration,
                "the GitHub OAuth endpoint is invalid",
            )
        })?;
        if !self.same_origin(&target) {
            return Err(DeviceFlowError::new(
                DeviceFlowErrorCode::InvalidConfiguration,
                "the GitHub OAuth endpoint is unsafe",
            ));
        }
        let mut response = self
            .client
            .post(target)
            .header(USER_AGENT, "skibidibi-git")
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body.to_vec())
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    DeviceFlowError::retryable(
                        DeviceFlowErrorCode::TimedOut,
                        "the GitHub OAuth request timed out",
                    )
                } else {
                    DeviceFlowError::retryable(
                        DeviceFlowErrorCode::Network,
                        "GitHub OAuth could not be reached",
                    )
                }
            })?;
        if response.content_length().is_some_and(|length| {
            length > u64::try_from(self.max_response_bytes).unwrap_or(u64::MAX)
        }) {
            return Err(response_too_large());
        }
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take((self.max_response_bytes + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                DeviceFlowError::retryable(
                    DeviceFlowErrorCode::Network,
                    "the GitHub OAuth response could not be read",
                )
            })?;
        if bytes.len() > self.max_response_bytes {
            return Err(response_too_large());
        }
        Ok(HttpResponse {
            status,
            body: bytes,
        })
    }

    fn same_origin(&self, target: &Url) -> bool {
        target.scheme() == self.origin.scheme()
            && target.host_str() == self.origin.host_str()
            && target.port_or_known_default() == self.origin.port_or_known_default()
            && target.username().is_empty()
            && target.password().is_none()
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct DeviceAuthorizationWire {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct TokenWire {
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    refresh_token_expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct ErrorWire {
    error: String,
}

fn decode_success<T: for<'de> Deserialize<'de>>(
    response: HttpResponse,
) -> Result<T, DeviceFlowError> {
    if !(200..300).contains(&response.status) {
        return Err(provider_http_error(response.status));
    }
    let body = Zeroizing::new(response.body);
    serde_json::from_slice(body.as_slice()).map_err(|_| invalid_response())
}

fn decode_poll(response: HttpResponse) -> Result<DeviceFlowPoll, DeviceFlowError> {
    if !(200..300).contains(&response.status) {
        return Err(provider_http_error(response.status));
    }
    let body = Zeroizing::new(response.body);
    let value: serde_json::Value =
        serde_json::from_slice(body.as_slice()).map_err(|_| invalid_response())?;
    if value.get("access_token").is_some() {
        let wire: TokenWire = serde_json::from_value(value).map_err(|_| invalid_response())?;
        return token_set(wire).map(DeviceFlowPoll::Authorized);
    }
    let wire: ErrorWire = serde_json::from_value(value).map_err(|_| invalid_response())?;
    match wire.error.as_str() {
        "authorization_pending" => Ok(DeviceFlowPoll::Pending),
        "slow_down" => Ok(DeviceFlowPoll::SlowDown),
        "expired_token" => Ok(DeviceFlowPoll::Expired),
        "access_denied" => Ok(DeviceFlowPoll::AccessDenied),
        _ => Err(provider_rejected()),
    }
}

fn decode_token_response(response: HttpResponse) -> Result<OAuthTokenSet, DeviceFlowError> {
    if !(200..300).contains(&response.status) {
        return Err(provider_http_error(response.status));
    }
    let body = Zeroizing::new(response.body);
    let value: serde_json::Value =
        serde_json::from_slice(body.as_slice()).map_err(|_| invalid_response())?;
    if value.get("error").is_some() {
        let wire: ErrorWire = serde_json::from_value(value).map_err(|_| invalid_response())?;
        return Err(match wire.error.as_str() {
            "bad_refresh_token" | "invalid_grant" | "access_denied" => authentication_required(),
            _ => provider_rejected(),
        });
    }
    let wire: TokenWire = serde_json::from_value(value).map_err(|_| invalid_response())?;
    token_set(wire)
}

fn token_set(wire: TokenWire) -> Result<OAuthTokenSet, DeviceFlowError> {
    validate_public_field(&wire.token_type)?;
    if !wire.token_type.eq_ignore_ascii_case("bearer")
        || wire.expires_in == Some(0)
        || wire.refresh_token_expires_in == Some(0)
    {
        return Err(invalid_response());
    }
    Ok(OAuthTokenSet {
        access_token: OAuthAccessToken::new(wire.access_token)?,
        refresh_token: wire.refresh_token.map(OAuthRefreshToken::new).transpose()?,
        access_token_expires_in_seconds: wire.expires_in,
        refresh_token_expires_in_seconds: wire.refresh_token_expires_in,
        token_type: wire.token_type,
    })
}

fn form_body(fields: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish()
}

fn validate_client_id(client_id: &str) -> Result<(), DeviceFlowError> {
    if client_id.trim().is_empty()
        || client_id.len() > MAX_CLIENT_ID_BYTES
        || client_id.contains(['\r', '\n'])
    {
        return Err(DeviceFlowError::new(
            DeviceFlowErrorCode::InvalidRequest,
            "the GitHub client id is invalid",
        ));
    }
    Ok(())
}

fn validate_secret(value: &str) -> Result<(), DeviceFlowError> {
    if value.trim().is_empty() || value.len() > MAX_SECRET_BYTES || value.contains(['\r', '\n']) {
        return Err(invalid_response());
    }
    Ok(())
}

fn validate_public_field(value: &str) -> Result<(), DeviceFlowError> {
    if value.trim().is_empty()
        || value.len() > MAX_PUBLIC_FIELD_BYTES
        || value.contains(['\r', '\n'])
    {
        return Err(invalid_response());
    }
    Ok(())
}

fn invalid_response() -> DeviceFlowError {
    DeviceFlowError::new(
        DeviceFlowErrorCode::InvalidResponse,
        "GitHub returned an invalid OAuth response",
    )
}

fn provider_rejected() -> DeviceFlowError {
    DeviceFlowError::new(
        DeviceFlowErrorCode::ProviderRejected,
        "GitHub rejected the OAuth request",
    )
}

fn authentication_required() -> DeviceFlowError {
    DeviceFlowError::new(
        DeviceFlowErrorCode::AuthenticationRequired,
        "GitHub OAuth authorization is no longer valid",
    )
}

fn provider_http_error(status: u16) -> DeviceFlowError {
    if status == 429 || status >= 500 {
        DeviceFlowError::retryable(
            DeviceFlowErrorCode::ProviderRejected,
            "GitHub temporarily rejected the OAuth request",
        )
    } else {
        provider_rejected()
    }
}

fn response_too_large() -> DeviceFlowError {
    DeviceFlowError::new(
        DeviceFlowErrorCode::ResponseTooLarge,
        "the GitHub OAuth response exceeded the configured limit",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    fn client_and_server(
        status: u16,
        response: &'static str,
    ) -> (GitHubDeviceFlowClient, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = response.replace("{ORIGIN}", &format!("http://{address}"));
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let header_end = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .unwrap()
                        + 4;
                    if request.len() >= header_end + length {
                        break;
                    }
                }
            }
            let reason = if status == 200 { "OK" } else { "Bad Request" };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
            String::from_utf8(request).unwrap()
        });
        let origin = Url::parse(&format!("http://{address}/")).unwrap();
        (
            GitHubDeviceFlowClient::build(origin, DEFAULT_MAX_RESPONSE_BYTES).unwrap(),
            handle,
        )
    }

    #[test]
    fn starts_device_flow_with_json_accept_and_form_body() {
        let response = r#"{"device_code":"secret-device","user_code":"ABCD-1234","verification_uri":"{ORIGIN}/verify","expires_in":900,"interval":5}"#;
        let (client, server) = client_and_server(200, response);
        let authorization = client.start("Iv123").unwrap();
        assert_eq!(authorization.user_code, "ABCD-1234");
        assert_eq!(authorization.interval_seconds, 5);
        assert_eq!(
            format!("{:?}", authorization.device_code),
            "DeviceCode([REDACTED])"
        );
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /login/device/code HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: application/json")
        );
        assert!(request.ends_with("client_id=Iv123"));
    }

    #[test]
    fn maps_poll_states_and_redacts_authorized_tokens() {
        let response = r#"{"access_token":"ghu_secret","expires_in":28800,"refresh_token":"ghr_secret","refresh_token_expires_in":15811200,"token_type":"bearer"}"#;
        let (client, server) = client_and_server(200, response);
        let code = DeviceCode::new("device-secret".to_owned()).unwrap();
        let result = client.poll("Iv123", &code).unwrap();
        let DeviceFlowPoll::Authorized(tokens) = result else {
            panic!("expected authorized result");
        };
        assert_eq!(tokens.access_token.expose_secret(), "ghu_secret");
        assert_eq!(tokens.access_token_expires_in_seconds, Some(28_800));
        let debug = format!("{tokens:?}");
        assert!(!debug.contains("ghu_secret"));
        assert!(!debug.contains("ghr_secret"));
        let request = server.join().unwrap();
        assert!(
            request.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code")
        );
        assert!(request.contains("device_code=device-secret"));
    }

    #[test]
    fn maps_provider_pending_slow_down_expired_and_denied() {
        for (error, expected) in [
            ("authorization_pending", "Pending"),
            ("slow_down", "SlowDown"),
            ("expired_token", "Expired"),
            ("access_denied", "AccessDenied"),
        ] {
            let response = Box::leak(format!(r#"{{"error":"{error}"}}"#).into_boxed_str());
            let (client, server) = client_and_server(200, response);
            let result = client
                .poll(
                    "Iv123",
                    &DeviceCode::new("device-secret".to_owned()).unwrap(),
                )
                .unwrap();
            assert_eq!(format!("{result:?}"), expected);
            server.join().unwrap();
        }
    }

    #[test]
    fn refresh_does_not_send_a_client_secret() {
        let response = r#"{"access_token":"ghu_new","expires_in":28800,"refresh_token":"ghr_new","refresh_token_expires_in":15811200,"token_type":"bearer"}"#;
        let (client, server) = client_and_server(200, response);
        let token = OAuthRefreshToken::new("ghr_old".to_owned()).unwrap();
        let refreshed = client.refresh("Iv123", &token).unwrap();
        assert_eq!(refreshed.access_token.expose_secret(), "ghu_new");
        let request = server.join().unwrap();
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=ghr_old"));
        assert!(!request.contains("client_secret"));
    }

    #[test]
    fn revoked_refresh_token_requires_reauthentication() {
        let (client, server) = client_and_server(200, r#"{"error":"bad_refresh_token"}"#);
        let token = OAuthRefreshToken::new("ghr_revoked".to_owned()).unwrap();

        let error = client.refresh("Iv123", &token).unwrap_err();

        assert_eq!(error.code, DeviceFlowErrorCode::AuthenticationRequired);
        assert!(!error.retryable);
        server.join().unwrap();
    }

    #[test]
    fn rejects_oversized_response_and_unknown_provider_error() {
        let oversized = Box::leak("x".repeat(129).into_boxed_str());
        let (mut client, server) = client_and_server(200, oversized);
        client.max_response_bytes = 128;
        let error = client.start("Iv123").unwrap_err();
        assert_eq!(error.code, DeviceFlowErrorCode::ResponseTooLarge);
        server.join().unwrap();

        let (client, server) = client_and_server(200, r#"{"error":"incorrect_device_code"}"#);
        let error = client
            .poll(
                "Iv123",
                &DeviceCode::new("device-secret".to_owned()).unwrap(),
            )
            .unwrap_err();
        assert_eq!(error.code, DeviceFlowErrorCode::ProviderRejected);
        server.join().unwrap();
    }
}
