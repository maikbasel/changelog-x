use config::{Config, Environment, File, FileFormat};
use directories::ProjectDirs;
use serde::Deserialize;
use std::path::PathBuf;

use crate::error::ConfigError;

/// Main application configuration
#[derive(Debug, Deserialize, Default)]
pub struct AppConfig {
    /// Changelog generation settings
    #[serde(default)]
    pub changelog: ChangelogConfig,

    /// AI enhancement settings
    #[serde(default)]
    pub ai: AiConfig,
}

/// Changelog generation configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ChangelogConfig {
    /// Output file path
    #[serde(default = "default_output")]
    pub output: String,

    /// Include unreleased changes
    #[serde(default = "default_true")]
    pub unreleased: bool,

    /// Tag pattern for version matching
    #[serde(default)]
    pub tag_pattern: Option<String>,
}

impl Default for ChangelogConfig {
    fn default() -> Self {
        Self {
            output: default_output(),
            unreleased: true,
            tag_pattern: None,
        }
    }
}

/// AI enhancement configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AiConfig {
    /// Enable AI enhancement
    #[serde(default)]
    pub enabled: bool,

    /// AI provider (openai, anthropic, gemini, ollama, groq, deepseek)
    #[serde(default)]
    pub provider: Option<String>,

    /// Model name to use
    #[serde(default)]
    pub model: Option<String>,
}

fn default_output() -> String {
    "CHANGELOG.md".to_string()
}

fn default_true() -> bool {
    true
}

/// Get the user config directory path (cross-platform)
pub fn get_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "changelog-x").map(|dirs| dirs.config_dir().to_path_buf())
}

/// Get the user config file path
pub fn get_user_config_path() -> Option<PathBuf> {
    get_config_dir().map(|dir| dir.join("config.toml"))
}

/// Load configuration with layered precedence:
/// 1. Built-in defaults
/// 2. User config: ~/.config/changelog-x/config.toml
/// 3. Project config: .changelog-x.toml in current directory
/// 4. Environment variables: CHANGELOG_X_* prefix
pub fn load_config(config_override: Option<&str>) -> Result<AppConfig, ConfigError> {
    let mut builder = Config::builder();

    // Layer 1: User config (if exists)
    if let Some(user_config) = get_user_config_path()
        && user_config.exists()
    {
        builder = builder.add_source(File::from(user_config).required(false));
    }

    // Layer 2: Project config (if exists)
    builder = builder.add_source(File::new(".changelog-x", FileFormat::Toml).required(false));

    // Layer 3: Override config file (if specified via CLI)
    if let Some(config_path) = config_override {
        builder = builder.add_source(File::new(config_path, FileFormat::Toml).required(true));
    }

    // Layer 4: Environment variables with CHANGELOG_X_ prefix
    builder = builder.add_source(
        Environment::with_prefix("CHANGELOG_X")
            .separator("_")
            .try_parsing(true),
    );

    let config = builder
        .build()
        .map_err(|e| ConfigError::Load(e.to_string()))?;

    config
        .try_deserialize()
        .map_err(|e| ConfigError::Parse(e.to_string()))
}
