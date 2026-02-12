use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec};
use genai::resolver::{AuthData, AuthResolver};
use genai::{Client, ModelIden};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};
use tracing::debug;

use crate::ai::context::{self, CommitSummary};
use crate::ai::credentials::{self, Provider};
use crate::config::{AiConfig, ChangelogFormat};
use crate::error::AiError;

// ---------------------------------------------------------------------------
// Structured AI output types
// ---------------------------------------------------------------------------

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct EnhancedChangelog {
    releases: Vec<ChangelogRelease>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct ChangelogRelease {
    /// Version heading as-is, e.g. "[0.1.1] - 2024-01-15" or "Unreleased"
    heading: String,
    sections: Vec<ChangelogSection>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct ChangelogSection {
    /// Section name: Added, Changed, Deprecated, Removed, Fixed, or Security
    name: String,
    entries: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tera templates
// ---------------------------------------------------------------------------

const KEEP_A_CHANGELOG_TEMPLATE: &str = "\
{%- for release in releases %}
## {{ release.heading }}
{% for section in release.sections %}
### {{ section.name }}
{% for entry in section.entries %}
- {{ entry }}
{%- endfor %}
{% endfor %}
{%- endfor %}";

const COMMON_CHANGELOG_TEMPLATE: &str = "\
{%- for release in releases %}
## {{ release.heading }}
{% for section in release.sections %}
### {{ section.name }}
{% for entry in section.entries %}
- {{ entry }}
{%- endfor %}
{% endfor %}
{%- endfor %}";

// ---------------------------------------------------------------------------
// Section ordering and normalization
// ---------------------------------------------------------------------------

/// Canonical section order for Keep a Changelog
const KAC_SECTION_ORDER: &[&str] = &[
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];

/// Canonical section order for Common Changelog
const CC_SECTION_ORDER: &[&str] = &["Changed", "Added", "Removed", "Fixed"];

/// Normalize a section name to the closest Keep a Changelog name.
fn normalize_section_name(name: &str) -> &'static str {
    match name.to_lowercase().trim() {
        s if s == "added" || s == "features" || s == "new" => "Added",
        s if s == "changed"
            || s == "breaking changes"
            || s == "performance"
            || s == "refactoring"
            || s == "improvements" =>
        {
            "Changed"
        }
        s if s == "deprecated" || s == "deprecations" => "Deprecated",
        s if s == "removed" || s == "removals" => "Removed",
        s if s == "fixed" || s == "bug fixes" || s == "bugfixes" => "Fixed",
        "security" => "Security",
        _ => "Changed",
    }
}

/// Remap sections for Common Changelog (only 4 allowed).
fn remap_for_common_changelog(name: &str) -> &str {
    match name {
        "Deprecated" => "Changed",
        "Security" => "Fixed",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render an `EnhancedChangelog` to markdown using the given format.
fn render_markdown(
    changelog: &EnhancedChangelog,
    format: &ChangelogFormat,
) -> Result<String, AiError> {
    let (section_order, template_name, template_src) = match format {
        ChangelogFormat::KeepAChangelog => (
            KAC_SECTION_ORDER,
            "keep-a-changelog",
            KEEP_A_CHANGELOG_TEMPLATE,
        ),
        ChangelogFormat::CommonChangelog => (
            CC_SECTION_ORDER,
            "common-changelog",
            COMMON_CHANGELOG_TEMPLATE,
        ),
    };

    // Normalize, remap, merge, and reorder sections
    let normalized = normalize_changelog(changelog, format, section_order);

    let norm_sections: usize = normalized.releases.iter().map(|r| r.sections.len()).sum();
    let norm_entries: usize = normalized
        .releases
        .iter()
        .flat_map(|r| &r.sections)
        .map(|s| s.entries.len())
        .sum();
    debug!(
        format = ?format,
        sections = norm_sections,
        entries = norm_entries,
        "Normalized changelog structure"
    );

    let mut tera = Tera::default();
    tera.add_raw_template(template_name, template_src)
        .map_err(|e| AiError::InvalidResponse(format!("Template error: {e}")))?;

    let ctx = Context::from_serialize(&normalized)
        .map_err(|e| AiError::InvalidResponse(format!("Serialization error: {e}")))?;

    tera.render(template_name, &ctx)
        .map(|s| {
            let output = s.trim().to_string();
            debug!(output_len = output.len(), "Rendered markdown output");
            output
        })
        .map_err(|e| AiError::InvalidResponse(format!("Render error: {e}")))
}

/// Normalize section names, remap for format, merge duplicates, and reorder.
fn normalize_changelog(
    changelog: &EnhancedChangelog,
    format: &ChangelogFormat,
    section_order: &[&str],
) -> EnhancedChangelog {
    let releases = changelog
        .releases
        .iter()
        .map(|release| {
            // Collect entries by normalized section name
            let mut merged: indexmap::IndexMap<&str, Vec<String>> = indexmap::IndexMap::new();

            for section in &release.sections {
                let mut name = normalize_section_name(&section.name);

                if *format == ChangelogFormat::CommonChangelog {
                    name = remap_for_common_changelog(name);
                }

                if name != section.name {
                    debug!(
                        original = section.name.as_str(),
                        normalized = name,
                        "Remapped section name"
                    );
                }

                merged
                    .entry(name)
                    .or_default()
                    .extend(section.entries.clone());
            }

            // Reorder by canonical order, drop sections not in the format
            // or with no entries (AI sometimes returns empty sections)
            let sections = section_order
                .iter()
                .filter_map(|&ordered_name| {
                    merged
                        .swap_remove(ordered_name)
                        .filter(|entries| !entries.is_empty())
                        .map(|entries| ChangelogSection {
                            name: ordered_name.to_string(),
                            entries,
                        })
                })
                .collect();

            if !merged.is_empty() {
                let dropped: Vec<&str> = merged.keys().copied().collect();
                debug!(
                    release = release.heading.as_str(),
                    dropped_sections = ?dropped,
                    "Dropped sections not in canonical order"
                );
            }

            ChangelogRelease {
                heading: release.heading.clone(),
                sections,
            }
        })
        .collect();

    EnhancedChangelog { releases }
}

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

fn build_system_prompt(schema_json: &str) -> String {
    format!(
        "\
You are a technical writer producing concise, developer-facing changelogs. You receive \
raw changelog entries from git-cliff and return clean, structured JSON.

<rules>
RELEASE STRUCTURE
- Keep every release heading exactly as provided (e.g. \"[Unreleased]\", \
\"[0.2.0] - 2024-03-15\", a full markdown link).
- Never invent, rename, merge, or remove release headings.
- Never add releases that do not exist in the input.

FILTER (critical — apply before all other steps)
Delete these entries completely. They must NOT appear in the output in any form — do not \
rewrite them, do not reclassify them, do not move them to another section. Drop them:
- CI/CD: GitHub Actions, GitLab CI, Jenkins, workflows, release pipelines, PAT tokens.
- Infrastructure: Docker, Kubernetes, Terraform, Helm, Nginx, Coolify, compose files.
- Dependency bumps: lockfile updates, \"bump X from Y to Z\", dependency group updates, \
audit deps. This includes entries in a \"Dependencies\" section.
- Dev tooling: linting config, pre-commit hooks, editor settings, formatting config, \
coding guidelines, test configuration.
- Repository housekeeping: release workflows, branch policies, migration scripts.
- Project scaffolding: initial skeleton setup, boilerplate generation, \"init\" commits.
- Entries whose conventional-commit scope is ci, cd, deps, build, init, or docker — \
regardless of the commit type or the section git-cliff placed them in (including \
fix(ci):, feat(ci):, chore(deps):, feat(init):). \
A \"Bug Fix\" with scope ci is NOT a bug fix — drop it. \
A \"Feature\" with scope init is NOT a feature — drop it.
When a GIT COMMIT CONTEXT section is provided, cross-reference commit scopes to confirm \
filtering decisions.

MERGE RELATED ENTRIES
Combine commits that contribute to the same feature or bug fix into one entry:
- Multiple WIP commits for one feature → one final entry describing the completed feature.
- Incremental improvements to the same area → one entry covering the end result.
- A feature commit plus its tracing/logging/test commits → one entry about the feature.

CLASSIFY (Keep a Changelog sections)
Map remaining entries to exactly one of: Added, Changed, Deprecated, Removed, Fixed, Security.
- New capabilities → Added
- Modifications to existing behavior, refactoring, performance → Changed
- Deprecated features → Deprecated
- Removed features → Removed
- Bug fixes → Fixed
- Security patches → Security
Only include sections that have at least one entry after filtering. \
If filtering removes all entries in a section, omit that section entirely. \
Never output a section with an empty entries array.

WRITE EACH ENTRY
- One short sentence, at most 15 words.
- Start with an imperative verb: Add, Fix, Improve, Remove, Update, Deprecate.
- Prefix every entry with a bold scope inferred from its content: **ai:**, **ui:**, \
**config:**, **backend:**, **frontend:**, **api:**, **core:**, etc.
- Use **core:** for general or project-wide changes.
- No file paths, function names, or internal implementation details.
- Preserve PR/issue references (#123) and author attributions when present.
- No markdown formatting inside entries except the bold scope prefix.
</rules>

<example>
INPUT:
## [Unreleased]

### Bug Fixes
- **ci:** Downgrade GitHub Actions versions to match cargo-dist v0.30.3

### Dependencies
- Bump the rust-dependencies group with 5 updates

### Features
- **ai:** Add model suggestions with autocomplete to setup prompt
- **ai:** Implement setup, status, and auth subcommands for AI configuration
- Add progress pipeline UI and unreleased flag conflict tests
- Implement config subcommands
- **init:** Setup project skeleton

### Miscellaneous Tasks
- Add PAT to release please workflow
- Create release workflow

OUTPUT:
{{{{
  \"releases\": [
    {{{{
      \"heading\": \"[Unreleased]\",
      \"sections\": [
        {{{{
          \"name\": \"Added\",
          \"entries\": [
            \"**ai:** Add setup, status, and auth subcommands with model autocomplete\",
            \"**ui:** Add progress pipeline and config subcommands\"
          ]
        }}}}
      ]
    }}}}
  ]
}}}}
</example>

JSON SCHEMA:
{schema_json}

Return ONLY valid JSON matching the schema. No commentary, no explanation, no code fences."
    )
}

const CONTEXT_ADDENDUM: &str = "\n\n\
A GIT COMMIT CONTEXT section is appended to the user message. Use it to understand \
the intent behind entries and to confirm filtering decisions. If a commit scope is \
ci, cd, deps, build, or docker, drop the corresponding changelog entry. \
Do not include commit context in the JSON output.";

// ---------------------------------------------------------------------------
// AiEnhancer
// ---------------------------------------------------------------------------

/// Enhances changelog content using AI for improved readability
pub struct AiEnhancer {
    config: AiConfig,
}

impl AiEnhancer {
    /// Create a new AI enhancer with the given configuration
    #[must_use]
    pub const fn new(config: AiConfig) -> Self {
        Self { config }
    }

    /// Check if AI enhancement is configured (provider is set)
    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.config.is_configured()
    }

    /// Enhance the changelog content using AI
    ///
    /// The AI returns structured JSON which is parsed and rendered to markdown
    /// using the specified changelog format.
    ///
    /// # Errors
    ///
    /// Returns `AiError::NotConfigured` if no AI provider is configured.
    /// Returns `AiError::Connection` if the provider cannot be reached.
    /// Returns `AiError::InvalidResponse` if the AI returns unparseable output.
    pub async fn enhance(
        &self,
        changelog: &str,
        commit_context: Option<&[CommitSummary]>,
        format: &ChangelogFormat,
    ) -> Result<String, AiError> {
        let provider_name = self
            .config
            .provider
            .as_deref()
            .ok_or(AiError::NotConfigured)?;

        let provider = Provider::from_config_str(provider_name)
            .ok_or_else(|| AiError::Request(format!("Unknown AI provider: {provider_name}")))?;

        let model_name = self
            .config
            .model
            .as_deref()
            .unwrap_or_else(|| provider.default_model());

        debug!(
            provider = provider_name,
            model = model_name,
            "Starting AI enhancement"
        );

        // Generate JSON schema for the response format
        let schema = schemars::schema_for!(EnhancedChangelog);
        let schema_json = serde_json::to_string_pretty(&schema).unwrap_or_default();

        // Build auth resolver: env var first, then keyring fallback
        let auth_resolver = build_auth_resolver(provider);

        let client = Client::builder().with_auth_resolver(auth_resolver).build();

        let model_iden = format!("{}::{}", provider.as_genai_adapter(), model_name);

        // Build system prompt — embed schema and append context addendum when commits provided
        let has_context = commit_context.is_some_and(|c| !c.is_empty());
        let commit_count = commit_context.map_or(0, <[CommitSummary]>::len);
        let system_prompt = if has_context {
            format!("{}{CONTEXT_ADDENDUM}", build_system_prompt(&schema_json))
        } else {
            build_system_prompt(&schema_json)
        };

        debug!(
            system_prompt_len = system_prompt.len(),
            has_commit_context = has_context,
            commit_count = commit_count,
            "Built system prompt"
        );

        // Build user message — wrap changelog in tags, append commit history
        let user_message = commit_context.filter(|c| !c.is_empty()).map_or_else(
            || format!("<changelog>\n{changelog}\n</changelog>"),
            |commits| {
                let mut msg = format!("<changelog>\n{changelog}\n</changelog>");
                msg.push_str(&context::format_commit_context(commits));
                msg
            },
        );

        debug!(
            changelog_len = changelog.len(),
            user_message_len = user_message.len(),
            "Built user message"
        );

        // Build chat options with JSON schema response format
        let schema_value: serde_json::Value = serde_json::to_value(&schema)
            .map_err(|e| AiError::Request(format!("Failed to serialize schema: {e}")))?;

        let json_spec = JsonSpec::new("enhanced_changelog", schema_value);
        let chat_options = ChatOptions::default().with_response_format(json_spec);

        let chat_req = ChatRequest::default()
            .with_system(system_prompt)
            .append_message(ChatMessage::user(user_message));

        debug!(
            model_iden = model_iden,
            has_context = has_context,
            "Sending chat request"
        );

        let response = client
            .exec_chat(&model_iden, chat_req, Some(&chat_options))
            .await?;

        let raw_text = response
            .into_first_text()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| AiError::InvalidResponse("AI returned an empty response".to_string()))?;

        debug!(raw_len = raw_text.len(), "Received AI response");

        // Strip markdown code fences if the model wraps the JSON
        let json_str = strip_code_fences(&raw_text);

        let enhanced: EnhancedChangelog = serde_json::from_str(json_str).map_err(|e| {
            AiError::InvalidResponse(format!(
                "Failed to parse AI JSON response: {e}\nRaw response:\n{raw_text}"
            ))
        })?;

        let total_sections: usize = enhanced.releases.iter().map(|r| r.sections.len()).sum();
        let total_entries: usize = enhanced
            .releases
            .iter()
            .flat_map(|r| &r.sections)
            .map(|s| s.entries.len())
            .sum();
        debug!(
            release_count = enhanced.releases.len(),
            total_sections = total_sections,
            total_entries = total_entries,
            "Parsed AI response structure"
        );

        render_markdown(&enhanced, format)
    }
}

/// Strip optional markdown code fences (```json ... ```) from a string.
fn strip_code_fences(text: &str) -> &str {
    let trimmed = text.trim();

    // Try to strip ```json ... ``` or ``` ... ```
    let inner = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"));

    let stripped = inner.is_some();
    debug!(stripped = stripped, "Code fence stripping");

    inner.map_or(trimmed, str::trim)
}

/// Build an `AuthResolver` that checks the environment variable first,
/// then falls back to the system keyring.
fn build_auth_resolver(provider: Provider) -> AuthResolver {
    AuthResolver::from_resolver_fn(
        move |_model_iden: ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> {
            // Ollama doesn't need auth
            let Some(env_name) = provider.env_var_name() else {
                return Ok(None);
            };

            // 1. Try environment variable
            if let Ok(key) = std::env::var(env_name) {
                debug!(env_var = env_name, "Using API key from environment");
                return Ok(Some(AuthData::from_single(key)));
            }

            // 2. Fall back to system keyring
            credentials::get_api_key(provider.as_config_str()).map_or_else(
                |_| {
                    Err(genai::resolver::Error::ApiKeyEnvNotFound {
                        env_name: env_name.to_string(),
                    })
                },
                |key| {
                    debug!(
                        provider = provider.as_config_str(),
                        "Using API key from keyring"
                    );
                    Ok(Some(AuthData::from_single(key)))
                },
            )
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_section_name_features() {
        assert_eq!(normalize_section_name("Features"), "Added");
        assert_eq!(normalize_section_name("features"), "Added");
        assert_eq!(normalize_section_name("New"), "Added");
    }

    #[test]
    fn test_normalize_section_name_bug_fixes() {
        assert_eq!(normalize_section_name("Bug Fixes"), "Fixed");
        assert_eq!(normalize_section_name("bug fixes"), "Fixed");
        assert_eq!(normalize_section_name("Bugfixes"), "Fixed");
    }

    #[test]
    fn test_normalize_section_name_changed() {
        assert_eq!(normalize_section_name("Performance"), "Changed");
        assert_eq!(normalize_section_name("Breaking Changes"), "Changed");
        assert_eq!(normalize_section_name("Refactoring"), "Changed");
    }

    #[test]
    fn test_normalize_section_name_deprecated() {
        assert_eq!(normalize_section_name("Deprecated"), "Deprecated");
        assert_eq!(normalize_section_name("Deprecations"), "Deprecated");
    }

    #[test]
    fn test_normalize_section_name_removed() {
        assert_eq!(normalize_section_name("Removed"), "Removed");
        assert_eq!(normalize_section_name("Removals"), "Removed");
    }

    #[test]
    fn test_normalize_section_name_security() {
        assert_eq!(normalize_section_name("Security"), "Security");
    }

    #[test]
    fn test_normalize_section_name_unknown_defaults_to_changed() {
        assert_eq!(normalize_section_name("Miscellaneous"), "Changed");
        assert_eq!(normalize_section_name("Other"), "Changed");
    }

    #[test]
    fn test_remap_for_common_changelog() {
        assert_eq!(remap_for_common_changelog("Deprecated"), "Changed");
        assert_eq!(remap_for_common_changelog("Security"), "Fixed");
        assert_eq!(remap_for_common_changelog("Added"), "Added");
        assert_eq!(remap_for_common_changelog("Changed"), "Changed");
    }

    #[test]
    fn test_strip_code_fences_json() {
        let input = "```json\n{\"releases\": []}\n```";
        assert_eq!(strip_code_fences(input), "{\"releases\": []}");
    }

    #[test]
    fn test_strip_code_fences_plain() {
        let input = "```\n{\"releases\": []}\n```";
        assert_eq!(strip_code_fences(input), "{\"releases\": []}");
    }

    #[test]
    fn test_strip_code_fences_none() {
        let input = "{\"releases\": []}";
        assert_eq!(strip_code_fences(input), "{\"releases\": []}");
    }

    #[test]
    fn test_render_keep_a_changelog() {
        let changelog = EnhancedChangelog {
            releases: vec![ChangelogRelease {
                heading: "[1.0.0] - 2024-01-15".to_string(),
                sections: vec![
                    ChangelogSection {
                        name: "Added".to_string(),
                        entries: vec!["Add OAuth2 login flow (#42)".to_string()],
                    },
                    ChangelogSection {
                        name: "Fixed".to_string(),
                        entries: vec!["Fix crash on empty input (#38)".to_string()],
                    },
                ],
            }],
        };

        let result = render_markdown(&changelog, &ChangelogFormat::KeepAChangelog);
        assert!(result.is_ok());
        let md = result.unwrap_or_default();
        assert!(md.contains("## [1.0.0] - 2024-01-15"));
        assert!(md.contains("### Added"));
        assert!(md.contains("### Fixed"));
        assert!(md.contains("- Add OAuth2 login flow (#42)"));
        assert!(md.contains("- Fix crash on empty input (#38)"));
    }

    #[test]
    fn test_render_common_changelog_remaps_deprecated() {
        let changelog = EnhancedChangelog {
            releases: vec![ChangelogRelease {
                heading: "[1.0.0] - 2024-01-15".to_string(),
                sections: vec![
                    ChangelogSection {
                        name: "Deprecated".to_string(),
                        entries: vec!["Deprecate old API endpoint".to_string()],
                    },
                    ChangelogSection {
                        name: "Security".to_string(),
                        entries: vec!["Fix XSS vulnerability".to_string()],
                    },
                ],
            }],
        };

        let result = render_markdown(&changelog, &ChangelogFormat::CommonChangelog);
        assert!(result.is_ok());
        let md = result.unwrap_or_default();
        // Deprecated → Changed, Security → Fixed
        assert!(md.contains("### Changed"));
        assert!(md.contains("- Deprecate old API endpoint"));
        assert!(md.contains("### Fixed"));
        assert!(md.contains("- Fix XSS vulnerability"));
        assert!(!md.contains("### Deprecated"));
        assert!(!md.contains("### Security"));
    }

    #[test]
    fn test_render_section_ordering_kac() {
        let changelog = EnhancedChangelog {
            releases: vec![ChangelogRelease {
                heading: "Unreleased".to_string(),
                sections: vec![
                    ChangelogSection {
                        name: "Fixed".to_string(),
                        entries: vec!["Fix a bug".to_string()],
                    },
                    ChangelogSection {
                        name: "Added".to_string(),
                        entries: vec!["Add a feature".to_string()],
                    },
                ],
            }],
        };

        let result = render_markdown(&changelog, &ChangelogFormat::KeepAChangelog);
        assert!(result.is_ok());
        let md = result.unwrap_or_default();
        // Added should come before Fixed in KAC order
        let added_pos = md.find("### Added");
        let fixed_pos = md.find("### Fixed");
        assert!(added_pos < fixed_pos);
    }

    #[test]
    fn test_normalize_merges_duplicate_sections() {
        let changelog = EnhancedChangelog {
            releases: vec![ChangelogRelease {
                heading: "Unreleased".to_string(),
                sections: vec![
                    ChangelogSection {
                        name: "Features".to_string(),
                        entries: vec!["Add feature A".to_string()],
                    },
                    ChangelogSection {
                        name: "New".to_string(),
                        entries: vec!["Add feature B".to_string()],
                    },
                ],
            }],
        };

        let result = render_markdown(&changelog, &ChangelogFormat::KeepAChangelog);
        assert!(result.is_ok());
        let md = result.unwrap_or_default();
        // Both should be merged into "Added"
        assert_eq!(md.matches("### Added").count(), 1);
        assert!(md.contains("- Add feature A"));
        assert!(md.contains("- Add feature B"));
    }

    #[test]
    fn test_schema_generation() {
        let schema = schemars::schema_for!(EnhancedChangelog);
        let json = serde_json::to_string(&schema);
        assert!(json.is_ok());
        let s = json.unwrap_or_default();
        assert!(s.contains("releases"));
        assert!(s.contains("heading"));
        assert!(s.contains("sections"));
        assert!(s.contains("entries"));
    }
}
