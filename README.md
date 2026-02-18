# ChangelogX (cgx)

Generate high-quality changelogs from [conventional commits](https://www.conventionalcommits.org), optionally enhanced with AI.

`cgx` parses your git history, groups commits by type, and outputs a structured changelog in [Keep a Changelog](https://keepachangelog.com) or [Common Changelog](https://common-changelog.org) format. When AI enhancement is enabled, it rewrites entries for clarity and consistency using the provider of your choice.

## Features

- Changelog generation from conventional commits via [git-cliff](https://git-cliff.org)
- AI enhancement with support for OpenAI, Anthropic, Gemini, Groq, DeepSeek, and Ollama (local)
- Multiple output formats (Keep a Changelog 1.1.0, Common Changelog)
- Layered configuration (user defaults, project overrides, environment variables)
- Secure credential storage via OS keyring
- CI/CD friendly with full environment variable support

## Prerequisites

- [Rust](https://rustup.rs/) 1.85+ (edition 2024)
- Git
- A system keyring service (GNOME Keyring / KWallet on Linux, Keychain on macOS, Credential Manager on Windows)

## Getting started

```bash
git clone https://github.com/maikbasel/changelog-x.git
cd changelog-x
cargo build
```

The first `cargo test` installs [cargo-husky](https://github.com/nickel-org/cargo-husky) pre-commit hooks that automatically run `cargo test`, `cargo clippy`, and `cargo fmt` on each commit.

## Architecture

```
src/
  main.rs             CLI entry point and command dispatch (clap)
  lib.rs              Library re-exports
  error.rs            Error types (thiserror)
  ai/
    enhancer.rs       AI-powered changelog enhancement via genai
    credentials.rs    Provider enum, keyring storage, API key resolution
    context.rs        Commit context passed to AI prompts
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
| `keyring` | Cross-platform secure credential storage |
| `inquire` | Interactive terminal prompts |
| `config` | Layered configuration with env var support |
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

AI enhancement uses the `genai` crate for provider-agnostic access. API keys are resolved from environment variables first, then the system keyring. The `AiEnhancer` in `src/ai/enhancer.rs` builds a chat request with system prompt and commit context, sends it to the configured provider, and returns the enhanced changelog text.

Relevant code: `src/ai/enhancer.rs`, `src/ai/credentials.rs`

## Development

```bash
# Build (debug)
cargo build

# Run the CLI
cargo run -- generate --stdout
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