use anyhow::{Context, Result};
use changelog_x::ai::AiEnhancer;
use changelog_x::ai::credentials::{self, Provider};
use changelog_x::changelog::{ChangelogGenerator, GenerateOptions, read_commit_summaries};
use changelog_x::config::{
    ChangelogFormat, get_user_config_path, load_config, save_user_ai_config,
};
use changelog_x::ui::{self, Pipeline};
use changelog_x::{AppError, ChangelogError};
use clap::{Parser, Subcommand};
use console::{Term, style};
use inquire::{Editor, InquireError};
use std::fs;
use std::path::Path;
use tracing::debug;
use tracing_subscriber::EnvFilter;

/// Generate high-quality changelogs from conventional commits with AI enhancement
#[derive(Parser)]
#[command(name = "cgx")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Increase verbosity (-v debug, -vv trace)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a changelog from conventional commits
    Generate {
        /// Enhance changelog with AI after generation
        #[arg(long)]
        ai: bool,

        /// Print to stdout instead of writing file
        #[arg(long)]
        stdout: bool,

        /// Output file path
        #[arg(short, long, default_value = "CHANGELOG.md")]
        output: String,

        /// Start from this git tag
        #[arg(long, conflicts_with = "unreleased")]
        from: Option<String>,

        /// End at this git tag
        #[arg(long, conflicts_with = "unreleased")]
        to: Option<String>,

        /// Generate changelog for unreleased commits only (since latest tag)
        #[arg(long, conflicts_with_all = ["from", "to"])]
        unreleased: bool,

        /// Changelog format: keep-a-changelog or common-changelog
        #[arg(long)]
        format: Option<String>,
    },

    /// Enhance an existing changelog file with AI
    #[command(arg_required_else_help = true)]
    Enhance {
        /// File to enhance
        file: Option<String>,

        /// Print result without overwriting file
        #[arg(long)]
        dry_run: bool,

        /// Write to different file instead of overwriting
        #[arg(short, long)]
        output: Option<String>,

        /// Changelog format: keep-a-changelog or common-changelog
        #[arg(long)]
        format: Option<String>,
    },

    /// Initialize cgx in the current project
    Init {
        /// Overwrite existing configuration
        #[arg(long)]
        force: bool,

        /// Skip AI configuration prompt
        #[arg(long)]
        skip_ai: bool,
    },

    /// AI configuration management
    Ai {
        #[command(subcommand)]
        action: Option<AiAction>,
    },

    /// View and manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Subcommand)]
enum AiAction {
    /// Show AI configuration status
    Status,

    /// Configure AI provider, model, and credentials
    Setup,

    /// Manage API key in system keyring
    Auth {
        #[command(subcommand)]
        action: Option<AuthAction>,
    },
}

