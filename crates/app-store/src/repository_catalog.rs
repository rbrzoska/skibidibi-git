use std::path::Path;

use app_domain::{
    HostedRepositoryIdentity, IntegrationHealth, IntegrationHealthIssue, IntegrationHealthState,
    RememberRepositoryInput, RememberedRepository, RepositoryAvailability, RepositoryGitIdentity,
    RepositoryGroupRelation, RepositoryHealthUpdate, RepositoryProvider, RepositoryRelationKind,
    RepositoryTransport, RepositoryWorktreeRole,
};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

const SCHEMA_VERSION: i64 = 3;

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
    #[error("repository group does not exist: {0}")]
    MissingRepositoryGroup(String),
    #[error("repository groups cannot be related to themselves")]
    SelfRelation,
    #[error("repository relation would create a cycle")]
    RelationCycle,
    #[error("repository relation path must be a safe relative path")]
    InvalidRelationPath,
}

pub struct RepositoryCatalog {
    connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryCatalogReconciliation {
    pub repository_id: String,
    pub availability: RepositoryAvailability,
    /// `None` preserves the last known identity, which is required for missing paths.
    pub git_identity: Option<RepositoryGitIdentity>,
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
        if current_version < 1 {
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
                "INSERT INTO schema_migrations(version, applied_at) VALUES (1, 0)",
                [],
            )?;
        }
        if current_version < 2 {
            transaction.execute_batch(
                "CREATE TABLE repository_groups (
                    id TEXT PRIMARY KEY NOT NULL,
                    canonical_common_dir TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                ALTER TABLE remembered_repositories
                    ADD COLUMN repository_group_id TEXT REFERENCES repository_groups(id) ON DELETE SET NULL;
                ALTER TABLE remembered_repositories
                    ADD COLUMN worktree_role TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (worktree_role IN ('main', 'linked', 'bare', 'unknown'));
                CREATE INDEX remembered_repositories_group_idx
                    ON remembered_repositories(repository_group_id);
                INSERT INTO schema_migrations(version, applied_at) VALUES (2, 0);",
            )?;
        }
        if current_version < SCHEMA_VERSION {
            transaction.execute_batch(
                "CREATE TABLE repository_group_relations (
                    parent_group_id TEXT NOT NULL
                        REFERENCES repository_groups(id) ON DELETE CASCADE,
                    child_group_id TEXT NOT NULL
                        REFERENCES repository_groups(id) ON DELETE CASCADE,
                    kind TEXT NOT NULL CHECK (kind IN ('submodule')),
                    relative_path TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (parent_group_id, kind, relative_path),
                    CHECK (parent_group_id <> child_group_id)
                );
                CREATE INDEX repository_group_relations_child_idx
                    ON repository_group_relations(child_group_id, kind);
                INSERT INTO schema_migrations(version, applied_at) VALUES (3, 0);",
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
        let transaction = self.connection.transaction()?;
        let repository_group_id = if let Some(identity) = input.git_identity.as_ref() {
            transaction.execute(
                "INSERT INTO repository_groups (id, canonical_common_dir, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT(canonical_common_dir) DO UPDATE SET updated_at = excluded.updated_at",
                params![
                    identity.repository_group_id,
                    identity.canonical_common_dir,
                    input.now
                ],
            )?;
            Some(transaction.query_row(
                "SELECT id FROM repository_groups WHERE canonical_common_dir = ?1",
                [&identity.canonical_common_dir],
                |row| row.get::<_, String>(0),
            )?)
        } else {
            None
        };
        let worktree_role = input
            .git_identity
            .as_ref()
            .map_or(RepositoryWorktreeRole::Unknown, |identity| {
                identity.worktree_role
            });
        transaction.execute(
            "INSERT INTO remembered_repositories (
                id, canonical_path, display_name, provider, transport,
                hosted_host, hosted_owner, hosted_name, availability, created_at, updated_at,
                repository_group_id, worktree_role
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'available', ?9, ?9, ?10, ?11)
            ON CONFLICT(canonical_path) DO UPDATE SET
                display_name = excluded.display_name, provider = excluded.provider,
                transport = excluded.transport, hosted_host = excluded.hosted_host,
                hosted_owner = excluded.hosted_owner, hosted_name = excluded.hosted_name,
                availability = 'available', updated_at = excluded.updated_at,
                repository_group_id = COALESCE(excluded.repository_group_id, remembered_repositories.repository_group_id),
                worktree_role = CASE WHEN excluded.repository_group_id IS NULL
                    THEN remembered_repositories.worktree_role ELSE excluded.worktree_role END",
            params![
                input.id,
                canonical_path,
                input.display_name,
                provider_str(input.provider),
                transport_str(input.transport),
                host,
                owner,
                name,
                input.now,
                repository_group_id,
                worktree_role_str(worktree_role),
            ],
        )?;
        transaction.commit()?;
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

