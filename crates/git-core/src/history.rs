use app_domain::{
    ChangedFileStatus, ChangedFileSummary, CommitAuthor, CommitDetails, CommitListItem,
};
use thiserror::Error;

const LIST_FIELD_COUNT: usize = 7;
const DETAIL_FIELD_COUNT: usize = 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HistoryParseError {
    #[error("history output ended in the middle of a record")]
    TruncatedRecord,
    #[error("history field {field} is not valid UTF-8")]
    InvalidUtf8 { field: &'static str },
    #[error("history contains an invalid object id: {0}")]
    InvalidObjectId(String),
    #[error("invalid numstat record: {0}")]
    InvalidNumstat(String),
    #[error("invalid name-status record: {0}")]
    InvalidNameStatus(String),
    #[error("name-status and numstat outputs describe different files")]
    FileSummaryMismatch,
}

pub fn parse_commit_list(input: &[u8]) -> Result<Vec<CommitListItem>, HistoryParseError> {
    parse_list_records(input)
}

fn parse_list_records(input: &[u8]) -> Result<Vec<CommitListItem>, HistoryParseError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields = nul_fields(input);
    if fields.last() == Some(&b"".as_slice()) {
        fields.pop();
    }
    if fields.len() % LIST_FIELD_COUNT != 0 {
        return Err(HistoryParseError::TruncatedRecord);
    }
    fields
        .chunks_exact(LIST_FIELD_COUNT)
        .map(parse_list_record)
        .collect()
}

fn parse_list_record(fields: &[&[u8]]) -> Result<CommitListItem, HistoryParseError> {
    let oid = text(fields[0], "oid")?.to_owned();
    validate_oid(&oid)?;
    Ok(CommitListItem {
        oid,
        parents: parse_oids(fields[1])?,
        author: CommitAuthor {
            name: text(fields[2], "author name")?.to_owned(),
            email: text(fields[3], "author email")?.to_owned(),
            authored_at: text(fields[4], "author time")?.to_owned(),
        },
        summary: text(fields[5], "summary")?.to_owned(),
        refs: parse_refs(text(fields[6], "refs")?),
    })
}

pub fn parse_commit_details_header(input: &[u8]) -> Result<CommitDetails, HistoryParseError> {
    let mut fields = nul_fields(input);
    if fields.last() == Some(&b"".as_slice()) {
        fields.pop();
    }
    if fields.len() != DETAIL_FIELD_COUNT {
        return Err(HistoryParseError::TruncatedRecord);
    }
    let oid = text(fields[0], "oid")?.to_owned();
    validate_oid(&oid)?;
    Ok(CommitDetails {
        oid,
        parents: parse_oids(fields[1])?,
        author: CommitAuthor {
            name: text(fields[2], "author name")?.to_owned(),
            email: text(fields[3], "author email")?.to_owned(),
            authored_at: text(fields[4], "author time")?.to_owned(),
        },
        summary: text(fields[5], "summary")?.to_owned(),
        refs: parse_refs(text(fields[6], "refs")?),
        full_message: text(fields[7], "full message")?.to_owned(),
        files: Vec::new(),
    })
}

pub fn parse_changed_files(
    name_status: &[u8],
    numstat: &[u8],
) -> Result<Vec<ChangedFileSummary>, HistoryParseError> {
    let names = parse_name_status(name_status)?;
    let numbers = parse_numstat(numstat)?;
    if names.len() != numbers.len() {
        return Err(HistoryParseError::FileSummaryMismatch);
    }
    names
        .into_iter()
        .zip(numbers)
        .map(
            |(
                (status, old_path, path),
                (number_old, number_path, additions, deletions, binary),
            )| {
                if path != number_path || old_path != number_old {
                    return Err(HistoryParseError::FileSummaryMismatch);
                }
                Ok(ChangedFileSummary {
                    status,
                    path,
                    old_path,
                    additions,
                    deletions,
                    binary,
                })
            },
        )
        .collect()
}

type NamedFile = (ChangedFileStatus, Option<String>, String);
type NumberedFile = (Option<String>, String, Option<u64>, Option<u64>, bool);

fn parse_name_status(input: &[u8]) -> Result<Vec<NamedFile>, HistoryParseError> {
    let fields = terminated_nul_fields(input)?;
    let mut result = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status_text = text(fields[index], "file status")?;
        index += 1;
        let status_code = status_text
            .chars()
            .next()
            .ok_or_else(|| HistoryParseError::InvalidNameStatus(status_text.to_owned()))?;
        let status = match status_code {
            'A' => ChangedFileStatus::Added,
            'C' => ChangedFileStatus::Copied,
            'D' => ChangedFileStatus::Deleted,
            'M' => ChangedFileStatus::Modified,
            'R' => ChangedFileStatus::Renamed,
            'T' => ChangedFileStatus::TypeChanged,
            'U' => ChangedFileStatus::Unmerged,
            _ => ChangedFileStatus::Unknown,
        };
        let path_count = if matches!(status_code, 'R' | 'C') {
            2
        } else {
            1
        };
        if index + path_count > fields.len() {
            return Err(HistoryParseError::TruncatedRecord);
        }
        // Paths are display-only in this DTO. Keep mutation APIs byte-oriented when they are
        // introduced; this intentionally does not retain raw non-UTF-8 filename bytes.
        let first = display_path(fields[index]);
        index += 1;
        let (old_path, path) = if path_count == 2 {
            let second = display_path(fields[index]);
            index += 1;
            (Some(first), second)
        } else {
            (None, first)
        };
        result.push((status, old_path, path));
    }
    Ok(result)
}

