use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PaneConfig;

/// Build the chained `tmux` argument vector that creates the whole session
/// in a single fork. Sub-commands are separated by `;` arguments — tmux
/// reads each `;` as a command boundary.
fn build_tmux_args(
    project: &str,
    panes: &[PaneConfig],
    layout: &str,
    max_per_tab: Option<usize>,
) -> Vec<String> {
    let session = session_name(project);
    let limit = max_per_tab.unwrap_or(panes.len()).max(1);
    let chunks: Vec<&[PaneConfig]> = panes.chunks(limit).collect();
    let tmux_layout = if layout == "grid" {
        "tiled"
    } else {
        "even-horizontal"
    };

    let mut args: Vec<String> = Vec::new();
    let push_sep = |args: &mut Vec<String>| {
        if !args.is_empty() {
            args.push(";".to_string());
        }
    };

    for (win_idx, chunk) in chunks.iter().enumerate() {
        let win_target = format!("{session}:{win_idx}");

        push_sep(&mut args);
        if win_idx == 0 {
            args.extend(
                ["new-session", "-d", "-s", &session, "-n", project]
                    .iter()
                    .map(|s| (*s).to_string()),
            );
        } else {
            let win_name = format!("{project}-{}", win_idx + 1);
            args.extend(
                ["new-window", "-t", &session, "-n", &win_name]
                    .iter()
                    .map(|s| (*s).to_string()),
            );
        }

        for _ in 1..chunk.len() {
            push_sep(&mut args);
            args.extend(
                ["split-window", "-t", &win_target]
                    .iter()
                    .map(|s| (*s).to_string()),
            );
        }

        push_sep(&mut args);
        args.extend(
            ["select-layout", "-t", &win_target, tmux_layout]
                .iter()
                .map(|s| (*s).to_string()),
        );

        for (i, pane) in chunk.iter().enumerate() {
            let target = format!("{session}:{win_idx}.{i}");
            let cmd = pane.build_command(project, false);
            push_sep(&mut args);
            args.extend(
                ["send-keys", "-t", &target, &cmd, "Enter"]
                    .iter()
                    .map(|s| (*s).to_string()),
            );
        }
    }

    args
}

/// Create a tmux session with panes arranged according to layout (non-blocking).
/// When `max_per_tab` is set, panes are split across multiple tmux windows.
/// Issues every tmux sub-command in a single fork via `;`-chained args.
pub fn open_panes(
    project: &str,
    panes: &[PaneConfig],
    layout: &str,
    max_per_tab: Option<usize>,
) -> Result<()> {
    if panes.is_empty() {
        return Ok(());
    }

    let session = session_name(project);

    // Kill stale session if it exists (separate call — chaining a failing
    // kill would cancel the whole chain).
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .output();

    let args = build_tmux_args(project, panes, layout, max_per_tab);
    let status = Command::new("tmux")
        .args(&args)
        .status()
        .context("Failed to start tmux. Is tmux installed?")?;
    if !status.success() {
        bail!("tmux session setup failed for '{session}'");
    }
    Ok(())
}

/// Attach to or switch to the tmux session (blocking)
pub fn attach(project: &str) -> Result<()> {
    let session = session_name(project);

    if std::env::var("TMUX").is_ok() {
        // Already inside tmux — switch client
        Command::new("tmux")
            .args(["switch-client", "-t", &session])
            .status()
            .context("Failed to switch tmux client")?;
    } else {
        // Not in tmux — attach
        Command::new("tmux")
            .args(["attach", "-t", &session])
            .status()
            .context("Failed to attach to tmux session")?;
    }

    Ok(())
}

/// Kill the tmux session for a project
pub fn close_session(project: &str) {
    let session = session_name(project);
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .output();
}

/// Capture the scrollback buffer of a tmux pane by index
pub fn capture_pane(project: &str, pane_index: usize) -> Result<String> {
    let session = session_name(project);
    let target = format!("{session}:0.{pane_index}");
    let output = Command::new("tmux")
        .args(["capture-pane", "-t", &target, "-p", "-S", "-"])
        .output()
        .context("Failed to run tmux capture-pane")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        bail!(
            "tmux capture-pane failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn session_name(project: &str) -> String {
    format!("on_{project}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn session_name_format() {
        assert_eq!(session_name("myproject"), "on_myproject");
    }

    #[test]
    fn empty_panes_is_ok() {
        assert!(open_panes("proj", &[], "vertical", None).is_ok());
    }

    fn pane(name: &str, dir: &str, cmd: Option<&str>) -> PaneConfig {
        PaneConfig {
            name: name.to_string(),
            dir: dir.to_string(),
            cmd: cmd.map(str::to_owned),
            env: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn args_chain_single_pane_single_command() {
        let panes = vec![pane("dev", "/tmp", Some("echo dev"))];
        let args = build_tmux_args("proj", &panes, "vertical", None);
        // new-session, select-layout, send-keys → 2 ";" separators.
        assert_eq!(args.iter().filter(|a| *a == ";").count(), 2);
        assert!(args.iter().any(|a| a == "new-session"));
        assert!(args.iter().any(|a| a == "send-keys"));
        // No splits for one pane.
        assert!(!args.iter().any(|a| a == "split-window"));
    }

    #[test]
    fn args_chain_three_panes() {
        let panes = vec![
            pane("a", "/tmp", Some("echo a")),
            pane("b", "/tmp", Some("echo b")),
            pane("c", "/tmp", Some("echo c")),
        ];
        let args = build_tmux_args("proj", &panes, "vertical", None);
        // 1 new-session + 2 split-window + 1 select-layout + 3 send-keys = 7 cmds, 6 ";".
        assert_eq!(args.iter().filter(|a| *a == ";").count(), 6);
        assert_eq!(args.iter().filter(|a| *a == "split-window").count(), 2);
        assert_eq!(args.iter().filter(|a| *a == "send-keys").count(), 3);
    }

    #[test]
    fn args_chain_multi_window_when_max_per_tab() {
        let panes = vec![
            pane("a", "/tmp", Some("a")),
            pane("b", "/tmp", Some("b")),
            pane("c", "/tmp", Some("c")),
        ];
        let args = build_tmux_args("proj", &panes, "vertical", Some(2));
        // Window 1: new-session + 1 split + 1 layout + 2 send-keys = 5
        // Window 2: new-window + 0 splits + 1 layout + 1 send-keys = 3
        // total = 8 commands, 7 separators
        assert_eq!(args.iter().filter(|a| *a == "new-window").count(), 1);
        assert_eq!(args.iter().filter(|a| *a == "send-keys").count(), 3);
    }
}
