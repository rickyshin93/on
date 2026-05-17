use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use colored::Colorize;

use crate::selection::LaunchSelection;
use crate::{browser, config, editor, git, iterm, port, state, tmux};

/// Interpret a yes/no prompt response, defaulting to yes on empty input.
fn parse_yes_default(input: &str) -> bool {
    !matches!(input.trim().to_lowercase().as_str(), "n" | "no")
}

/// Map an `ON_NONINTERACTIVE` env var value to "should skip prompts".
/// Unset, empty, `0`, `false`, and `no` are interactive; anything else
/// disables prompts (defaults are chosen automatically).
fn parse_non_interactive(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        None | Some("" | "0") => false,
        Some(v) => !matches!(v.to_lowercase().as_str(), "false" | "no"),
    }
}

fn is_non_interactive() -> bool {
    parse_non_interactive(std::env::var("ON_NONINTERACTIVE").ok().as_deref())
}

#[derive(Debug, PartialEq, Eq)]
enum PortAction {
    Kill,
    Skip,
    Abort,
}

/// Interpret a kill/skip/abort prompt response, defaulting to skip.
fn parse_port_action(input: &str) -> PortAction {
    match input.trim().to_lowercase().as_str() {
        "k" | "kill" => PortAction::Kill,
        "a" | "abort" => PortAction::Abort,
        _ => PortAction::Skip,
    }
}

/// Read a single line from stdin and return the prompt response interpreted
/// as yes/no. In non-interactive mode (`ON_NONINTERACTIVE=1`) returns `true`
/// without touching stdin so scripts don't hang on the prompt.
fn prompt_yes_default(prompt: &str) -> Result<bool> {
    if is_non_interactive() {
        return Ok(true);
    }
    print!("{prompt}");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read stdin")?;
    Ok(parse_yes_default(&input))
}

/// Print a prompt, read a single line, return the trimmed value.
fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read stdin")?;
    Ok(input.trim().to_string())
}

fn prompt_port_action(prompt: &str) -> Result<PortAction> {
    if is_non_interactive() {
        // Safe default: don't kill anyone else's process behind the user's back.
        return Ok(PortAction::Skip);
    }
    print!("{prompt}");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read stdin")?;
    Ok(parse_port_action(&input))
}

fn run_hooks(hooks: &[String], phase: &str) -> Result<()> {
    for cmd in hooks {
        println!("  {} {cmd}", phase.dimmed());
        let status = Command::new("sh")
            .args(["-c", cmd])
            .status()
            .with_context(|| format!("Failed to run {phase} hook: {cmd}"))?;
        if !status.success() {
            bail!("{phase} hook failed: {cmd}");
        }
    }
    Ok(())
}

/// Returns `Ok(false)` when the user chose to abort, `Ok(true)` to proceed.
fn confirm_restart_if_running(name: &str, selection: LaunchSelection) -> Result<bool> {
    if !(selection.terminal && state::is_running(name)?) {
        return Ok(true);
    }
    println!(
        "{}",
        format!("Project '{name}' is already running.").yellow()
    );
    if !prompt_yes_default("Restart? [Y/n] ")? {
        println!("Aborted.");
        return Ok(false);
    }
    stop(name)?;
    Ok(true)
}

fn confirm_clean_git(cfg: &config::Config, selection: LaunchSelection) -> Result<bool> {
    let dirty_git_enabled = cfg
        .checks
        .as_ref()
        .and_then(|c| c.dirty_git)
        .unwrap_or(false);
    if !(selection.terminal && dirty_git_enabled) {
        return Ok(true);
    }
    let Some(ref term_cfg) = cfg.terminal else {
        return Ok(true);
    };
    let dirs: Vec<String> = term_cfg.panes.iter().map(|p| p.dir.clone()).collect();
    let dirty = git::check_status(&dirs);
    if dirty.is_empty() {
        return Ok(true);
    }
    for d in &dirty {
        println!(
            "{}",
            format!("  {} has {} uncommitted file(s)", d.dir, d.file_count).yellow()
        );
    }
    if !prompt_yes_default("Continue? [Y/n] ")? {
        println!("Aborted.");
        return Ok(false);
    }
    Ok(true)
}

