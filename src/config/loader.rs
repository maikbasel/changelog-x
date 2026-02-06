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

/// Changelog generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogConfig {
    /// Output file path (default: "CHANGELOG.md")
    #[serde(default = "default_output")]
    pub output: String,

    /// Tag pattern for version matching
    #[serde(default)]
    pub tag_pattern: Option<String>,
}

impl Default for ChangelogConfig {
    fn default() -> Self {
        Self {
            output: default_output(),
            tag_pattern: None,
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

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

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
            model: None,
        };
        assert!(config.is_configured());
    }

    #[test]
    fn test_changelog_config_default() {
        let config = ChangelogConfig::default();
        assert_eq!(config.output, "CHANGELOG.md");
        assert_eq!(config.tag_pattern, None);
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
            },
            ai: AiConfig {
                provider: Some("gemini".to_string()),
                model: Some("gemini-pro".to_string()),
            },
        };

        let toml_str = toml::to_string(&config).expect("Failed to serialize");

        assert!(toml_str.contains("output = \"HISTORY.md\""));
        assert!(toml_str.contains("tag_pattern = \"release-*\""));
        assert!(toml_str.contains("provider = \"gemini\""));
        assert!(toml_str.contains("model = \"gemini-pro\""));
    }

    #[test]
    fn test_get_user_config_path_contains_config_toml() {
        if let Some(path) = get_user_config_path() {
            assert!(path.ends_with("config.toml"));
        }
        // If None, the system doesn't support ProjectDirs (acceptable)
    }
}
