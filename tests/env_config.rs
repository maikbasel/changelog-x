//! Integration tests for environment variable configuration loading.
//!
//! These tests spawn the CLI as a subprocess to test environment variable loading
//! without requiring unsafe code in the test process.
//!
//! Each test isolates `HOME` to a temporary directory so the user's real
//! `~/.config/cgx/config.toml` is never picked up.

#![expect(clippy::expect_used)] // Tests use expect for clearer failure messages

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Run `cgx config show` in a temp HOME with optional extra env vars.
/// This prevents the user's real config from leaking into tests.
fn run_cgx_isolated(env_vars: &[(&str, &str)]) -> (String, TempDir) {
    let home = TempDir::new().expect("Failed to create temp HOME");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cgx"));
    cmd.arg("config").arg("show");
    cmd.env("HOME", home.path());

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("Failed to execute cgx");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (stdout, home)
}

/// Run `cgx config show` in a specific directory with isolated HOME.
fn run_cgx_in_dir(dir: &Path, env_vars: &[(&str, &str)]) -> String {
    let home = TempDir::new().expect("Failed to create temp HOME");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cgx"));
    cmd.arg("config").arg("show");
    cmd.current_dir(dir);
    cmd.env("HOME", home.path());

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("Failed to execute cgx");
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn test_env_ai_provider() {
    let (output, _home) = run_cgx_isolated(&[("CGX_AI__PROVIDER", "openai")]);
    assert!(
        output.contains("provider = \"openai\""),
        "Expected provider = \"openai\" in output:\n{output}"
    );
}

#[test]
fn test_env_ai_provider_and_model() {
    let (output, _home) = run_cgx_isolated(&[
        ("CGX_AI__PROVIDER", "anthropic"),
        ("CGX_AI__MODEL", "claude-3-opus"),
    ]);
    assert!(
        output.contains("provider = \"anthropic\""),
        "Expected provider = \"anthropic\" in output:\n{output}"
    );
    assert!(
        output.contains("model = \"claude-3-opus\""),
        "Expected model = \"claude-3-opus\" in output:\n{output}"
    );
}

#[test]
fn test_env_changelog_output() {
    let (output, _home) = run_cgx_isolated(&[("CGX_CHANGELOG__OUTPUT", "RELEASES.md")]);
    assert!(
        output.contains("output = \"RELEASES.md\""),
        "Expected output = \"RELEASES.md\" in output:\n{output}"
    );
}

#[test]
fn test_env_multiple_sections() {
    let (output, _home) = run_cgx_isolated(&[
        ("CGX_AI__PROVIDER", "gemini"),
        ("CGX_AI__MODEL", "gemini-pro"),
        ("CGX_CHANGELOG__OUTPUT", "HISTORY.md"),
    ]);
    assert!(
        output.contains("provider = \"gemini\""),
        "Expected provider = \"gemini\" in output:\n{output}"
    );
    assert!(
        output.contains("model = \"gemini-pro\""),
        "Expected model = \"gemini-pro\" in output:\n{output}"
    );
    assert!(
        output.contains("output = \"HISTORY.md\""),
        "Expected output = \"HISTORY.md\" in output:\n{output}"
    );
}

#[test]
fn test_default_changelog_output_without_env() {
    let (output, _home) = run_cgx_isolated(&[]);
    assert!(
        output.contains("output = \"CHANGELOG.md\""),
        "Expected default output = \"CHANGELOG.md\" in output:\n{output}"
    );
}

#[test]
fn test_no_config_files_message_shown() {
    let (output, _home) = run_cgx_isolated(&[]);
    assert!(
        output.contains("no config files found"),
        "Expected 'no config files found' message in output:\n{output}"
    );
}

#[test]
fn test_with_config_file_no_warning_shown() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join(".cgx.toml");

    fs::write(
        &config_path,
        r#"
[changelog]
output = "RELEASES.md"
"#,
    )
    .expect("Failed to write config file");

    let output = run_cgx_in_dir(temp_dir.path(), &[]);
    assert!(
        !output.contains("no config files found"),
        "Should NOT show 'no config files found' when config exists:\n{output}"
    );
    assert!(
        output.contains("output = \"RELEASES.md\""),
        "Expected configured output path in output:\n{output}"
    );
}
