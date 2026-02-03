use crate::config::ChangelogConfig;
use crate::error::ChangelogError;

/// Generates changelogs from conventional commits using git-cliff
pub struct ChangelogGenerator {
    config: ChangelogConfig,
}

impl ChangelogGenerator {
    /// Create a new changelog generator with the given configuration
    pub fn new(config: ChangelogConfig) -> Self {
        Self { config }
    }

    /// Generate a changelog from the repository at the given path
    pub fn generate(&self, _repo_path: Option<&str>) -> Result<String, ChangelogError> {
        // TODO: Implement git-cliff integration
        // This will:
        // 1. Open the git repository
        // 2. Parse conventional commits
        // 3. Generate changelog using git-cliff-core
        let _ = &self.config;
        Err(ChangelogError::Generation(
            "Not yet implemented".to_string(),
        ))
    }
}
