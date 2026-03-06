use std::collections::HashMap;

use git2::Repository;
use regex::Regex;
use serde::Serialize;
use tracing::debug;

use crate::error::ChangelogError;

/// Structured commit data extracted from git history for AI processing.
#[derive(Debug, Clone, Serialize)]
pub struct CommitData {
    pub id: String,
    pub commit_type: Option<String>,
    pub scope: Option<String>,
    pub subject: String,
    pub body: Option<String>,
    pub breaking: bool,
    pub author: String,
    pub timestamp: i64,
    pub diff_stats: DiffStats,
}

/// File-level diff statistics for a single commit.
#[derive(Debug, Clone, Serialize)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub changed_files: Vec<String>,
}

/// A group of commits belonging to a single release (tag boundary).
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseData {
    pub version: Option<String>,
    pub timestamp: Option<i64>,
    pub commits: Vec<CommitData>,
}

/// Maximum number of changed file paths to include per commit.
const MAX_CHANGED_FILES: usize = 10;

/// Extract releases from the git repository in the current directory.
///
/// Groups commits by tag boundaries, parses conventional commit metadata,
/// and computes diff stats for each commit.
///
/// # Errors
///
/// Returns `ChangelogError::Repository` if the repository cannot be opened or
/// commits cannot be read.
/// Returns `ChangelogError::DiffStats` if diff computation fails for a commit.
pub fn extract_releases(
    from_tag: Option<&str>,
    to_tag: Option<&str>,
    unreleased: bool,
    tag_pattern: Option<&Regex>,
) -> Result<Vec<ReleaseData>, ChangelogError> {
    let repo = Repository::open(".")
        .map_err(|e| ChangelogError::Repository(format!("Failed to open git repository: {e}")))?;

    // Resolve commit range
    let range = resolve_range(&repo, from_tag, to_tag, unreleased, tag_pattern)?;

    // Walk commits
    let commits = walk_commits(&repo, range.as_deref())?;

    if commits.is_empty() {
        return Err(ChangelogError::NoCommits);
    }

    // Collect tags for grouping
    let tags = collect_tags(&repo, tag_pattern)?;

    // Build releases from commits grouped by tags
    let releases = build_releases(&repo, &commits, &tags)?;

    debug!(
        releases = releases.len(),
        total_commits = commits.len(),
        "Extracted release data from git history"
    );

    Ok(releases)
}

/// Resolve the effective commit range string from CLI options.
fn resolve_range(
    repo: &Repository,
    from_tag: Option<&str>,
    to_tag: Option<&str>,
    unreleased: bool,
    tag_pattern: Option<&Regex>,
) -> Result<Option<String>, ChangelogError> {
    if unreleased {
        let latest = find_latest_tag(repo, tag_pattern)?;
        return Ok(latest.map(|tag| format!("{tag}..HEAD")));
    }

    match (from_tag, to_tag) {
        (Some(from), Some(to)) => Ok(Some(format!("{from}..{to}"))),
        (Some(from), None) => Ok(Some(format!("{from}..HEAD"))),
        (None, Some(to)) => Ok(Some(to.to_string())),
        (None, None) => Ok(None),
    }
}

/// Find the latest tag in the repository matching the optional pattern.
fn find_latest_tag(
    repo: &Repository,
    tag_pattern: Option<&Regex>,
) -> Result<Option<String>, ChangelogError> {
    let tags = repo
        .tag_names(None)
        .map_err(|e| ChangelogError::Repository(format!("Failed to list tags: {e}")))?;

    let mut latest_name: Option<String> = None;
    let mut latest_time: Option<i64> = None;

    for tag_name in tags.iter().flatten() {
        if let Some(pat) = tag_pattern
            && !pat.is_match(tag_name)
        {
            continue;
        }

        let time = tag_commit_time(repo, tag_name).unwrap_or(0);
        if latest_time.is_none_or(|t| time > t) {
            latest_time = Some(time);
            latest_name = Some(tag_name.to_string());
        }
    }

    Ok(latest_name)
}

/// Get the commit timestamp for a tag reference.
fn tag_commit_time(repo: &Repository, tag_name: &str) -> Option<i64> {
    let refname = format!("refs/tags/{tag_name}");
    let reference = repo.find_reference(&refname).ok()?;
    let commit = reference.peel_to_commit().ok()?;
    Some(commit.time().seconds())
}

/// Walk the commit history and return `git2::Oid`s in topological order.
fn walk_commits(repo: &Repository, range: Option<&str>) -> Result<Vec<git2::Oid>, ChangelogError> {
    let mut revwalk = repo
        .revwalk()
        .map_err(|e| ChangelogError::Repository(format!("Failed to create revwalk: {e}")))?;

    revwalk
        .set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .ok();

    if let Some(range_str) = range {
        revwalk
            .push_range(range_str)
            .map_err(|e| ChangelogError::Repository(format!("Invalid range '{range_str}': {e}")))?;
    } else {
        revwalk
            .push_head()
            .map_err(|e| ChangelogError::Repository(format!("Failed to push HEAD: {e}")))?;
    }

    let oids: Vec<git2::Oid> = revwalk.filter_map(Result::ok).collect();

    Ok(oids)
}

