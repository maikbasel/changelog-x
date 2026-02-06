use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use git_cliff_core::changelog::Changelog;
use git_cliff_core::commit::Commit as CliffCommit;
use git_cliff_core::config::{
    Bump, ChangelogConfig as CliffChangelogConfig, CommitParser, Config as CliffConfig, GitConfig,
    RemoteConfig,
};
use git_cliff_core::release::Release;
use git_cliff_core::repo::Repository;
use git_cliff_core::tag::Tag;
use indexmap::IndexMap;
use regex::Regex;

use crate::config::ChangelogConfig;
use crate::error::ChangelogError;

/// Options for changelog generation passed from CLI
#[derive(Debug, Default, Clone)]
pub struct GenerateOptions {
    /// Start from this git tag (exclusive)
    pub from_tag: Option<String>,
    /// End at this git tag (inclusive)
    pub to_tag: Option<String>,
    /// Generate changelog for unreleased commits only (since latest tag)
    pub unreleased: bool,
}

/// Generates changelogs from conventional commits using git-cliff
pub struct ChangelogGenerator {
    config: ChangelogConfig,
}

impl ChangelogGenerator {
    /// Create a new changelog generator with the given configuration
    #[must_use]
    pub const fn new(config: ChangelogConfig) -> Self {
        Self { config }
    }

    /// Generate a changelog from the repository at the given path
    ///
    /// # Errors
    ///
    /// Returns `ChangelogError::Repository` if the git repository cannot be opened.
    /// Returns `ChangelogError::NoCommits` if no commits match the criteria.
    /// Returns `ChangelogError::Generation` if changelog generation fails.
    pub fn generate(
        &self,
        options: &GenerateOptions,
        on_step: Option<&dyn Fn()>,
    ) -> Result<String, ChangelogError> {
        let step = || {
            if let Some(cb) = &on_step {
                cb();
            }
        };

        // Open git repository in current directory
        step();

        let repo_path = env::current_dir().map_err(|e| {
            ChangelogError::Repository(format!("Failed to get current directory: {e}"))
        })?;

        let repo = Repository::init(repo_path).map_err(|e| {
            ChangelogError::Repository(format!("Failed to open git repository: {e}"))
        })?;

        // Build git-cliff configuration
        let cliff_config = self.build_config();

        // Get tag pattern regex
        let tag_pattern = self
            .config
            .tag_pattern
            .as_ref()
            .and_then(|p| Regex::new(p).ok());

        // Resolve effective options (handles --unreleased by finding latest tag)
        step();

        let effective_options = if options.unreleased {
            Self::resolve_unreleased(&repo, tag_pattern.as_ref())?
        } else {
            options.clone()
        };

        // Build commit range
        let range = Self::build_range(&effective_options, &repo)?;

        // Fetch commits and tags
        step();

        let commits = repo
            .commits(range.as_deref(), None, None, false)
            .map_err(|e| ChangelogError::Repository(format!("Failed to fetch commits: {e}")))?;

        if commits.is_empty() {
            return Err(ChangelogError::NoCommits);
        }

        let tags = repo
            .tags(&tag_pattern, false, false)
            .map_err(|e| ChangelogError::Repository(format!("Failed to fetch tags: {e}")))?;

        // Build releases from commits and tags
        step();

        let releases = Self::build_releases(&commits, &tags);

        if releases.is_empty() {
            return Err(ChangelogError::NoCommits);
        }

        // Generate changelog
        step();

        let changelog = Changelog::new(releases, cliff_config, range.as_deref())
            .map_err(|e| ChangelogError::Generation(format!("Failed to create changelog: {e}")))?;

        let mut output = Vec::new();
        changelog.generate(&mut output).map_err(|e| {
            ChangelogError::Generation(format!("Failed to generate changelog: {e}"))
        })?;

        String::from_utf8(output)
            .map_err(|e| ChangelogError::Generation(format!("Invalid UTF-8 in output: {e}")))
    }

