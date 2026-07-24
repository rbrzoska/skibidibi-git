use std::{ffi::OsString, fmt::Write as _, path::Path, time::Duration};

use app_domain::{FileBlame, FileBlameLine, FileBlameState, FileHistoryPage};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitOutputStream, GitRunError, GitRunner,
    HistoryParseError, parse_commit_list,
};
use thiserror::Error;

use crate::RepositoryRuntime;

pub const FILE_HISTORY_PAGE_SIZE: usize = 100;
const MAX_FILE_HISTORY_OFFSET: usize = 10_000_000;
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const HISTORY_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const BLOB_SIZE_OUTPUT_LIMIT: usize = 128;
const MAX_BLAME_FILE_BYTES: usize = 2 * 1024 * 1024;
const BLAME_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
// Until the renderer has a virtualized blame table, cap this at a size that remains responsive
// on the smallest supported desktop viewport.
const MAX_BLAME_LINES: usize = 5_000;
// Git accepts arbitrary pathspec lengths, but the native IPC boundary must not turn a renderer
// string into an unbounded argv allocation or an opaque cursor twice its size.
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_CURSOR_BYTES: usize = 3 + 1 + 64 + 1 + MAX_PATH_BYTES * 2 + 1 + 20;
const STDERR_LIMIT: usize = 256 * 1024;
const LITERAL_PATHSPEC_PREFIX: &str = ":(literal)";
const COMMIT_FORMAT: &str = "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00%D";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileHistoryQuery {
    HistoryPage {
        start_oid: String,
        path: String,
        offset: usize,
    },
    BlobSize {
        oid: String,
        path: String,
    },
    BlobContents {
        oid: String,
        path: String,
    },
    Blame {
        oid: String,
        path: String,
    },
}

impl FileHistoryQuery {
    fn arguments(&self) -> Vec<OsString> {
        let values = match self {
            Self::HistoryPage {
                start_oid,
                path,
                offset,
            } => vec![
                "log".to_owned(),
                "-z".to_owned(),
                "--topo-order".to_owned(),
                "--follow".to_owned(),
                format!("--max-count={}", FILE_HISTORY_PAGE_SIZE + 1),
                format!("--skip={offset}"),
                COMMIT_FORMAT.to_owned(),
                start_oid.clone(),
                "--".to_owned(),
                format!("{LITERAL_PATHSPEC_PREFIX}{path}"),
            ],
            Self::BlobSize { oid, path } => vec![
                "cat-file".to_owned(),
                "-s".to_owned(),
                object_path(oid, path),
            ],
            Self::BlobContents { oid, path } => vec![
                "cat-file".to_owned(),
                "blob".to_owned(),
                object_path(oid, path),
            ],
            Self::Blame { oid, path } => vec![
                "--literal-pathspecs".to_owned(),
                "blame".to_owned(),
                "--line-porcelain".to_owned(),
                "--no-progress".to_owned(),
                oid.clone(),
                "--".to_owned(),
                path.clone(),
            ],
        };
        values.into_iter().map(OsString::from).collect()
    }

    fn stdout_limit(&self) -> usize {
        match self {
            Self::HistoryPage { .. } => HISTORY_OUTPUT_LIMIT,
            Self::BlobSize { .. } => BLOB_SIZE_OUTPUT_LIMIT,
            Self::BlobContents { .. } => MAX_BLAME_FILE_BYTES,
            Self::Blame { .. } => BLAME_OUTPUT_LIMIT,
        }
    }
}

