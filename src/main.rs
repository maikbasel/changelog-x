use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use changelog_x::config::{get_config_dir, get_user_config_path, load_config};

#[derive(Parser)]
#[command(name = "changelog-x")]
#[command(
    author,
    version,
    about = "Generate high-quality changelogs from conventional commits with AI enhancement"
)]
#[command(propagate_version = true)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Path to configuration file
    #[arg(short, long, global = true)]
    config: Option<String>,

    /// Output file path (overrides config)
    #[arg(short, long, global = true)]
    output: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a changelog from conventional commits
    Generate {
        /// Include unreleased changes
        #[arg(long)]
        unreleased: bool,

        /// Only include commits from this tag onwards
        #[arg(long)]
        from: Option<String>,

        /// Only include commits up to this tag
        #[arg(long)]
        to: Option<String>,

        /// Enable AI enhancement for the changelog
        #[arg(long)]
        ai: bool,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Initialize changelog-x in the current project
    Init {
        /// Overwrite existing configuration
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,

    /// Show configuration file paths
    Path,

    /// Edit user configuration
    Edit,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load configuration
    let config = load_config(cli.config.as_deref()).context("Failed to load configuration")?;

    if cli.verbose {
        eprintln!("Configuration loaded: {:?}", config);
    }

    match cli.command {
        Some(Commands::Generate {
            unreleased,
            from,
            to,
            ai,
        }) => cmd_generate(&config, unreleased, from, to, ai, cli.output).await,
        Some(Commands::Config { action }) => cmd_config(action),
        Some(Commands::Init { force }) => cmd_init(force),
        None => {
            // Default action: generate changelog
            cmd_generate(&config, true, None, None, false, cli.output).await
        }
    }
}

async fn cmd_generate(
    config: &changelog_x::config::AppConfig,
    _unreleased: bool,
    _from: Option<String>,
    _to: Option<String>,
    _ai: bool,
    _output: Option<String>,
) -> Result<()> {
    use changelog_x::ai::AiEnhancer;
    use changelog_x::changelog::ChangelogGenerator;

    let generator = ChangelogGenerator::new(config.changelog.clone());
    let enhancer = AiEnhancer::new(config.ai.clone());

    // Generate the changelog
    let changelog = generator.generate(None)?;

    // Optionally enhance with AI
    let final_changelog = if enhancer.is_available() {
        enhancer.enhance(&changelog).await?
    } else {
        changelog
    };

    println!("{}", final_changelog);
    Ok(())
}

fn cmd_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Show => {
            let config = load_config(None)?;
            println!("{:#?}", config);
        }
        ConfigAction::Path => {
            println!("User config directory: {:?}", get_config_dir());
            println!("User config file: {:?}", get_user_config_path());
            println!("Project config file: .changelog-x.toml");
        }
        ConfigAction::Edit => {
            if let Some(config_path) = get_user_config_path() {
                if let Some(parent) = config_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if !config_path.exists() {
                    std::fs::write(&config_path, default_config_content())?;
                }
                println!("Config file: {}", config_path.display());
                println!("Open this file in your editor to configure changelog-x");
            } else {
                anyhow::bail!("Could not determine config directory");
            }
        }
    }
    Ok(())
}

fn cmd_init(force: bool) -> Result<()> {
    let config_path = std::path::Path::new(".changelog-x.toml");

    if config_path.exists() && !force {
        anyhow::bail!("Configuration file already exists. Use --force to overwrite.");
    }

    std::fs::write(config_path, default_config_content())?;
    println!("Created .changelog-x.toml");
    println!("Edit this file to customize changelog generation.");
    Ok(())
}

fn default_config_content() -> &'static str {
    r#"# changelog-x configuration

[changelog]
# Output file path
output = "CHANGELOG.md"

# Include unreleased changes
unreleased = true

# Tag pattern for version matching (optional)
# tag_pattern = "v*"

[ai]
# Enable AI enhancement
enabled = false

# AI provider: openai, anthropic, gemini, ollama, groq, deepseek
# provider = "openai"

# Model name (provider-specific)
# model = "gpt-4"
"#
}
