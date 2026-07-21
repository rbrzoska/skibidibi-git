use std::path::Path;

use app_domain::{GitHubAccountState, GitHubAccountSummary, GitHubAuthKind};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use thiserror::Error;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubAccount {
    pub id: String,
    pub host: String,
    pub provider_user_id: String,
    pub login: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub auth_kind: GitHubAuthKind,
    pub scopes: Vec<String>,
    pub state: GitHubAccountState,
    pub access_token_expires_at: Option<i64>,
    pub last_validated_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl GitHubAccount {
    pub fn summary(&self) -> GitHubAccountSummary {
        GitHubAccountSummary {
            id: self.id.clone(),
            host: self.host.clone(),
            login: self.login.clone(),
            display_name: self.display_name.clone(),
            avatar_url: self.avatar_url.clone(),
            auth_kind: self.auth_kind,
            scopes: self.scopes.clone(),
            state: self.state,
            last_validated_at: self.last_validated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertGitHubAccount {
    pub id: String,
    pub host: String,
    pub provider_user_id: String,
    pub login: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub auth_kind: GitHubAuthKind,
    pub scopes: Vec<String>,
    pub state: GitHubAccountState,
    pub access_token_expires_at: Option<i64>,
    pub last_validated_at: Option<i64>,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepositoryBinding {
    pub repository_id: String,
    pub account_id: String,
    pub provider_repository_id: Option<String>,
    pub owner: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertGitHubRepositoryBinding {
    pub repository_id: String,
    pub account_id: String,
    pub provider_repository_id: Option<String>,
    pub owner: String,
    pub name: String,
    pub now: i64,
}

#[derive(Debug, Error)]
pub enum GitHubAccountStoreError {
    #[error("GitHub account metadata field {field} is invalid")]
    InvalidInput { field: &'static str },
    #[error("GitHub account metadata contains an invalid {field} value")]
    InvalidStoredValue { field: &'static str },
    #[error("GitHub account metadata database error: {0}")]
    Database(#[from] rusqlite::Error),
}

pub struct GitHubAccountStore {
    connection: Connection,
}

impl GitHubAccountStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GitHubAccountStoreError> {
        let connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, GitHubAccountStoreError> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn upsert_account(
        &mut self,
        input: &UpsertGitHubAccount,
    ) -> Result<GitHubAccount, GitHubAccountStoreError> {
        validate_account(input)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO github_accounts (
                id, host, provider_user_id, login, display_name, avatar_url, auth_kind,
                state, access_token_expires_at, last_validated_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(host, provider_user_id) DO UPDATE SET
                login = excluded.login,
                display_name = excluded.display_name,
                avatar_url = excluded.avatar_url,
                auth_kind = excluded.auth_kind,
                state = excluded.state,
                access_token_expires_at = excluded.access_token_expires_at,
                last_validated_at = excluded.last_validated_at,
                updated_at = excluded.updated_at",
            params![
                input.id,
                normalize_host(&input.host),
                input.provider_user_id,
                input.login,
                input.display_name,
                input.avatar_url,
                auth_kind_str(input.auth_kind),
                account_state_str(input.state),
                input.access_token_expires_at,
                input.last_validated_at,
                input.now,
            ],
        )?;
        let account_id: String = transaction.query_row(
            "SELECT id FROM github_accounts WHERE host = ?1 AND provider_user_id = ?2",
            params![normalize_host(&input.host), input.provider_user_id],
            |row| row.get(0),
        )?;
        replace_scopes(&transaction, &account_id, &input.scopes)?;
        transaction.commit()?;
        self.get_account(&account_id)?
            .ok_or_else(|| GitHubAccountStoreError::Database(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn get_account(&self, id: &str) -> Result<Option<GitHubAccount>, GitHubAccountStoreError> {
        let account = self
            .connection
            .query_row(
                &format!("{SELECT_ACCOUNT} WHERE id = ?1"),
                [id],
                map_account,
            )
            .optional()?;
        account.map(|account| self.with_scopes(account)).transpose()
    }

    pub fn list_accounts(&self) -> Result<Vec<GitHubAccount>, GitHubAccountStoreError> {
        let mut statement = self.connection.prepare(&format!(
            "{SELECT_ACCOUNT} ORDER BY host COLLATE NOCASE, login COLLATE NOCASE, id"
        ))?;
        let accounts = statement
            .query_map([], map_account)?
            .collect::<Result<Vec<_>, _>>()?;
        accounts
            .into_iter()
            .map(|account| self.with_scopes(account))
            .collect()
    }

    pub fn delete_account(&self, id: &str) -> Result<bool, GitHubAccountStoreError> {
        Ok(self
            .connection
            .execute("DELETE FROM github_accounts WHERE id = ?1", [id])?
            > 0)
    }

    pub fn upsert_repository_binding(
        &self,
        input: &UpsertGitHubRepositoryBinding,
    ) -> Result<GitHubRepositoryBinding, GitHubAccountStoreError> {
        validate_binding(input)?;
        self.connection.execute(
            "INSERT INTO github_repository_bindings (
                repository_id, account_id, provider_repository_id, owner, name,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(repository_id) DO UPDATE SET
                account_id = excluded.account_id,
                provider_repository_id = excluded.provider_repository_id,
                owner = excluded.owner,
                name = excluded.name,
                updated_at = excluded.updated_at",
            params![
                input.repository_id,
                input.account_id,
                input.provider_repository_id,
                input.owner,
                input.name,
                input.now,
            ],
        )?;
        self.get_repository_binding(&input.repository_id)?
            .ok_or_else(|| GitHubAccountStoreError::Database(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn get_repository_binding(
        &self,
        repository_id: &str,
    ) -> Result<Option<GitHubRepositoryBinding>, GitHubAccountStoreError> {
        self.connection
            .query_row(
                &format!("{SELECT_BINDING} WHERE repository_id = ?1"),
                [repository_id],
                map_binding,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_repository_bindings(
        &self,
    ) -> Result<Vec<GitHubRepositoryBinding>, GitHubAccountStoreError> {
        let mut statement = self.connection.prepare(&format!(
            "{SELECT_BINDING} ORDER BY owner COLLATE NOCASE, name COLLATE NOCASE, repository_id"
        ))?;
        statement
            .query_map([], map_binding)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn delete_repository_binding(
        &self,
        repository_id: &str,
    ) -> Result<bool, GitHubAccountStoreError> {
        Ok(self.connection.execute(
            "DELETE FROM github_repository_bindings WHERE repository_id = ?1",
            [repository_id],
        )? > 0)
    }

    fn migrate(&mut self) -> Result<(), GitHubAccountStoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS github_account_schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )?;
        let current_version = transaction.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM github_account_schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        if current_version < SCHEMA_VERSION {
            transaction.execute_batch(
                "CREATE TABLE github_accounts (
                    id TEXT PRIMARY KEY NOT NULL,
                    host TEXT NOT NULL,
                    provider_user_id TEXT NOT NULL,
                    login TEXT NOT NULL,
                    display_name TEXT,
                    avatar_url TEXT,
                    auth_kind TEXT NOT NULL,
                    state TEXT NOT NULL,
                    access_token_expires_at INTEGER,
                    last_validated_at INTEGER,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    UNIQUE(host, provider_user_id)
                );
                CREATE TABLE github_account_scopes (
                    account_id TEXT NOT NULL REFERENCES github_accounts(id) ON DELETE CASCADE,
                    scope TEXT NOT NULL,
                    PRIMARY KEY(account_id, scope)
                );
                CREATE TABLE github_repository_bindings (
                    repository_id TEXT PRIMARY KEY NOT NULL,
                    account_id TEXT NOT NULL REFERENCES github_accounts(id) ON DELETE CASCADE,
                    provider_repository_id TEXT,
                    owner TEXT NOT NULL,
                    name TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE INDEX github_repository_bindings_account
                    ON github_repository_bindings(account_id);",
            )?;
            transaction.execute(
                "INSERT INTO github_account_schema_migrations(version, applied_at) VALUES (?1, 0)",
                [SCHEMA_VERSION],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn with_scopes(
        &self,
        mut account: GitHubAccount,
    ) -> Result<GitHubAccount, GitHubAccountStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT scope FROM github_account_scopes WHERE account_id = ?1 ORDER BY scope",
        )?;
        account.scopes = statement
            .query_map([&account.id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(account)
    }
}

const SELECT_ACCOUNT: &str = "SELECT id, host, provider_user_id, login, display_name, avatar_url,
    auth_kind, state, access_token_expires_at, last_validated_at, created_at, updated_at
    FROM github_accounts";
const SELECT_BINDING: &str = "SELECT repository_id, account_id, provider_repository_id, owner,
    name, created_at, updated_at FROM github_repository_bindings";

fn map_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitHubAccount> {
    let auth_kind = parse_auth_kind(row.get(6)?).map_err(sql_conversion_error)?;
    let state = parse_account_state(row.get(7)?).map_err(sql_conversion_error)?;
    Ok(GitHubAccount {
        id: row.get(0)?,
        host: row.get(1)?,
        provider_user_id: row.get(2)?,
        login: row.get(3)?,
        display_name: row.get(4)?,
        avatar_url: row.get(5)?,
        auth_kind,
        scopes: Vec::new(),
        state,
        access_token_expires_at: row.get(8)?,
        last_validated_at: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn map_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitHubRepositoryBinding> {
    Ok(GitHubRepositoryBinding {
        repository_id: row.get(0)?,
        account_id: row.get(1)?,
        provider_repository_id: row.get(2)?,
        owner: row.get(3)?,
        name: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn replace_scopes(
    transaction: &Transaction<'_>,
    account_id: &str,
    scopes: &[String],
) -> Result<(), GitHubAccountStoreError> {
    transaction.execute(
        "DELETE FROM github_account_scopes WHERE account_id = ?1",
        [account_id],
    )?;
    for scope in normalized_scopes(scopes)? {
        transaction.execute(
            "INSERT INTO github_account_scopes(account_id, scope) VALUES (?1, ?2)",
            params![account_id, scope],
        )?;
    }
    Ok(())
}

fn normalized_scopes(scopes: &[String]) -> Result<Vec<&str>, GitHubAccountStoreError> {
    let mut normalized = Vec::with_capacity(scopes.len());
    for scope in scopes {
        validate_text(scope, "scopes")?;
        normalized.push(scope.as_str());
    }
    normalized.sort_unstable();
    normalized.dedup();
    Ok(normalized)
}

fn validate_account(input: &UpsertGitHubAccount) -> Result<(), GitHubAccountStoreError> {
    validate_text(&input.id, "id")?;
    validate_text(&input.host, "host")?;
    validate_text(&input.provider_user_id, "provider_user_id")?;
    validate_text(&input.login, "login")?;
    validate_optional_text(input.display_name.as_deref(), "display_name")?;
    validate_optional_text(input.avatar_url.as_deref(), "avatar_url")?;
    let normalized = normalize_host(&input.host);
    if !valid_host(&normalized) {
        return Err(GitHubAccountStoreError::InvalidInput { field: "host" });
    }
    normalized_scopes(&input.scopes)?;
    Ok(())
}

fn validate_binding(input: &UpsertGitHubRepositoryBinding) -> Result<(), GitHubAccountStoreError> {
    validate_text(&input.repository_id, "repository_id")?;
    validate_text(&input.account_id, "account_id")?;
    validate_text(&input.owner, "owner")?;
    validate_text(&input.name, "name")?;
    validate_optional_text(
        input.provider_repository_id.as_deref(),
        "provider_repository_id",
    )?;
    if input.owner.contains('/') || input.name.contains('/') {
        return Err(GitHubAccountStoreError::InvalidInput {
            field: "repository_identity",
        });
    }
    Ok(())
}

fn validate_text(value: &str, field: &'static str) -> Result<(), GitHubAccountStoreError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(GitHubAccountStoreError::InvalidInput { field })
    } else {
        Ok(())
    }
}

fn validate_optional_text(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), GitHubAccountStoreError> {
    value.map_or(Ok(()), |value| validate_text(value, field))
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn auth_kind_str(value: GitHubAuthKind) -> &'static str {
    match value {
        GitHubAuthKind::PersonalAccessToken => "personal_access_token",
        GitHubAuthKind::OAuthDevice => "oauth_device",
        GitHubAuthKind::GitHubCli => "github_cli",
    }
}

fn parse_auth_kind(value: String) -> Result<GitHubAuthKind, GitHubAccountStoreError> {
    match value.as_str() {
        "personal_access_token" => Ok(GitHubAuthKind::PersonalAccessToken),
        "oauth_device" => Ok(GitHubAuthKind::OAuthDevice),
        "github_cli" => Ok(GitHubAuthKind::GitHubCli),
        _ => Err(GitHubAccountStoreError::InvalidStoredValue { field: "auth_kind" }),
    }
}

fn account_state_str(value: GitHubAccountState) -> &'static str {
    match value {
        GitHubAccountState::Unknown => "unknown",
        GitHubAccountState::Connected => "connected",
        GitHubAccountState::AuthenticationRequired => "authentication_required",
        GitHubAccountState::Unavailable => "unavailable",
    }
}

fn parse_account_state(value: String) -> Result<GitHubAccountState, GitHubAccountStoreError> {
    match value.as_str() {
        "unknown" => Ok(GitHubAccountState::Unknown),
        "connected" => Ok(GitHubAccountState::Connected),
        "authentication_required" => Ok(GitHubAccountState::AuthenticationRequired),
        "unavailable" => Ok(GitHubAccountState::Unavailable),
        _ => Err(GitHubAccountStoreError::InvalidStoredValue { field: "state" }),
    }
}

fn sql_conversion_error(error: GitHubAccountStoreError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn account(id: &str, user_id: &str, now: i64) -> UpsertGitHubAccount {
        UpsertGitHubAccount {
            id: id.to_owned(),
            host: "GitHub.COM.".to_owned(),
            provider_user_id: user_id.to_owned(),
            login: "octocat".to_owned(),
            display_name: Some("The Octocat".to_owned()),
            avatar_url: Some("https://avatars.githubusercontent.com/u/1".to_owned()),
            auth_kind: GitHubAuthKind::OAuthDevice,
            scopes: vec!["pull_requests:read".to_owned(), "metadata:read".to_owned()],
            state: GitHubAccountState::Connected,
            access_token_expires_at: Some(1000),
            last_validated_at: Some(now),
            now,
        }
    }

    fn binding(repository_id: &str, account_id: &str, now: i64) -> UpsertGitHubRepositoryBinding {
        UpsertGitHubRepositoryBinding {
            repository_id: repository_id.to_owned(),
            account_id: account_id.to_owned(),
            provider_repository_id: Some("R_123".to_owned()),
            owner: "owner".to_owned(),
            name: "repository".to_owned(),
            now,
        }
    }

    #[test]
    fn account_upsert_normalizes_identity_preserves_id_and_replaces_scopes() {
        let mut store = GitHubAccountStore::open_in_memory().unwrap();
        let inserted = store
            .upsert_account(&account("account-1", "1", 10))
            .unwrap();
        assert_eq!(inserted.host, "github.com");
        assert_eq!(inserted.scopes, ["metadata:read", "pull_requests:read"]);

        let mut update = account("replacement-id", "1", 20);
        update.login = "updated-login".to_owned();
        update.auth_kind = GitHubAuthKind::PersonalAccessToken;
        update.scopes = vec![
            "pull_requests:read".to_owned(),
            "pull_requests:read".to_owned(),
        ];
        let updated = store.upsert_account(&update).unwrap();

        assert_eq!(updated.id, "account-1");
        assert_eq!(updated.login, "updated-login");
        assert_eq!(updated.auth_kind, GitHubAuthKind::PersonalAccessToken);
        assert_eq!(updated.scopes, ["pull_requests:read"]);
        assert_eq!((updated.created_at, updated.updated_at), (10, 20));
        assert_eq!(store.list_accounts().unwrap().len(), 1);
    }

    #[test]
    fn account_summary_uses_the_shared_domain_contract() {
        let mut store = GitHubAccountStore::open_in_memory().unwrap();
        let stored = store
            .upsert_account(&account("account-1", "1", 10))
            .unwrap();
        let summary = stored.summary();

        assert_eq!(summary.id, "account-1");
        assert_eq!(summary.auth_kind, GitHubAuthKind::OAuthDevice);
        assert_eq!(summary.state, GitHubAccountState::Connected);
        assert_eq!(summary.scopes, ["metadata:read", "pull_requests:read"]);
    }

    #[test]
    fn repository_binding_can_be_switched_and_removed() {
        let mut store = GitHubAccountStore::open_in_memory().unwrap();
        store.upsert_account(&account("first", "1", 1)).unwrap();
        store.upsert_account(&account("second", "2", 2)).unwrap();
        store
            .upsert_repository_binding(&binding("repo", "first", 3))
            .unwrap();
        let switched = store
            .upsert_repository_binding(&binding("repo", "second", 4))
            .unwrap();

        assert_eq!(switched.account_id, "second");
        assert_eq!((switched.created_at, switched.updated_at), (3, 4));
        assert_eq!(store.list_repository_bindings().unwrap().len(), 1);
        assert!(store.delete_repository_binding("repo").unwrap());
        assert!(!store.delete_repository_binding("repo").unwrap());
    }

    #[test]
    fn deleting_an_account_cascades_only_its_repository_bindings() {
        let mut store = GitHubAccountStore::open_in_memory().unwrap();
        store.upsert_account(&account("first", "1", 1)).unwrap();
        store.upsert_account(&account("second", "2", 2)).unwrap();
        store
            .upsert_repository_binding(&binding("repo-1", "first", 3))
            .unwrap();
        store
            .upsert_repository_binding(&binding("repo-2", "second", 3))
            .unwrap();

        assert!(store.delete_account("first").unwrap());
        assert!(store.get_account("first").unwrap().is_none());
        assert!(store.get_repository_binding("repo-1").unwrap().is_none());
        assert!(store.get_repository_binding("repo-2").unwrap().is_some());
    }

    #[test]
    fn rejects_invalid_metadata_and_unknown_binding_accounts() {
        let mut store = GitHubAccountStore::open_in_memory().unwrap();
        let mut invalid = account("account", "1", 1);
        invalid.host = "https://github.com".to_owned();
        assert!(matches!(
            store.upsert_account(&invalid),
            Err(GitHubAccountStoreError::InvalidInput { field: "host" })
        ));
        assert!(matches!(
            store.upsert_repository_binding(&binding("repo", "missing", 1)),
            Err(GitHubAccountStoreError::Database(
                rusqlite::Error::SqliteFailure(_, _)
            ))
        ));
    }

    #[test]
    fn persisted_schema_contains_metadata_only_and_no_token_or_secret_columns() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("accounts.sqlite3");
        let mut store = GitHubAccountStore::open(&path).unwrap();
        store.upsert_account(&account("account", "1", 1)).unwrap();
        drop(store);

        let connection = Connection::open(path).unwrap();
        let schema: String = connection
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE name LIKE 'github_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let lower = schema.to_ascii_lowercase();
        assert!(!lower.contains("access_token text"));
        assert!(!lower.contains("refresh_token"));
        assert!(!lower.contains("secret"));
    }
}
