use config::{Case, Config, Environment, File, FileFormat};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::ConfigError;

/// Main application configuration
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AppConfig {
    /// Changelog generation settings
    #[serde(default)]
    pub changelog: ChangelogConfig,

    /// AI enhancement settings
    #[serde(default)]
    pub ai: AiConfig,
}

/// Changelog output format
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum ChangelogFormat {
    /// Keep a Changelog 1.1.0 — Added, Changed, Deprecated, Removed, Fixed, Security
    #[default]
    #[serde(rename = "keep-a-changelog")]
    KeepAChangelog,

    /// Common Changelog — Changed, Added, Removed, Fixed
    #[serde(rename = "common-changelog")]
    CommonChangelog,
}

/// Changelog generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogConfig {
    /// Output file path (default: "CHANGELOG.md")
    #[serde(default = "default_output")]
    pub output: String,

    /// Tag pattern for version matching
    #[serde(default)]
    pub tag_pattern: Option<String>,

    /// Output format (default: keep-a-changelog)
    #[serde(default)]
    pub format: ChangelogFormat,
}

impl Default for ChangelogConfig {
    fn default() -> Self {
        Self {
            output: default_output(),
            tag_pattern: None,
            format: ChangelogFormat::default(),
        }
    }
}

/// AI enhancement configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    /// AI provider: openai, anthropic, gemini, ollama, groq, deepseek
    #[serde(default)]
    pub provider: Option<String>,

    /// Model name to use
    #[serde(default)]
    pub model: Option<String>,

    /// Sampling temperature (0.0–2.0). Falls back to 0.3 when unset.
    #[serde(default)]
    pub temperature: Option<f64>,

    /// Override project description for AI context (replaces auto-detected value)
    #[serde(default)]
    pub project_description: Option<String>,

    /// Target audience: "end-users", "developers", "library-consumers"
    #[serde(default)]
    pub target_audience: Option<String>,
}

impl AiConfig {
    /// Check if AI is configured (provider is set)
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.provider.is_some()
    }
}

fn default_output() -> String {
    "CHANGELOG.md".to_string()
}

/// Get the user config directory path (cross-platform)
///
/// Returns the path to `~/.config/cgx/` on Linux/macOS
/// or the equivalent on Windows.
#[must_use]
pub fn get_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "cgx").map(|dirs| dirs.config_dir().to_path_buf())
}

/// Get the user config file path
#[must_use]
pub fn get_user_config_path() -> Option<PathBuf> {
    get_config_dir().map(|dir| dir.join("config.toml"))
}

/// Load configuration with layered precedence:
/// 1. Built-in defaults
/// 2. User config: ~/.config/cgx/config.toml
/// 3. Project config: .cgx.toml in current directory
/// 4. Environment variables: CGX_* prefix (use `__` for nested fields)
///
/// # Environment Variable Examples
///
/// ```bash
/// CGX_AI__PROVIDER=openai      # Sets ai.provider
/// CGX_AI__MODEL=gpt-4o         # Sets ai.model
/// CGX_CHANGELOG__OUTPUT=RELEASES.md  # Sets changelog.output
/// ```
///
/// # Errors
///
/// Returns `ConfigError::Load` if configuration sources cannot be read.
/// Returns `ConfigError::Parse` if configuration cannot be deserialized.
pub fn load_config(config_override: Option<&str>) -> Result<AppConfig, ConfigError> {
    let mut builder = Config::builder();

    // Layer 1: User config (if exists)
    if let Some(user_config) = get_user_config_path()
        && user_config.exists()
    {
        builder = builder.add_source(File::from(user_config).required(false));
    }

    // Layer 2: Project config (if exists)
    builder = builder.add_source(File::new(".cgx", FileFormat::Toml).required(false));

    // Layer 3: Override config file (if specified via CLI)
    if let Some(config_path) = config_override {
        builder = builder.add_source(File::new(config_path, FileFormat::Toml).required(true));
    }

    // Layer 4: Environment variables with CGX_ prefix
    // Uses `__` (double underscore) as separator for nested fields
    // prefix_separator("_") strips the underscore after CGX prefix
    // Case conversion ensures CGX_AI__PROVIDER becomes ai.provider (lowercase)
    builder = builder.add_source(
        Environment::with_prefix("CGX")
            .prefix_separator("_")
            .separator("__")
            .convert_case(Case::Lower)
            .try_parsing(true),
    );

    let config = builder
        .build()
        .map_err(|e| ConfigError::Load(e.to_string()))?;

    config
        .try_deserialize()
        .map_err(|e| ConfigError::Parse(e.to_string()))
}

