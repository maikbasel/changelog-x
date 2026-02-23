use std::fmt;
use std::fmt::Write;
use std::path::Path;

use tracing::debug;

/// Auto-detected project type based on Cargo.toml structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    Cli,
    Library,
    LibraryWithCli,
}

impl fmt::Display for ProjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cli => write!(f, "CLI application"),
            Self::Library => write!(f, "library"),
            Self::LibraryWithCli => write!(f, "library with CLI"),
        }
    }
}

/// Lightweight project metadata injected into AI prompts for domain awareness.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub name: String,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub version: Option<String>,
    pub project_type: ProjectType,
}

/// Gather project context from `Cargo.toml` in the current directory (best-effort).
///
/// Returns `None` if the file cannot be read or parsed.
#[must_use]
pub fn gather_project_context() -> Option<ProjectContext> {
    gather_project_context_from(Path::new("Cargo.toml"))
}

/// Gather project context from a specific `Cargo.toml` path.
fn gather_project_context_from(path: &Path) -> Option<ProjectContext> {
    let content = std::fs::read_to_string(path).ok()?;
    let doc: toml::Table = toml::from_str(&content).ok()?;

    let package = doc.get("package")?.as_table()?;

    let name = package.get("name")?.as_str()?.to_string();
    let description = package
        .get("description")
        .and_then(toml::Value::as_str)
        .map(String::from);
    let repository = package
        .get("repository")
        .and_then(toml::Value::as_str)
        .map(String::from);
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .map(String::from);

    let has_bin = doc
        .get("bin")
        .and_then(toml::Value::as_array)
        .is_some_and(|a| !a.is_empty());

    let has_lib = doc.contains_key("lib");

    let project_type = match (has_bin, has_lib) {
        (true, true) => ProjectType::LibraryWithCli,
        (true, false) => ProjectType::Cli,
        (false, true) => ProjectType::Library,
        // Default: if there's a src/main.rs it's likely a CLI, otherwise library
        (false, false) => {
            if Path::new("src/main.rs").exists() {
                ProjectType::Cli
            } else {
                ProjectType::Library
            }
        }
    };

    debug!(
        name = name.as_str(),
        project_type = %project_type,
        "Gathered project context from Cargo.toml"
    );

    Some(ProjectContext {
        name,
        description,
        repository,
        version,
        project_type,
    })
}

/// Format project context into a compact block for AI prompt injection.
#[must_use]
pub fn format_project_context(ctx: &ProjectContext) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "PROJECT: {}", ctx.name);
    if let Some(desc) = &ctx.description {
        let _ = writeln!(out, "DESCRIPTION: {desc}");
    }
    let _ = writeln!(out, "TYPE: {}", ctx.project_type);
    if let Some(repo) = &ctx.repository {
        let _ = writeln!(out, "REPOSITORY: {repo}");
    }
    if let Some(version) = &ctx.version {
        let _ = writeln!(out, "VERSION: {version}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> ProjectContext {
        ProjectContext {
            name: "changelog-x".into(),
            description: Some(
                "Generate high-quality changelogs from conventional commits with AI enhancement"
                    .into(),
            ),
            repository: Some("https://github.com/maikbasel/changelog-x".into()),
            version: Some("0.1.0".into()),
            project_type: ProjectType::Cli,
        }
    }

    #[test]
    fn test_format_project_context_full() {
        let ctx = sample_context();
        let result = format_project_context(&ctx);
        assert!(result.contains("PROJECT: changelog-x"));
        assert!(result.contains("DESCRIPTION: Generate high-quality changelogs"));
        assert!(result.contains("TYPE: CLI application"));
        assert!(result.contains("REPOSITORY: https://github.com/maikbasel/changelog-x"));
        assert!(result.contains("VERSION: 0.1.0"));
    }

    #[test]
    fn test_format_project_context_minimal() {
        let ctx = ProjectContext {
            name: "my-lib".into(),
            description: None,
            repository: None,
            version: None,
            project_type: ProjectType::Library,
        };
        let result = format_project_context(&ctx);
        assert!(result.contains("PROJECT: my-lib"));
        assert!(result.contains("TYPE: library"));
        assert!(!result.contains("DESCRIPTION:"));
        assert!(!result.contains("REPOSITORY:"));
        assert!(!result.contains("VERSION:"));
    }

    #[test]
    fn test_project_type_display() {
        assert_eq!(ProjectType::Cli.to_string(), "CLI application");
        assert_eq!(ProjectType::Library.to_string(), "library");
        assert_eq!(ProjectType::LibraryWithCli.to_string(), "library with CLI");
    }

    #[test]
    fn test_gather_from_cargo_toml() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("Failed to create tempdir: {e}"));
        let cargo_path = dir.path().join("Cargo.toml");
        std::fs::write(
            &cargo_path,
            r#"
[package]
name = "test-project"
version = "1.2.3"
edition = "2021"
description = "A test project"
repository = "https://github.com/test/test-project"

[[bin]]
name = "test-cli"
path = "src/main.rs"

[lib]
name = "test_project"
"#,
        )
        .unwrap_or_else(|e| panic!("Failed to write Cargo.toml: {e}"));

        let ctx = gather_project_context_from(&cargo_path);
        assert!(ctx.is_some());
        let ctx = ctx.unwrap_or_else(|| panic!("Expected Some"));
        assert_eq!(ctx.name, "test-project");
        assert_eq!(ctx.version.as_deref(), Some("1.2.3"));
        assert_eq!(ctx.description.as_deref(), Some("A test project"));
        assert_eq!(
            ctx.repository.as_deref(),
            Some("https://github.com/test/test-project")
        );
        assert_eq!(ctx.project_type, ProjectType::LibraryWithCli);
    }

    #[test]
    fn test_gather_returns_none_for_missing_file() {
        let result = gather_project_context_from(Path::new("/nonexistent/Cargo.toml"));
        assert!(result.is_none());
    }

    #[test]
    fn test_gather_library_only() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("Failed to create tempdir: {e}"));
        let cargo_path = dir.path().join("Cargo.toml");
        std::fs::write(
            &cargo_path,
            r#"
[package]
name = "my-lib"
version = "0.1.0"
edition = "2021"

[lib]
name = "my_lib"
"#,
        )
        .unwrap_or_else(|e| panic!("Failed to write Cargo.toml: {e}"));

        let ctx = gather_project_context_from(&cargo_path);
        assert!(ctx.is_some());
        let ctx = ctx.unwrap_or_else(|| panic!("Expected Some"));
        assert_eq!(ctx.project_type, ProjectType::Library);
    }
}