pub trait FileHistoryGitExecutor: Send + Sync {
    fn execute_file_history(
        &self,
        repository: &Path,
        query: FileHistoryQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl FileHistoryGitExecutor for GitRunner {
    fn execute_file_history(
        &self,
        repository: &Path,
        query: FileHistoryQuery,
    ) -> Result<GitOutput, GitRunError> {
        let stdout_limit = query.stdout_limit();
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(stdout_limit, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

impl<E: FileHistoryGitExecutor> RepositoryRuntime<E> {
    pub fn file_history(
        &self,
        repository: &Path,
        start_oid: &str,
        path: &str,
        cursor: Option<&str>,
    ) -> Result<FileHistoryPage, FileHistoryRuntimeError> {
        file_history(&self.executor, repository, start_oid, path, cursor)
    }

    pub fn file_blame(
        &self,
        repository: &Path,
        oid: &str,
        path: &str,
    ) -> Result<FileBlame, FileHistoryRuntimeError> {
        file_blame(&self.executor, repository, oid, path)
    }
}

#[derive(Debug, Error)]
pub enum FileHistoryRuntimeError {
    #[error("file history requires an exact 40- or 64-character commit object id")]
    InvalidObjectId,
    #[error("file path must be non-empty and losslessly representable as UTF-8")]
    InvalidPath,
    #[error("invalid file-history cursor")]
    InvalidCursor,
    #[error("Git returned an invalid blob size")]
    InvalidBlobSize,
    #[error("Git returned malformed line-porcelain blame output")]
    InvalidBlame,
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    History(#[from] HistoryParseError),
}

pub fn file_history<E: FileHistoryGitExecutor>(
    executor: &E,
    repository: &Path,
    start_oid: &str,
    path: &str,
    cursor: Option<&str>,
) -> Result<FileHistoryPage, FileHistoryRuntimeError> {
    validate_inputs(start_oid, path)?;
    let offset = match cursor {
        Some(cursor) => parse_cursor(cursor, start_oid, path)?,
        None => 0,
    };
    let output = executor.execute_file_history(
        repository,
        FileHistoryQuery::HistoryPage {
            start_oid: start_oid.to_owned(),
            path: path.to_owned(),
            offset,
        },
    )?;
    let mut commits = parse_commit_list(&output.stdout)?;
    let has_more = commits.len() > FILE_HISTORY_PAGE_SIZE;
    commits.truncate(FILE_HISTORY_PAGE_SIZE);
    let next_cursor = if has_more {
        let next_offset = offset
            .checked_add(commits.len())
            .filter(|next_offset| *next_offset <= MAX_FILE_HISTORY_OFFSET)
            .ok_or(FileHistoryRuntimeError::InvalidCursor)?;
        Some(encode_cursor(start_oid, path, next_offset))
    } else {
        None
    };
    Ok(FileHistoryPage {
        start_oid: start_oid.to_owned(),
        path: path.to_owned(),
        commits,
        next_cursor,
    })
}

pub fn file_blame<E: FileHistoryGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
    path: &str,
) -> Result<FileBlame, FileHistoryRuntimeError> {
    validate_inputs(oid, path)?;
    let size = blob_size(executor, repository, oid, path)?;
    if size > MAX_BLAME_FILE_BYTES {
        return Ok(unavailable_blame(oid, path, FileBlameState::Oversized));
    }
    let blob = executor.execute_file_history(
        repository,
        FileHistoryQuery::BlobContents {
            oid: oid.to_owned(),
            path: path.to_owned(),
        },
    )?;
    if blob.stdout.contains(&0) || std::str::from_utf8(&blob.stdout).is_err() {
        return Ok(unavailable_blame(oid, path, FileBlameState::Binary));
    }

    let output = match executor.execute_file_history(
        repository,
        FileHistoryQuery::Blame {
            oid: oid.to_owned(),
            path: path.to_owned(),
        },
    ) {
        Ok(output) => output,
        Err(GitRunError::OutputLimitExceeded {
            stream: GitOutputStream::Stdout,
            ..
        }) => return Ok(unavailable_blame(oid, path, FileBlameState::Oversized)),
        Err(error) => return Err(error.into()),
    };
    let lines = parse_line_porcelain(&output.stdout)?;
    if lines.len() > MAX_BLAME_LINES {
        return Ok(unavailable_blame(oid, path, FileBlameState::Oversized));
    }
    Ok(FileBlame {
        oid: oid.to_owned(),
        path: path.to_owned(),
        state: FileBlameState::Available,
        lines,
        truncated: false,
    })
}

fn blob_size<E: FileHistoryGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
    path: &str,
) -> Result<usize, FileHistoryRuntimeError> {
    let output = executor.execute_file_history(
        repository,
        FileHistoryQuery::BlobSize {
            oid: oid.to_owned(),
            path: path.to_owned(),
        },
    )?;
    std::str::from_utf8(&output.stdout)
        .ok()
        .map(str::trim)
        .filter(|size| !size.is_empty() && size.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|size| size.parse::<usize>().ok())
        .ok_or(FileHistoryRuntimeError::InvalidBlobSize)
}

fn unavailable_blame(oid: &str, path: &str, state: FileBlameState) -> FileBlame {
    FileBlame {
        oid: oid.to_owned(),
        path: path.to_owned(),
        state,
        lines: Vec::new(),
        truncated: false,
    }
}

#[derive(Default)]
struct BlameMetadata {
    oid: String,
    original_line_number: u64,
    final_line_number: u64,
    author_name: Option<String>,
    author_email: Option<String>,
    author_time: Option<i64>,
    author_timezone: Option<String>,
    summary: Option<String>,
}

fn parse_line_porcelain(output: &[u8]) -> Result<Vec<FileBlameLine>, FileHistoryRuntimeError> {
    let text = std::str::from_utf8(output).map_err(|_| FileHistoryRuntimeError::InvalidBlame)?;
    let mut result = Vec::new();
    let mut metadata: Option<BlameMetadata> = None;
    for line in text.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if let Some(content) = line.strip_prefix('\t') {
            let metadata = metadata
                .take()
                .ok_or(FileHistoryRuntimeError::InvalidBlame)?;
            result.push(FileBlameLine {
                line_number: metadata.final_line_number,
                oid: metadata.oid,
                original_line_number: metadata.original_line_number,
                final_line_number: metadata.final_line_number,
                author_name: metadata
                    .author_name
                    .ok_or(FileHistoryRuntimeError::InvalidBlame)?,
                author_email: metadata
                    .author_email
                    .ok_or(FileHistoryRuntimeError::InvalidBlame)?,
                authored_at: format_timestamp(
                    metadata
                        .author_time
                        .ok_or(FileHistoryRuntimeError::InvalidBlame)?,
                    metadata
                        .author_timezone
                        .as_deref()
                        .ok_or(FileHistoryRuntimeError::InvalidBlame)?,
                )?,
                summary: metadata
                    .summary
                    .ok_or(FileHistoryRuntimeError::InvalidBlame)?,
                content: content.to_owned(),
            });
            continue;
        }
        if let Some(header) = parse_blame_header(line)? {
            if metadata.is_some() {
                return Err(FileHistoryRuntimeError::InvalidBlame);
            }
            metadata = Some(header);
            continue;
        }
        let Some(metadata) = metadata.as_mut() else {
            if line.is_empty() {
                continue;
            }
            return Err(FileHistoryRuntimeError::InvalidBlame);
        };
        if line == "boundary" {
            continue;
        }
        let (key, value) = line
            .split_once(' ')
            .ok_or(FileHistoryRuntimeError::InvalidBlame)?;
        match key {
            "author" => metadata.author_name = Some(value.to_owned()),
            "author-mail" => {
                metadata.author_email = Some(
                    value
                        .strip_prefix('<')
                        .and_then(|value| value.strip_suffix('>'))
                        .ok_or(FileHistoryRuntimeError::InvalidBlame)?
                        .to_owned(),
                )
            }
            "author-time" => {
                metadata.author_time = value.parse().ok();
                if metadata.author_time.is_none() {
                    return Err(FileHistoryRuntimeError::InvalidBlame);
                }
            }
            "author-tz" => metadata.author_timezone = Some(value.to_owned()),
            "summary" => metadata.summary = Some(value.to_owned()),
            _ => {}
        }
    }
    if metadata.is_some() {
        return Err(FileHistoryRuntimeError::InvalidBlame);
    }
    Ok(result)
}

fn parse_blame_header(line: &str) -> Result<Option<BlameMetadata>, FileHistoryRuntimeError> {
    let mut values = line.split_ascii_whitespace();
    let Some(oid) = values.next() else {
        return Ok(None);
    };
    let Some(original_line_number) = values.next() else {
        return Ok(None);
    };
    let Some(final_line_number) = values.next() else {
        return Ok(None);
    };
    // The first line of a porcelain group includes a fourth group-size field. Subsequent lines
    // from the same commit have only the three identity/line-number fields.
    let group_size = values.next();
    if values.next().is_some() || !valid_oid(oid) {
        return Ok(None);
    }
    if group_size.is_some_and(|size| size.parse::<u64>().map_or(true, |size| size == 0)) {
        return Ok(None);
    }
    let original_line_number = original_line_number
        .parse()
        .map_err(|_| FileHistoryRuntimeError::InvalidBlame)?;
    let final_line_number = final_line_number
        .parse()
        .map_err(|_| FileHistoryRuntimeError::InvalidBlame)?;
    Ok(Some(BlameMetadata {
        oid: oid.to_owned(),
        original_line_number,
        final_line_number,
        ..BlameMetadata::default()
    }))
}

fn format_timestamp(timestamp: i64, timezone: &str) -> Result<String, FileHistoryRuntimeError> {
    let (sign, hours, minutes) = match timezone.as_bytes() {
        [b'+', h1, h2, m1, m2] | [b'-', h1, h2, m1, m2]
            if h1.is_ascii_digit()
                && h2.is_ascii_digit()
                && m1.is_ascii_digit()
                && m2.is_ascii_digit() =>
        {
            let hours = u32::from(*h1 - b'0') * 10 + u32::from(*h2 - b'0');
            let minutes = u32::from(*m1 - b'0') * 10 + u32::from(*m2 - b'0');
            if hours > 23 || minutes > 59 {
                return Err(FileHistoryRuntimeError::InvalidBlame);
            }
            (timezone.as_bytes()[0], hours, minutes)
        }
        _ => return Err(FileHistoryRuntimeError::InvalidBlame),
    };
    let offset_seconds = i64::from(hours * 60 * 60 + minutes * 60);
    let local = if sign == b'+' {
        timestamp.checked_add(offset_seconds)
    } else {
        timestamp.checked_sub(offset_seconds)
    }
    .ok_or(FileHistoryRuntimeError::InvalidBlame)?;
    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{}{:02}:{:02}",
        if sign == b'+' { '+' } else { '-' },
        hours,
        minutes
    ))
}

// Howard Hinnant's civil date conversion, adapted to a signed Unix-day count.
fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

fn validate_inputs(oid: &str, path: &str) -> Result<(), FileHistoryRuntimeError> {
    if !valid_oid(oid) {
        return Err(FileHistoryRuntimeError::InvalidObjectId);
    }
    if !valid_path(path) {
        return Err(FileHistoryRuntimeError::InvalidPath);
    }
    Ok(())
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_BYTES
        && !path.contains(['\0', '\u{fffd}'])
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|component| !matches!(component, std::path::Component::ParentDir))
}