#[derive(Debug, Subcommand)]
enum AuthAction {
    /// Remove API key from system keyring
    Clear,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum ConfigAction {
    /// Print fully resolved configuration
    Show,

    /// Show configuration file paths
    Path,

    /// Open user config in $EDITOR
    Edit,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        let term = Term::stderr();

        let _ = term.write_line(&format!("\n{} {err}", style("error:").red().bold()));

        let source_chain: Vec<_> = std::iter::successors(
            std::error::Error::source(err.as_ref() as &dyn std::error::Error),
            |e| e.source(),
        )
        .collect();

        for cause in &source_chain {
            let _ = term.write_line(&format!("  {} {cause}", style("caused by:").dim()));
        }

        if let Some(app_err) = err.downcast_ref::<AppError>()
            && let Some(help) = app_err.help_text()
        {
            let _ = term.write_line(&format!("  {} {help}", style("help:").yellow().bold()));
        }

        let _ = term.write_line("");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing with appropriate log level
    let filter = match cli.verbose {
        0 => EnvFilter::new("warn,git_cliff_core=off"),
        1 => EnvFilter::new("debug"),
        _ => EnvFilter::new("trace"),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(cli.verbose > 0)
        .without_time()
        .init();

    match cli.command {
        Some(Commands::Generate {
            ai,
            stdout,
            output,
            from,
            to,
            unreleased,
            format,
        }) => cmd_generate(ai, stdout, &output, from, to, unreleased, format).await,

        Some(Commands::Enhance {
            file,
            dry_run,
            output,
            format,
        }) => {
            let file = file.unwrap_or_else(|| "CHANGELOG.md".into());
            cmd_enhance(&file, dry_run, output, format).await
        }

        Some(Commands::Init { force, skip_ai }) => cmd_init(force, skip_ai).await,

        Some(Commands::Ai { action }) => cmd_ai(action.unwrap_or(AiAction::Status)).await,

        Some(Commands::Config { action }) => cmd_config(action),

        None => {
            // No subcommand: show help
            use clap::CommandFactory;
            Cli::command().print_help()?;
            Ok(())
        }
    }
}

/// Resolve the effective changelog format: CLI flag overrides config default.
fn resolve_format(
    cli_format: Option<&str>,
    config_format: &ChangelogFormat,
) -> Result<ChangelogFormat> {
    cli_format.map_or_else(
        || Ok(config_format.clone()),
        |f| match f {
            "keep-a-changelog" => Ok(ChangelogFormat::KeepAChangelog),
            "common-changelog" => Ok(ChangelogFormat::CommonChangelog),
            other => anyhow::bail!(
                "Unknown format '{other}'. Valid values: keep-a-changelog, common-changelog"
            ),
        },
    )
}

async fn cmd_generate(
    ai: bool,
    stdout: bool,
    output: &str,
    from: Option<String>,
    to: Option<String>,
    unreleased: bool,
    format_flag: Option<String>,
) -> Result<()> {
    debug!(
        "Generate command: ai={}, stdout={}, output={}, from={:?}, to={:?}, unreleased={}",
        ai, stdout, output, from, to, unreleased
    );

    let term = Term::stdout();

    let mut steps: Vec<&str> = vec![
        "Loading configuration",
        "Initializing repository",
        "Resolving commits",
        "Fetching data",
        "Building releases",
        "Generating changelog",
    ];
    if ai {
        steps.push("Enhancing with AI");
    }

    let pipeline = Pipeline::new(&steps);

    // Step 1: Loading configuration
    pipeline.advance();
    let config = load_config(None).context("Failed to load configuration")?;

    // Generate changelog (steps 2-6 are advanced by the callback)
    let generator = ChangelogGenerator::new(config.changelog.clone());
    let options = GenerateOptions {
        from_tag: from,
        to_tag: to,
        unreleased,
    };

    let generate_result = match generator.generate(&options, Some(&|| pipeline.advance())) {
        Ok(result) => {
            if !ai {
                pipeline.finish_all();
            }
            result
        }
        Err(ChangelogError::NoCommits) => {
            pipeline.fail("No conventional commits found");
            term.write_line("")?;
            term.write_line("cgx requires commits following the Conventional Commits format:")?;
            term.write_line(&format!("  {} add new feature", style("feat:").green()))?;
            term.write_line(&format!("  {} resolve bug", style("fix:").green()))?;
            term.write_line(&format!("  {} update readme", style("docs:").green()))?;
            term.write_line("")?;
            term.write_line(&format!(
                "See: {}",
                style("https://www.conventionalcommits.org")
                    .cyan()
                    .underlined()
            ))?;
            return Ok(());
        }
        Err(e) => {
            pipeline.fail(&format!("{e}"));
            return Err(e).context("Failed to generate changelog");
        }
    };

    let mut changelog = generate_result.changelog;

    // AI enhancement (if --ai flag)
    if ai {
        pipeline.advance();
        let fmt = resolve_format(format_flag.as_deref(), &config.changelog.format)?;
        let ai_enhancer = AiEnhancer::new(config.ai.clone());
        let context = if generate_result.commits.is_empty() {
            None
        } else {
            Some(generate_result.commits.as_slice())
        };
        match ai_enhancer.enhance(&changelog, context, &fmt).await {
            Ok(result) => {
                changelog = result;
                pipeline.finish_all();
            }
            Err(e) => {
                pipeline.fail(&format!("{e}"));
                return Err(e).context("AI enhancement failed");
            }
        }
    }

    // Output: stdout or file
    if stdout {
        term.write_line(&changelog)?;
    } else {
        // Use CLI output flag if not default, otherwise use config
        let output_path = if output == "CHANGELOG.md" {
            &config.changelog.output
        } else {
            output
        };

        fs::write(output_path, &changelog)
            .with_context(|| format!("Failed to write changelog to {output_path}"))?;

        term.write_line(&format!(
            "{} Changelog written to {}",
            style("✓").green().bold(),
            style(output_path).cyan()
        ))?;
    }

    Ok(())
}

async fn cmd_enhance(
    file: &str,
    dry_run: bool,
    output: Option<String>,
    format_flag: Option<String>,
) -> Result<()> {
    debug!(
        "Enhance command: file={}, dry_run={}, output={:?}",
        file, dry_run, output
    );

    let term = Term::stdout();

    let pipeline = Pipeline::new(&[
        "Loading configuration",
        "Reading file",
        "Reading git history",
        "Enhancing with AI",
    ]);

    // Step 1: Load config
    pipeline.advance();
    let config = load_config(None).context("Failed to load configuration")?;
    let fmt = resolve_format(format_flag.as_deref(), &config.changelog.format)?;

    // Step 2: Read the input file
    pipeline.advance();
    let content = fs::read_to_string(file).with_context(|| format!("Failed to read {file}"))?;

    // Step 3: Read git history (best-effort)
    pipeline.advance();
    let summaries = read_commit_summaries(500);
    let commit_ctx = if summaries.is_empty() {
        None
    } else {
        Some(summaries.as_slice())
    };

    // Step 4: Enhance with AI
    pipeline.advance();
    let ai_enhancer = AiEnhancer::new(config.ai.clone());
    let result = match ai_enhancer.enhance(&content, commit_ctx, &fmt).await {
        Ok(text) => {
            pipeline.finish_all();
            text
        }
        Err(e) => {
            pipeline.fail(&format!("{e}"));
            return Err(e).context("AI enhancement failed");
        }
    };

    // Output
    if dry_run {
        term.write_line(&result)?;
    } else {
        let output_path = output.as_deref().unwrap_or(file);
        fs::write(output_path, &result)
            .with_context(|| format!("Failed to write to {output_path}"))?;

        term.write_line(&format!(
            "{} Enhanced changelog written to {}",
            style("✓").green().bold(),
            style(output_path).cyan()
        ))?;
    }

    Ok(())
}

#[allow(clippy::unused_async)] // Will use async when implemented
async fn cmd_init(force: bool, skip_ai: bool) -> Result<()> {
    debug!("Init command: force={}, skip_ai={}", force, skip_ai);

    let term = Term::stdout();
    term.write_line(&format!(
        "{}",
        style("Init wizard not yet implemented").yellow()
    ))?;
    Ok(())
}

#[allow(clippy::unused_async)]
async fn cmd_ai(action: AiAction) -> Result<()> {
    debug!("AI command: {:?}", action);

    match action {
        AiAction::Status => cmd_ai_status(),
        AiAction::Setup => cmd_ai_setup(),
        AiAction::Auth { action } => match action {
            None => cmd_ai_auth(),
            Some(AuthAction::Clear) => cmd_ai_auth_clear(),
        },
    }
}

fn cmd_ai_status() -> Result<()> {
    let term = Term::stdout();
    let config = load_config(None).context("Failed to load configuration")?;

    term.write_line(&format!("{}", style("AI Configuration").bold()))?;
    term.write_line("")?;

    if let Some(ref provider_name) = config.ai.provider {
        let provider_display = Provider::from_config_str(provider_name)
            .map_or_else(|| provider_name.clone(), |p| p.to_string());

        term.write_line(&format!(
            "  {} {}",
            style("Provider:").cyan(),
            provider_display
        ))?;

        let model_display = config.ai.model.as_deref().unwrap_or("(default)");
        term.write_line(&format!(
            "  {}    {}",
            style("Model:").cyan(),
            model_display
        ))?;

        let provider = Provider::from_config_str(provider_name);
        if provider.is_some_and(|p| !p.requires_api_key()) {
            term.write_line(&format!(
                "  {}  {}",
                style("API key:").cyan(),
                style("not required").dim()
            ))?;
        } else if credentials::has_api_key(provider_name) {
            term.write_line(&format!(
                "  {}  {}",
                style("API key:").cyan(),
                style("stored in system keyring").green()
            ))?;
        } else {
            term.write_line(&format!(
                "  {}  {}",
                style("API key:").cyan(),
                style("not set").red()
            ))?;
            term.write_line(&format!(
                "\n  {} Run {} to store an API key",
                style("hint:").yellow(),
                style("cgx ai auth").cyan()
            ))?;
        }
    } else {
        term.write_line(&format!(
            "  {}",
            style("No AI provider configured").yellow()
        ))?;
        term.write_line(&format!(
            "\n  {} Run {} to set up AI",
            style("hint:").yellow(),
            style("cgx ai setup").cyan()
        ))?;
    }

    term.write_line("")?;
    Ok(())
}

fn cmd_ai_setup() -> Result<()> {
    let term = Term::stdout();
    let config = load_config(None).context("Failed to load configuration")?;

    // If already configured, confirm overwrite
    if config.ai.is_configured() {
        let provider_name = config.ai.provider.as_deref().unwrap_or("unknown");
        term.write_line(&format!(
            "AI is already configured with provider: {}",
            style(provider_name).cyan()
        ))?;

        match ui::confirm("Overwrite existing configuration?", false) {
            Ok(true) => {}
            Ok(false) => {
                term.write_line(&format!("{}", style("Setup cancelled.").dim()))?;
                return Ok(());
            }
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                term.write_line(&format!("\n{}", style("Setup cancelled.").dim()))?;
                return Ok(());
            }
            Err(e) => return Err(e).context("Prompt failed"),
        }
    }

    // Select provider
    let provider = match ui::select_option("Select AI provider:", Provider::ALL.to_vec()) {
        Ok(p) => p,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            term.write_line(&format!("\n{}", style("Setup cancelled.").dim()))?;
            return Ok(());
        }
        Err(e) => return Err(e).context("Provider selection failed"),
    };

