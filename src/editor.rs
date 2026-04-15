use anyhow::{Context, Result};
use std::process::Command;

use crate::config::{self, EditorConfig};

/// Open editor with workspace file or configured folders.
/// Workspace takes priority over folders if both are set.
/// When only folders are configured, a `.code-workspace` file is auto-created.
pub fn open(editor: Option<&EditorConfig>, project: &str) -> Result<()> {
    let Some(editor) = editor else {
        return Ok(());
    };

    let cmd = editor.cmd.as_deref().unwrap_or("code");

    // Use explicit workspace if configured
    if let Some(ref workspace) = editor.workspace {
        let expanded = shellexpand::tilde(workspace).to_string();
        Command::new(cmd)
            .arg(&expanded)
            .spawn()
            .with_context(|| format!("Failed to open workspace '{expanded}' with '{cmd}'"))?;
        return Ok(());
    }

    let folders = match &editor.folders {
        Some(f) if !f.is_empty() => f,
        _ => return Ok(()),
    };

    // Auto-create workspace file from folders
    let ws_path = config::base_dir().join(format!("{project}.code-workspace"));
    if !ws_path.exists() {
        let entries: Vec<String> = folders
            .iter()
            .map(|f| format!("\t\t{{\n\t\t\t\"path\": \"{f}\"\n\t\t}}"))
            .collect();
        let content = format!(
            "{{\n\t\"folders\": [\n{}\n\t],\n\t\"settings\": {{}}\n}}\n",
            entries.join(",\n")
        );
        std::fs::write(&ws_path, content)
            .with_context(|| format!("Failed to create workspace {}", ws_path.display()))?;
    }

    Command::new(cmd)
        .arg(ws_path.to_str().unwrap())
        .spawn()
        .with_context(|| format!("Failed to launch editor '{cmd}'"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_editor_is_ok() {
        assert!(open(None, "test").is_ok());
    }

    #[test]
    fn empty_folders_is_ok() {
        let editor = EditorConfig {
            cmd: Some("code".to_string()),
            folders: Some(vec![]),
            workspace: None,
        };
        assert!(open(Some(&editor), "test").is_ok());
    }

    #[test]
    fn no_folders_field_is_ok() {
        let editor = EditorConfig {
            cmd: Some("code".to_string()),
            folders: None,
            workspace: None,
        };
        assert!(open(Some(&editor), "test").is_ok());
    }
}