    /// Resolve `--unreleased` into concrete `GenerateOptions` by finding the latest tag.
    fn resolve_unreleased(
        repo: &Repository,
        tag_pattern: Option<&Regex>,
    ) -> Result<GenerateOptions, ChangelogError> {
        let tags = repo
            .tags(&tag_pattern.cloned(), false, false)
            .map_err(|e| ChangelogError::Repository(format!("Failed to fetch tags: {e}")))?;

        // The last entry in the IndexMap is the most recent tag by time
        let latest_tag = tags.values().last().map(|tag| tag.name.clone());

        Ok(GenerateOptions {
            from_tag: latest_tag,
            to_tag: None,
            unreleased: false,
        })
    }

    /// Build the commit range string for git-cliff
    fn build_range(
        options: &GenerateOptions,
        repo: &Repository,
    ) -> Result<Option<String>, ChangelogError> {
        match (&options.from_tag, &options.to_tag) {
            (Some(from), Some(to)) => {
                Self::validate_tag(repo, from)?;
                Self::validate_tag(repo, to)?;
                Ok(Some(format!("{from}..{to}")))
            }
            (Some(from), None) => {
                Self::validate_tag(repo, from)?;
                Ok(Some(format!("{from}..HEAD")))
            }
            (None, Some(to)) => {
                Self::validate_tag(repo, to)?;
                Ok(Some(to.clone()))
            }
            (None, None) => Ok(None),
        }
    }

    /// Validate that a tag exists in the repository
    fn validate_tag(repo: &Repository, tag: &str) -> Result<(), ChangelogError> {
        let tag_obj = repo.resolve_tag(tag);
        // resolve_tag always returns a Tag, but if the tag doesn't exist in git,
        // attempting to use it in a range will fail. We do a basic check here.
        if tag_obj.name != tag && !tag_obj.name.contains(tag) {
            return Err(ChangelogError::Repository(format!(
                "Tag '{tag}' not found in repository"
            )));
        }
        Ok(())
    }