    // Model input with suggestions from popular models
    let suggestions: Vec<String> = provider
        .popular_models()
        .iter()
        .map(|s| (*s).into())
        .collect();
    let model = match ui::text_input_with_suggestions(
        "Model name:",
        Some(provider.default_model()),
        suggestions,
    ) {
        Ok(m) => m,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            term.write_line(&format!("\n{}", style("Setup cancelled.").dim()))?;
            return Ok(());
        }
        Err(e) => return Err(e).context("Model input failed"),
    };

    // API key (skip for Ollama)
    if provider.requires_api_key() {
        match ui::password_input(&format!("API key for {provider}:")) {
            Ok(key) => {
                credentials::store_api_key(&provider, &key)?;
            }
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                term.write_line(&format!("\n{}", style("Setup cancelled.").dim()))?;
                return Ok(());
            }
            Err(e) => return Err(e).context("API key input failed"),
        }
    }

    // Save provider + model to config
    save_user_ai_config(provider.as_config_str(), &model)?;

    // Print summary
    term.write_line("")?;
    term.write_line(&format!(
        "{} AI configured successfully",
        style("✓").green().bold()
    ))?;
    term.write_line(&format!("  {} {}", style("Provider:").cyan(), provider))?;
    term.write_line(&format!("  {}    {}", style("Model:").cyan(), model))?;
    if provider.requires_api_key() {
        term.write_line(&format!(
            "  {}  {}",
            style("API key:").cyan(),
            style("stored in system keyring").green()
        ))?;
    }
    term.write_line("")?;

    Ok(())
}