    pub fn list_relations(&self) -> Result<Vec<RepositoryGroupRelation>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT parent_group_id, child_group_id, kind, relative_path
             FROM repository_group_relations
             ORDER BY parent_group_id, kind, relative_path",
        )?;
        statement
            .query_map([], |row| {
                let kind = parse_relation_kind(row.get(2)?).map_err(sql_conversion_error)?;
                Ok(RepositoryGroupRelation {
                    parent_repository_group_id: row.get(0)?,
                    child_repository_group_id: row.get(1)?,
                    kind,
                    relative_path: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn link_submodule(
        &mut self,
        parent_group_id: &str,
        child_group_id: &str,
        relative_path: &str,
        now: i64,
    ) -> Result<RepositoryGroupRelation, CatalogError> {
        if parent_group_id == child_group_id {
            return Err(CatalogError::SelfRelation);
        }
        if !is_safe_relation_path(relative_path) {
            return Err(CatalogError::InvalidRelationPath);
        }

        let transaction = self.connection.transaction()?;
        for group_id in [parent_group_id, child_group_id] {
            let exists = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM repository_groups WHERE id = ?1)",
                [group_id],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                return Err(CatalogError::MissingRepositoryGroup(group_id.to_owned()));
            }
        }
        let creates_cycle = transaction.query_row(
            "WITH RECURSIVE descendants(group_id) AS (
                SELECT child_group_id FROM repository_group_relations
                 WHERE parent_group_id = ?1 AND kind = 'submodule'
                UNION
                SELECT relation.child_group_id
                  FROM repository_group_relations relation
                  JOIN descendants ON relation.parent_group_id = descendants.group_id
                 WHERE relation.kind = 'submodule'
             )
             SELECT EXISTS (SELECT 1 FROM descendants WHERE group_id = ?2)",
            params![child_group_id, parent_group_id],
            |row| row.get::<_, bool>(0),
        )?;
        if creates_cycle {
            return Err(CatalogError::RelationCycle);
        }

        transaction.execute(
            "INSERT INTO repository_group_relations (
                parent_group_id, child_group_id, kind, relative_path, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(parent_group_id, kind, relative_path) DO UPDATE SET
                child_group_id = excluded.child_group_id,
                updated_at = excluded.updated_at",
            params![
                parent_group_id,
                child_group_id,
                relation_kind_str(RepositoryRelationKind::Submodule),
                relative_path,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(RepositoryGroupRelation {
            parent_repository_group_id: parent_group_id.to_owned(),
            child_repository_group_id: child_group_id.to_owned(),
            kind: RepositoryRelationKind::Submodule,
            relative_path: relative_path.to_owned(),
        })
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

    pub fn set_git_identity(
        &mut self,
        id: &str,
        identity: &RepositoryGitIdentity,
        now: i64,
    ) -> Result<bool, CatalogError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO repository_groups (id, canonical_common_dir, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(canonical_common_dir) DO UPDATE SET updated_at = excluded.updated_at",
            params![
                identity.repository_group_id,
                identity.canonical_common_dir,
                now
            ],
        )?;
        let group_id = transaction.query_row(
            "SELECT id FROM repository_groups WHERE canonical_common_dir = ?1",
            [&identity.canonical_common_dir],
            |row| row.get::<_, String>(0),
        )?;
        let changed = transaction.execute(
            "UPDATE remembered_repositories SET repository_group_id = ?2, worktree_role = ?3,
             updated_at = ?4 WHERE id = ?1",
            params![id, group_id, worktree_role_str(identity.worktree_role), now],
        )? > 0;
        transaction.commit()?;
        Ok(changed)
    }

    /// Reconciles a launcher scan in one transaction and writes only changed rows.
    pub fn reconcile(
        &mut self,
        updates: &[RepositoryCatalogReconciliation],
        now: i64,
    ) -> Result<usize, CatalogError> {
        let transaction = self.connection.transaction()?;
        let mut changed = 0;
        for update in updates {
            let availability = availability_str(update.availability);
            let Some(identity) = update.git_identity.as_ref() else {
                changed += transaction.execute(
                    "UPDATE remembered_repositories SET availability = ?2, updated_at = ?3
                     WHERE id = ?1 AND availability <> ?2",
                    params![update.repository_id, availability, now],
                )?;
                continue;
            };

            let repository_exists = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM remembered_repositories WHERE id = ?1)",
                [&update.repository_id],
                |row| row.get::<_, bool>(0),
            )?;
            if !repository_exists {
                continue;
            }

            let identity_matches = transaction.query_row(
                "SELECT EXISTS (
                    SELECT 1 FROM remembered_repositories repository
                    JOIN repository_groups repository_group
                      ON repository_group.id = repository.repository_group_id
                    WHERE repository.id = ?1
                      AND repository.availability = ?2
                      AND repository_group.canonical_common_dir = ?3
                      AND repository.worktree_role = ?4
                 )",
                params![
                    update.repository_id,
                    availability,
                    identity.canonical_common_dir,
                    worktree_role_str(identity.worktree_role),
                ],
                |row| row.get::<_, bool>(0),
            )?;
            if identity_matches {
                continue;
            }

            transaction.execute(
                "INSERT INTO repository_groups (id, canonical_common_dir, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT(canonical_common_dir) DO NOTHING",
                params![
                    identity.repository_group_id,
                    identity.canonical_common_dir,
                    now
                ],
            )?;
            let group_id = transaction.query_row(
                "SELECT id FROM repository_groups WHERE canonical_common_dir = ?1",
                [&identity.canonical_common_dir],
                |row| row.get::<_, String>(0),
            )?;
            changed += transaction.execute(
                "UPDATE remembered_repositories
                 SET availability = ?2, repository_group_id = ?3, worktree_role = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![
                    update.repository_id,
                    availability,
                    group_id,
                    worktree_role_str(identity.worktree_role),
                    now,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
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
        pinned, open_count, last_opened_at, created_at, updated_at,
        repository_group_id, worktree_role FROM remembered_repositories";

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
        repository_group_id: row.get(20)?,
        worktree_role: parse_worktree_role(row.get(21)?).map_err(sql_conversion_error)?,
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
string_enum!(worktree_role_str, parse_worktree_role, RepositoryWorktreeRole, {
    RepositoryWorktreeRole::Main => "main", RepositoryWorktreeRole::Linked => "linked", RepositoryWorktreeRole::Bare => "bare", RepositoryWorktreeRole::Unknown => "unknown"
});
string_enum!(relation_kind_str, parse_relation_kind, RepositoryRelationKind, {
    RepositoryRelationKind::Submodule => "submodule"
});

fn is_safe_relation_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

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
            git_identity: None,
            now,
        }
    }

    fn git_identity(
        group_id: &str,
        common_dir: &Path,
        worktree_role: RepositoryWorktreeRole,
    ) -> RepositoryGitIdentity {
        RepositoryGitIdentity {
            repository_group_id: group_id.into(),
            canonical_common_dir: common_dir.to_string_lossy().into_owned(),
            worktree_role,
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

    #[test]
    fn repositories_with_the_same_common_dir_reuse_the_group() {
        let dirs = [tempdir().unwrap(), tempdir().unwrap()];
        let common_dir = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();

        let mut main = input(dirs[0].path(), "main", "main", 1);
        main.git_identity = Some(git_identity(
            "first-group",
            common_dir.path(),
            RepositoryWorktreeRole::Main,
        ));
        let first = catalog.upsert(&main).unwrap();

        let mut linked = input(dirs[1].path(), "linked", "linked", 2);
        linked.git_identity = Some(git_identity(
            "ignored-candidate",
            common_dir.path(),
            RepositoryWorktreeRole::Linked,
        ));
        let second = catalog.upsert(&linked).unwrap();

        assert_eq!(first.repository_group_id.as_deref(), Some("first-group"));
        assert_eq!(second.repository_group_id, first.repository_group_id);
        assert_eq!(first.worktree_role, RepositoryWorktreeRole::Main);
        assert_eq!(second.worktree_role, RepositoryWorktreeRole::Linked);
    }

    #[test]
    fn lazy_identity_update_preserves_existing_catalog_metadata() {
        let directory = tempdir().unwrap();
        let common_dir = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        catalog
            .upsert(&input(directory.path(), "repo", "repo", 1))
            .unwrap();
        catalog.touch_opened("repo", 2).unwrap();
        catalog.set_pinned("repo", true, 3).unwrap();

        catalog
            .set_git_identity(
                "repo",
                &git_identity("group", common_dir.path(), RepositoryWorktreeRole::Linked),
                4,
            )
            .unwrap();
        let repository = catalog.get("repo").unwrap().unwrap();

        assert_eq!(repository.repository_group_id.as_deref(), Some("group"));
        assert_eq!(repository.worktree_role, RepositoryWorktreeRole::Linked);
        assert_eq!(repository.open_count, 1);
        assert_eq!(repository.last_opened_at, Some(2));
        assert!(repository.pinned);
    }

    #[test]
    fn unchanged_batch_reconciliation_does_not_write_or_touch_updated_at() {
        let directory = tempdir().unwrap();
        let common_dir = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        let identity = git_identity("group", common_dir.path(), RepositoryWorktreeRole::Main);
        let mut repository_input = input(directory.path(), "repo", "repo", 10);
        repository_input.git_identity = Some(identity.clone());
        let original = catalog.upsert(&repository_input).unwrap();

        let changed = catalog
            .reconcile(
                &[RepositoryCatalogReconciliation {
                    repository_id: "repo".into(),
                    availability: RepositoryAvailability::Available,
                    git_identity: Some(RepositoryGitIdentity {
                        repository_group_id: "unused-candidate".into(),
                        ..identity
                    }),
                }],
                99,
            )
            .unwrap();
        let reconciled = catalog.get("repo").unwrap().unwrap();

        assert_eq!(changed, 0);
        assert_eq!(reconciled.updated_at, original.updated_at);
        assert_eq!(reconciled.repository_group_id, original.repository_group_id);
    }

    #[test]
    fn missing_reconciliation_changes_availability_but_preserves_identity() {
        let directory = tempdir().unwrap();
        let common_dir = tempdir().unwrap();
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        let mut repository_input = input(directory.path(), "repo", "repo", 10);
        repository_input.git_identity = Some(git_identity(
            "group",
            common_dir.path(),
            RepositoryWorktreeRole::Linked,
        ));
        catalog.upsert(&repository_input).unwrap();

        let changed = catalog
            .reconcile(
                &[RepositoryCatalogReconciliation {
                    repository_id: "repo".into(),
                    availability: RepositoryAvailability::Missing,
                    git_identity: None,
                }],
                99,
            )
            .unwrap();
        let reconciled = catalog.get("repo").unwrap().unwrap();

        assert_eq!(changed, 1);
        assert_eq!(reconciled.availability, RepositoryAvailability::Missing);
        assert_eq!(reconciled.repository_group_id.as_deref(), Some("group"));
        assert_eq!(reconciled.worktree_role, RepositoryWorktreeRole::Linked);
        assert_eq!(reconciled.updated_at, 99);
    }

    #[test]
    fn v1_catalog_migrates_without_losing_repository_metadata() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("catalog.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
                 INSERT INTO schema_migrations VALUES (1, 0);
                 CREATE TABLE remembered_repositories (
                    id TEXT PRIMARY KEY NOT NULL, canonical_path TEXT NOT NULL UNIQUE,
                    display_name TEXT NOT NULL, provider TEXT NOT NULL, transport TEXT NOT NULL,
                    hosted_host TEXT, hosted_owner TEXT, hosted_name TEXT,
                    availability TEXT NOT NULL DEFAULT 'available',
                    git_health_state TEXT NOT NULL DEFAULT 'unknown', git_health_issue TEXT,
                    git_health_checked_at INTEGER, github_health_state TEXT NOT NULL DEFAULT 'unknown',
                    github_health_issue TEXT, github_health_checked_at INTEGER,
                    pinned INTEGER NOT NULL DEFAULT 0, open_count INTEGER NOT NULL DEFAULT 0,
                    last_opened_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
                 );
                 INSERT INTO remembered_repositories VALUES (
                    'legacy', '/legacy/repo', 'Legacy', 'local', 'local', NULL, NULL, NULL,
                    'missing', 'unknown', NULL, NULL, 'unknown', NULL, NULL, 1, 7, 42, 1, 43
                 );",
            )
            .unwrap();
        drop(connection);

        let catalog = RepositoryCatalog::open(&database).unwrap();
        let repository = catalog.get("legacy").unwrap().unwrap();

        assert_eq!(repository.open_count, 7);
        assert_eq!(repository.last_opened_at, Some(42));
        assert!(repository.pinned);
        assert_eq!(repository.repository_group_id, None);
        assert_eq!(repository.worktree_role, RepositoryWorktreeRole::Unknown);
    }

    #[test]
    fn submodule_relations_are_upserted_and_listed_by_relative_path() {
        let dirs = [tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap()];
        let common_dirs = [tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap()];
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        for index in 0..3 {
            let mut repository_input = input(
                dirs[index].path(),
                &format!("repo-{index}"),
                &format!("repo-{index}"),
                1,
            );
            repository_input.git_identity = Some(git_identity(
                &format!("group-{index}"),
                common_dirs[index].path(),
                RepositoryWorktreeRole::Main,
            ));
            catalog.upsert(&repository_input).unwrap();
        }

        catalog
            .link_submodule("group-0", "group-1", "vendor/library", 10)
            .unwrap();
        catalog
            .link_submodule("group-0", "group-2", "vendor/library", 11)
            .unwrap();

        assert_eq!(
            catalog.list_relations().unwrap(),
            vec![RepositoryGroupRelation {
                parent_repository_group_id: "group-0".into(),
                child_repository_group_id: "group-2".into(),
                kind: RepositoryRelationKind::Submodule,
                relative_path: "vendor/library".into(),
            }]
        );
    }

    #[test]
    fn submodule_relations_reject_self_links_cycles_and_unsafe_paths() {
        let dirs = [tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap()];
        let common_dirs = [tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap()];
        let mut catalog = RepositoryCatalog::open_in_memory().unwrap();
        for index in 0..3 {
            let mut repository_input = input(
                dirs[index].path(),
                &format!("repo-{index}"),
                &format!("repo-{index}"),
                1,
            );
            repository_input.git_identity = Some(git_identity(
                &format!("group-{index}"),
                common_dirs[index].path(),
                RepositoryWorktreeRole::Main,
            ));
            catalog.upsert(&repository_input).unwrap();
        }

        assert!(matches!(
            catalog.link_submodule("group-0", "group-0", "self", 2),
            Err(CatalogError::SelfRelation)
        ));
        assert!(matches!(
            catalog.link_submodule("group-0", "group-1", "../escape", 2),
            Err(CatalogError::InvalidRelationPath)
        ));
        catalog
            .link_submodule("group-0", "group-1", "one", 2)
            .unwrap();
        catalog
            .link_submodule("group-1", "group-2", "two", 3)
            .unwrap();
        assert!(matches!(
            catalog.link_submodule("group-2", "group-0", "cycle", 4),
            Err(CatalogError::RelationCycle)
        ));
    }

    #[test]
    fn v2_catalog_migrates_relations_without_recreating_group_tables() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("catalog.sqlite");
        let mut catalog = RepositoryCatalog::open(&database).unwrap();
        let repository_dir = tempdir().unwrap();
        let common_dir = tempdir().unwrap();
        let mut repository_input = input(repository_dir.path(), "repo", "repo", 1);
        repository_input.git_identity = Some(git_identity(
            "group",
            common_dir.path(),
            RepositoryWorktreeRole::Main,
        ));
        catalog.upsert(&repository_input).unwrap();
        drop(catalog);

        let connection = Connection::open(&database).unwrap();
        connection
            .execute("DELETE FROM schema_migrations WHERE version = 3", [])
            .unwrap();
        connection
            .execute("DROP TABLE repository_group_relations", [])
            .unwrap();
        drop(connection);

        let catalog = RepositoryCatalog::open(&database).unwrap();
        assert_eq!(
            catalog
                .get("repo")
                .unwrap()
                .unwrap()
                .repository_group_id
                .as_deref(),
            Some("group")
        );
        assert!(catalog.list_relations().unwrap().is_empty());
    }
}