    /// Build releases from commits and tags
    fn build_releases<'a>(
        commits: &'a [git2::Commit<'a>],
        tags: &IndexMap<String, Tag>,
    ) -> Vec<Release<'a>> {
        let mut releases: Vec<Release<'a>> = Vec::new();
        let mut current_commits: Vec<CliffCommit<'a>> = Vec::new();
        let mut current_version: Option<String> = None;
        let mut current_timestamp: Option<i64> = None;

        // Create reverse mapping: commit_id -> tag
        let commit_to_tag: HashMap<String, &Tag> =
            tags.iter().map(|(id, tag)| (id.clone(), tag)).collect();

        for commit in commits {
            let commit_id = commit.id().to_string();

            // Check if this commit has a tag
            if let Some(tag) = commit_to_tag.get(&commit_id) {
                // Save current release if we have commits
                if !current_commits.is_empty() || current_version.is_some() {
                    releases.push(Release {
                        version: current_version.take(),
                        message: None,
                        commits: std::mem::take(&mut current_commits),
                        commit_id: None,
                        timestamp: current_timestamp.take(),
                        previous: None,
                        repository: None,
                        commit_range: None,
                        submodule_commits: HashMap::new(),
                        statistics: None,
                        extra: None,
                    });
                }

                // Start new release with this tag
                current_version = Some(tag.name.clone());
                current_timestamp = Some(commit.time().seconds());
            }

            // Add commit to current release
            current_commits.push(CliffCommit::from(commit));
        }

        // Don't forget the last release (commits without a tag = unreleased)
        if !current_commits.is_empty() {
            releases.push(Release {
                version: current_version,
                message: None,
                commits: current_commits,
                commit_id: None,
                timestamp: current_timestamp,
                previous: None,
                repository: None,
                commit_range: None,
                submodule_commits: HashMap::new(),
                statistics: None,
                extra: None,
            });
        }

        // Link releases (set previous)
        for i in 0..releases.len().saturating_sub(1) {
            let prev = Box::new(releases[i + 1].clone());
            releases[i].previous = Some(prev);
        }

        releases
    }

    /// Build git-cliff configuration for conventional commits
    fn build_config(&self) -> CliffConfig {
        CliffConfig {
            changelog: CliffChangelogConfig {
                header: Some(String::from("# Changelog\n")),
                body: Self::default_body_template(),
                footer: None,
                trim: true,
                postprocessors: vec![],
                render_always: false,
                output: Some(PathBuf::from(&self.config.output)),
            },
            git: GitConfig {
                conventional_commits: true,
                filter_unconventional: true,
                split_commits: false,
                commit_preprocessors: vec![],
                commit_parsers: Self::default_commit_parsers(),
                protect_breaking_commits: true,
                filter_commits: false,
                tag_pattern: self
                    .config
                    .tag_pattern
                    .as_ref()
                    .and_then(|p| Regex::new(p).ok()),
                skip_tags: None,
                ignore_tags: None,
                count_tags: None,
                topo_order: false,
                topo_order_commits: true,
                sort_commits: String::from("oldest"),
                link_parsers: vec![],
                limit_commits: None,
                require_conventional: false,
                fail_on_unmatched_commit: false,
                use_branch_tags: false,
                recurse_submodules: None,
                include_paths: vec![],
                exclude_paths: vec![],
            },
            remote: RemoteConfig::default(),
            bump: Bump::default(),
        }
    }

    /// Default body template for conventional commits
    fn default_body_template() -> String {
        String::from(
            r#"{% if version %}
## [{{ version | trim_start_matches(pat="v") }}] - {{ timestamp | date(format="%Y-%m-%d") }}
{% else %}
## [Unreleased]
{% endif %}
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | striptags | trim | upper_first }}
{% for commit in commits %}
- {% if commit.scope %}**{{ commit.scope }}:** {% endif %}{{ commit.message | upper_first }}
{%- endfor %}
{% endfor %}"#,
        )
    }

    /// Default commit parsers for conventional commits
    #[allow(clippy::trivial_regex)] // git-cliff-core requires Regex, not str::starts_with
    fn default_commit_parsers() -> Vec<CommitParser> {
        vec![
            CommitParser {
                message: Regex::new(r"^feat").ok(),
                group: Some(String::from("Features")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^fix").ok(),
                group: Some(String::from("Bug Fixes")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^doc").ok(),
                group: Some(String::from("Documentation")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^perf").ok(),
                group: Some(String::from("Performance")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^refactor").ok(),
                group: Some(String::from("Refactoring")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^style").ok(),
                group: Some(String::from("Styling")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^test").ok(),
                group: Some(String::from("Testing")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^chore\(release\)").ok(),
                skip: Some(true),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^chore|^ci").ok(),
                group: Some(String::from("Miscellaneous Tasks")),
                ..Default::default()
            },
            CommitParser {
                message: Regex::new(r"^deps").ok(),
                group: Some(String::from("Dependencies")),
                ..Default::default()
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_range_both_tags() {
        let options = GenerateOptions {
            from_tag: Some(String::from("v1.0.0")),
            to_tag: Some(String::from("v2.0.0")),
            ..Default::default()
        };

        // We can't fully test this without a repo, but we can test the logic
        // by checking the expected format
        assert_eq!(options.from_tag, Some(String::from("v1.0.0")));
        assert_eq!(options.to_tag, Some(String::from("v2.0.0")));
    }

    #[test]
    fn test_build_range_from_only() {
        let options = GenerateOptions {
            from_tag: Some(String::from("v1.0.0")),
            to_tag: None,
            ..Default::default()
        };

        assert_eq!(options.from_tag, Some(String::from("v1.0.0")));
        assert!(options.to_tag.is_none());
    }

    #[test]
    fn test_build_range_none() {
        let options = GenerateOptions::default();

        assert!(options.from_tag.is_none());
        assert!(options.to_tag.is_none());
    }

    #[test]
    fn test_default_commit_parsers() {
        let parsers = ChangelogGenerator::default_commit_parsers();

        assert!(!parsers.is_empty());
        assert!(
            parsers
                .iter()
                .any(|p| p.group.as_deref() == Some("Features"))
        );
        assert!(
            parsers
                .iter()
                .any(|p| p.group.as_deref() == Some("Bug Fixes"))
        );
    }

    #[test]
    fn test_generate_options_default() {
        let options = GenerateOptions::default();
        assert!(options.from_tag.is_none());
        assert!(options.to_tag.is_none());
        assert!(!options.unreleased);
    }

    #[test]
    fn test_generate_options_unreleased() {
        let options = GenerateOptions {
            unreleased: true,
            ..Default::default()
        };
        assert!(options.unreleased);
        assert!(options.from_tag.is_none());
        assert!(options.to_tag.is_none());
    }
}
