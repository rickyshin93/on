use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use colored::Colorize;

use crate::{browser, config, editor, git, iterm, port, state, tmux};

/// Main launch flow for a project
#[allow(clippy::too_many_lines)]
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

    // Git status check (only when checks.dirty_git: true in config)
    let dirty_git_enabled = cfg
        .checks
        .as_ref()
        .and_then(|c| c.dirty_git)
        .unwrap_or(false);
    if dirty_git_enabled {
        if let Some(ref term_cfg) = cfg.terminal {
            let dirs: Vec<String> = term_cfg.panes.iter().map(|p| p.dir.clone()).collect();
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
    }

    // Port conflict check
    let urls: Vec<String> = cfg.browser.clone().unwrap_or_default();
    let cmds: Vec<String> = cfg
        .terminal
        .as_ref()
        .map(|t| t.panes.iter().filter_map(|p| p.cmd.clone()).collect())
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

    let mut terminal_type = String::new();

    // Open terminal panes
    if let Some(ref term_cfg) = cfg.terminal {
        let layout = term_cfg.layout.as_deref().unwrap_or("vertical");
        terminal_type.clone_from(&term_cfg.terminal_type);

        let max_per_tab = Some(term_cfg.max_panes_per_tab.unwrap_or(4));
        match term_cfg.terminal_type.as_str() {
            "tmux" => {
                tmux::open_panes(name, &term_cfg.panes, layout, max_per_tab)?;
            }
            _ => {
                iterm::open_panes(name, &term_cfg.panes, layout, max_per_tab)?;
            }
        }

        // Collect PIDs from pid files
        let pane_states = collect_pids(name, &term_cfg.panes);
        if !pane_states.is_empty() {
            let project_state = state::ProjectState {
                project: name.to_string(),
                started_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                terminal_type: terminal_type.clone(),
                panes: pane_states,
            };
            state::save(&project_state)?;
        }
    }

    // Open editor
    editor::open(cfg.editor.as_ref(), name)?;

    // Open browser
    browser::open(cfg.browser.as_ref())?;

    // For tmux, attach last (this blocks)
    if terminal_type == "tmux" {
        println!(
            "{}",
            format!("Attaching to tmux session for '{name}'...").green()
        );
        tmux::attach(name)?;
    } else {
        println!("{}", format!("Project '{name}' is on!").green());
    }

    Ok(())
}

/// Poll for PID files after terminal panes are opened
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

/// Stop a project: kill processes, close terminal, remove state
pub fn stop(name: &str) -> Result<()> {
    match state::load(name)? {
        None => {
            println!("Project '{name}' is not running.");
            return Ok(());
        }
        Some(s) => {
            kill_all_process_groups(&s.panes);
            match s.terminal_type.as_str() {
                "tmux" => tmux::close_session(name),
                _ => iterm::close_tabs(name),
            }
        }
    }

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

/// Kill all process trees: SIGTERM groups + children, wait, then SIGKILL survivors
fn kill_all_process_groups(panes: &[state::PaneState]) {
    let targets: Vec<(u32, String)> = panes
        .iter()
        .map(|p| (p.pid, resolve_kill_target(p.pid)))
        .collect();

    // SIGTERM all process groups at once
    for (_, target) in &targets {
        let _ = Command::new("kill").args(["--", target]).output();
    }

    // Also SIGTERM child processes (command may be in a different process group)
    for (pid, _) in &targets {
        let _ = Command::new("pkill")
            .args(["-TERM", "-P", &pid.to_string()])
            .output();
    }

    // Single wait
    thread::sleep(Duration::from_millis(300));

    // SIGKILL any survivors
    for (pid, target) in &targets {
        if state::is_pid_alive(*pid) {
            let _ = Command::new("kill").args(["-9", "--", target]).output();
        }
        let _ = Command::new("pkill")
            .args(["-9", "-P", &pid.to_string()])
            .output();
    }
}

/// Resolve a PID to its process group kill target
fn resolve_kill_target(pid: u32) -> String {
    Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .map(|g| format!("-{g}"))
            } else {
                None
            }
        })
        .unwrap_or_else(|| pid.to_string())
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
            "Config not found: {}\nRun `on new {name}` to create one.",
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

    // Check iTerm2 (macOS only)
    if cfg!(target_os = "macos") {
        let iterm_installed = Path::new("/Applications/iTerm.app").exists();
        print_check(iterm_installed, "iTerm2 installed");
    }

    // Check tmux
    let tmux_ok = Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success());
    print_check(tmux_ok, "tmux available");

    // Check ~/.on/ directory
    let on_dir_exists = config::base_dir().exists();
    print_check(on_dir_exists, "~/.on/ directory exists");

    // Check project count
    let projects = config::list_projects();
    println!("  {} {} project(s) configured", "i".blue(), projects.len());

    // Check git
    let git_ok = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    print_check(git_ok, "git available");

    // Check lsof
    let lsof_ok = Command::new("lsof")
        .arg("-v")
        .output()
        .is_ok_and(|_| true);
    print_check(lsof_ok, "lsof available");

    println!();
    if tmux_ok && git_ok && lsof_ok {
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
