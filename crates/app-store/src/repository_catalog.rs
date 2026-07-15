use std::path::Path;

use app_domain::{
    HostedRepositoryIdentity, IntegrationHealth, IntegrationHealthIssue, IntegrationHealthState,
    RememberRepositoryInput, RememberedRepository, RepositoryAvailability, RepositoryHealthUpdate,
    RepositoryProvider, RepositoryTransport,
};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("repository path cannot be canonicalized: {0}")]
    Canonicalize(#[source] std::io::Error),
    #[error("repository path is not valid UTF-8")]
    NonUtf8Path,
    #[error("repository catalog database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("repository catalog contains invalid {field} value: {value}")]
    InvalidStoredValue { field: &'static str, value: String },
    #[error("repository catalog contains an invalid open count: {0}")]
    InvalidOpenCount(i64),
}

pub struct RepositoryCatalog {
    connection: Connection,
}

impl RepositoryCatalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let mut catalog = Self { connection };
        catalog.migrate()?;
        Ok(catalog)
    }

    pub fn open_in_memory() -> Result<Self, CatalogError> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let mut catalog = Self { connection };
        catalog.migrate()?;
        Ok(catalog)
    }

    fn migrate(&mut self) -> Result<(), CatalogError> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )?;
        let current_version = transaction.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        if current_version < SCHEMA_VERSION {
            transaction.execute_batch(
                "CREATE TABLE remembered_repositories (
                    id TEXT PRIMARY KEY NOT NULL,
                    canonical_path TEXT NOT NULL UNIQUE,
                    display_name TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    transport TEXT NOT NULL,
                    hosted_host TEXT,
                    hosted_owner TEXT,
                    hosted_name TEXT,
                    availability TEXT NOT NULL DEFAULT 'available',
                    git_health_state TEXT NOT NULL DEFAULT 'unknown',
                    git_health_issue TEXT,
                    git_health_checked_at INTEGER,
                    github_health_state TEXT NOT NULL DEFAULT 'unknown',
                    github_health_issue TEXT,
                    github_health_checked_at INTEGER,
                    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
                    open_count INTEGER NOT NULL DEFAULT 0 CHECK (open_count >= 0),
                    last_opened_at INTEGER,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    CHECK ((hosted_host IS NULL AND hosted_owner IS NULL AND hosted_name IS NULL)
                        OR (hosted_host IS NOT NULL AND hosted_owner IS NOT NULL AND hosted_name IS NOT NULL))
                );
                CREATE INDEX remembered_repositories_sort
                    ON remembered_repositories(pinned DESC, last_opened_at DESC, display_name ASC);",
            )?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, 0)",
                [SCHEMA_VERSION],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert(
        &mut self,
        input: &RememberRepositoryInput,
    ) -> Result<RememberedRepository, CatalogError> {
        let canonical_path = canonical_path(&input.path)?;
        let (host, owner, name) = hosted_parts(input.hosted_identity.as_ref());
        self.connection.execute(
            "INSERT INTO remembered_repositories (
                id, canonical_path, display_name, provider, transport,
                hosted_host, hosted_owner, hosted_name, availability, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'available', ?9, ?9)
            ON CONFLICT(canonical_path) DO UPDATE SET
                display_name = excluded.display_name, provider = excluded.provider,
                transport = excluded.transport, hosted_host = excluded.hosted_host,
                hosted_owner = excluded.hosted_owner, hosted_name = excluded.hosted_name,
                availability = 'available', updated_at = excluded.updated_at",
            params![
                input.id,
                canonical_path,
                input.display_name,
                provider_str(input.provider),
                transport_str(input.transport),
                host,
                owner,
                name,
                input.now
            ],
        )?;
        self.get_by_path(&canonical_path)?
            .ok_or_else(|| CatalogError::Database(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn list(&self) -> Result<Vec<RememberedRepository>, CatalogError> {
        let mut statement = self.connection.prepare(&format!(
            "{} ORDER BY pinned DESC, last_opened_at DESC, display_name COLLATE NOCASE ASC",
            SELECT_REPOSITORY
        ))?;
        statement
            .query_map([], map_repository)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get(&self, id: &str) -> Result<Option<RememberedRepository>, CatalogError> {
        self.connection
            .query_row(
                &format!("{} WHERE id = ?1", SELECT_REPOSITORY),
                [id],
                map_repository,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_pinned(&self, id: &str, pinned: bool, now: i64) -> Result<bool, CatalogError> {
        Ok(self.connection.execute(
            "UPDATE remembered_repositories SET pinned = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, pinned, now],
        )? > 0)
    }

    pub fn forget(&self, id: &str) -> Result<bool, CatalogError> {
        Ok(self
            .connection
            .execute("DELETE FROM remembered_repositories WHERE id = ?1", [id])?
            > 0)
    }

    pub fn touch_opened(&self, id: &str, now: i64) -> Result<bool, CatalogError> {
        Ok(self.connection.execute(
            "UPDATE remembered_repositories SET open_count = open_count + 1,
             last_opened_at = ?2, availability = 'available', updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )? > 0)
    }

    pub fn set_availability(
        &self,
        id: &str,
        availability: RepositoryAvailability,
        now: i64,
    ) -> Result<bool, CatalogError> {
        Ok(self.connection.execute(
            "UPDATE remembered_repositories SET availability = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, availability_str(availability), now],
        )? > 0)
    }

    pub fn refresh_availability(&self, id: &str, now: i64) -> Result<bool, CatalogError> {
        let path = self
            .connection
            .query_row(
                "SELECT canonical_path FROM remembered_repositories WHERE id = ?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(path) = path else { return Ok(false) };
        let availability = match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => RepositoryAvailability::Available,
            Ok(_) => RepositoryAvailability::Missing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                RepositoryAvailability::Missing
            }
            Err(_) => RepositoryAvailability::Inaccessible,
        };
        self.set_availability(id, availability, now)
    }

    pub fn update_health(
        &self,
        id: &str,
        update: &RepositoryHealthUpdate,
    ) -> Result<bool, CatalogError> {
        let changed = match (&update.git, &update.github) {
            (None, None) => return Ok(false),
            (Some(git), None) => self.connection.execute(
                "UPDATE remembered_repositories SET git_health_state = ?2, git_health_issue = ?3,
                 git_health_checked_at = ?4, updated_at = ?5 WHERE id = ?1",
                params![id, health_state_str(git.state), git.issue.map(health_issue_str), git.checked_at, update.now],
            )?,
            (None, Some(github)) => self.connection.execute(
                "UPDATE remembered_repositories SET github_health_state = ?2, github_health_issue = ?3,
                 github_health_checked_at = ?4, updated_at = ?5 WHERE id = ?1",
                params![id, health_state_str(github.state), github.issue.map(health_issue_str), github.checked_at, update.now],
            )?,
            (Some(git), Some(github)) => self.connection.execute(
                "UPDATE remembered_repositories SET git_health_state = ?2, git_health_issue = ?3,
                 git_health_checked_at = ?4, github_health_state = ?5, github_health_issue = ?6,
                 github_health_checked_at = ?7, updated_at = ?8 WHERE id = ?1",
                params![id, health_state_str(git.state), git.issue.map(health_issue_str), git.checked_at,
                    health_state_str(github.state), github.issue.map(health_issue_str), github.checked_at, update.now],
            )?,
        };
        Ok(changed > 0)
    }

    fn get_by_path(&self, path: &str) -> Result<Option<RememberedRepository>, CatalogError> {
        self.connection
            .query_row(
                &format!("{} WHERE canonical_path = ?1", SELECT_REPOSITORY),
                [path],
                map_repository,
            )
            .optional()
            .map_err(Into::into)
    }
}

const SELECT_REPOSITORY: &str = "SELECT id, canonical_path, display_name, provider, transport,
        hosted_host, hosted_owner, hosted_name, availability,
        git_health_state, git_health_issue, git_health_checked_at,
        github_health_state, github_health_issue, github_health_checked_at,
        pinned, open_count, last_opened_at, created_at, updated_at FROM remembered_repositories";

fn canonical_path(path: &str) -> Result<String, CatalogError> {
    std::fs::canonicalize(path)
        .map_err(CatalogError::Canonicalize)?
        .into_os_string()
        .into_string()
        .map_err(|_| CatalogError::NonUtf8Path)
}

fn hosted_parts(
    identity: Option<&HostedRepositoryIdentity>,
) -> (Option<&str>, Option<&str>, Option<&str>) {
    identity
        .map(|identity| {
            (
                Some(identity.host.as_str()),
                Some(identity.owner.as_str()),
                Some(identity.name.as_str()),
            )
        })
        .unwrap_or((None, None, None))
}

fn map_repository(row: &rusqlite::Row<'_>) -> rusqlite::Result<RememberedRepository> {
    let hosted = match (row.get(5)?, row.get(6)?, row.get(7)?) {
        (Some(host), Some(owner), Some(name)) => {
            Some(HostedRepositoryIdentity { host, owner, name })
        }
        _ => None,
    };
    let stored_open_count: i64 = row.get(16)?;
    let open_count = u64::try_from(stored_open_count)
        .map_err(|_| sql_conversion_error(CatalogError::InvalidOpenCount(stored_open_count)))?;
    Ok(RememberedRepository {
        id: row.get(0)?,
        canonical_path: row.get(1)?,
        display_name: row.get(2)?,
        provider: parse_provider(row.get(3)?).map_err(sql_conversion_error)?,
        transport: parse_transport(row.get(4)?).map_err(sql_conversion_error)?,
        hosted_identity: hosted,
        availability: parse_availability(row.get(8)?).map_err(sql_conversion_error)?,
        git_health: IntegrationHealth {
            state: parse_health_state(row.get(9)?).map_err(sql_conversion_error)?,
            issue: parse_optional_issue(row.get(10)?).map_err(sql_conversion_error)?,
            checked_at: row.get(11)?,
        },
        github_health: IntegrationHealth {
            state: parse_health_state(row.get(12)?).map_err(sql_conversion_error)?,
            issue: parse_optional_issue(row.get(13)?).map_err(sql_conversion_error)?,
            checked_at: row.get(14)?,
        },
        pinned: row.get(15)?,
        open_count,
        last_opened_at: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

fn sql_conversion_error(error: CatalogError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

macro_rules! string_enum {
    ($format:ident, $parse:ident, $type:ty, {$($variant:path => $value:literal),+ $(,)?}) => {
        fn $format(value: $type) -> &'static str { match value { $($variant => $value),+ } }
        fn $parse(value: String) -> Result<$type, CatalogError> {
            match value.as_str() {
                $($value => Ok($variant),)+
                _ => Err(CatalogError::InvalidStoredValue { field: stringify!($type), value }),
            }
        }
    };
}

string_enum!(provider_str, parse_provider, RepositoryProvider, {
    RepositoryProvider::Local => "local", RepositoryProvider::GitHub => "github", RepositoryProvider::Other => "other"
});
string_enum!(transport_str, parse_transport, RepositoryTransport, {
    RepositoryTransport::Local => "local", RepositoryTransport::Ssh => "ssh", RepositoryTransport::Https => "https", RepositoryTransport::Other => "other"
});
string_enum!(availability_str, parse_availability, RepositoryAvailability, {
    RepositoryAvailability::Unknown => "unknown", RepositoryAvailability::Available => "available", RepositoryAvailability::Missing => "missing", RepositoryAvailability::Inaccessible => "inaccessible"
});
string_enum!(health_state_str, parse_health_state, IntegrationHealthState, {
    IntegrationHealthState::Unknown => "unknown", IntegrationHealthState::Healthy => "healthy", IntegrationHealthState::Degraded => "degraded", IntegrationHealthState::Unavailable => "unavailable"
});
string_enum!(health_issue_str, parse_health_issue, IntegrationHealthIssue, {
    IntegrationHealthIssue::Authentication => "authentication", IntegrationHealthIssue::Authorization => "authorization", IntegrationHealthIssue::Network => "network", IntegrationHealthIssue::NotFound => "not_found", IntegrationHealthIssue::InvalidConfiguration => "invalid_configuration", IntegrationHealthIssue::OperationFailed => "operation_failed"
});

fn parse_optional_issue(
    value: Option<String>,
) -> Result<Option<IntegrationHealthIssue>, CatalogError> {
    value.map(parse_health_issue).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn input(path: &Path, id: &str, name: &str, now: i64) -> RememberRepositoryInput {
        RememberRepositoryInput {
            id: id.into(),
            path: path.to_string_lossy().into_owned(),
            display_name: name.into(),
            provider: RepositoryProvider::GitHub,
            transport: RepositoryTransport::Ssh,
            hosted_identity: Some(HostedRepositoryIdentity {
                host: "github.com".into(),
                owner: "owner".into(),
                name: name.into(),
            }),
            now,
        }
    }

    #[test]
    fn upsert_canonicalizes_identity_and_preserves_open_metadata() {
        let directory = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        catalog
            .upsert(&input(directory.path(), "first", "before", 10))
            .unwrap();
        catalog.touch_opened("first", 11).unwrap();
        catalog.set_pinned("first", true, 12).unwrap();
        let updated = catalog
            .upsert(&input(
                &directory.path().join("."),
                "replacement",
                "after",
                13,
            ))
            .unwrap();
        assert_eq!(updated.id, "first");
        assert_eq!(
            (
                updated.display_name.as_str(),
                updated.open_count,
                updated.last_opened_at,
                updated.pinned
            ),
            ("after", 1, Some(11), true)
        );
        assert_eq!(catalog.list().unwrap().len(), 1);
    }

    #[test]
    fn list_prioritizes_pinned_then_recently_opened() {
        let dirs = [tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap()];
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        for (dir, id) in dirs.iter().zip(["first", "second", "third"]) {
            catalog.upsert(&input(dir.path(), id, id, 1)).unwrap();
        }
        catalog.touch_opened("first", 20).unwrap();
        catalog.touch_opened("second", 30).unwrap();
        catalog.set_pinned("third", true, 40).unwrap();
        let ids: Vec<_> = catalog
            .list()
            .unwrap()
            .into_iter()
            .map(|repo| repo.id)
            .collect();
        assert_eq!(ids, ["third", "second", "first"]);
    }

    #[test]
    fn get_resolves_only_the_requested_repository_id() {
        let dirs = [tempdir().unwrap(), tempdir().unwrap()];
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        catalog
            .upsert(&input(dirs[0].path(), "first", "first", 1))
            .unwrap();
        catalog
            .upsert(&input(dirs[1].path(), "second", "second", 2))
            .unwrap();

        assert_eq!(catalog.get("second").unwrap().unwrap().id, "second");
        assert!(catalog.get("missing").unwrap().is_none());
    }

    #[test]
    fn health_channels_are_independent() {
        let directory = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        catalog
            .upsert(&input(directory.path(), "repo", "repo", 1))
            .unwrap();
        catalog
            .update_health(
                "repo",
                &RepositoryHealthUpdate {
                    git: Some(IntegrationHealth {
                        state: IntegrationHealthState::Healthy,
                        issue: None,
                        checked_at: Some(20),
                    }),
                    github: None,
                    now: 20,
                },
            )
            .unwrap();
        catalog
            .update_health(
                "repo",
                &RepositoryHealthUpdate {
                    git: None,
                    github: Some(IntegrationHealth {
                        state: IntegrationHealthState::Unavailable,
                        issue: Some(IntegrationHealthIssue::Authentication),
                        checked_at: Some(21),
                    }),
                    now: 21,
                },
            )
            .unwrap();
        let repo = catalog.list().unwrap().remove(0);
        assert_eq!(repo.git_health.state, IntegrationHealthState::Healthy);
        assert_eq!(
            repo.github_health.issue,
            Some(IntegrationHealthIssue::Authentication)
        );
    }

    #[test]
    fn forget_and_availability_are_persisted() {
        let directory = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        catalog
            .upsert(&input(directory.path(), "repo", "repo", 1))
            .unwrap();
        catalog
            .set_availability("repo", RepositoryAvailability::Missing, 2)
            .unwrap();
        assert_eq!(
            catalog.list().unwrap()[0].availability,
            RepositoryAvailability::Missing
        );
        assert!(catalog.forget("repo").unwrap());
        assert!(catalog.list().unwrap().is_empty());
    }

    #[test]
    fn nonexistent_paths_are_not_inserted() {
        let directory = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        assert!(matches!(
            catalog.upsert(&input(&directory.path().join("missing"), "repo", "repo", 1)),
            Err(CatalogError::Canonicalize(_))
        ));
        assert!(catalog.list().unwrap().is_empty());
    }
}
