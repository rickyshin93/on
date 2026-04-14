use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use colored::Colorize;

use crate::{browser, config, editor, git, iterm, port, state};

/// Main launch flow for a project
pub fn run(name: &str) -> Result<()> {
    config::ensure_dirs()?;
    let cfg = config::load(name)?;

    // Check if already running
    if state::is_running(name)? {
        println!(
            "{}",
            format!("Project '{name}' is already running.").yellow()
        );
        print!("Restart? [Y/n] ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim().to_lowercase();
        if input == "n" || input == "no" {
            println!("Aborted.");
            return Ok(());
        }
        stop(name)?;
    }

    // Git status check
    if let Some(ref iterm_cfg) = cfg.iterm {
        let dirs: Vec<String> = iterm_cfg.panes.iter().map(|p| p.dir.clone()).collect();
        let dirty = git::check_status(&dirs);
        if !dirty.is_empty() {
            for d in &dirty {
                println!(
                    "{}",
                    format!("  {} has {} uncommitted file(s)", d.dir, d.file_count).yellow()
                );
            }
            print!("Continue? [Y/n] ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            let input = input.trim().to_lowercase();
            if input == "n" || input == "no" {
                println!("Aborted.");
                return Ok(());
            }
        }
    }

    // Port conflict check
    let urls: Vec<String> = cfg.browser.clone().unwrap_or_default();
    let cmds: Vec<String> = cfg
        .iterm
        .as_ref()
        .map(|i| i.panes.iter().filter_map(|p| p.cmd.clone()).collect())
        .unwrap_or_default();
    let ports = port::extract_ports(&urls, &cmds);

    for p in &ports {
        if let Some(conflict) = port::check_port(*p) {
            println!(
                "{}",
                format!(
                    "  Port {} is occupied (process: {}, PID: {})",
                    conflict.port, conflict.process_name, conflict.pid
                )
                .yellow()
            );
            print!("[K]ill / [S]kip / [A]bort? ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            match input.trim().to_lowercase().as_str() {
                "k" | "kill" => {
                    port::kill_pid(conflict.pid);
                    println!("  Killed PID {}", conflict.pid);
                }
                "a" | "abort" => {
                    println!("Aborted.");
                    return Ok(());
                }
                _ => {
                    println!("  Skipped port {p}");
                }
            }
        }
    }

    // Open iTerm2 panes
    if let Some(ref iterm_cfg) = cfg.iterm {
        let layout = iterm_cfg.layout.as_deref().unwrap_or("vertical");
        iterm::open_panes(name, &iterm_cfg.panes, layout)?;

        // Collect PIDs from pid files
        let pane_states = collect_pids(name, &iterm_cfg.panes);
        if !pane_states.is_empty() {
            let project_state = state::ProjectState {
                project: name.to_string(),
                started_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                panes: pane_states,
            };
            state::save(&project_state)?;
        }
    }

    // Open editor
    editor::open(cfg.editor.as_ref())?;

    // Open browser
    browser::open(cfg.browser.as_ref())?;

    println!("{}", format!("Project '{name}' is on!").green());
    Ok(())
}

/// Poll for PID files after iTerm2 panes are opened
fn collect_pids(project: &str, panes: &[config::PaneConfig]) -> Vec<state::PaneState> {
    let mut results = Vec::new();

    for pane in panes {
        if pane.cmd.is_none() {
            continue;
        }
        let pid_file = format!("/tmp/.on_{project}_{}.pid", pane.name);
        if let Some(pid) = poll_pid_file(&pid_file) {
            results.push(state::PaneState {
                name: pane.name.clone(),
                pid,
            });
        }
    }

    results
}

/// Poll for a PID file, checking every 100ms for up to 3 seconds
fn poll_pid_file(path: &str) -> Option<u32> {
    for _ in 0..30 {
        if let Ok(content) = std::fs::read_to_string(path) {
            let pid = content.trim().parse::<u32>().ok();
            if pid.is_some() {
                let _ = std::fs::remove_file(path);
                return pid;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    None
}

/// Stop a project: kill processes, close iTerm2 tabs, remove state
pub fn stop(name: &str) -> Result<()> {
    match state::load(name)? {
        None => {
            println!("Project '{name}' is not running.");
            return Ok(());
        }
        Some(s) => {
            for pane in &s.panes {
                kill_process_group(pane.pid);
            }
        }
    }

    iterm::close_tabs(name);
    state::remove(name)?;
    println!("{}", format!("Project '{name}' stopped.").green());
    Ok(())
}

/// Stop all running projects
pub fn stop_all() -> Result<()> {
    let projects = state::running_projects();
    if projects.is_empty() {
        println!("No projects are running.");
        return Ok(());
    }
    for project in &projects {
        stop(project)?;
    }
    Ok(())
}

/// Kill a process group: SIGTERM first, wait 3s, then SIGKILL if needed
#[allow(clippy::similar_names)]
fn kill_process_group(pid: u32) {
    let pgid = Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
            } else {
                None
            }
        });

    let target = match pgid {
        Some(g) => format!("-{g}"),
        None => pid.to_string(),
    };

    let _ = Command::new("kill").args(["--", &target]).output();
    thread::sleep(Duration::from_secs(3));

    if state::is_pid_alive(pid) {
        let _ = Command::new("kill").args(["-9", "--", &target]).output();
    }
}

/// List all projects and their status
pub fn list() -> Result<()> {
    config::ensure_dirs()?;
    let projects = config::list_projects();

    if projects.is_empty() {
        println!("No projects configured. Run `on new <name>` to create one.");
        return Ok(());
    }

    println!("{:<20} {:<12} Panes", "Project", "Status");
    println!("{}", "-".repeat(50));

    for project in &projects {
        let (status, pane_names) = match state::load(project)? {
            Some(s) => {
                let alive: Vec<&str> = s
                    .panes
                    .iter()
                    .filter(|p| state::is_pid_alive(p.pid))
                    .map(|p| p.name.as_str())
                    .collect();
                if alive.is_empty() {
                    ("stopped".to_string(), "-".to_string())
                } else {
                    ("running".green().to_string(), alive.join(", "))
                }
            }
            None => ("stopped".to_string(), "-".to_string()),
        };
        println!("{project:<20} {status:<12} {pane_names}");
    }
    Ok(())
}

/// Edit a project config in $EDITOR
pub fn edit(name: &str) -> Result<()> {
    let path = config::config_path(name);
    if !path.exists() {
        bail!(
            "Config not found: {}\nRun `launch new {name}` to create one.",
            path.display(),
        );
    }

    let editor_cmd = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    Command::new(&editor_cmd)
        .arg(path.to_str().unwrap())
        .status()
        .with_context(|| format!("Failed to open editor '{editor_cmd}'"))?;
    Ok(())
}

/// Create a new project config from template
pub fn new_project(name: &str) -> Result<()> {
    let path = config::create_template(name)?;
    println!("{}", format!("Created config: {}", path.display()).green());

    let editor_cmd = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    Command::new(&editor_cmd)
        .arg(path.to_str().unwrap())
        .status()
        .with_context(|| format!("Failed to open editor '{editor_cmd}'"))?;
    Ok(())
}

/// Check environment for common issues
#[allow(clippy::unnecessary_wraps)]
pub fn doctor() -> Result<()> {
    use std::path::Path;

    println!("on doctor\n");

    // Check iTerm2
    let iterm_installed = Path::new("/Applications/iTerm.app").exists();
    print_check(iterm_installed, "iTerm2 installed");

    // Check ~/.launch/ directory
    let on_dir_exists = config::base_dir().exists();
    print_check(on_dir_exists, "~/.on/ directory exists");

    // Check project count
    let projects = config::list_projects();
    println!("  {} {} project(s) configured", "i".blue(), projects.len());

    // Check git
    let git_ok = Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    print_check(git_ok, "git available");

    // Check lsof
    let lsof_ok = Command::new("lsof")
        .arg("-v")
        .output()
        .map(|_| true)
        .unwrap_or(false);
    print_check(lsof_ok, "lsof available");

    println!();
    if iterm_installed && git_ok && lsof_ok {
        println!("{}", "All checks passed!".green());
    } else {
        println!("{}", "Some checks failed. See above.".yellow());
    }

    Ok(())
}

fn print_check(ok: bool, label: &str) {
    if ok {
        println!("  {} {label}", "✓".green());
    } else {
        println!("  {} {label}", "✗".red());
    }
}