fn cmd_ai_auth() -> Result<()> {
    let term = Term::stdout();
    let config = load_config(None).context("Failed to load configuration")?;

    let provider_name = config
        .ai
        .provider
        .as_deref()
        .ok_or(AppError::Ai(changelog_x::AiError::NotConfigured))?;

    let provider = Provider::from_config_str(provider_name);

    if provider.is_some_and(|p| !p.requires_api_key()) {
        term.write_line(&format!(
            "{} {} does not require an API key",
            style("note:").yellow(),
            provider.map_or_else(|| provider_name.to_string(), |p| p.to_string())
        ))?;
        return Ok(());
    }

    let display_name = provider.map_or_else(|| provider_name.to_string(), |p| p.to_string());

    match ui::password_input(&format!("API key for {display_name}:")) {
        Ok(key) => {
            if let Some(p) = provider {
                credentials::store_api_key(&p, &key)?;
            } else {
                // Unknown provider — store using raw config string
                let entry = keyring::Entry::new("cgx", provider_name)
                    .map_err(|e| changelog_x::CredentialError::Store(e.to_string()))?;
                entry
                    .set_password(&key)
                    .map_err(|e| changelog_x::CredentialError::Store(e.to_string()))?;
            }
        }
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            term.write_line(&format!("\n{}", style("Cancelled.").dim()))?;
            return Ok(());
        }
        Err(e) => return Err(e).context("API key input failed"),
    }

    term.write_line(&format!(
        "\n{} API key stored for {}",
        style("✓").green().bold(),
        style(display_name).cyan()
    ))?;

    Ok(())
}

fn cmd_ai_auth_clear() -> Result<()> {
    let term = Term::stdout();
    let config = load_config(None).context("Failed to load configuration")?;

    let provider_name = config
        .ai
        .provider
        .as_deref()
        .ok_or(AppError::Ai(changelog_x::AiError::NotConfigured))?;

    if !credentials::has_api_key(provider_name) {
        term.write_line(&format!(
            "{} No API key stored for {}",
            style("note:").yellow(),
            style(provider_name).cyan()
        ))?;
        return Ok(());
    }

    let display_name = Provider::from_config_str(provider_name)
        .map_or_else(|| provider_name.to_string(), |p| p.to_string());

    match ui::confirm(
        &format!("Remove API key for {display_name} from system keyring?"),
        false,
    ) {
        Ok(true) => {
            credentials::delete_api_key(provider_name)?;
            term.write_line(&format!(
                "{} API key removed for {}",
                style("✓").green().bold(),
                style(display_name).cyan()
            ))?;
        }
        Ok(false) => {
            term.write_line(&format!("{}", style("Cancelled.").dim()))?;
        }
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            term.write_line(&format!("\n{}", style("Cancelled.").dim()))?;
        }
        Err(e) => return Err(e).context("Confirmation failed"),
    }

    Ok(())
}

