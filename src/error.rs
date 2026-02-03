use thiserror::Error;

/// Application-level errors for changelog-x
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Changelog generation error: {0}")]
    Changelog(#[from] ChangelogError),

    #[error("AI enhancement error: {0}")]
    Ai(#[from] AiError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuration loading and parsing errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load configuration: {0}")]
    Load(String),

    #[error("Failed to parse configuration: {0}")]
    Parse(String),

    #[error("Invalid configuration value for '{key}': {message}")]
    InvalidValue { key: String, message: String },

    #[error("Missing required configuration: {0}")]
    Missing(String),
}

/// Git-cliff and changelog generation errors
#[derive(Error, Debug)]
pub enum ChangelogError {
    #[error("Git repository error: {0}")]
    Repository(String),

    #[error("No commits found matching the criteria")]
    NoCommits,

    #[error("Failed to parse commits: {0}")]
    ParseCommits(String),

    #[error("Failed to generate changelog: {0}")]
    Generation(String),
}

/// AI provider and enhancement errors
#[derive(Error, Debug)]
pub enum AiError {
    #[error("AI provider not configured")]
    NotConfigured,

    #[error("Failed to connect to AI provider: {0}")]
    Connection(String),

    #[error("AI request failed: {0}")]
    Request(String),

    #[error("Invalid AI response: {0}")]
    InvalidResponse(String),

    #[error("AI rate limit exceeded")]
    RateLimited,
}
