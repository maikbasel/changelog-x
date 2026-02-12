use std::fmt::Write;

/// Owned commit summary for AI context, decoupled from git2 lifetimes.
#[derive(Debug, Clone)]
pub struct CommitSummary {
    pub short_hash: String,
    pub summary: String,
    pub body: Option<String>,
    pub author: String,
}

/// Format commits into a context block appended to the AI user message.
#[must_use]
pub fn format_commit_context(commits: &[CommitSummary]) -> String {
    let mut out = String::from("\n\n=== GIT COMMIT CONTEXT ===\n");
    for c in commits {
        let _ = writeln!(out, "{} - {}", c.short_hash, c.summary);
        let _ = writeln!(out, "  Author: {}", c.author);
        if let Some(body) = &c.body {
            let _ = writeln!(out, "  Body: {body}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_commit_context_basic() {
        let commits = vec![
            CommitSummary {
                short_hash: "abc1234".into(),
                summary: "feat(ui): add field preview".into(),
                body: Some("Implements a cropped PDF snippet".into()),
                author: "Maik Basel".into(),
            },
            CommitSummary {
                short_hash: "def5678".into(),
                summary: "fix(backend): wrong JS function names".into(),
                body: None,
                author: "Maik Basel".into(),
            },
        ];

        let result = format_commit_context(&commits);
        assert!(result.contains("=== GIT COMMIT CONTEXT ==="));
        assert!(result.contains("abc1234 - feat(ui): add field preview"));
        assert!(result.contains("Body: Implements a cropped PDF snippet"));
        assert!(result.contains("def5678 - fix(backend): wrong JS function names"));
        assert!(!result.contains("Body: \n")); // No body line for None
    }

    #[test]
    fn test_format_commit_context_empty() {
        let result = format_commit_context(&[]);
        assert!(result.contains("=== GIT COMMIT CONTEXT ==="));
    }
}
