mod cli;
mod client;
mod device_flow;
mod error;
mod transport;

pub use cli::GitHubCliTransport;
pub use client::{GitHubClient, GitHubClientConfig, PullRequestListScope};
pub use device_flow::{
    DeviceAuthorization, DeviceCode, DeviceFlowError, DeviceFlowErrorCode, DeviceFlowPoll,
    GitHubDeviceFlowClient, OAuthAccessToken, OAuthRefreshToken, OAuthTokenSet,
};
pub use error::{GitHubClientError, GitHubErrorCode};
pub use transport::{
    GitHubMethod, GitHubRequest, GitHubResponse, GitHubTransport, PersonalAccessToken,
    ReqwestTransport, TransportError,
};
