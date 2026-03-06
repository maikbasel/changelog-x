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

    #[error("Credential error: {0}")]
    Credential(#[from] CredentialError),

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

    #[error("Failed to compute diff stats: {0}")]
    DiffStats(String),
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

impl ConfigError {
    #[must_use]
    pub const fn help_text(&self) -> Option<&str> {
        match self {
            Self::Load(_) => Some("Run `cgx config path` to see expected config file locations"),
            Self::Parse(_) => Some("Run `cgx config edit` to fix the config file"),
            Self::InvalidValue { .. } => {
                Some("Run `cgx config show` to inspect resolved configuration")
            }
            Self::Missing(_) => {
                Some("Add the missing key to your config file or set it via CGX_ env var")
            }
        }
    }
}

impl ChangelogError {
    #[must_use]
    pub const fn help_text(&self) -> Option<&str> {
        match self {
            Self::Repository(_) => {
                Some("Ensure you are inside a git repository with at least one commit")
            }
            Self::NoCommits => Some("cgx requires Conventional Commits (e.g. feat:, fix:, docs:)"),
            Self::ParseCommits(_) => Some("Ensure commits follow: <type>[scope]: <description>"),
            Self::Generation(_) => {
                Some("This may be a bug — please file an issue at the project repository")
            }
            Self::DiffStats(_) => {
                Some("Diff stats computation failed — the commit may have an unusual structure")
            }
        }
    }
}

/// Credential storage and retrieval errors
#[derive(Error, Debug)]
pub enum CredentialError {
    #[error("Failed to store credentials: {0}")]
    Store(String),

    #[error("Failed to retrieve credentials: {0}")]
    Retrieve(String),

    #[error("Failed to delete credentials: {0}")]
    Delete(String),

    #[error("No credentials found for provider '{0}'")]
    NotFound(String),
}

impl CredentialError {
    #[must_use]
    pub const fn help_text(&self) -> Option<&str> {
        match self {
            Self::Retrieve(_) => Some("Run `cgx ai auth` to store a new API key"),
            Self::NotFound(_) => Some("Run `cgx ai auth` to store an API key for this provider"),
            Self::Store(_) | Self::Delete(_) => {
                Some("Check that your system keyring is available and unlocked")
            }
        }
    }
}

impl From<genai::Error> for AiError {
    fn from(err: genai::Error) -> Self {
        match &err {
            genai::Error::RequiresApiKey { .. }
            | genai::Error::NoAuthResolver { .. }
            | genai::Error::NoAuthData { .. } => Self::NotConfigured,
            genai::Error::WebModelCall { .. } | genai::Error::WebAdapterCall { .. } => {
                Self::Connection(err.to_string())
            }
            genai::Error::NoChatResponse { .. }
            | genai::Error::InvalidJsonResponseElement { .. }
            | genai::Error::StreamParse { .. } => Self::InvalidResponse(err.to_string()),
            _ => Self::Request(err.to_string()),
        }
    }
}

impl AiError {
    #[must_use]
    pub const fn help_text(&self) -> Option<&str> {
        match self {
            Self::NotConfigured => Some("Run `cgx ai setup` to configure an AI provider"),
            Self::Connection(_) => {
                Some("Check your network connection and verify the provider endpoint")
            }
            Self::Request(_) => Some(
                "Verify your API key: run `cgx ai status`, check keyring with `secret-tool lookup service cgx username <provider>`, or set the env var directly",
            ),
            Self::InvalidResponse(_) => Some("Try again or switch to a different model"),
            Self::RateLimited => Some("Wait a moment and retry, or switch to a different provider"),
        }
    }
}

impl AppError {
    #[must_use]
    pub const fn help_text(&self) -> Option<&str> {
        match self {
            Self::Config(e) => e.help_text(),
            Self::Changelog(e) => e.help_text(),
            Self::Ai(e) => e.help_text(),
            Self::Credential(e) => e.help_text(),
            Self::Io(_) => Some("Check file permissions and available disk space"),
        }
    }
}
