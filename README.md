# ChangelogX (cgx)

[![CI](https://github.com/maikbasel/changelog-x/actions/workflows/ci.yml/badge.svg)](https://github.com/maikbasel/changelog-x/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE.md)

Generate high-quality changelogs from your git history using AI, or from conventional commits via [git-cliff](https://git-cliff.org).

`cgx ai generate` uses AI to read your full git history — including diff stats and project context — and produces a polished, user-centric changelog regardless of commit format. `cgx generate` parses [conventional commits](https://www.conventionalcommits.org) via git-cliff for structured output without AI. Both modes support [Keep a Changelog](https://keepachangelog.com) and [Common Changelog](https://common-changelog.org) formats, rendered via Tera templates.

## Features

- AI changelog generation from git history &mdash; works with any commit format, enriched with diff stats and project context
- Conventional commit changelog via [git-cliff](https://git-cliff.org) (no AI required)
- AI support for OpenAI, Anthropic, Gemini, Groq, DeepSeek, and Ollama (local)
- Project context awareness &mdash; auto-gathers README, docs, Cargo.toml metadata, and AI instruction files
- Template-based rendering via [Tera](https://keats.github.io/tera/)
- Multiple output formats (Keep a Changelog 1.1.0, Common Changelog)
- Layered configuration (user defaults, project overrides, environment variables)
- Secure credential storage via OS keyring
- CI/CD friendly with full environment variable support

## Installation

### Homebrew (macOS / Linux)

```bash
brew install maikbasel/tap/changelog-x
```

### Shell installer (macOS / Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/maikbasel/changelog-x/releases/latest/download/changelog-x-installer.sh | sh
```

### PowerShell installer (Windows)

```powershell
powershell -executionpolicy bypass -c "irm https://github.com/maikbasel/changelog-x/releases/latest/download/changelog-x-installer.ps1 | iex"
```

### Build from source

```bash
git clone https://github.com/maikbasel/changelog-x.git
cd changelog-x
cargo install --path .
```

### Pre-built binaries

Download pre-built binaries from the [GitHub Releases](https://github.com/maikbasel/changelog-x/releases) page. Supported platforms:

- macOS (Apple Silicon, Intel)
- Linux (x86_64, ARM64)
- Windows (x86_64)

## Commands

```
cgx generate                Generate changelog from conventional commits
cgx ai generate             Generate changelog directly via AI from structured commit data
cgx ai status               Show AI configuration status
cgx ai setup                Interactive provider/model/key configuration
cgx ai auth                 Store API key in system keyring
cgx ai auth clear            Remove API key from system keyring
cgx config show             Print fully resolved configuration
cgx config path             Show configuration file paths
cgx config edit             Open user config in $EDITOR
```

Common flags for `generate` and `ai generate`:

| Flag | Description |
|------|-------------|
| `--stdout` | Print to stdout instead of writing a file |
| `-o, --output <path>` | Output file path (default: `CHANGELOG.md`) |
| `--from <tag>` | Start from this git tag |
| `--to <tag>` | End at this git tag |
| `--unreleased` | Only include commits since the latest tag |
| `--format <fmt>` | `keep-a-changelog` or `common-changelog` |

Global: `-v` / `-vv` for debug / trace logging.

## Architecture

```
src/
  main.rs             CLI entry point and command dispatch (clap)
  lib.rs              Library re-exports
  error.rs            Error types (thiserror)
  ai/
    generator.rs      AI-powered changelog generation (genai + Tera)
    commit_data.rs    Structured commit data extraction with diff stats (git2)
    credentials.rs    Provider enum, keyring storage, API key resolution
    context.rs        Project context gathering (Cargo.toml, README, docs, AI instructions)
  changelog/
    generator.rs      Changelog generation via git-cliff-core
  config/
    loader.rs         Layered TOML config (user -> project -> env vars)
  ui/
    progress.rs       Step-based progress pipeline (indicatif)
    prompts.rs        Interactive prompts (inquire)
tests/
  env_config.rs       Environment variable configuration tests
  generate_unreleased.rs  Unreleased changelog generation tests
```

### Key dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI parsing with derive macros |
| `tokio` | Async runtime |
| `git-cliff-core` | Changelog generation from conventional commits |
| `genai` | Provider-agnostic AI (OpenAI, Anthropic, Gemini, Ollama, Groq, DeepSeek) |
| `git2` | Git repository access for commit and diff extraction |
| `tera` | Template rendering for changelog output |
| `schemars` | JSON schema generation for structured AI output |
| `keyring` | Cross-platform secure credential storage |
| `inquire` | Interactive terminal prompts |
| `config` | Layered configuration with env var support |
| `console` / `indicatif` | Terminal output and progress display |
| `tracing` / `tracing-subscriber` | Structured logging |
| `directories` | XDG-compliant config paths |
| `regex` | Pattern matching (tag filters) |
| `indexmap` | Ordered maps for deterministic output |
| `thiserror` / `anyhow` | Library and application error handling |

### Configuration system

Configuration is loaded in layers (higher overrides lower):

1. CLI arguments
2. Environment variables (`CGX_` prefix, `__` separator for nesting)
3. Project config (`.cgx.toml` in repository root)
4. User config (`~/.config/cgx/config.toml`)
5. Built-in defaults

Relevant code: `src/config/loader.rs`

### AI integration

`cgx ai generate` uses the `genai` crate for provider-agnostic access. The flow:

1. **Extract commits** &mdash; `commit_data.rs` reads the full git history via `git2` and computes per-commit diff stats (files changed, insertions, deletions). Works with any commit format &mdash; conventional commits are not required.
2. **Gather project context** &mdash; `context.rs` collects metadata from `Cargo.toml`, the project README, files in `docs/`, and optional AI instruction files to give the model domain awareness.
3. **Analyse via AI** &mdash; `generator.rs` (`AiGenerator`) sends structured commit data and project context to the configured provider, receiving a plain-text analysis that groups changes, identifies user-facing impact, and filters noise (CI, tooling, deps, etc.).
4. **Write via AI** &mdash; a second request takes the analysis and produces a structured JSON changelog (`EnhancedChangelog`), merging related commits into concise entries written for end users.
5. **Render with Tera** &mdash; the structured response is rendered into the chosen changelog format using Tera templates embedded in `generator.rs`.

API keys are resolved from environment variables first, then the system keyring.

Relevant code: `src/ai/generator.rs`, `src/ai/commit_data.rs`, `src/ai/context.rs`, `src/ai/credentials.rs`

## Development

### Prerequisites

- [Rust](https://rustup.rs/) 1.85+ (edition 2024)
- Git
- A system keyring service (GNOME Keyring / KWallet on Linux, Keychain on macOS, Credential Manager on Windows)

The first `cargo test` installs [cargo-husky](https://github.com/nickel-org/cargo-husky) pre-commit hooks that automatically run `cargo test`, `cargo clippy`, and `cargo fmt` on each commit.

```bash
# Build (debug)
cargo build

# Run the CLI
cargo run -- generate --stdout
cargo run -- ai generate --stdout
cargo run -- ai status

# Run tests
cargo test

# Lint (strict: clippy::all + pedantic + nursery)
cargo clippy --all-targets --all-features

# Format
cargo fmt

# Build rustdoc
cargo doc --no-deps --all-features
```

### Lint policy

The project enforces strict clippy lints:

- `clippy::all`, `clippy::pedantic`, `clippy::nursery` &mdash; all set to `warn`
- `clippy::unwrap_used`, `clippy::expect_used` &mdash; warned (use `?` or proper error handling)
- `unsafe_code` &mdash; forbidden

CI runs with `RUSTFLAGS=-Dwarnings`, promoting all warnings to errors.

### Running CI checks locally

Replicate the full CI pipeline before pushing:

```bash
cargo fmt --all --check \
  && cargo clippy --all-targets --all-features \
  && cargo test --all-features \
  && cargo doc --no-deps --all-features
```

CI also runs [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) for dependency auditing and [cargo-machete](https://github.com/bnjbvr/cargo-machete) for unused dependency detection.

### Commit convention

This project uses [Conventional Commits](https://www.conventionalcommits.org):

```
feat: add new feature
fix: resolve bug
docs: update documentation
refactor: restructure code without behavior change
test: add or update tests
ci: change CI/CD configuration
chore: maintenance tasks
```

### Releases

Releases are automated via [Release Please](https://github.com/googleapis/release-please) and [cargo-dist](https://opensource.axo.dev/cargo-dist/). Pushing to `master` with conventional commits triggers version bumps and artifact builds.

## License

[MIT](LICENSE)
