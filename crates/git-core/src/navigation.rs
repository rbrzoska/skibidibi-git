use app_domain::{RepositoryBranch, RepositoryBranchKind, RepositoryStash, RepositoryWorktree};
use thiserror::Error;

const BRANCH_FIELD_COUNT: usize = 7;
const STASH_FIELD_COUNT: usize = 5;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NavigationParseError {
    #[error("branch record has {actual} fields; expected {expected}")]
    InvalidBranchFieldCount { expected: usize, actual: usize },
    #[error("unsupported branch ref {0:?}")]
    UnsupportedBranchRef(String),
    #[error("invalid branch tracking value {0:?}")]
    InvalidTracking(String),
    #[error("worktree record is missing its path")]
    MissingWorktreePath,
    #[error("worktree record contains an invalid line {0:?}")]
    InvalidWorktreeLine(String),
    #[error("stash record has {actual} fields; expected {expected}")]
    InvalidStashFieldCount { expected: usize, actual: usize },
}

pub fn parse_branch_records(input: &[u8]) -> Result<Vec<RepositoryBranch>, NavigationParseError> {
    records(input, BRANCH_FIELD_COUNT)
        .map(|fields| {
            let fields = fields?;
            let full_name = text(fields[0]);
            let kind = if full_name.starts_with("refs/heads/") {
                RepositoryBranchKind::Local
            } else if full_name.starts_with("refs/remotes/") {
                RepositoryBranchKind::Remote
            } else {
                return Err(NavigationParseError::UnsupportedBranchRef(full_name));
            };
            let (ahead, behind, upstream_gone) = parse_tracking(&text(fields[5]))?;

            Ok(RepositoryBranch {
                kind,
                full_name,
                name: text(fields[1]),
                oid: text(fields[2]),
                current: fields[3] == b"*",
                upstream: optional_text(fields[4]),
                ahead,
                behind,
                upstream_gone,
                symbolic_target: optional_text(fields[6]),
            })
        })
        .collect()
}

pub fn parse_worktree_porcelain_z(
    input: &[u8],
) -> Result<Vec<RepositoryWorktree>, NavigationParseError> {
    input
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>()
        .split(|line| line.is_empty())
        .filter(|record| record.iter().any(|line| !line.is_empty()))
        .map(parse_worktree_record)
        .collect()
}

pub fn parse_stash_records(input: &[u8]) -> Result<Vec<RepositoryStash>, NavigationParseError> {
    records(input, STASH_FIELD_COUNT)
        .map(|fields| {
            let fields = fields?;
            Ok(RepositoryStash {
                oid: text(fields[0]),
                selector: text(fields[1]),
                message: text(fields[2]),
                author: text(fields[3]),
                authored_at: text(fields[4]),
            })
        })
        .collect()
}

fn records(
    input: &[u8],
    field_count: usize,
) -> impl Iterator<Item = Result<Vec<&[u8]>, NavigationParseError>> {
    let mut fields = input.split(|byte| *byte == 0).peekable();
    std::iter::from_fn(move || {
        while fields
            .peek()
            .is_some_and(|field| trim_record_separator(field).is_empty())
        {
            fields.next();
        }
        fields.peek()?;

        let record = fields
            .by_ref()
            .take(field_count)
            .enumerate()
            .map(|(index, field)| {
                if index == 0 {
                    trim_record_separator(field)
                } else {
                    field
                }
            })
            .collect::<Vec<_>>();

        if record.len() == field_count {
            Some(Ok(record))
        } else {
            Some(Err(if field_count == BRANCH_FIELD_COUNT {
                NavigationParseError::InvalidBranchFieldCount {
                    expected: field_count,
                    actual: record.len(),
                }
            } else {
                NavigationParseError::InvalidStashFieldCount {
                    expected: field_count,
                    actual: record.len(),
                }
            }))
        }
    })
}