fn cmd_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Show => cmd_config_show(),
        ConfigAction::Path => cmd_config_path(),
        ConfigAction::Edit => cmd_config_edit(),
    }
}

fn cmd_config_show() -> Result<()> {
    let term = Term::stdout();
    let config = load_config(None).context("Failed to load configuration")?;

    // Check which config sources exist
    let user_config_exists = get_user_config_path().is_some_and(|p| p.exists());
    let project_config_exists = Path::new(".cgx.toml").exists();
    let has_config_file = user_config_exists || project_config_exists;

    debug!("Configuration sources (in precedence order):");
    debug!("  1. Built-in defaults");

    if let Some(user_path) = get_user_config_path() {
        let status = if user_path.exists() {
            "found"
        } else {
            "not found"
        };
        debug!("  2. User config: {} ({})", user_path.display(), status);
    }

    let project_config = Path::new(".cgx.toml");
    let status = if project_config.exists() {
        "found"
    } else {
        "not found"
    };
    debug!("  3. Project config: .cgx.toml ({})", status);
    debug!("  4. Environment variables: CGX_* prefix");

    let toml_output =
        toml::to_string_pretty(&config).context("Failed to serialize configuration")?;

    if has_config_file {
        term.write_line(&format!("{}", style("# Resolved configuration").dim()))?;
    } else {
        term.write_line(&format!(
            "{}",
            style("# Resolved configuration (no config files found, showing defaults + env vars)")
                .dim()
        ))?;
    }
    term.write_line(&toml_output)?;
    Ok(())
}

fn cmd_config_path() -> Result<()> {
    let term = Term::stdout();

    term.write_line(&format!("{}", style("Configuration file paths:").bold()))?;
    term.write_line("")?;

    // User config
    if let Some(user_path) = get_user_config_path() {
        let (status, status_style) = if user_path.exists() {
            ("exists", style("exists").green())
        } else {
            ("not found", style("not found").yellow())
        };
        debug!("User config status: {}", status);
        term.write_line(&format!(
            "  {} {}  ({})",
            style("User config:").cyan(),
            user_path.display(),
            status_style
        ))?;
    } else {
        term.write_line(&format!(
            "  {} {}",
            style("User config:").cyan(),
            style("(unable to determine path)").red()
        ))?;
    }

    // Project config
    let project_path = Path::new(".cgx.toml");
    let status_style = if project_path.exists() {
        style("exists").green()
    } else {
        style("not found").yellow()
    };

    if let Ok(cwd) = std::env::current_dir() {
        term.write_line(&format!(
            "  {} {}  ({})",
            style("Project config:").cyan(),
            cwd.join(".cgx.toml").display(),
            status_style
        ))?;
    } else {
        term.write_line(&format!(
            "  {} {}  ({})",
            style("Project config:").cyan(),
            ".cgx.toml",
            status_style
        ))?;
    }

    debug!("Environment variable prefix: CGX_");
    debug!("  Use __ (double underscore) for nested fields");

    Ok(())
}

fn cmd_config_edit() -> Result<()> {
    let term = Term::stdout();
    let config_path =
        get_user_config_path().context("Unable to determine user config directory")?;

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent()
        && !parent.exists()
    {
        debug!("Creating config directory: {}", parent.display());
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Read existing content or use template
    let existing_content = if config_path.exists() {
        fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read: {}", config_path.display()))?
    } else {
        debug!("No existing config, using template");
        String::from(
            r#"# cgx user configuration

[changelog]
# output = "CHANGELOG.md"
# tag_pattern = "v*"

[ai]
# provider = "openai"  # openai, anthropic, gemini, ollama, groq, deepseek
# model = "gpt-4o"
"#,
        )
    };

    // Open editor with existing content
    let edited = Editor::new("Edit configuration:")
        .with_predefined_text(&existing_content)
        .with_file_extension(".toml")
        .prompt()
        .context("Editor cancelled or failed")?;

    // Write updated content
    fs::write(&config_path, &edited)
        .with_context(|| format!("Failed to write: {}", config_path.display()))?;

    term.write_line(&format!(
        "{} Configuration saved to {}",
        style("✓").green().bold(),
        style(config_path.display()).cyan()
    ))?;
    Ok(())
}