/// Collect all tags mapping commit OID → tag name.
fn collect_tags(
    repo: &Repository,
    tag_pattern: Option<&Regex>,
) -> Result<HashMap<git2::Oid, String>, ChangelogError> {
    let tag_names = repo
        .tag_names(None)
        .map_err(|e| ChangelogError::Repository(format!("Failed to list tags: {e}")))?;

    let mut map = HashMap::new();

    for name in tag_names.iter().flatten() {
        if let Some(pat) = tag_pattern
            && !pat.is_match(name)
        {
            continue;
        }

        let refname = format!("refs/tags/{name}");
        if let Ok(reference) = repo.find_reference(&refname)
            && let Ok(commit) = reference.peel_to_commit()
        {
            map.insert(commit.id(), name.to_string());
        }
    }

    Ok(map)
}

/// Build `ReleaseData` from commits grouped by tag boundaries.
fn build_releases(
    repo: &Repository,
    oids: &[git2::Oid],
    tags: &HashMap<git2::Oid, String>,
) -> Result<Vec<ReleaseData>, ChangelogError> {
    let mut releases: Vec<ReleaseData> = Vec::new();
    let mut current_commits: Vec<CommitData> = Vec::new();
    let mut current_version: Option<String> = None;
    let mut current_timestamp: Option<i64> = None;

    for &oid in oids {
        let commit = repo
            .find_commit(oid)
            .map_err(|e| ChangelogError::Repository(format!("Failed to find commit {oid}: {e}")))?;

        // If this commit has a tag, finalize previous release and start a new one
        if let Some(tag_name) = tags.get(&oid) {
            if !current_commits.is_empty() || current_version.is_some() {
                releases.push(ReleaseData {
                    version: current_version.take(),
                    timestamp: current_timestamp.take(),
                    commits: std::mem::take(&mut current_commits),
                });
            }

            current_version = Some(tag_name.clone());
            current_timestamp = Some(commit.time().seconds());
        }

        let commit_data = extract_commit_data(repo, &commit)?;
        current_commits.push(commit_data);
    }

    // Remaining commits (unreleased or last tagged release)
    if !current_commits.is_empty() {
        releases.push(ReleaseData {
            version: current_version,
            timestamp: current_timestamp,
            commits: current_commits,
        });
    }

    Ok(releases)
}

/// Extract structured data from a single commit.
fn extract_commit_data(
    repo: &Repository,
    commit: &git2::Commit<'_>,
) -> Result<CommitData, ChangelogError> {
    let message = commit.message().unwrap_or("");
    let first_line = message.lines().next().unwrap_or("");
    let rest = message
        .strip_prefix(first_line)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    // Try conventional commit parsing
    let (commit_type, scope, subject, body, breaking) =
        if let Ok(conv) = git_conventional::Commit::parse(message) {
            (
                Some(conv.type_().to_string()),
                conv.scope().map(|s| s.to_string()),
                conv.description().to_string(),
                conv.body().map(String::from).or(rest),
                conv.breaking(),
            )
        } else {
            (None, None, first_line.to_string(), rest, false)
        };

    let author = commit.author().name().unwrap_or("unknown").to_string();

    let diff_stats = compute_diff_stats(repo, commit)?;

    Ok(CommitData {
        id: commit.id().to_string(),
        commit_type,
        scope,
        subject,
        body,
        breaking,
        author,
        timestamp: commit.time().seconds(),
        diff_stats,
    })
}

/// Compute diff stats for a commit by comparing with its parent.
fn compute_diff_stats(
    repo: &Repository,
    commit: &git2::Commit<'_>,
) -> Result<DiffStats, ChangelogError> {
    let tree = commit.tree().map_err(|e| {
        ChangelogError::DiffStats(format!("Failed to get tree for {}: {e}", commit.id()))
    })?;

    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .map_err(|e| {
            ChangelogError::DiffStats(format!("Failed to compute diff for {}: {e}", commit.id()))
        })?;

    let stats = diff.stats().map_err(|e| {
        ChangelogError::DiffStats(format!("Failed to get diff stats for {}: {e}", commit.id()))
    })?;

    let mut changed_files = Vec::new();
    let num_deltas = diff.deltas().len();
    for (i, delta) in diff.deltas().enumerate() {
        if i >= MAX_CHANGED_FILES {
            break;
        }
        if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
            changed_files.push(path.to_string());
        }
    }

    Ok(DiffStats {
        files_changed: num_deltas,
        insertions: stats.insertions(),
        deletions: stats.deletions(),
        changed_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_stats_default() {
        let stats = DiffStats {
            files_changed: 0,
            insertions: 0,
            deletions: 0,
            changed_files: vec![],
        };
        assert_eq!(stats.files_changed, 0);
    }

    #[test]
    fn test_commit_data_serializes() {
        let data = CommitData {
            id: "abc123".to_string(),
            commit_type: Some("feat".to_string()),
            scope: Some("ai".to_string()),
            subject: "add new feature".to_string(),
            body: None,
            breaking: false,
            author: "test".to_string(),
            timestamp: 1_700_000_000,
            diff_stats: DiffStats {
                files_changed: 1,
                insertions: 10,
                deletions: 2,
                changed_files: vec!["src/main.rs".to_string()],
            },
        };

        let json = serde_json::to_string(&data);
        assert!(json.is_ok());
        let s = json.unwrap_or_default();
        assert!(s.contains("feat"));
        assert!(s.contains("ai"));
    }

    #[test]
    fn test_release_data_serializes() {
        let release = ReleaseData {
            version: Some("v1.0.0".to_string()),
            timestamp: Some(1_700_000_000),
            commits: vec![],
        };

        let json = serde_json::to_string(&release);
        assert!(json.is_ok());
    }
}