fn parse_worktree_record(lines: &[&[u8]]) -> Result<RepositoryWorktree, NavigationParseError> {
    let mut worktree = RepositoryWorktree {
        path: String::new(),
        head: None,
        branch: None,
        detached: false,
        bare: false,
        locked: false,
        lock_reason: None,
        prunable: false,
        prunable_reason: None,
    };

    for line in lines {
        let (key, value) = line
            .iter()
            .position(|byte| *byte == b' ')
            .map_or((*line, &[][..]), |separator| {
                (&line[..separator], &line[separator + 1..])
            });
        match key {
            b"worktree" => worktree.path = text(value),
            b"HEAD" => worktree.head = optional_text(value),
            b"branch" => worktree.branch = optional_text(value),
            b"detached" => worktree.detached = true,
            b"bare" => worktree.bare = true,
            b"locked" => {
                worktree.locked = true;
                worktree.lock_reason = optional_text(value);
            }
            b"prunable" => {
                worktree.prunable = true;
                worktree.prunable_reason = optional_text(value);
            }
            _ => return Err(NavigationParseError::InvalidWorktreeLine(text(line))),
        }
    }

    if worktree.path.is_empty() {
        return Err(NavigationParseError::MissingWorktreePath);
    }
    Ok(worktree)
}

fn parse_tracking(value: &str) -> Result<(u64, u64, bool), NavigationParseError> {
    if value.is_empty() {
        return Ok((0, 0, false));
    }
    if value == "gone" {
        return Ok((0, 0, true));
    }

    let mut ahead = 0;
    let mut behind = 0;
    for part in value.split(',').map(str::trim) {
        let (direction, count) = part
            .split_once(' ')
            .ok_or_else(|| NavigationParseError::InvalidTracking(value.to_owned()))?;
        let count = count
            .parse::<u64>()
            .map_err(|_| NavigationParseError::InvalidTracking(value.to_owned()))?;
        match direction {
            "ahead" => ahead = count,
            "behind" => behind = count,
            _ => return Err(NavigationParseError::InvalidTracking(value.to_owned())),
        }
    }
    Ok((ahead, behind, false))
}

fn trim_record_separator(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        value = &value[1..];
    }
    value
}

fn text(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

fn optional_text(value: &[u8]) -> Option<String> {
    (!value.is_empty()).then(|| text(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_remote_and_symbolic_branches() {
        let input = b"refs/heads/main\0main\0aaaaaaaa\0*\0origin/main\0ahead 2, behind 3\0\0\nrefs/remotes/origin/HEAD\0origin\0aaaaaaaa\0 \0\0\0refs/remotes/origin/main\0\n";

        let branches = parse_branch_records(input).expect("valid branches");

        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].kind, RepositoryBranchKind::Local);
        assert!(branches[0].current);
        assert_eq!((branches[0].ahead, branches[0].behind), (2, 3));
        assert_eq!(branches[1].kind, RepositoryBranchKind::Remote);
        assert_eq!(
            branches[1].symbolic_target.as_deref(),
            Some("refs/remotes/origin/main")
        );
    }

    #[test]
    fn parses_gone_upstream() {
        let input = b"refs/heads/old\0old\0bbbb\0 \0origin/old\0gone\0\0\n";
        let branches = parse_branch_records(input).expect("valid branch");
        assert!(branches[0].upstream_gone);
    }

    #[test]
    fn parses_detached_locked_and_prunable_worktrees() {
        let input = b"worktree /repo/main\0HEAD aaaa\0branch refs/heads/main\0\0worktree /repo/detached\0HEAD bbbb\0detached\0locked maintenance\0prunable gitdir file points to non-existent location\0\0";

        let worktrees = parse_worktree_porcelain_z(input).expect("valid worktrees");

        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].branch.as_deref(), Some("refs/heads/main"));
        assert!(worktrees[1].detached);
        assert!(worktrees[1].locked);
        assert_eq!(worktrees[1].lock_reason.as_deref(), Some("maintenance"));
        assert!(worktrees[1].prunable);
    }

    #[test]
    fn parses_stash_records_and_empty_input() {
        let input =
            b"abc123\0stash@{0}\0WIP on main: message\0Rafal\x002026-07-15T12:30:00+02:00\0\n";

        let stashes = parse_stash_records(input).expect("valid stashes");

        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].selector, "stash@{0}");
        assert!(
            parse_stash_records(b"")
                .expect("empty stash list")
                .is_empty()
        );
    }
}
