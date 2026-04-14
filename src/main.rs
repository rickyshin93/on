mod browser;
mod config;
mod editor;
mod git;
mod iterm;
mod port;
mod process;
mod state;

use anyhow::{anyhow, bail, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use dialoguer::FuzzySelect;

#[derive(Parser)]
#[command(
    name = "on",
    version,
    about = "One-command dev environment launcher"
)]
struct Cli {
    /// Project name to launch
    project: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Stop project services
    Stop {
        /// Project name to stop
        project: Option<String>,
        /// Stop all running projects
        #[arg(long)]
        all: bool,
    },
    /// List all projects and their status
    List,
    /// Edit project config in $EDITOR
    Edit {
        /// Project name
        project: String,
    },
    /// Create new project config from template
    New {
        /// Project name
        project: String,
    },
    /// Check environment for common issues
    Doctor,
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Stop { project, all }) => {
            if all {
                process::stop_all()
            } else if let Some(name) = project {
                process::stop(&name)
            } else {
                Err(anyhow!("Usage: on stop <project> or on stop --all"))
            }
        }
        Some(Commands::List) => process::list(),
        Some(Commands::Edit { project }) => process::edit(&project),
        Some(Commands::New { project }) => process::new_project(&project),
        Some(Commands::Doctor) => process::doctor(),
        Some(Commands::Completions { shell }) => {
            generate(shell, &mut Cli::command(), "on", &mut std::io::stdout());
            Ok(())
        }
        None => {
            if let Some(name) = cli.project {
                process::run(&name)
            } else {
                fuzzy_select()
            }
        }
    };

    if let Err(e) = result {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

fn fuzzy_select() -> Result<()> {
    config::ensure_dirs()?;
    let projects = config::list_projects();
    if projects.is_empty() {
        bail!("No projects configured. Run `on new <name>` to create one.");
    }

    let selection = FuzzySelect::new()
        .with_prompt("Select project")
        .items(&projects)
        .interact_opt()
        .map_err(|e| anyhow::anyhow!("Selection error: {e}"))?;

    if let Some(idx) = selection {
        process::run(&projects[idx])
    } else {
        println!("Cancelled.");
        Ok(())
    }
}
