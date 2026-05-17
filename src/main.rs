mod browser;
mod config;
mod editor;
mod git;
mod iterm;
mod port;
mod process;
mod selection;
mod state;
mod tmux;

use anyhow::{anyhow, bail, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use dialoguer::FuzzySelect;

use crate::selection::{Component, LaunchSelection};

#[derive(Parser)]
#[command(name = "on", version, about = "One-command dev environment launcher")]
struct Cli {
    /// Project name to launch
    project: Option<String>,

    /// Launch only the selected components (repeatable). Default: all.
    #[arg(long = "only", value_name = "COMPONENT", value_enum)]
    only: Vec<Component>,

    /// Launch only the editor (combinable with -t / -b)
    #[arg(short = 'e', long = "editor")]
    editor: bool,

    /// Launch only the terminal panes (combinable with -e / -b)
    #[arg(short = 't', long = "terminal")]
    terminal: bool,

    /// Launch only the browser (combinable with -e / -t)
    #[arg(short = 'b', long = "browser")]
    browser: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// View pane output logs
    Log {
        /// Project name
        project: String,
        /// Pane name (optional, shows all if omitted)
        pane: Option<String>,
        /// Follow log output in real-time (iTerm only)
        #[arg(short, long)]
        follow: bool,
    },
    /// Show detailed project status
    Status {
        /// Project name
        project: String,
    },
    /// Restart project (stop + start)
    Restart {
        /// Project name to restart
        project: String,
    },
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
    /// Clone an existing project config
    Clone {
        /// Source project name
        source: String,
        /// New project name
        target: String,
    },
    /// Auto-detect project and create config from current directory
    Init,
    /// Check environment for common issues
    Doctor,
    /// Validate one or all project configs without launching anything
    Validate {
        /// Project name (omit to validate all configs)
        project: Option<String>,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Log {
            project,
            pane,
            follow,
        }) => process::log(&project, pane.as_deref(), follow),
        Some(Commands::Status { project }) => process::status(&project),
        Some(Commands::Restart { project }) => process::restart(&project),
        Some(Commands::Stop { project, all }) => {
            if all {
                process::stop_all()
            } else if let Some(name) = project {
                process::stop(&name)
            } else {
                Err(anyhow!("Usage: on stop <project> or on stop --all"))
            }
        }
        Some(Commands::Clone { source, target }) => process::clone_project(&source, &target),
        Some(Commands::Init) => process::init(),
        Some(Commands::List) => process::list(),
        Some(Commands::Edit { project }) => process::edit(&project),
        Some(Commands::New { project }) => process::new_project(&project),
        Some(Commands::Doctor) => process::doctor(),
        Some(Commands::Validate { project }) => process::validate(project.as_deref()),
        Some(Commands::Completions { shell }) => {
            generate(shell, &mut Cli::command(), "on", &mut std::io::stdout());
            Ok(())
        }
        None => {
            let selection =
                LaunchSelection::from_flags(&cli.only, cli.editor, cli.terminal, cli.browser);
            if let Some(name) = cli.project {
                process::run(&name, selection)
            } else {
                fuzzy_select(selection)
            }
        }
    };

    if let Err(e) = result {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

fn fuzzy_select(selection: LaunchSelection) -> Result<()> {
    config::ensure_dirs()?;
    let projects = config::list_projects();
    if projects.is_empty() {
        bail!("No projects configured. Run `on new <name>` to create one.");
    }

    let picked = FuzzySelect::new()
        .with_prompt("Select project")
        .items(&projects)
        .interact_opt()
        .map_err(|e| anyhow::anyhow!("Selection error: {e}"))?;

    if let Some(idx) = picked {
        process::run(&projects[idx], selection)
    } else {
        println!("Cancelled.");
        Ok(())
    }
}
