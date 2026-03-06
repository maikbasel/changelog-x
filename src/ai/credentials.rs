use std::fmt;

use keyring::Entry;

use crate::error::CredentialError;

const KEYRING_SERVICE: &str = "cgx";

/// Supported AI providers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
    Groq,
    DeepSeek,
}

impl Provider {
    /// All available providers (for select prompts)
    pub const ALL: &[Self] = &[
        Self::OpenAi,
        Self::Anthropic,
        Self::Gemini,
        Self::Ollama,
        Self::Groq,
        Self::DeepSeek,
    ];

    /// Config/keyring string representation
    #[must_use]
    pub const fn as_config_str(&self) -> &str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::Groq => "groq",
            Self::DeepSeek => "deepseek",
        }
    }

    /// Default model for this provider
    #[must_use]
    pub const fn default_model(&self) -> &str {
        match self {
            Self::OpenAi => "gpt-4o",
            Self::Anthropic => "claude-sonnet-4-20250514",
            Self::Gemini => "gemini-2.0-flash",
            Self::Ollama => "llama3.1",
            Self::Groq => "llama-3.3-70b-versatile",
            Self::DeepSeek => "deepseek-chat",
        }
    }

    /// Curated list of popular models for this provider.
    /// The first entry is the recommended default.
    #[must_use]
    pub fn popular_models(&self) -> Vec<&'static str> {
        match self {
            Self::OpenAi => vec!["gpt-4o", "gpt-4o-mini", "o3-mini", "gpt-4-turbo"],
            Self::Anthropic => vec![
                "claude-sonnet-4-20250514",
                "claude-haiku-4-5-20251001",
                "claude-opus-4-6",
            ],
            Self::Gemini => vec!["gemini-2.0-flash", "gemini-2.5-pro", "gemini-2.5-flash"],
            Self::Ollama => vec!["llama3.1", "llama3.2", "mistral", "codellama", "phi3"],
            Self::Groq => vec![
                "llama-3.3-70b-versatile",
                "llama-3.1-8b-instant",
                "mixtral-8x7b-32768",
            ],
            Self::DeepSeek => vec!["deepseek-chat", "deepseek-reasoner"],
        }
    }

    /// Standard environment variable name for this provider's API key.
    ///
    /// Returns `None` for providers that don't require an API key (e.g. Ollama).
    #[must_use]
    pub const fn env_var_name(&self) -> Option<&str> {
        match self {
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::Gemini => Some("GEMINI_API_KEY"),
            Self::Groq => Some("GROQ_API_KEY"),
            Self::DeepSeek => Some("DEEPSEEK_API_KEY"),
            Self::Ollama => None,
        }
    }

    /// The genai adapter namespace string used to construct model identifiers.
    #[must_use]
    pub const fn as_genai_adapter(&self) -> &str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::Groq => "groq",
            Self::DeepSeek => "deepseek",
        }
    }

    /// Whether this provider requires an API key
    #[must_use]
    pub const fn requires_api_key(&self) -> bool {
        !matches!(self, Self::Ollama)
    }

    /// Parse from config string
    #[must_use]
    pub fn from_config_str(s: &str) -> Option<Self> {
        match s {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            "ollama" => Some(Self::Ollama),
            "groq" => Some(Self::Groq),
            "deepseek" => Some(Self::DeepSeek),
            _ => None,
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAi => write!(f, "OpenAI"),
            Self::Anthropic => write!(f, "Anthropic"),
            Self::Gemini => write!(f, "Google Gemini"),
            Self::Ollama => write!(f, "Ollama (local)"),
            Self::Groq => write!(f, "Groq"),
            Self::DeepSeek => write!(f, "DeepSeek"),
        }
    }
}

/// Store an API key in the system keyring
///
/// # Errors
///
/// Returns `CredentialError::Store` if the keyring entry cannot be created or written.
pub fn store_api_key(provider: &Provider, api_key: &str) -> Result<(), CredentialError> {
    let entry = Entry::new(KEYRING_SERVICE, provider.as_config_str())
        .map_err(|e| CredentialError::Store(e.to_string()))?;
    entry
        .set_password(api_key)
        .map_err(|e| CredentialError::Store(e.to_string()))
}

