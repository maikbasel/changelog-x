use crate::config::AiConfig;
use crate::error::AiError;

/// Enhances changelog content using AI for improved readability
pub struct AiEnhancer {
    config: AiConfig,
}

impl AiEnhancer {
    /// Create a new AI enhancer with the given configuration
    pub fn new(config: AiConfig) -> Self {
        Self { config }
    }

    /// Check if AI enhancement is configured (provider is set)
    pub fn is_available(&self) -> bool {
        self.config.is_configured()
    }

    /// Enhance the changelog content using AI
    pub async fn enhance(&self, _changelog: &str) -> Result<String, AiError> {
        if !self.config.is_configured() {
            return Err(AiError::NotConfigured);
        }

        // TODO: Implement AI enhancement using genai crate
        // This will:
        // 1. Connect to the configured AI provider
        // 2. Send the changelog for enhancement
        // 3. Return the improved version
        Err(AiError::NotConfigured)
    }
}