fn parse_numstat(input: &[u8]) -> Result<Vec<NumberedFile>, HistoryParseError> {
    let fields = terminated_nul_fields(input)?;
    let mut result = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let header = fields[index];
        index += 1;
        let first_tab = header
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                HistoryParseError::InvalidNumstat(String::from_utf8_lossy(header).into_owned())
            })?;
        let second_relative = header[first_tab + 1..]
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                HistoryParseError::InvalidNumstat(String::from_utf8_lossy(header).into_owned())
            })?;
        let second_tab = first_tab + 1 + second_relative;
        let added = &header[..first_tab];
        let deleted = &header[first_tab + 1..second_tab];
        let embedded_path = &header[second_tab + 1..];
        let binary = added == b"-" || deleted == b"-";
        let additions = if binary {
            None
        } else {
            Some(parse_count(added)?)
        };
        let deletions = if binary {
            None
        } else {
            Some(parse_count(deleted)?)
        };
        let (old_path, path) = if embedded_path.is_empty() {
            if index + 2 > fields.len() {
                return Err(HistoryParseError::TruncatedRecord);
            }
            let old = display_path(fields[index]);
            let new = display_path(fields[index + 1]);
            index += 2;
            (Some(old), new)
        } else {
            (None, display_path(embedded_path))
        };
        result.push((old_path, path, additions, deletions, binary));
    }
    Ok(result)
}

fn parse_count(value: &[u8]) -> Result<u64, HistoryParseError> {
    text(value, "line count")?
        .parse()
        .map_err(|_| HistoryParseError::InvalidNumstat(String::from_utf8_lossy(value).into_owned()))
}

fn nul_fields(input: &[u8]) -> Vec<&[u8]> {
    input.split(|byte| *byte == 0).collect()
}

fn terminated_nul_fields(input: &[u8]) -> Result<Vec<&[u8]>, HistoryParseError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if !input.ends_with(&[0]) {
        return Err(HistoryParseError::TruncatedRecord);
    }
    let mut fields = nul_fields(input);
    fields.pop();
    Ok(fields)
}

fn text<'a>(value: &'a [u8], field: &'static str) -> Result<&'a str, HistoryParseError> {
    std::str::from_utf8(value).map_err(|_| HistoryParseError::InvalidUtf8 { field })
}

fn display_path(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

fn validate_oid(value: &str) -> Result<(), HistoryParseError> {
    if (value.len() == 40 || value.len() == 64)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(HistoryParseError::InvalidObjectId(value.to_owned()))
    }
}

fn parse_oids(value: &[u8]) -> Result<Vec<String>, HistoryParseError> {
    let text = text(value, "parents")?;
    text.split_whitespace()
        .map(|oid| {
            validate_oid(oid)?;
            Ok(oid.to_owned())
        })
        .collect()
}

fn parse_refs(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const OID: &str = "0123456789012345678901234567890123456789";
    const PARENT: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";

    #[test]
    fn parses_nul_delimited_history_with_newlines_and_empty_refs() {
        let input = format!(
            "{OID}\0{PARENT}\0A\nName\0a@b.test\02026-07-15T10:00:00+02:00\0line one\nline two\0\0"
        );
        let parsed = parse_commit_list(input.as_bytes()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].summary, "line one\nline two");
        assert!(parsed[0].refs.is_empty());
    }

    #[test]
    fn combines_rename_and_binary_file_summaries() {
        let names = b"R100\0old name\0new name\0M\0image.png\0";
        let numbers = b"4\t2\t\0old name\0new name\0-\t-\timage.png\0";
        let files = parse_changed_files(names, numbers).unwrap();
        assert_eq!(files[0].old_path.as_deref(), Some("old name"));
        assert_eq!(files[0].additions, Some(4));
        assert!(files[1].binary);
    }

    #[test]
    fn rejects_truncated_and_mismatched_outputs() {
        assert_eq!(
            parse_commit_list(OID.as_bytes()),
            Err(HistoryParseError::TruncatedRecord)
        );
        assert_eq!(
            parse_changed_files(b"M\0a\0", b"1\t1\tb\0"),
            Err(HistoryParseError::FileSummaryMismatch)
        );
    }

    #[test]
    fn decodes_non_utf8_display_paths_lossily_without_losing_commit_details() {
        let files = parse_changed_files(b"M\0bad-\xff-name\0", b"1\t2\tbad-\xff-name\0").unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "bad-\u{fffd}-name");
        assert_eq!(files[0].additions, Some(1));
    }
}
