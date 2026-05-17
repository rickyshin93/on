use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::config;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProjectState {
    pub project: String,
    pub started_at: String,
    #[serde(default = "default_terminal_type")]
    pub terminal_type: String,
    pub panes: Vec<PaneState>,
}

fn default_terminal_type() -> String {
    "iterm".to_string()
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PaneState {
    pub name: String,
    pub pid: u32,
    /// Process start time, captured at save time, used to detect PID reuse.
    /// `None` for state files written by older versions of `on`.
    #[serde(default)]
    pub process_started_at: Option<String>,
}

/// Path to state file: ~/.on/state/<project>.json
pub fn state_path(project: &str) -> PathBuf {
    config::base_dir()
        .join("state")
        .join(format!("{project}.json"))
}

/// Save project state to JSON
pub fn save(state: &ProjectState) -> Result<()> {
    let path = state_path(&state.project);
    let json = serde_json::to_string_pretty(state).context("Failed to serialize state")?;
    fs::write(&path, json).with_context(|| format!("Failed to write state {}", path.display()))?;
    Ok(())
}

/// Load project state from JSON
pub fn load(project: &str) -> Result<Option<ProjectState>> {
    let path = state_path(project);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read state {}", path.display()))?;
    let state: ProjectState = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse state {}", path.display()))?;
    Ok(Some(state))
}

/// Delete state file
pub fn remove(project: &str) -> Result<()> {
    let path = state_path(project);
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove state {}", path.display()))?;
    }
    Ok(())
}

/// Capture a process's start time as a stable string.
///
/// Used at save time and re-checked at status time to detect PID reuse:
/// if the PID is alive but its start time differs from what we recorded,
/// the kernel has handed our old PID to an unrelated process.
///
/// Returns `None` when the PID is gone or `ps` fails.
pub fn process_started_at(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Determine whether a pane's process is still the same one we launched.
///
/// `process_started_at == None` (state file from older version) falls back
/// to the bare PID liveness check.
pub fn verify_pane_alive(pane: &PaneState) -> bool {
    if !is_pid_alive(pane.pid) {
        return false;
    }
    match &pane.process_started_at {
        None => true,
        Some(recorded) => process_started_at(pane.pid).as_ref() == Some(recorded),
    }
}

/// Check if a PID is still alive.
///
/// Uses `kill(pid, 0)` via the safe `nix` wrapper instead of forking
/// `/bin/kill`, which is both ~100× faster and correctly distinguishes
/// `EPERM` (process exists, no permission to signal) from `ESRCH` (process
/// gone). A non-root caller checking PID 1 must return true.
pub fn is_pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    matches!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Ok(()) | Err(nix::errno::Errno::EPERM)
    )
}

/// Names of panes whose underlying process is still the same one we launched.
/// Used by `list` and `status` so they agree on what "alive" means.
pub fn alive_pane_names(state: &ProjectState) -> Vec<&str> {
    state
        .panes
        .iter()
        .filter(|p| verify_pane_alive(p))
        .map(|p| p.name.as_str())
        .collect()
}

/// Check if any pane in the project's state is still alive (with PID-reuse guard).
pub fn is_running(project: &str) -> Result<bool> {
    match load(project)? {
        None => Ok(false),
        Some(state) => Ok(state.panes.iter().any(verify_pane_alive)),
    }
}

/// List all projects that have state files
pub fn running_projects() -> Vec<String> {
    let state_dir = config::base_dir().join("state");
    let mut projects = Vec::new();
    if let Ok(entries) = fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    projects.push(stem.to_string());
                }
            }
        }
    }
    projects
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn test_state() -> ProjectState {
        ProjectState {
            project: "_on_test_state".to_string(),
            started_at: "2026-04-12T10:00:00".to_string(),
            terminal_type: "iterm".to_string(),
            panes: vec![
                PaneState {
                    name: "frontend".to_string(),
                    pid: 99999,
                    process_started_at: None,
                },
                PaneState {
                    name: "backend".to_string(),
                    pid: 99998,
                    process_started_at: None,
                },
            ],
        }
    }

    #[test]
    fn process_started_at_for_self_is_some() {
        let lstart = process_started_at(std::process::id());
        assert!(lstart.is_some());
        assert!(!lstart.unwrap().is_empty());
    }

    #[test]
    fn process_started_at_for_dead_pid_is_none() {
        assert!(process_started_at(99999).is_none());
    }

    #[test]
    fn verify_pane_alive_without_lstart_falls_back_to_pid_check() {
        let pane = PaneState {
            name: "self".to_string(),
            pid: std::process::id(),
            process_started_at: None,
        };
        assert!(verify_pane_alive(&pane));
    }

    #[test]
    fn verify_pane_alive_with_matching_lstart_is_alive() {
        let lstart = process_started_at(std::process::id()).unwrap();
        let pane = PaneState {
            name: "self".to_string(),
            pid: std::process::id(),
            process_started_at: Some(lstart),
        };
        assert!(verify_pane_alive(&pane));
    }

    #[test]
    fn verify_pane_alive_with_stale_lstart_is_dead() {
        // Live PID, but recorded start time doesn't match — PID was reused.
        let pane = PaneState {
            name: "self".to_string(),
            pid: std::process::id(),
            process_started_at: Some("Wed Jan  1 00:00:00 1970".to_string()),
        };
        assert!(!verify_pane_alive(&pane));
    }

    #[test]
    fn verify_pane_alive_dead_pid_is_dead() {
        let pane = PaneState {
            name: "gone".to_string(),
            pid: 99999,
            process_started_at: Some("anything".to_string()),
        };
        assert!(!verify_pane_alive(&pane));
    }

    #[test]
    fn save_and_load_roundtrip() {
        config::ensure_dirs().unwrap();
        let state = test_state();

        save(&state).unwrap();
        let loaded = load(&state.project).unwrap().unwrap();
        assert_eq!(state, loaded);

        remove(&state.project).unwrap();
        assert!(load(&state.project).unwrap().is_none());
    }

    #[test]
    fn load_nonexistent_returns_none() {
        assert!(load("_on_nonexistent_xyz").unwrap().is_none());
    }

    #[test]
    fn dead_pid_is_not_alive() {
        assert!(!is_pid_alive(99999));
    }

    #[test]
    fn self_pid_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn pid_existing_without_permission_is_alive() {
        // PID 1 (init/launchd) is owned by root. As a non-root user we get EPERM
        // when signalling it — but the process clearly exists. The previous
        // `kill -0` fork implementation incorrectly treated EPERM as "dead".
        // Skip when running as root (CI sometimes does), since then we have permission.
        if nix::unistd::geteuid().is_root() {
            return;
        }
        assert!(is_pid_alive(1));
    }

    #[test]
    fn is_running_false_when_no_state() {
        assert!(!is_running("_on_nonexistent_xyz").unwrap());
    }

    #[test]
    fn alive_pane_names_filters_dead() {
        let state = ProjectState {
            project: "x".to_string(),
            started_at: "now".to_string(),
            terminal_type: "tmux".to_string(),
            panes: vec![
                PaneState {
                    name: "live".to_string(),
                    pid: std::process::id(),
                    process_started_at: None,
                },
                PaneState {
                    name: "dead".to_string(),
                    pid: 99999,
                    process_started_at: None,
                },
            ],
        };
        let alive = alive_pane_names(&state);
        assert_eq!(alive, vec!["live"]);
    }

    #[test]
    fn state_path_format() {
        let path = state_path("myproject");
        assert!(path.ends_with("state/myproject.json"));
    }
}
