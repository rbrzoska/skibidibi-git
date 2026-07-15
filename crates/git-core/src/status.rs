use std::str;

use app_domain::{BranchStatus, RepositoryStatus, StatusCode, StatusEntry, StatusEntryKind};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StatusParseError {
    #[error("porcelain v2 record {record} is not valid UTF-8")]
    InvalidUtf8 { record: usize },
    #[error("malformed porcelain v2 record {record}: {reason}")]
    Malformed { record: usize, reason: String },
    #[error("unsupported porcelain v2 record type {record_type:?} at record {record}")]
    UnsupportedRecordType { record: usize, record_type: char },
}

pub fn parse_porcelain_v2_z(input: &[u8]) -> Result<RepositoryStatus, StatusParseError> {
    let records: Vec<&[u8]> = input.split(|byte| *byte == 0).collect();
    let mut status = RepositoryStatus::default();
    let mut index = 0;

    while index < records.len() {
        let raw_record = records[index];
        if raw_record.is_empty() {
            index += 1;
            continue;
        }

        let record_number = index + 1;
        let record = str::from_utf8(raw_record).map_err(|_| StatusParseError::InvalidUtf8 {
            record: record_number,
        })?;

        if record.starts_with("# ") {
            parse_header(record, record_number, &mut status.branch)?;
        } else if record.starts_with("1 ") {
            status.entries.push(parse_ordinary(record, record_number)?);
        } else if record.starts_with("2 ") {
            let original_path_record = records.get(index + 1).copied().ok_or_else(|| {
                malformed(
                    record_number,
                    "rename/copy record is missing the original path",
                )
            })?;
            if original_path_record.is_empty() {
                return Err(malformed(
                    record_number,
                    "rename/copy record has an empty original path",
                ));
            }
            let original_path = str::from_utf8(original_path_record)
                .map_err(|_| StatusParseError::InvalidUtf8 {
                    record: record_number + 1,
                })?
                .to_owned();
            status
                .entries
                .push(parse_renamed(record, original_path, record_number)?);
            index += 1;
        } else if record.starts_with("u ") {
            status.entries.push(parse_unmerged(record, record_number)?);
        } else if record.starts_with("? ") {
            status.entries.push(parse_simple(
                record,
                record_number,
                StatusEntryKind::Untracked,
                StatusCode::Untracked,
            )?);
        } else if record.starts_with("! ") {
            status.entries.push(parse_simple(
                record,
                record_number,
                StatusEntryKind::Ignored,
                StatusCode::Ignored,
            )?);
        } else {
            return Err(StatusParseError::UnsupportedRecordType {
                record: record_number,
                record_type: record.chars().next().expect("record is not empty"),
            });
        }

        index += 1;
    }

    Ok(status)
}

fn parse_header(
    record: &str,
    record_number: usize,
    branch: &mut BranchStatus,
) -> Result<(), StatusParseError> {
    let Some((key, value)) = record
        .strip_prefix("# ")
        .and_then(|line| line.split_once(' '))
    else {
        return Err(malformed(record_number, "invalid header"));
    };

    match key {
        "branch.oid" if value == "(initial)" => branch.unborn = true,
        "branch.oid" => branch.oid = Some(value.to_owned()),
        "branch.head" if value == "(detached)" => branch.detached = true,
        "branch.head" => branch.head = Some(value.to_owned()),
        "branch.upstream" => branch.upstream = Some(value.to_owned()),
        "branch.ab" => {
            let mut values = value.split_whitespace();
            branch.ahead = parse_count(values.next(), '+', record_number)?;
            branch.behind = parse_count(values.next(), '-', record_number)?;
            if values.next().is_some() {
                return Err(malformed(record_number, "invalid branch.ab header"));
            }
        }
        _ => {
            // Porcelain v2 headers are extensible; unknown headers must be ignored.
        }
    }

    Ok(())
}

fn parse_count(
    value: Option<&str>,
    prefix: char,
    record_number: usize,
) -> Result<u64, StatusParseError> {
    value
        .and_then(|value| value.strip_prefix(prefix))
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| malformed(record_number, "invalid branch.ab header"))
}

fn parse_ordinary(record: &str, record_number: usize) -> Result<StatusEntry, StatusParseError> {
    let fields = fields(record, 9, record_number)?;
    let (index_status, worktree_status) = parse_xy(fields[1], record_number)?;

    Ok(StatusEntry {
        kind: StatusEntryKind::Ordinary,
        path: fields[8].to_owned(),
        original_path: None,
        index_status,
        worktree_status,
        submodule: parse_submodule(fields[2]),
    })
}

fn parse_renamed(
    record: &str,
    original_path: String,
    record_number: usize,
) -> Result<StatusEntry, StatusParseError> {
    let fields = fields(record, 10, record_number)?;
    let (index_status, worktree_status) = parse_xy(fields[1], record_number)?;

    Ok(StatusEntry {
        kind: StatusEntryKind::RenamedOrCopied,
        path: fields[9].to_owned(),
        original_path: Some(original_path),
        index_status,
        worktree_status,
        submodule: parse_submodule(fields[2]),
    })
}

fn parse_unmerged(record: &str, record_number: usize) -> Result<StatusEntry, StatusParseError> {
    let fields = fields(record, 11, record_number)?;
    let (index_status, worktree_status) = parse_xy(fields[1], record_number)?;

    Ok(StatusEntry {
        kind: StatusEntryKind::Unmerged,
        path: fields[10].to_owned(),
        original_path: None,
        index_status,
        worktree_status,
        submodule: parse_submodule(fields[2]),
    })
}