fn object_path(oid: &str, path: &str) -> String {
    format!("{oid}:{path}")
}

fn encode_cursor(start_oid: &str, path: &str, offset: usize) -> String {
    let mut encoded_path = String::with_capacity(path.len() * 2);
    for byte in path.bytes() {
        let _ = write!(encoded_path, "{byte:02x}");
    }
    format!("v1:{start_oid}:{encoded_path}:{offset}")
}

fn parse_cursor(
    cursor: &str,
    start_oid: &str,
    path: &str,
) -> Result<usize, FileHistoryRuntimeError> {
    if cursor.len() > MAX_CURSOR_BYTES {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    }
    let mut fields = cursor.split(':');
    let (Some(version), Some(cursor_oid), Some(path_hex), Some(offset), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    };
    if version != "v1" || cursor_oid != start_oid || !valid_oid(cursor_oid) {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    }
    let decoded_path = decode_path(path_hex)?;
    if decoded_path != path {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    }
    let offset = offset
        .parse::<usize>()
        .map_err(|_| FileHistoryRuntimeError::InvalidCursor)?;
    if offset > MAX_FILE_HISTORY_OFFSET {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    }
    Ok(offset)
}

fn decode_path(encoded: &str) -> Result<String, FileHistoryRuntimeError> {
    if encoded.is_empty() || encoded.len() % 2 != 0 {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    }
    let bytes = encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_value(pair[0])?;
            let low = hex_value(pair[1])?;
            Ok(high << 4 | low)
        })
        .collect::<Result<Vec<_>, FileHistoryRuntimeError>>()?;
    let path = String::from_utf8(bytes).map_err(|_| FileHistoryRuntimeError::InvalidCursor)?;
    if !valid_path(&path) {
        return Err(FileHistoryRuntimeError::InvalidCursor);
    }
    Ok(path)
}

