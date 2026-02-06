//! Integration tests for the `--unreleased` flag on `cgx generate`.
//!
//! Verifies that clap correctly rejects mutually exclusive flag combinations.

#![expect(clippy::expect_used)] // Tests use expect for clearer failure messages

use std::process::Command;

fn run_cgx(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cgx"))
        .args(args)
        .output()
        .expect("Failed to execute cgx")
}

#[test]
fn test_unreleased_conflicts_with_from() {
    let output = run_cgx(&["generate", "--unreleased", "--from", "v1.0.0"]);

    assert!(
        !output.status.success(),
        "Expected failure when combining --unreleased and --from"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "Expected conflict error in stderr:\n{stderr}"
    );
}

#[test]
fn test_unreleased_conflicts_with_to() {
    let output = run_cgx(&["generate", "--unreleased", "--to", "v2.0.0"]);

    assert!(
        !output.status.success(),
        "Expected failure when combining --unreleased and --to"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "Expected conflict error in stderr:\n{stderr}"
    );
}

#[test]
fn test_unreleased_conflicts_with_from_and_to() {
    let output = run_cgx(&[
        "generate",
        "--unreleased",
        "--from",
        "v1.0.0",
        "--to",
        "v2.0.0",
    ]);

    assert!(
        !output.status.success(),
        "Expected failure when combining --unreleased with --from and --to"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "Expected conflict error in stderr:\n{stderr}"
    );
}