fn parse_simple(
    record: &str,
    record_number: usize,
    kind: StatusEntryKind,
    code: StatusCode,
) -> Result<StatusEntry, StatusParseError> {
    let path = record
        .get(2..)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| malformed(record_number, "record is missing a path"))?;

    Ok(StatusEntry {
        kind,
        path: path.to_owned(),
        original_path: None,
        index_status: code,
        worktree_status: code,
        submodule: None,
    })
}

fn fields(
    record: &str,
    expected: usize,
    record_number: usize,
) -> Result<Vec<&str>, StatusParseError> {
    let fields: Vec<_> = record.splitn(expected, ' ').collect();
    if fields.len() != expected || fields.last().is_none_or(|path| path.is_empty()) {
        return Err(malformed(record_number, "record has missing fields"));
    }
    Ok(fields)
}

fn parse_xy(
    value: &str,
    record_number: usize,
) -> Result<(StatusCode, StatusCode), StatusParseError> {
    let mut characters = value.chars();
    let index = characters
        .next()
        .ok_or_else(|| malformed(record_number, "missing index status"))?;
    let worktree = characters
        .next()
        .ok_or_else(|| malformed(record_number, "missing worktree status"))?;
    if characters.next().is_some() {
        return Err(malformed(record_number, "invalid XY status"));
    }
    Ok((
        parse_status_code(index, record_number)?,
        parse_status_code(worktree, record_number)?,
    ))
}

fn parse_status_code(value: char, record_number: usize) -> Result<StatusCode, StatusParseError> {
    match value {
        '.' => Ok(StatusCode::Unmodified),
        'M' => Ok(StatusCode::Modified),
        'T' => Ok(StatusCode::TypeChanged),
        'A' => Ok(StatusCode::Added),
        'D' => Ok(StatusCode::Deleted),
        'R' => Ok(StatusCode::Renamed),
        'C' => Ok(StatusCode::Copied),
        'U' => Ok(StatusCode::Unmerged),
        _ => Err(malformed(record_number, "unknown XY status code")),
    }
}

fn parse_submodule(value: &str) -> Option<String> {
    (value != "N...").then(|| value.to_owned())
}

fn malformed(record: usize, reason: impl Into<String>) -> StatusParseError {
    StatusParseError::Malformed {
        record,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_ordinary_and_untracked_records() {
        let input = concat!(
            "# branch.oid abc123\0",
            "# branch.head feature/status\0",
            "# branch.upstream origin/feature/status\0",
            "# branch.ab +2 -3\0",
            "1 .M N... 100644 100644 100644 abc123 abc123 tracked file.txt\0",
            "? untracked\nfile.txt\0",
        );

        let status = parse_porcelain_v2_z(input.as_bytes()).expect("valid status");

        assert_eq!(status.branch.oid.as_deref(), Some("abc123"));
        assert_eq!(status.branch.head.as_deref(), Some("feature/status"));
        assert_eq!(
            status.branch.upstream.as_deref(),
            Some("origin/feature/status")
        );
        assert_eq!((status.branch.ahead, status.branch.behind), (2, 3));
        assert_eq!(status.entries.len(), 2);
        assert_eq!(status.entries[0].path, "tracked file.txt");
        assert_eq!(status.entries[0].index_status, StatusCode::Unmodified);
        assert_eq!(status.entries[0].worktree_status, StatusCode::Modified);
        assert_eq!(status.entries[1].path, "untracked\nfile.txt");
        assert_eq!(status.entries[1].kind, StatusEntryKind::Untracked);
    }

    #[test]
    fn parses_rename_with_nul_separated_original_path() {
        let input =
            b"2 R. N... 100644 100644 100644 abc123 def456 R100 new name.txt\0old name.txt\0";

        let status = parse_porcelain_v2_z(input).expect("valid rename");

        assert_eq!(status.entries[0].path, "new name.txt");
        assert_eq!(
            status.entries[0].original_path.as_deref(),
            Some("old name.txt")
        );
        assert_eq!(status.entries[0].index_status, StatusCode::Renamed);
    }

    #[test]
    fn parses_unborn_and_detached_branch_markers() {
        let status =
            parse_porcelain_v2_z(b"# branch.oid (initial)\0# branch.head (detached)\0# stash 4\0")
                .expect("valid headers");

        assert!(status.branch.unborn);
        assert!(status.branch.detached);
        assert_eq!(status.branch.oid, None);
        assert_eq!(status.branch.head, None);
    }

    #[test]
    fn rejects_a_rename_without_original_path() {
        let error = parse_porcelain_v2_z(
            b"2 R. N... 100644 100644 100644 abc123 def456 R100 new-name.txt\0",
        )
        .expect_err("rename must include original path");

        assert!(matches!(
            error,
            StatusParseError::Malformed { record: 1, .. }
        ));
    }

    #[test]
    fn rejects_record_markers_without_the_required_space() {
        for input in [
            b"1.M N... fields\0".as_slice(),
            b"2R. N... fields\0old\0".as_slice(),
            b"uUU N... fields\0".as_slice(),
            b"?path\0".as_slice(),
            b"!path\0".as_slice(),
        ] {
            assert!(matches!(
                parse_porcelain_v2_z(input),
                Err(StatusParseError::UnsupportedRecordType { record: 1, .. })
            ));
        }
    }
}