fn hex_value(byte: u8) -> Result<u8, FileHistoryRuntimeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(FileHistoryRuntimeError::InvalidCursor),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex};

    use super::*;

    const OID: &str = "0123456789012345678901234567890123456789";

    struct RecordingExecutor {
        queries: Mutex<Vec<FileHistoryQuery>>,
        outputs: Mutex<Vec<Result<GitOutput, GitRunError>>>,
    }

    impl FileHistoryGitExecutor for RecordingExecutor {
        fn execute_file_history(
            &self,
            _repository: &Path,
            query: FileHistoryQuery,
        ) -> Result<GitOutput, GitRunError> {
            self.queries.lock().expect("queries lock").push(query);
            self.outputs.lock().expect("outputs lock").remove(0)
        }
    }

    #[test]
    fn cursor_is_bound_to_the_exact_start_commit_and_literal_path() {
        let cursor = encode_cursor(OID, "src/a: b.rs", 100);
        assert!(matches!(parse_cursor(&cursor, OID, "src/a: b.rs"), Ok(100)));
        assert!(matches!(
            parse_cursor(&cursor, OID, "src/other.rs"),
            Err(FileHistoryRuntimeError::InvalidCursor)
        ));
        assert!(matches!(
            parse_cursor(&cursor, &"f".repeat(40), "src/a: b.rs"),
            Err(FileHistoryRuntimeError::InvalidCursor)
        ));
    }

    #[test]
    fn history_query_has_one_bounded_follow_shape_and_literal_pathspec() {
        let query = FileHistoryQuery::HistoryPage {
            start_oid: OID.to_owned(),
            path: "--output=/tmp/pwn :(glob)*.rs".to_owned(),
            offset: 0,
        };
        let arguments = query
            .arguments()
            .into_iter()
            .map(|value| value.into_string().expect("UTF-8 argv"))
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "log".to_owned(),
                "-z".to_owned(),
                "--topo-order".to_owned(),
                "--follow".to_owned(),
                "--max-count=101".to_owned(),
                "--skip=0".to_owned(),
                COMMIT_FORMAT.to_owned(),
                OID.to_owned(),
                "--".to_owned(),
                ":(literal)--output=/tmp/pwn :(glob)*.rs".to_owned(),
            ]
        );
    }

    #[test]
    fn binary_blame_returns_an_explicit_unavailable_state_before_running_blame() {
        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(vec![
                Ok(GitOutput {
                    stdout: b"3\n".to_vec(),
                    stderr: Vec::new(),
                }),
                Ok(GitOutput {
                    stdout: vec![1, 0, 2],
                    stderr: Vec::new(),
                }),
            ]),
        };
        let response =
            file_blame(&executor, Path::new("/repo"), OID, "image.bin").expect("binary response");
        assert_eq!(response.state, FileBlameState::Binary);
        assert!(response.lines.is_empty());
        assert_eq!(executor.queries.lock().expect("queries lock").len(), 2);
    }

    #[test]
    fn rejects_an_oversized_path_before_running_git() {
        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(Vec::new()),
        };
        let oversized = "a".repeat(MAX_PATH_BYTES + 1);
        assert!(matches!(
            file_history(&executor, Path::new("/repo"), OID, &oversized, None),
            Err(FileHistoryRuntimeError::InvalidPath)
        ));
        assert!(executor.queries.lock().expect("queries lock").is_empty());
    }

    #[test]
    fn parses_line_porcelain_with_an_iso_timestamp() {
        let input = concat!(
            "0123456789012345678901234567890123456789 7 3 1\n",
            "author A Person\n",
            "author-mail <a@example.test>\n",
            "author-time 0\n",
            "author-tz +0230\n",
            "summary Add file\n",
            "filename src/file.rs\n",
            "\tline text\n"
        );
        let lines = parse_line_porcelain(input.as_bytes()).expect("parse blame");
        assert_eq!(lines[0].line_number, 3);
        assert_eq!(lines[0].authored_at, "1970-01-01T02:30:00+02:30");
    }

    #[test]
    fn blame_over_the_line_boundary_is_explicitly_unavailable() {
        let repeated = concat!(
            "0123456789012345678901234567890123456789 1 1 1\n",
            "author A Person\n",
            "author-mail <a@example.test>\n",
            "author-time 0\n",
            "author-tz +0000\n",
            "summary Add file\n",
            "filename src/file.rs\n",
            "\tline text\n"
        );
        let blame = repeated.repeat(MAX_BLAME_LINES + 1).into_bytes();
        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(vec![
                Ok(GitOutput {
                    stdout: b"1\n".to_vec(),
                    stderr: Vec::new(),
                }),
                Ok(GitOutput {
                    stdout: b"x".to_vec(),
                    stderr: Vec::new(),
                }),
                Ok(GitOutput {
                    stdout: blame,
                    stderr: Vec::new(),
                }),
            ]),
        };
        let response = file_blame(&executor, Path::new("/repo"), OID, "src/file.rs")
            .expect("oversized blame response");
        assert_eq!(response.state, FileBlameState::Oversized);
        assert!(response.lines.is_empty());
    }
}