fn resolve_port_conflicts(cfg: &config::Config, selection: LaunchSelection) -> Result<bool> {
    let urls: Vec<String> = if selection.browser {
        cfg.browser.clone().unwrap_or_default()
    } else {
        Vec::new()
    };
    let cmds: Vec<String> = if selection.terminal {
        cfg.terminal
            .as_ref()
            .map(|t| t.panes.iter().filter_map(|p| p.cmd.clone()).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let ports = port::extract_ports(&urls, &cmds);
    let conflicts = port::check_ports(&ports);

    for conflict in &conflicts {
        let cmdline =
            port::process_cmdline(conflict.pid).unwrap_or_else(|| conflict.process_name.clone());
        println!(
            "{}",
            format!(
                "  Port {} is occupied (PID {}: {})",
                conflict.port, conflict.pid, cmdline
            )
            .yellow()
        );
        match prompt_port_action("[K]ill / [S]kip / [A]bort? ")? {
            PortAction::Kill => {
                port::kill_pid(conflict.pid);
                println!("  Killed PID {}", conflict.pid);
            }
            PortAction::Abort => {
                println!("Aborted.");
                return Ok(false);
            }
            PortAction::Skip => {
                println!("  Skipped port {}", conflict.port);
            }
        }
    }
    Ok(true)
}

/// Main launch flow for a project
pub fn run(name: &str, selection: LaunchSelection) -> Result<()> {
    config::ensure_dirs()?;
    let cfg = config::load(name)?;

    if !confirm_restart_if_running(name, selection)? {
        return Ok(());
    }
    if !confirm_clean_git(&cfg, selection)? {
        return Ok(());
    }
    if !resolve_port_conflicts(&cfg, selection)? {
        return Ok(());
    }

    // Pre-launch hooks
    if let Some(ref hooks) = cfg.hooks {
        if let Some(ref cmds) = hooks.pre_launch {
            run_hooks(cmds, "pre_launch")?;
        }
    }

    let mut terminal_type = String::new();

    // Open terminal panes
    if selection.terminal {
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
    }

    // Open editor
    if selection.editor {
        editor::open(cfg.editor.as_ref(), name)?;
    }

    // Open browser
    if selection.browser {
        browser::open(cfg.browser.as_ref())?;
    }

    // Post-launch hooks
    if let Some(ref hooks) = cfg.hooks {
        if let Some(ref cmds) = hooks.post_launch {
            run_hooks(cmds, "post_launch")?;
        }
    }

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
        let pid_file = config::pid_file_path(project, &pane.name);
        if let Some(pid) = poll_pid_file(&pid_file) {
            results.push(state::PaneState {
                name: pane.name.clone(),
                pid,
                process_started_at: state::process_started_at(pid),
            });
        }
    }

    results
}

/// Poll for a PID file, checking every 100ms for up to 3 seconds
fn poll_pid_file(path: &std::path::Path) -> Option<u32> {
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

/// Restart a project: stop (if running) then start
pub fn restart(name: &str) -> Result<()> {
    config::ensure_dirs()?;
    config::load(name)?;

    if state::is_running(name)? {
        stop(name)?;
    }
    run(name, LaunchSelection::all())
}

/// View pane output logs
pub fn log(name: &str, pane: Option<&str>, follow: bool) -> Result<()> {
    let s =
        state::load(name)?.ok_or_else(|| anyhow::anyhow!("Project '{name}' is not running."))?;

    let target_panes: Vec<&state::PaneState> = match pane {
        Some(p) => {
            if let Some(ps) = s.panes.iter().find(|ps| ps.name == p) {
                vec![ps]
            } else {
                let names: Vec<&str> = s.panes.iter().map(|ps| ps.name.as_str()).collect();
                bail!("Pane '{p}' not found. Available: {}", names.join(", "));
            }
        }
        None => s.panes.iter().collect(),
    };

    match s.terminal_type.as_str() {
        "tmux" => {
            for (i, ps) in s.panes.iter().enumerate() {
                if !target_panes.iter().any(|t| t.name == ps.name) {
                    continue;
                }
                if target_panes.len() > 1 {
                    println!("{}", format!("--- {} ---", ps.name).bold());
                }
                match tmux::capture_pane(name, i) {
                    Ok(output) => print!("{output}"),
                    Err(e) => println!("  (capture failed: {e})"),
                }
            }
        }
        _ => {
            for ps in &target_panes {
                let log_file = config::log_path(name, &ps.name);
                if target_panes.len() > 1 {
                    println!("{}", format!("--- {} ---", ps.name).bold());
                }
                if log_file.exists() {
                    if follow {
                        Command::new("tail")
                            .arg("-f")
                            .arg(&log_file)
                            .status()
                            .context("Failed to tail log file")?;
                    } else {
                        let content = std::fs::read_to_string(&log_file)
                            .with_context(|| format!("Failed to read {}", log_file.display()))?;
                        print!("{content}");
                    }
                } else {
                    println!("  (no log file yet)");
                }
            }
        }
    }
    Ok(())
}

/// Show detailed status of a project
pub fn status(name: &str) -> Result<()> {
    config::ensure_dirs()?;
    let cfg = config::load(name)?;

    match state::load(name)? {
        None => {
            println!("Project '{name}' is not running.");
        }
        Some(s) => {
            println!("Project: {}", name.bold());
            println!(
                "Started: {} ({})",
                s.started_at,
                format_duration(&s.started_at)
            );
            println!();

            println!("Panes:");
            for pane in &s.panes {
                let alive = state::verify_pane_alive(pane);
                if alive {
                    println!(
                        "  {} {:<12} PID {:<8} {}",
                        "●".green(),
                        pane.name,
                        pane.pid,
                        "alive".green()
                    );
                } else {
                    println!(
                        "  {} {:<12} PID {:<8} {}",
                        "✗".red(),
                        pane.name,
                        pane.pid,
                        "dead".red()
                    );
                }
            }

            let urls: Vec<String> = cfg.browser.unwrap_or_default();
            let cmds: Vec<String> = cfg
                .terminal
                .as_ref()
                .map(|t| t.panes.iter().filter_map(|p| p.cmd.clone()).collect())
                .unwrap_or_default();
            let ports = port::extract_ports(&urls, &cmds);
            if !ports.is_empty() {
                println!();
                println!("Ports:");
                let conflicts = port::check_ports(&ports);
                let by_port: std::collections::HashMap<u16, &port::PortConflict> =
                    conflicts.iter().map(|c| (c.port, c)).collect();
                for p in &ports {
                    if let Some(c) = by_port.get(p) {
                        println!("  {}  {} (PID {})", p, "listening".green(), c.pid);
                    } else {
                        println!("  {}  {}", p, "free".dimmed());
                    }
                }
            }
        }
    }
    Ok(())
}

fn format_duration(started_at: &str) -> String {
    let started = chrono::NaiveDateTime::parse_from_str(started_at, "%Y-%m-%dT%H:%M:%S");
    match started {
        Ok(start) => {
            let now = chrono::Local::now().naive_local();
            let dur = now.signed_duration_since(start);
            let total_secs = dur.num_seconds();
            if total_secs < 0 {
                return "just now".to_string();
            }
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            if hours > 0 {
                format!("{hours}h {mins}m ago")
            } else if mins > 0 {
                format!("{mins}m ago")
            } else {
                "just now".to_string()
            }
        }
        Err(_) => "unknown".to_string(),
    }
}

/// Stop a project: kill processes, close terminal, remove state
pub fn stop(name: &str) -> Result<()> {
    match state::load(name)? {
        None => {
            println!("Project '{name}' is not running.");
            return Ok(());
        }
        Some(s) => {
            if let Ok(cfg) = config::load(name) {
                if let Some(ref hooks) = cfg.hooks {
                    if let Some(ref cmds) = hooks.pre_stop {
                        run_hooks(cmds, "pre_stop")?;
                    }
                }
            }
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

/// Parse `ps -axo pid,ppid` output into a parent → children map.
/// Ignores the header line and any line that doesn't have two integers.
fn parse_ps_pid_ppid(output: &str) -> std::collections::HashMap<u32, Vec<u32>> {
    let mut tree: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for line in output.lines() {
        let mut it = line.split_whitespace();
        let (Some(child_s), Some(parent_s)) = (it.next(), it.next()) else {
            continue;
        };
        let (Ok(child), Ok(parent)) = (child_s.parse::<u32>(), parent_s.parse::<u32>()) else {
            continue;
        };
        tree.entry(parent).or_default().push(child);
    }
    tree
}

/// Breadth-first collect every descendant PID of `root` from a parent→children map.
/// Defends against cycles (which shouldn't exist in /proc) by visiting each PID once.
fn descendants_in(tree: &std::collections::HashMap<u32, Vec<u32>>, root: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(root);
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root);
    while let Some(parent) = queue.pop_front() {
        if let Some(children) = tree.get(&parent) {
            for &child in children {
                if seen.insert(child) {
                    out.push(child);
                    queue.push_back(child);
                }
            }
        }
    }
    out
}

/// Snapshot the system process tree by running `ps -axo pid,ppid`.
fn process_tree() -> std::collections::HashMap<u32, Vec<u32>> {
    let Ok(output) = Command::new("ps").args(["-axo", "pid,ppid"]).output() else {
        return std::collections::HashMap::new();
    };
    if !output.status.success() {
        return std::collections::HashMap::new();
    }
    parse_ps_pid_ppid(&String::from_utf8_lossy(&output.stdout))
}

struct KillTarget {
    pids: Vec<u32>,
    pg: String,
}

/// Kill every descendant of each pane PID (and the PG it belongs to).
///
/// `pkill -P` only catches direct children, which misses grandchildren that
/// daemonized themselves into a new session — e.g. `pnpm dev` spawns `node`
/// which spawns `rspack-node` and the latter survives a SIGTERM aimed at
/// the pnpm process. Snapshot the system process tree once with `ps`, BFS
/// all descendants, signal them explicitly.
fn kill_all_process_groups(panes: &[state::PaneState]) {
    let tree = process_tree();

    // For each pane root, gather (root + every descendant) and its PG.
    let targets: Vec<KillTarget> = panes
        .iter()
        .map(|p| {
            let mut pids = vec![p.pid];
            pids.extend(descendants_in(&tree, p.pid));
            KillTarget {
                pids,
                pg: resolve_kill_target(p.pid),
            }
        })
        .collect();

    // SIGTERM the process group (catches everything still in PG) plus each
    // PID individually (catches the orphans that broke out).
    for t in &targets {
        let _ = Command::new("kill").args(["--", &t.pg]).output();
        for pid in &t.pids {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .output();
        }
    }

    // Give them a moment to clean up.
    thread::sleep(Duration::from_millis(300));

    // SIGKILL any survivor.
    for t in &targets {
        let _ = Command::new("kill").args(["-9", "--", &t.pg]).output();
        for pid in &t.pids {
            if state::is_pid_alive(*pid) {
                let _ = Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
            }
        }
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
                let alive = state::alive_pane_names(&s);
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
        .arg(&path)
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
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to open editor '{editor_cmd}'"))?;
    Ok(())
}

/// Clone an existing project config to a new name
pub fn clone_project(source: &str, target: &str) -> Result<()> {
    config::ensure_dirs()?;
    let src_path = config::config_path(source);
    if !src_path.exists() {
        bail!(
            "Source config not found: {}\nRun `on list` to see available projects.",
            src_path.display(),
        );
    }
    let tgt_path = config::config_path(target);
    if tgt_path.exists() {
        bail!("Target config already exists: {}", tgt_path.display());
    }

    let content = std::fs::read_to_string(&src_path)
        .with_context(|| format!("Failed to read {}", src_path.display()))?;
    let content = content.replacen(&format!("name: {source}"), &format!("name: {target}"), 1);
    std::fs::write(&tgt_path, &content)
        .with_context(|| format!("Failed to write {}", tgt_path.display()))?;

    println!(
        "{}",
        format!("Cloned '{source}' → '{target}': {}", tgt_path.display()).green()
    );

    let editor_cmd = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    Command::new(&editor_cmd)
        .arg(&tgt_path)
        .status()
        .with_context(|| format!("Failed to open editor '{editor_cmd}'"))?;
    Ok(())
}

/// Auto-detect project structure and create config interactively
pub fn init() -> Result<()> {
    config::ensure_dirs()?;

    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let detected = config::detect_project(&cwd);

    println!("{}", "Detected project structure:".green());
    for pane in &detected.panes {
        if let Some(ref cmd) = pane.cmd {
            println!("  {} {} ({})", "●".green(), pane.name, cmd);
        } else {
            println!("  {} {} (shell)", "●".green(), pane.name);
        }
    }
    println!();

    // Confirm project name
    let name_input = prompt_line(&format!("Project name [{}]: ", detected.name))?;
    let name: &str = if name_input.is_empty() {
        &detected.name
    } else {
        &name_input
    };

    let path = config::config_path(name);
    if path.exists() {
        bail!(
            "Config already exists: {}\nRun `on edit {name}` to modify it.",
            path.display(),
        );
    }

    // Confirm editor
    let editor_input = prompt_line("Editor [code]: ")?;
    let editor_cmd: &str = if editor_input.is_empty() {
        "code"
    } else {
        &editor_input
    };

    let yaml = config::create_config_from_detection(name, &detected, editor_cmd);
    std::fs::write(&path, &yaml).with_context(|| format!("Failed to write {}", path.display()))?;

    println!("{}", format!("Created config: {}", path.display()).green());

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to open editor '{editor}'"))?;
    Ok(())
}

/// Validate one or all project configs (parse + extends sanity). Exits with
/// non-zero status when any issue is found, so it's usable in scripts.
pub fn validate(project: Option<&str>) -> Result<()> {
    config::ensure_dirs()?;
    let issues = config::validate_configs_in(&config::base_dir());
    let filtered = config::filter_issues_by_project(issues, project);

    if filtered.is_empty() {
        match project {
            Some(name) => println!("{}", format!("Config '{name}' is valid.").green()),
            None => println!("{}", "All configs are valid.".green()),
        }
        return Ok(());
    }

    for issue in &filtered {
        println!("{} {}: {}", "✗".red(), issue.project.bold(), issue.message);
    }
    bail!("{} config issue(s) found", filtered.len());
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

    // Validate every yaml in ~/.on/
    let issues = config::validate_configs_in(&config::base_dir());
    let configs_ok = issues.is_empty();
    print_check(configs_ok, "all project configs parse cleanly");
    for issue in &issues {
        println!("    {} {}: {}", "·".yellow(), issue.project, issue.message);
    }

    // Check git
    let git_ok = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    print_check(git_ok, "git available");

    // Check lsof
    let lsof_ok = Command::new("lsof").arg("-v").output().is_ok_and(|_| true);
    print_check(lsof_ok, "lsof available");

    println!();
    if tmux_ok && git_ok && lsof_ok && configs_ok {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn parse_yes_default_empty_is_yes() {
        assert!(parse_yes_default(""));
        assert!(parse_yes_default("\n"));
    }

    #[test]
    fn parse_yes_default_y_variants() {
        assert!(parse_yes_default("y"));
        assert!(parse_yes_default("Y"));
        assert!(parse_yes_default("yes"));
        assert!(parse_yes_default("  YES \n"));
    }

    #[test]
    fn parse_yes_default_no_variants() {
        assert!(!parse_yes_default("n"));
        assert!(!parse_yes_default("N"));
        assert!(!parse_yes_default("no"));
        assert!(!parse_yes_default(" NO\n"));
    }

    #[test]
    fn parse_port_action_kill() {
        assert_eq!(parse_port_action("k"), PortAction::Kill);
        assert_eq!(parse_port_action("KILL"), PortAction::Kill);
    }

    #[test]
    fn parse_port_action_abort() {
        assert_eq!(parse_port_action("a"), PortAction::Abort);
        assert_eq!(parse_port_action("Abort"), PortAction::Abort);
    }

    #[test]
    fn parse_port_action_defaults_to_skip() {
        assert_eq!(parse_port_action(""), PortAction::Skip);
        assert_eq!(parse_port_action("s"), PortAction::Skip);
        assert_eq!(parse_port_action("anything else"), PortAction::Skip);
    }

    #[test]
    fn parse_ps_pid_ppid_skips_header_and_garbage() {
        let output = "  PID  PPID\n  100    1\n  200  100\n  300  100\n  400  200\nweird line\n";
        let tree = parse_ps_pid_ppid(output);
        assert_eq!(tree.get(&1), Some(&vec![100]));
        let mut children_of_100 = tree.get(&100).cloned().unwrap_or_default();
        children_of_100.sort_unstable();
        assert_eq!(children_of_100, vec![200, 300]);
        assert_eq!(tree.get(&200), Some(&vec![400]));
    }

    #[test]
    fn descendants_walks_full_tree() {
        // 1 -> {2, 3}; 2 -> {4, 5}; 4 -> {6}
        let mut tree: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        tree.insert(1, vec![2, 3]);
        tree.insert(2, vec![4, 5]);
        tree.insert(4, vec![6]);
        let mut d = descendants_in(&tree, 1);
        d.sort_unstable();
        assert_eq!(d, vec![2, 3, 4, 5, 6]);
    }

    #[test]
    fn descendants_empty_when_no_children() {
        let tree = std::collections::HashMap::new();
        assert!(descendants_in(&tree, 42).is_empty());
    }

    #[test]
    fn descendants_handles_bogus_cycle() {
        // ps shouldn't produce a cycle, but defend against it anyway.
        let mut tree: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        tree.insert(1, vec![2]);
        tree.insert(2, vec![1]);
        let d = descendants_in(&tree, 1);
        // Each PID visited at most once: from 1 we collect 2, then 2's child 1
        // is the root itself (already visited) → don't recurse.
        assert_eq!(d, vec![2]);
    }

    #[test]
    fn non_interactive_true_for_truthy_env() {
        assert!(parse_non_interactive(Some("1")));
        assert!(parse_non_interactive(Some("true")));
        assert!(parse_non_interactive(Some("yes")));
        assert!(parse_non_interactive(Some("anything")));
    }

    #[test]
    fn non_interactive_false_for_unset_or_falsy() {
        assert!(!parse_non_interactive(None));
        assert!(!parse_non_interactive(Some("")));
        assert!(!parse_non_interactive(Some("0")));
        assert!(!parse_non_interactive(Some("false")));
        assert!(!parse_non_interactive(Some("no")));
    }
}
