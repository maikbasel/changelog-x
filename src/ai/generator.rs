use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec};
use genai::resolver::{AuthData, AuthResolver};
use genai::{Client, ModelIden};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};
use tracing::{debug, warn};

use crate::ai::commit_data::ReleaseData;
use crate::ai::context::ProjectContext;
use crate::ai::credentials::{self, Provider};
use crate::config::{AiConfig, ChangelogFormat};
use crate::error::AiError;

/// Default sampling temperature when not set in config.
const DEFAULT_TEMPERATURE: f64 = 0.3;

// ---------------------------------------------------------------------------
// Structured AI output types (shared with rendering)
// ---------------------------------------------------------------------------

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct EnhancedChangelog {
    pub releases: Vec<ChangelogRelease>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ChangelogRelease {
    /// Version heading as-is, e.g. "[0.1.1] - 2024-01-15" or "Unreleased"
    pub heading: String,
    pub sections: Vec<ChangelogSection>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ChangelogSection {
    /// Section name: Added, Changed, Deprecated, Removed, Fixed, or Security
    pub name: String,
    pub entries: Vec<String>,
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
pub fn render_markdown(
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
// AI prompts for direct generation from commit data
// ---------------------------------------------------------------------------

/// System prompt for Step 1: commit analysis from structured data.
fn build_analysis_prompt(project: Option<&ProjectContext>) -> String {
    use std::fmt::Write;

    let mut prompt = String::new();
    if let Some(ctx) = project {
        let _ = write!(
            prompt,
            "You are analyzing changes for \"{}\", {}. This is a {}.\n\n",
            ctx.name,
            ctx.description.as_deref().unwrap_or("a software project"),
            ctx.project_type,
        );

        if let Some(ref readme) = ctx.readme_summary {
            let _ = write!(prompt, "README excerpt:\n{readme}\n\n");
        }

        if !ctx.doc_summaries.is_empty() {
            let _ = writeln!(prompt, "Documentation excerpts:");
            for doc in &ctx.doc_summaries {
                let _ = writeln!(prompt, "- {doc}");
            }
            prompt.push('\n');
        }

        if let Some(ref ai_ctx) = ctx.ai_instructions {
            let _ = write!(prompt, "Project conventions:\n{ai_ctx}\n\n");
        }
    }

    prompt.push_str(
        "\
You are a senior software engineer analyzing git commits for a changelog.

You receive structured JSON commit data with types, scopes, subjects, bodies, \
diff stats, and breaking flags. Use all of this information in your analysis.

Produce a structured analysis covering:

1. USER-FACING CHANGES — List every change that matters to end-users or developers \
consuming this project. For each, note the relevant commit and why it matters. \
Use diff stats to gauge significance (large diffs = major changes).

2. GROUPING — Identify commits that belong to the same feature or fix. \
Use diff stats to find commits touching the same files. List each group \
with its member commits and the single theme that unifies them.

3. BREAKING CHANGES & SECURITY — Flag any commits with breaking=true or \
security fixes explicitly. Quote the relevant commit subjects.

4. NOISE TO FILTER — List commits that should be dropped from the final changelog:
   - CI/CD (GitHub Actions, GitLab CI, workflows, release pipelines)
   - Infrastructure (Docker, Kubernetes, Terraform, Helm, Nginx, compose)
   - Dependency bumps (lockfile updates, \"bump X from Y to Z\")
   - Dev tooling (linting config, pre-commit hooks, editor settings, formatting)
   - Repository housekeeping (release workflows, branch policies, migration scripts)
   - Project scaffolding (initial skeleton setup, boilerplate, \"init\" commits)
   - Commits with type ci, cd, deps, build, init, or docker — \
regardless of section. A fix(ci) is NOT a bug fix. A feat(init) is NOT a feature.
   - Non-conventional commits that are clearly infrastructure or tooling.

Write your analysis as plain text. Be thorough but concise.",
    );

    prompt
}

/// System prompt for Step 2: changelog writing with JSON output from commit data.
fn build_writing_prompt(schema_json: &str, project: Option<&ProjectContext>) -> String {
    let identity = project.map_or_else(String::new, |ctx| {
        format!(
            "You are writing a changelog for \"{}\", {}. This is a {}.\n\n",
            ctx.name,
            ctx.description.as_deref().unwrap_or("a software project"),
            ctx.project_type,
        )
    });

    format!(
        "\
{identity}You are an expert technical writer producing polished, user-facing changelogs.

You receive two inputs:
1. Structured JSON commit data with types, scopes, subjects, bodies, diff stats, \
and breaking change flags.
2. An analysis from a senior engineer identifying user-facing changes, groupings, \
noise to filter, and breaking/security items.

Your job is to transform these into a clean, structured JSON changelog.

<rules>
RELEASE STRUCTURE
- For each release in the commit data, create a heading:
  - If version is present: \"[VERSION] - YYYY-MM-DD\" (derive date from timestamp)
  - If version is null: \"[Unreleased]\"
- Never invent, rename, merge, or remove releases.
- Never add releases that do not exist in the input.

REWRITING RULES
- Merge aggressively: combine related commits identified in the analysis into single entries.
- Write for users, not developers. Describe what changed from the user's perspective.
- Use imperative present tense: Add, Fix, Improve, Remove, Update, Deprecate.
- Drop all noise identified in the analysis. These entries must NOT appear in any form.
- Elevate breaking changes: prefix with **BREAKING:** in the entry text.
- Elevate security fixes: ensure they land in the Security section.
- Add context when the commit subject is cryptic — make it understandable without reading code.
- Preserve precision: do not lose specific details (version numbers, flag names, API changes).
- Use commit bodies and diff stats to enrich entries with context.

CLASSIFY (Keep a Changelog sections)
Map entries to exactly one of: Added, Changed, Deprecated, Removed, Fixed, Security.
- feat commits, new capabilities -> Added
- Modifications to existing behavior, refactoring, performance -> Changed
- Deprecated features -> Deprecated
- Removed features -> Removed
- fix commits, bug fixes -> Fixed
- Security patches -> Security
Only include sections with at least one entry. Never output an empty entries array.

WRITE EACH ENTRY
- One short sentence, at most 15 words.
- Prefix every entry with a bold scope: **ai:**, **ui:**, **config:**, **core:**, etc.
- Use the commit scope if available, otherwise infer from changed files or use **core:**.
- No file paths, function names, or internal implementation details.
- Preserve PR/issue references (#123) and author attributions when present.
- No markdown formatting inside entries except the bold scope prefix and BREAKING: prefix.

TONE & STYLE
- Clear, direct, professional. Consistent voice throughout.
- No filler words, no hype, no marketing language.
</rules>

JSON SCHEMA:
{schema_json}

Return ONLY valid JSON matching the schema. No commentary, no explanation, no code fences."
    )
}

// ---------------------------------------------------------------------------
// AiGenerator
// ---------------------------------------------------------------------------

/// Generates changelogs directly from structured commit data using AI.
pub struct AiGenerator {
    config: AiConfig,
}

impl AiGenerator {
    #[must_use]
    pub const fn new(config: AiConfig) -> Self {
        Self { config }
    }

    /// Generate a changelog from structured release/commit data using a two-step
    /// AI pipeline.
    ///
    /// **Step 1 — Analysis:** Reasons about the commit data and project context.
    /// **Step 2 — Writing:** Transforms commits + analysis into structured JSON.
    ///
    /// # Errors
    ///
    /// Returns `AiError::NotConfigured` if no AI provider is configured.
    /// Returns `AiError::Connection` if the provider cannot be reached.
    /// Returns `AiError::InvalidResponse` if the AI returns unparseable output.
    #[allow(clippy::future_not_send)]
    pub async fn generate(
        &self,
        releases: &[ReleaseData],
        project_context: Option<&ProjectContext>,
        format: &ChangelogFormat,
        on_step_completed: Option<&(dyn Fn() + Send + Sync)>,
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

        let temperature = self.config.temperature.unwrap_or(DEFAULT_TEMPERATURE);

        debug!(
            provider = provider_name,
            model = model_name,
            temperature = temperature,
            releases = releases.len(),
            "Starting two-step AI generation from commit data"
        );

        let auth_resolver = build_auth_resolver(provider);
        let client = Client::builder().with_auth_resolver(auth_resolver).build();
        let model_iden = format!("{}::{}", provider.as_genai_adapter(), model_name);

        // Serialize commit data as JSON
        let commits_json = serde_json::to_string_pretty(releases)
            .map_err(|e| AiError::Request(format!("Failed to serialize commit data: {e}")))?;

        let user_message = format!("<commits>\n{commits_json}\n</commits>");

        // -- Step 1: Analysis --------------------------------------------------
        let analysis_system = build_analysis_prompt(project_context);
        let analysis_options = ChatOptions::default().with_temperature(temperature);

        let analysis_req = ChatRequest::default()
            .with_system(analysis_system)
            .append_message(ChatMessage::user(&user_message));

        debug!(model_iden = model_iden, "Sending analysis request (step 1)");

        let analysis_response = client
            .exec_chat(&model_iden, analysis_req, Some(&analysis_options))
            .await?;

        let analysis_text = extract_response_text(analysis_response, "analysis")?;

        debug!(
            analysis_len = analysis_text.len(),
            "Received analysis (step 1)"
        );

        if let Some(cb) = on_step_completed {
            cb();
        }

        // -- Step 2: Writing (JSON) --------------------------------------------
        let schema = schemars::schema_for!(EnhancedChangelog);
        let schema_json = serde_json::to_string_pretty(&schema).unwrap_or_default();
        let writing_system = build_writing_prompt(&schema_json, project_context);

        let schema_value: serde_json::Value = serde_json::to_value(&schema)
            .map_err(|e| AiError::Request(format!("Failed to serialize schema: {e}")))?;

        let json_spec = JsonSpec::new("enhanced_changelog", schema_value);
        let writing_options = ChatOptions::default()
            .with_temperature(temperature)
            .with_response_format(json_spec);

        let writing_user_message = format!(
            "<commits>\n{commits_json}\n</commits>\n\n\
             <analysis>\n{analysis_text}\n</analysis>"
        );

        let writing_req = ChatRequest::default()
            .with_system(writing_system)
            .append_message(ChatMessage::user(writing_user_message));

        debug!(model_iden = model_iden, "Sending writing request (step 2)");

        let writing_response = client
            .exec_chat(&model_iden, writing_req, Some(&writing_options))
            .await?;

        let raw_text = extract_response_text(writing_response, "writing")?;
        debug!(
            raw_len = raw_text.len(),
            "Received writing response (step 2)"
        );

        let enhanced = parse_enhanced_changelog(&raw_text)?;

        render_markdown(&enhanced, format)
    }
}

// ---------------------------------------------------------------------------
// AiEnhancer (post-processing existing markdown — kept for `cgx generate --ai`)
// ---------------------------------------------------------------------------

/// System prompt for Step 1: analysis of existing markdown changelog.
const ENHANCER_ANALYSIS_PROMPT_BASE: &str = "\
You are a senior software engineer analyzing git commits for a changelog.

Given raw changelog entries, produce a structured analysis covering:

1. USER-FACING CHANGES — List every change that matters to end-users or developers \
consuming this project. For each, note the relevant changelog entry and why it matters.

2. GROUPING — Identify commits that belong to the same feature or fix. List each group \
with its member entries and the single theme that unifies them.

3. BREAKING CHANGES & SECURITY — Flag any breaking changes or security fixes explicitly. \
Quote the relevant entries.

4. NOISE TO FILTER — List entries that should be dropped from the final changelog:
   - CI/CD (GitHub Actions, GitLab CI, workflows, release pipelines, PAT tokens)
   - Infrastructure (Docker, Kubernetes, Terraform, Helm, Nginx, Coolify, compose)
   - Dependency bumps (lockfile updates, \"bump X from Y to Z\", dependency groups)
   - Dev tooling (linting config, pre-commit hooks, editor settings, formatting)
   - Repository housekeeping (release workflows, branch policies, migration scripts)
   - Project scaffolding (initial skeleton setup, boilerplate, \"init\" commits)
   - Entries with conventional-commit scope ci, cd, deps, build, init, or docker — \
regardless of type or section. A fix(ci) is NOT a bug fix. A feat(init) is NOT a feature.

Write your analysis as plain text. Be thorough but concise.";

fn build_enhancer_analysis_prompt(project: Option<&ProjectContext>) -> String {
    use std::fmt::Write;

    let mut prompt = String::new();
    if let Some(ctx) = project {
        let _ = write!(
            prompt,
            "You are analyzing changes for \"{}\", {}. This is a {}.\n\n",
            ctx.name,
            ctx.description.as_deref().unwrap_or("a software project"),
            ctx.project_type,
        );
    }
    prompt.push_str(ENHANCER_ANALYSIS_PROMPT_BASE);
    prompt
}

fn build_enhancer_writing_prompt(schema_json: &str, project: Option<&ProjectContext>) -> String {
    let identity = project.map_or_else(String::new, |ctx| {
        format!(
            "You are writing a changelog for \"{}\", {}. This is a {}.\n\n",
            ctx.name,
            ctx.description.as_deref().unwrap_or("a software project"),
            ctx.project_type,
        )
    });

    format!(
        "\
{identity}You are an expert technical writer producing polished, user-facing changelogs.

You receive two inputs:
1. The original raw changelog entries.
2. An analysis from a senior engineer identifying user-facing changes, groupings, \
noise to filter, and breaking/security items.

Your job is to transform these into a clean, structured JSON changelog.

<rules>
RELEASE STRUCTURE
- Keep every release heading exactly as provided (e.g. \"[Unreleased]\", \
\"[0.2.0] - 2024-03-15\", a full markdown link).
- Never invent, rename, merge, or remove release headings.
- Never add releases that do not exist in the input.

REWRITING RULES
- Merge aggressively: combine related commits identified in the analysis into single entries.
- Write for users, not developers. Describe what changed from the user's perspective.
- Use imperative present tense: Add, Fix, Improve, Remove, Update, Deprecate.
- Drop all noise identified in the analysis. These entries must NOT appear in any form.
- Elevate breaking changes: prefix with **BREAKING:** in the entry text.
- Elevate security fixes: ensure they land in the Security section.
- Add context when the raw entry is cryptic — make it understandable without reading code.
- Preserve precision: do not lose specific details (version numbers, flag names, API changes).

CLASSIFY (Keep a Changelog sections)
Map entries to exactly one of: Added, Changed, Deprecated, Removed, Fixed, Security.
- New capabilities -> Added
- Modifications to existing behavior, refactoring, performance -> Changed
- Deprecated features -> Deprecated
- Removed features -> Removed
- Bug fixes -> Fixed
- Security patches -> Security
Only include sections with at least one entry. Never output an empty entries array.

WRITE EACH ENTRY
- One short sentence, at most 15 words.
- Prefix every entry with a bold scope: **ai:**, **ui:**, **config:**, **core:**, etc.
- Use **core:** for general or project-wide changes.
- No file paths, function names, or internal implementation details.
- Preserve PR/issue references (#123) and author attributions when present.
- No markdown formatting inside entries except the bold scope prefix and BREAKING: prefix.

TONE & STYLE
- Clear, direct, professional. Consistent voice throughout.
- No filler words, no hype, no marketing language.
</rules>

JSON SCHEMA:
{schema_json}

Return ONLY valid JSON matching the schema. No commentary, no explanation, no code fences."
    )
}

/// Enhances existing changelog markdown using AI for improved readability.
pub struct AiEnhancer {
    config: AiConfig,
}

impl AiEnhancer {
    #[must_use]
    pub const fn new(config: AiConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.config.is_configured()
    }

    /// Enhance existing changelog markdown using a two-step AI pipeline.
    ///
    /// # Errors
    ///
    /// Returns `AiError` on configuration, connection, or parsing failures.
    #[allow(clippy::future_not_send)]
    pub async fn enhance(
        &self,
        changelog: &str,
        project_context: Option<&ProjectContext>,
        format: &ChangelogFormat,
        on_step_completed: Option<&(dyn Fn() + Send + Sync)>,
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

        let temperature = self.config.temperature.unwrap_or(DEFAULT_TEMPERATURE);

        debug!(
            provider = provider_name,
            model = model_name,
            temperature = temperature,
            "Starting two-step AI enhancement"
        );

        let auth_resolver = build_auth_resolver(provider);
        let client = Client::builder().with_auth_resolver(auth_resolver).build();
        let model_iden = format!("{}::{}", provider.as_genai_adapter(), model_name);

        let changelog_message = format!("<changelog>\n{changelog}\n</changelog>");

        // -- Step 1: Analysis --------------------------------------------------
        let analysis_system = build_enhancer_analysis_prompt(project_context);
        let analysis_options = ChatOptions::default().with_temperature(temperature);

        let analysis_req = ChatRequest::default()
            .with_system(analysis_system)
            .append_message(ChatMessage::user(&changelog_message));

        debug!(model_iden = model_iden, "Sending analysis request (step 1)");

        let analysis_response = client
            .exec_chat(&model_iden, analysis_req, Some(&analysis_options))
            .await?;

        let analysis_text = extract_response_text(analysis_response, "analysis")?;

        debug!(
            analysis_len = analysis_text.len(),
            "Received analysis (step 1)"
        );

        if let Some(cb) = on_step_completed {
            cb();
        }

        // -- Step 2: Writing (JSON) --------------------------------------------
        let schema = schemars::schema_for!(EnhancedChangelog);
        let schema_json = serde_json::to_string_pretty(&schema).unwrap_or_default();
        let writing_system = build_enhancer_writing_prompt(&schema_json, project_context);

        let schema_value: serde_json::Value = serde_json::to_value(&schema)
            .map_err(|e| AiError::Request(format!("Failed to serialize schema: {e}")))?;

        let json_spec = JsonSpec::new("enhanced_changelog", schema_value);
        let writing_options = ChatOptions::default()
            .with_temperature(temperature)
            .with_response_format(json_spec);

        let writing_user_message = format!(
            "<changelog>\n{changelog}\n</changelog>\n\n\
             <analysis>\n{analysis_text}\n</analysis>"
        );

        let writing_req = ChatRequest::default()
            .with_system(writing_system)
            .append_message(ChatMessage::user(writing_user_message));

        debug!(model_iden = model_iden, "Sending writing request (step 2)");

        let writing_response = client
            .exec_chat(&model_iden, writing_req, Some(&writing_options))
            .await?;

        let raw_text = extract_response_text(writing_response, "writing")?;
        debug!(
            raw_len = raw_text.len(),
            "Received writing response (step 2)"
        );

        let enhanced = parse_enhanced_changelog(&raw_text)?;

        render_markdown(&enhanced, format)
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Extract first non-empty text from a chat response.
fn extract_response_text(
    response: genai::chat::ChatResponse,
    step_label: &str,
) -> Result<String, AiError> {
    response
        .into_first_text()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            AiError::InvalidResponse(format!("AI returned an empty {step_label} response"))
        })
}

/// Parse a JSON response string into an `EnhancedChangelog`.
fn parse_enhanced_changelog(raw_text: &str) -> Result<EnhancedChangelog, AiError> {
    let json_str = strip_code_fences(raw_text);
    serde_json::from_str(json_str).map_err(|e| {
        AiError::InvalidResponse(format!(
            "Failed to parse AI JSON response: {e}\nRaw response:\n{raw_text}"
        ))
    })
}

/// Strip optional markdown code fences from a string.
fn strip_code_fences(text: &str) -> &str {
    let trimmed = text.trim();

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
            let Some(env_name) = provider.env_var_name() else {
                return Ok(None);
            };

            if let Ok(key) = std::env::var(env_name) {
                debug!(env_var = env_name, "Using API key from environment");
                return Ok(Some(AuthData::from_single(key)));
            }

            credentials::get_api_key(provider.as_config_str()).map_or_else(
                |e| {
                    warn!(
                        provider = provider.as_config_str(),
                        error = %e,
                        "Keyring lookup failed"
                    );
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