/// Retrieve an API key from the system keyring
///
/// # Errors
///
/// Returns `CredentialError::Retrieve` if the keyring entry cannot be accessed,
/// or `CredentialError::NotFound` if no key is stored for this provider.
pub fn get_api_key(provider_name: &str) -> Result<String, CredentialError> {
    let entry = Entry::new(KEYRING_SERVICE, provider_name)
        .map_err(|e| CredentialError::Retrieve(e.to_string()))?;
    entry
        .get_password()
        .map_err(|e| CredentialError::NotFound(e.to_string()))
}

/// Check if an API key exists in the system keyring
#[must_use]
pub fn has_api_key(provider_name: &str) -> bool {
    match Entry::new(KEYRING_SERVICE, provider_name).and_then(|entry| entry.get_password()) {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!(provider = provider_name, error = %e, "Keyring check failed");
            false
        }
    }
}

/// Delete an API key from the system keyring
///
/// # Errors
///
/// Returns `CredentialError::Delete` if the keyring entry cannot be removed.
pub fn delete_api_key(provider_name: &str) -> Result<(), CredentialError> {
    let entry = Entry::new(KEYRING_SERVICE, provider_name)
        .map_err(|e| CredentialError::Delete(e.to_string()))?;
    entry
        .delete_credential()
        .map_err(|e| CredentialError::Delete(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_all_count() {
        assert_eq!(Provider::ALL.len(), 6);
    }

    #[test]
    fn test_provider_config_str_roundtrip() {
        for provider in Provider::ALL {
            let config_str = provider.as_config_str();
            let parsed = Provider::from_config_str(config_str);
            assert_eq!(parsed, Some(*provider));
        }
    }

    #[test]
    fn test_provider_from_config_str_unknown() {
        assert_eq!(Provider::from_config_str("unknown"), None);
    }

    #[test]
    fn test_provider_requires_api_key() {
        assert!(Provider::OpenAi.requires_api_key());
        assert!(Provider::Anthropic.requires_api_key());
        assert!(Provider::Gemini.requires_api_key());
        assert!(!Provider::Ollama.requires_api_key());
        assert!(Provider::Groq.requires_api_key());
        assert!(Provider::DeepSeek.requires_api_key());
    }

    #[test]
    fn test_provider_display() {
        assert_eq!(Provider::OpenAi.to_string(), "OpenAI");
        assert_eq!(Provider::Anthropic.to_string(), "Anthropic");
        assert_eq!(Provider::Gemini.to_string(), "Google Gemini");
        assert_eq!(Provider::Ollama.to_string(), "Ollama (local)");
        assert_eq!(Provider::Groq.to_string(), "Groq");
        assert_eq!(Provider::DeepSeek.to_string(), "DeepSeek");
    }

    #[test]
    fn test_provider_default_model_not_empty() {
        for provider in Provider::ALL {
            assert!(!provider.default_model().is_empty());
        }
    }

    #[test]
    fn test_provider_env_var_name() {
        assert_eq!(Provider::OpenAi.env_var_name(), Some("OPENAI_API_KEY"));
        assert_eq!(
            Provider::Anthropic.env_var_name(),
            Some("ANTHROPIC_API_KEY")
        );
        assert_eq!(Provider::Gemini.env_var_name(), Some("GEMINI_API_KEY"));
        assert_eq!(Provider::Groq.env_var_name(), Some("GROQ_API_KEY"));
        assert_eq!(Provider::DeepSeek.env_var_name(), Some("DEEPSEEK_API_KEY"));
        assert_eq!(Provider::Ollama.env_var_name(), None);
    }

    #[test]
    fn test_provider_as_genai_adapter() {
        assert_eq!(Provider::OpenAi.as_genai_adapter(), "openai");
        assert_eq!(Provider::Anthropic.as_genai_adapter(), "anthropic");
        assert_eq!(Provider::Gemini.as_genai_adapter(), "gemini");
        assert_eq!(Provider::Ollama.as_genai_adapter(), "ollama");
        assert_eq!(Provider::Groq.as_genai_adapter(), "groq");
        assert_eq!(Provider::DeepSeek.as_genai_adapter(), "deepseek");
    }
}