/// Save AI provider and model to the user config file.
///
/// Reads the existing config (or creates defaults), updates the `ai.provider`
/// and `ai.model` fields, and writes back to `~/.config/cgx/config.toml`.
///
/// # Errors
///
/// Returns `ConfigError::Load` if the config file cannot be read or written.
pub fn save_user_ai_config(provider: &str, model: &str) -> Result<(), ConfigError> {
    let config_path = get_user_config_path().ok_or_else(|| {
        ConfigError::Load("Unable to determine user config directory".to_string())
    })?;

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConfigError::Load(format!("Failed to create config directory: {e}")))?;
    }

    // Read existing config or start from defaults
    let mut app_config: AppConfig = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| ConfigError::Load(format!("Failed to read config: {e}")))?;
        toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))?
    } else {
        AppConfig::default()
    };

    // Update AI fields
    app_config.ai.provider = Some(provider.to_string());
    app_config.ai.model = Some(model.to_string());

    // Write back
    let toml_str =
        toml::to_string_pretty(&app_config).map_err(|e| ConfigError::Parse(e.to_string()))?;
    std::fs::write(&config_path, toml_str)
        .map_err(|e| ConfigError::Load(format!("Failed to write config: {e}")))?;

    Ok(())
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_config_default_temperature() {
        let config = AiConfig::default();
        assert_eq!(config.temperature, None);
    }

    #[test]
    fn test_ai_config_default() {
        let config = AiConfig::default();
        assert_eq!(config.provider, None);
        assert_eq!(config.model, None);
    }

    #[test]
    fn test_ai_is_configured_false_when_no_provider() {
        let config = AiConfig::default();
        assert!(!config.is_configured());
    }

    #[test]
    fn test_ai_is_configured_true_when_provider_set() {
        let config = AiConfig {
            provider: Some("openai".to_string()),
            ..AiConfig::default()
        };
        assert!(config.is_configured());
    }

    #[test]
    fn test_changelog_config_default() {
        let config = ChangelogConfig::default();
        assert_eq!(config.output, "CHANGELOG.md");
        assert_eq!(config.tag_pattern, None);
        assert_eq!(config.format, ChangelogFormat::KeepAChangelog);
    }

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.changelog.output, "CHANGELOG.md");
        assert_eq!(config.changelog.tag_pattern, None);
        assert_eq!(config.ai.provider, None);
        assert_eq!(config.ai.model, None);
    }

    #[test]
    fn test_app_config_deserialize_from_toml() {
        let toml_str = r#"
            [changelog]
            output = "RELEASES.md"
            tag_pattern = "v*"

            [ai]
            provider = "anthropic"
            model = "claude-3-opus"
        "#;

        let config: AppConfig = toml::from_str(toml_str).expect("Failed to parse TOML");

        assert_eq!(config.changelog.output, "RELEASES.md");
        assert_eq!(config.changelog.tag_pattern, Some("v*".to_string()));
        assert_eq!(config.ai.provider, Some("anthropic".to_string()));
        assert_eq!(config.ai.model, Some("claude-3-opus".to_string()));
    }

    #[test]
    fn test_app_config_deserialize_partial_toml() {
        let toml_str = r#"
            [ai]
            provider = "openai"
        "#;

        let config: AppConfig = toml::from_str(toml_str).expect("Failed to parse TOML");

        // Defaults should apply for missing fields
        assert_eq!(config.changelog.output, "CHANGELOG.md");
        assert_eq!(config.changelog.tag_pattern, None);
        assert_eq!(config.ai.provider, Some("openai".to_string()));
        assert_eq!(config.ai.model, None);
    }

    #[test]
    fn test_app_config_serialize_to_toml() {
        let config = AppConfig {
            changelog: ChangelogConfig {
                output: "HISTORY.md".to_string(),
                tag_pattern: Some("release-*".to_string()),
                format: ChangelogFormat::default(),
            },
            ai: AiConfig {
                provider: Some("gemini".to_string()),
                model: Some("gemini-pro".to_string()),
                ..AiConfig::default()
            },
        };

        let toml_str = toml::to_string(&config).expect("Failed to serialize");

        assert!(toml_str.contains("output = \"HISTORY.md\""));
        assert!(toml_str.contains("tag_pattern = \"release-*\""));
        assert!(toml_str.contains("provider = \"gemini\""));
        assert!(toml_str.contains("model = \"gemini-pro\""));
    }

    #[test]
    fn test_app_config_deserialize_temperature_from_toml() {
        let toml_str = r#"
            [ai]
            provider = "openai"
            model = "gpt-4o"
            temperature = 0.5
        "#;

        let config: AppConfig = toml::from_str(toml_str).expect("Failed to parse TOML");

        assert_eq!(config.ai.temperature, Some(0.5));
    }

    #[test]
    fn test_get_user_config_path_contains_config_toml() {
        if let Some(path) = get_user_config_path() {
            assert!(path.ends_with("config.toml"));
        }
        // If None, the system doesn't support ProjectDirs (acceptable)
    }
}
