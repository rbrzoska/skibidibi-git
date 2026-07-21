use std::{collections::BTreeMap, str};

use thiserror::Error;

/// An index entry with gitlink mode (`160000`). The same path can appear at more than one stage
/// while a submodule merge is conflicted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlinkIndexEntry {
    pub path: String,
    pub oid: String,
    pub stage: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitmodulesEntry {
    pub name: String,
    pub path: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SubmoduleParseError {
    #[error("submodule record {record} is not valid UTF-8")]
    InvalidUtf8 { record: usize },
    #[error("malformed submodule index record {record}: {reason}")]
    MalformedIndex { record: usize, reason: String },
    #[error("malformed .gitmodules record {record}: {reason}")]
    MalformedGitmodules { record: usize, reason: String },
}

/// Parses NUL-delimited `git ls-files --stage -z` output and retains only immediate gitlinks.
pub fn parse_gitlink_index_entries(
    input: &[u8],
) -> Result<Vec<GitlinkIndexEntry>, SubmoduleParseError> {
    let mut entries = Vec::new();
    for (index, raw) in input.split(|byte| *byte == 0).enumerate() {
        if raw.is_empty() {
            continue;
        }
        let record = index + 1;
        let value = str::from_utf8(raw).map_err(|_| SubmoduleParseError::InvalidUtf8 { record })?;
        let Some((header, path)) = value.split_once('\t') else {
            return Err(index_error(record, "missing tab before path"));
        };
        if path.is_empty() {
            return Err(index_error(record, "empty path"));
        }
        let mut fields = header.split_whitespace();
        let mode = fields
            .next()
            .ok_or_else(|| index_error(record, "missing mode"))?;
        let oid = fields
            .next()
            .ok_or_else(|| index_error(record, "missing object id"))?;
        let stage = fields
            .next()
            .ok_or_else(|| index_error(record, "missing stage"))?;
        if fields.next().is_some() {
            return Err(index_error(record, "too many header fields"));
        }
        if mode != "160000" {
            continue;
        }
        if !valid_oid(oid) {
            return Err(index_error(record, "invalid object id"));
        }
        let stage = stage
            .parse::<u8>()
            .ok()
            .filter(|stage| *stage <= 3)
            .ok_or_else(|| index_error(record, "invalid stage"))?;
        entries.push(GitlinkIndexEntry {
            path: path.to_owned(),
            oid: oid.to_owned(),
            stage,
        });
    }
    Ok(entries)
}

/// Parses the fixed `git config --null --file .gitmodules --list` wire format. Unknown
/// configuration is ignored so a malicious `.gitmodules` cannot change command behavior.
pub fn parse_gitmodules_config(input: &[u8]) -> Result<Vec<GitmodulesEntry>, SubmoduleParseError> {
    let mut entries = BTreeMap::<String, GitmodulesEntry>::new();
    for (index, raw) in input.split(|byte| *byte == 0).enumerate() {
        if raw.is_empty() {
            continue;
        }
        let record = index + 1;
        let value = str::from_utf8(raw).map_err(|_| SubmoduleParseError::InvalidUtf8 { record })?;
        let Some((key, value)) = value.split_once('\n') else {
            return Err(config_error(record, "missing key/value separator"));
        };
        let Some(key) = key.strip_prefix("submodule.") else {
            continue;
        };
        let Some((name, field)) = key.rsplit_once('.') else {
            continue;
        };
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(config_error(record, "unsafe submodule name"));
        }
        let entry = entries
            .entry(name.to_owned())
            .or_insert_with(|| GitmodulesEntry {
                name: name.to_owned(),
                ..GitmodulesEntry::default()
            });
        match field {
            "path" => entry.path = Some(value.to_owned()),
            "url" => entry.url = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(entries.into_values().collect())
}

/// Removes remote credentials and URL query/fragment material before it can reach the UI. This
/// follows the same conservative format accepted by repository remote discovery.
pub fn sanitize_submodule_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let without_parameters = value.split_once(['?', '#']).map_or(value, |(safe, _)| safe);
    let Some((scheme, remainder)) = without_parameters.split_once("://") else {
        // SCP-style Git URLs are also allowed to carry `user@host:path`. The username can be a
        // token in hand-authored `.gitmodules`, so expose only the host portion. Do not treat a
        // Windows drive prefix as a remote authority.
        if let Some((authority, path)) = without_parameters.split_once(':')
            && !(authority.len() == 1 && authority.as_bytes()[0].is_ascii_alphabetic())
            && let Some((_, host)) = authority.rsplit_once('@')
            && !host.is_empty()
            && !path.is_empty()
        {
            return Some(format!("{host}:{path}"));
        }
        return Some(without_parameters.to_owned());
    };
    let (authority, path) = remainder
        .split_once('/')
        .map_or((remainder, ""), |(authority, path)| (authority, path));
    let safe_authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if safe_authority.is_empty() {
        return None;
    }
    Some(if path.is_empty() {
        format!("{scheme}://{safe_authority}")
    } else {
        format!("{scheme}://{safe_authority}/{path}")
    })
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn index_error(record: usize, reason: impl Into<String>) -> SubmoduleParseError {
    SubmoduleParseError::MalformedIndex {
        record,
        reason: reason.into(),
    }
}

fn config_error(record: usize, reason: impl Into<String>) -> SubmoduleParseError {
    SubmoduleParseError::MalformedGitmodules {
        record,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stage_zero_and_conflicted_gitlinks() {
        let output = concat!(
            "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0\tfile\0",
            "160000 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 0\tmodules/client\0",
            "160000 cccccccccccccccccccccccccccccccccccccccc 2\tmodules/conflict\0",
            "160000 dddddddddddddddddddddddddddddddddddddddd 3\tmodules/conflict\0"
        );
        let entries = parse_gitlink_index_entries(output.as_bytes()).expect("parse index");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "modules/client");
        assert_eq!(entries[1].stage, 2);
    }

    #[test]
    fn parses_gitmodules_config_and_ignores_other_keys() {
        let output = concat!(
            "submodule.client.path\nmodules/client\0",
            "submodule.client.url\nhttps://token@example.test/acme/client.git?secret=1\0",
            "core.repositoryformatversion\n0\0"
        );
        let entries = parse_gitmodules_config(output.as_bytes()).expect("parse config");
        assert_eq!(
            entries,
            vec![GitmodulesEntry {
                name: "client".into(),
                path: Some("modules/client".into()),
                url: Some("https://token@example.test/acme/client.git?secret=1".into()),
            }]
        );
        assert_eq!(
            sanitize_submodule_url(entries[0].url.as_deref().unwrap()),
            Some("https://example.test/acme/client.git".into())
        );
    }

    #[test]
    fn rejects_malformed_or_non_utf8_records() {
        assert!(matches!(
            parse_gitlink_index_entries(b"160000 abc 0\tmodule\0"),
            Err(SubmoduleParseError::MalformedIndex { .. })
        ));
        assert!(matches!(
            parse_gitmodules_config(b"submodule.x.path\xff\0"),
            Err(SubmoduleParseError::InvalidUtf8 { .. })
        ));
    }

    #[test]
    fn strips_scp_style_user_info() {
        assert_eq!(
            sanitize_submodule_url("token@git@github.example:acme/client.git"),
            Some("github.example:acme/client.git".into())
        );
        assert_eq!(
            sanitize_submodule_url("C:/repositories/client"),
            Some("C:/repositories/client".into())
        );
    }
}
