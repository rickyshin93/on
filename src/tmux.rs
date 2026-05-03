use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PaneConfig;

/// Create a tmux session with panes arranged according to layout (non-blocking).
/// When `max_per_tab` is set, panes are split across multiple tmux windows.
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

    // Kill stale session if it exists
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .output();

    let limit = max_per_tab.unwrap_or(panes.len());
    let chunks: Vec<&[PaneConfig]> = panes.chunks(limit).collect();

    for (win_idx, chunk) in chunks.iter().enumerate() {
        if win_idx == 0 {
            let status = Command::new("tmux")
                .args(["new-session", "-d", "-s", &session, "-n", project])
                .status()
                .context("Failed to start tmux. Is tmux installed?")?;
            if !status.success() {
                bail!("Failed to create tmux session '{session}'");
            }
        } else {
            let _ = Command::new("tmux")
                .args([
                    "new-window",
                    "-t",
                    &session,
                    "-n",
                    &format!("{project}-{}", win_idx + 1),
                ])
                .status();
        }

        // Split additional panes
        let win_target = format!("{session}:{win_idx}");
        for _ in 1..chunk.len() {
            let _ = Command::new("tmux")
                .args(["split-window", "-t", &win_target])
                .status();
        }

        // Apply layout
        let tmux_layout = match layout {
            "grid" => "tiled",
            _ => "even-horizontal",
        };
        let _ = Command::new("tmux")
            .args(["select-layout", "-t", &win_target, tmux_layout])
            .status();

        // Send commands to each pane
        for (i, pane) in chunk.iter().enumerate() {
            let target = format!("{session}:{win_idx}.{i}");
            let cmd = pane.build_command(project, false);
            let _ = Command::new("tmux")
                .args(["send-keys", "-t", &target, &cmd, "Enter"])
                .status();
        }
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
    use super::*;

    #[test]
    fn session_name_format() {
        assert_eq!(session_name("myproject"), "on_myproject");
    }

    #[test]
    fn empty_panes_is_ok() {
        assert!(open_panes("proj", &[], "vertical", None).is_ok());
    }
}
