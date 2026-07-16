use std::{collections::HashMap, sync::Mutex};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialKey {
    service: String,
    account: String,
}

impl CredentialKey {
    pub fn new(
        service: impl Into<String>,
        account: impl Into<String>,
    ) -> Result<Self, CredentialStoreError> {
        let key = Self {
            service: service.into(),
            account: account.into(),
        };
        validate_key_part(&key.service, CredentialKeyPart::Service)?;
        validate_key_part(&key.account, CredentialKeyPart::Account)?;
        Ok(key)
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    pub fn account(&self) -> &str {
        &self.account
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKeyPart {
    Service,
    Account,
}

impl std::fmt::Display for CredentialKeyPart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Service => "service",
            Self::Account => "account",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialOperation {
    Read,
    Write,
    Delete,
}

impl std::fmt::Display for CredentialOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredentialStoreError {
    #[error("credential {part} is invalid")]
    InvalidKey { part: CredentialKeyPart },
    #[error("credential secret cannot be empty")]
    EmptySecret,
    #[error("the operating-system credential store is unavailable for {operation}")]
    BackendUnavailable { operation: CredentialOperation },
    #[error("the operating-system credential store failed during {operation}")]
    BackendFailure { operation: CredentialOperation },
    #[error("the in-memory credential store lock is unavailable")]
    InMemoryLock,
}

pub trait CredentialStore: Send + Sync {
    fn read(&self, key: &CredentialKey) -> Result<Option<String>, CredentialStoreError>;
    fn write(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialStoreError>;
    fn delete(&self, key: &CredentialKey) -> Result<bool, CredentialStoreError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsCredentialStore;

impl OsCredentialStore {
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
impl CredentialStore for OsCredentialStore {
    fn read(&self, key: &CredentialKey) -> Result<Option<String>, CredentialStoreError> {
        let entry = os_entry(key, CredentialOperation::Read)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring_error(error, CredentialOperation::Read)),
        }
    }

    fn write(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialStoreError> {
        validate_secret(secret)?;
        let entry = os_entry(key, CredentialOperation::Write)?;
        entry
            .set_password(secret)
            .map_err(|error| map_keyring_error(error, CredentialOperation::Write))
    }

    fn delete(&self, key: &CredentialKey) -> Result<bool, CredentialStoreError> {
        let entry = os_entry(key, CredentialOperation::Delete)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(map_keyring_error(error, CredentialOperation::Delete)),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl CredentialStore for OsCredentialStore {
    fn read(&self, _key: &CredentialKey) -> Result<Option<String>, CredentialStoreError> {
        Err(unsupported_platform(CredentialOperation::Read))
    }

    fn write(&self, _key: &CredentialKey, _secret: &str) -> Result<(), CredentialStoreError> {
        Err(unsupported_platform(CredentialOperation::Write))
    }

    fn delete(&self, _key: &CredentialKey) -> Result<bool, CredentialStoreError> {
        Err(unsupported_platform(CredentialOperation::Delete))
    }
}

#[derive(Default)]
pub struct InMemoryCredentialStore {
    secrets: Mutex<HashMap<CredentialKey, String>>,
}

impl std::fmt::Debug for InMemoryCredentialStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryCredentialStore")
            .finish_non_exhaustive()
    }
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn read(&self, key: &CredentialKey) -> Result<Option<String>, CredentialStoreError> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| CredentialStoreError::InMemoryLock)?
            .get(key)
            .cloned())
    }

    fn write(&self, key: &CredentialKey, secret: &str) -> Result<(), CredentialStoreError> {
        validate_secret(secret)?;
        self.secrets
            .lock()
            .map_err(|_| CredentialStoreError::InMemoryLock)?
            .insert(key.clone(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &CredentialKey) -> Result<bool, CredentialStoreError> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| CredentialStoreError::InMemoryLock)?
            .remove(key)
            .is_some())
    }
}

fn validate_key_part(value: &str, part: CredentialKeyPart) -> Result<(), CredentialStoreError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(CredentialStoreError::InvalidKey { part })
    } else {
        Ok(())
    }
}

