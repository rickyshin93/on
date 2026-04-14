use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProjectState {
    pub project: String,
    pub started_at: String,
    pub panes: Vec<PaneState>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PaneState {
    pub name: String,
    pub pid: u32,
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

/// Check if a PID is still alive
pub fn is_pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if any PID in the state is still alive
pub fn is_running(project: &str) -> Result<bool> {
    match load(project)? {
        None => Ok(false),
        Some(state) => Ok(state.panes.iter().any(|p| is_pid_alive(p.pid))),
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
    use super::*;

    fn test_state() -> ProjectState {
        ProjectState {
            project: "_on_test_state".to_string(),
            started_at: "2026-04-12T10:00:00".to_string(),
            panes: vec![
                PaneState {
                    name: "frontend".to_string(),
                    pid: 99999,
                },
                PaneState {
                    name: "backend".to_string(),
                    pid: 99998,
                },
            ],
        }
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
    fn is_running_false_when_no_state() {
        assert!(!is_running("_on_nonexistent_xyz").unwrap());
    }

    #[test]
    fn state_path_format() {
        let path = state_path("myproject");
        assert!(path.ends_with("state/myproject.json"));
    }
}
