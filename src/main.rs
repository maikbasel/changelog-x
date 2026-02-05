use anyhow::{Context, Result};
use changelog_x::config::{get_user_config_path, load_config};
use clap::{Parser, Subcommand};
use console::{Term, style};
use inquire::Editor;
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
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

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
        #[arg(long)]
        from: Option<String>,

        /// End at this git tag
        #[arg(long)]
        to: Option<String>,
    },

    /// Enhance an existing changelog file with AI
    Enhance {
        /// File to enhance
        #[arg(default_value = "CHANGELOG.md")]
        file: String,

        /// Print result without overwriting file
        #[arg(long)]
        dry_run: bool,

        /// Write to different file instead of overwriting
        #[arg(short, long)]
        output: Option<String>,
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

    /// Configure AI provider and credentials
    Setup,

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
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing with appropriate log level
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("warn")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();

    match cli.command {
        Some(Commands::Generate {
            ai,
            stdout,
            output,
            from,
            to,
        }) => cmd_generate(ai, stdout, &output, from, to).await,

        Some(Commands::Enhance {
            file,
            dry_run,
            output,
        }) => cmd_enhance(&file, dry_run, output).await,

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

async fn cmd_generate(
    ai: bool,
    stdout: bool,
    output: &str,
    from: Option<String>,
    to: Option<String>,
) -> Result<()> {
    debug!(
        "Generate command: ai={}, stdout={}, output={}, from={:?}, to={:?}",
        ai, stdout, output, from, to
    );

    let term = Term::stdout();
    term.write_line(&format!(
        "{}",
        style("Changelog generation not yet implemented").yellow()
    ))?;
    Ok(())
}

async fn cmd_enhance(file: &str, dry_run: bool, output: Option<String>) -> Result<()> {
    debug!(
        "Enhance command: file={}, dry_run={}, output={:?}",
        file, dry_run, output
    );

    let term = Term::stdout();
    term.write_line(&format!(
        "{}",
        style("AI enhancement not yet implemented").yellow()
    ))?;
    Ok(())
}

async fn cmd_init(force: bool, skip_ai: bool) -> Result<()> {
    debug!("Init command: force={}, skip_ai={}", force, skip_ai);

    let term = Term::stdout();
    term.write_line(&format!(
        "{}",
        style("Init wizard not yet implemented").yellow()
    ))?;
    Ok(())
}

async fn cmd_ai(action: AiAction) -> Result<()> {
    debug!("AI command: {:?}", action);

    let term = Term::stdout();
    match action {
        AiAction::Status => {
            term.write_line(&format!(
                "{}",
                style("AI status not yet implemented").yellow()
            ))?;
        }
        AiAction::Setup => {
            term.write_line(&format!(
                "{}",
                style("AI setup not yet implemented").yellow()
            ))?;
        }
        AiAction::Clear => {
            term.write_line(&format!(
                "{}",
                style("AI clear not yet implemented").yellow()
            ))?;
        }
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