fn validate_secret(secret: &str) -> Result<(), CredentialStoreError> {
    if secret.is_empty() {
        Err(CredentialStoreError::EmptySecret)
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn os_entry(
    key: &CredentialKey,
    operation: CredentialOperation,
) -> Result<keyring::Entry, CredentialStoreError> {
    keyring::Entry::new(key.service(), key.account())
        .map_err(|error| map_keyring_error(error, operation))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn map_keyring_error(
    error: keyring::Error,
    operation: CredentialOperation,
) -> CredentialStoreError {
    match error {
        keyring::Error::NoStorageAccess(_) => {
            CredentialStoreError::BackendUnavailable { operation }
        }
        _ => CredentialStoreError::BackendFailure { operation },
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn unsupported_platform(operation: CredentialOperation) -> CredentialStoreError {
    CredentialStoreError::BackendUnavailable { operation }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(account: &str) -> CredentialKey {
        CredentialKey::new("skibidibi-git/github.com", account).unwrap()
    }

    #[test]
    fn in_memory_store_supports_read_write_replace_and_delete() {
        let store = InMemoryCredentialStore::new();
        let key = key("123/access");

        assert_eq!(store.read(&key).unwrap(), None);
        store.write(&key, "first-secret").unwrap();
        assert_eq!(store.read(&key).unwrap().as_deref(), Some("first-secret"));
        store.write(&key, "replacement-secret").unwrap();
        assert_eq!(
            store.read(&key).unwrap().as_deref(),
            Some("replacement-secret")
        );
        assert!(store.delete(&key).unwrap());
        assert!(!store.delete(&key).unwrap());
        assert_eq!(store.read(&key).unwrap(), None);
    }

    #[test]
    fn credentials_are_isolated_by_service_and_account() {
        let store = InMemoryCredentialStore::new();
        let access = key("123/access");
        let refresh = key("123/refresh");
        let other_host =
            CredentialKey::new("skibidibi-git/enterprise.example", "123/access").unwrap();

        store.write(&access, "access-secret").unwrap();
        store.write(&refresh, "refresh-secret").unwrap();
        store.write(&other_host, "enterprise-secret").unwrap();

        assert_eq!(
            store.read(&access).unwrap().as_deref(),
            Some("access-secret")
        );
        assert_eq!(
            store.read(&refresh).unwrap().as_deref(),
            Some("refresh-secret")
        );
        assert_eq!(
            store.read(&other_host).unwrap().as_deref(),
            Some("enterprise-secret")
        );
    }

    #[test]
    fn rejects_empty_and_control_character_key_parts_without_echoing_them() {
        let secret_marker = "sensitive\nidentifier";
        let error = CredentialKey::new("", "account").unwrap_err();
        assert_eq!(
            error,
            CredentialStoreError::InvalidKey {
                part: CredentialKeyPart::Service
            }
        );

        let error = CredentialKey::new("service", secret_marker).unwrap_err();
        assert_eq!(
            error,
            CredentialStoreError::InvalidKey {
                part: CredentialKeyPart::Account
            }
        );
        assert!(!error.to_string().contains(secret_marker));
    }

    #[test]
    fn backend_errors_are_redacted_and_never_contain_keys_or_secrets() {
        let key_marker = "private-account";
        let secret_marker = "ghp_private_token";
        for error in [
            CredentialStoreError::BackendUnavailable {
                operation: CredentialOperation::Read,
            },
            CredentialStoreError::BackendFailure {
                operation: CredentialOperation::Write,
            },
        ] {
            let rendered = error.to_string();
            assert!(!rendered.contains(key_marker));
            assert!(!rendered.contains(secret_marker));
        }
    }

    #[test]
    fn rejects_empty_secrets_and_debug_output_never_contains_stored_values() {
        let store = InMemoryCredentialStore::new();
        let key = key("123/access");
        assert_eq!(
            store.write(&key, "").unwrap_err(),
            CredentialStoreError::EmptySecret
        );

        store.write(&key, "ghp_private_token").unwrap();
        let rendered = format!("{store:?}");
        assert!(!rendered.contains("ghp_private_token"));
        assert!(!rendered.contains("123/access"));
    }
}
