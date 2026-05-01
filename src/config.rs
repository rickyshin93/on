use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Raw config as deserialized from YAML (supports both `terminal:` and legacy `iterm:`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct RawConfig {
    pub name: String,
    pub terminal: Option<TerminalConfig>,
    pub iterm: Option<LegacyItermConfig>,
    pub editor: Option<EditorConfig>,
    pub browser: Option<Vec<String>>,
    pub checks: Option<ChecksConfig>,
}

/// Resolved config used by the rest of the application
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub terminal: Option<TerminalConfig>,
    pub editor: Option<EditorConfig>,
    pub browser: Option<Vec<String>>,
    pub checks: Option<ChecksConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChecksConfig {
    /// Warn and prompt when repos have uncommitted changes (default: false)
    pub dirty_git: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TerminalConfig {
    #[serde(rename = "type", default = "default_terminal_type")]
    pub terminal_type: String,
    pub layout: Option<String>,
    pub max_panes_per_tab: Option<usize>,
    pub panes: Vec<PaneConfig>,
}

/// Legacy `iterm:` section (same shape but no `type` field)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LegacyItermConfig {
    pub layout: Option<String>,
    pub max_panes_per_tab: Option<usize>,
    pub panes: Vec<PaneConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaneConfig {
    pub name: String,
    pub dir: String,
    pub cmd: Option<String>,
}

impl PaneConfig {
    /// Build the shell command string for a pane (shared by iterm and tmux backends)
    pub fn build_command(&self, project: &str) -> String {
        match &self.cmd {
            Some(cmd) => {
                let pid_file = format!("/tmp/.on_{project}_{}.pid", self.name);
                format!("cd {} && echo $$ > {pid_file} && {cmd}", self.dir)
            }
            None => format!("cd {}", self.dir),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorConfig {
    pub cmd: Option<String>,
    pub folders: Option<Vec<String>>,
    pub workspace: Option<String>,
}

fn default_terminal_type() -> String {
    if cfg!(target_os = "macos") {
        "iterm".to_string()
    } else {
        "tmux".to_string()
    }
}

/// Returns the base directory: ~/.on/
pub fn base_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Cannot determine home directory");
    home.join(".on")
}

/// Returns the config file path for a project: ~/.on/<name>.yaml
pub fn config_path(name: &str) -> PathBuf {
    base_dir().join(format!("{name}.yaml"))
}

/// Ensure ~/.on/ and ~/.on/state/ directories exist
pub fn ensure_dirs() -> Result<()> {
    let base = base_dir();
    fs::create_dir_all(&base).context("Failed to create ~/.on/")?;
    fs::create_dir_all(base.join("state")).context("Failed to create ~/.on/state/")?;
    Ok(())
}

/// Load and parse a project config, expanding ~ paths
pub fn load(name: &str) -> Result<Config> {
    let path = config_path(name);
    if !path.exists() {
        bail!(
            "Config file not found: {}\nRun `on new {name}` to create one.",
            path.display(),
        );
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let raw: RawConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    let mut config = resolve_config(raw);
    expand_paths(&mut config);
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &Config) -> Result<()> {
    if let Some(ref term) = config.terminal {
        if let Some(max) = term.max_panes_per_tab {
            if !(2..=8).contains(&max) {
                bail!("max_panes_per_tab must be between 2 and 8, got {max}");
            }
        }
    }
    Ok(())
}

/// Resolve `RawConfig` into `Config`: merge legacy `iterm:` into `terminal:`
fn resolve_config(raw: RawConfig) -> Config {
    let terminal = match (raw.terminal, raw.iterm) {
        (Some(t), _) => Some(t),
        (None, Some(legacy)) => Some(TerminalConfig {
            terminal_type: "iterm".to_string(),
            layout: legacy.layout,
            max_panes_per_tab: legacy.max_panes_per_tab,
            panes: legacy.panes,
        }),
        (None, None) => None,
    };

    Config {
        name: raw.name,
        terminal,
        editor: raw.editor,
        browser: raw.browser,
        checks: raw.checks,
    }
}

/// Expand all ~ paths in the config to absolute paths
fn expand_paths(config: &mut Config) {
    if let Some(ref mut terminal) = config.terminal {
        for pane in &mut terminal.panes {
            pane.dir = shellexpand::tilde(&pane.dir).to_string();
        }
    }
    if let Some(ref mut editor) = config.editor {
        if let Some(ref mut folders) = editor.folders {
            for folder in folders.iter_mut() {
                *folder = shellexpand::tilde(folder).to_string();
            }
        }
    }
}

/// List all project names from ~/.on/*.yaml
pub fn list_projects() -> Vec<String> {
    let dir = base_dir();
    let mut projects = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    projects.push(stem.to_string());
                }
            }
        }
    }
    projects.sort();
    projects
}

/// Generate a template YAML config for a new project
pub fn create_template(name: &str) -> Result<PathBuf> {
    ensure_dirs()?;
    let path = config_path(name);
    if path.exists() {
        bail!("Config already exists: {}", path.display());
    }
    let terminal_type = default_terminal_type();
    let template = format!(
        r#"name: {name}

# Terminal panes — each pane opens in its own split
terminal:
  type: {terminal_type}       # iterm (macOS default) | tmux (Linux default)
  layout: vertical             # vertical (side-by-side) | grid (tiled)
  # max_panes_per_tab: 4       # max panes per tab (default 4, range 2-8)
  panes:
    - name: server
      dir: ~/projects/{name}
      cmd: echo "replace with your start command"
    # - name: frontend
    #   dir: ~/projects/{name}/frontend
    #   cmd: pnpm dev
    # - name: backend
    #   dir: ~/projects/{name}/backend
    #   cmd: uv run python src/main.py
    # - name: watch
    #   dir: ~/projects/{name}
    #   cmd: watchexec -e py,ts -- echo "changed"
    - name: shell
      dir: ~/projects/{name}

# Editor — opens your IDE with project folders or a workspace
editor:
  cmd: code                    # code | cursor | qoder | vim | ...
  folders:
    - ~/projects/{name}
  # workspace: ~/.on/{name}.code-workspace

# Browser — opens URLs in your default browser
# browser:
#   - http://localhost:3000
#   - http://localhost:8080/docs
#   - https://github.com/you/{name}

# Checks — optional startup warnings
# checks:
#   dirty_git: true   # warn and prompt when repos have uncommitted changes
"#,
    );
    fs::write(&path, &template).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_terminal_format() {
        let yaml = r#"
name: myproject
terminal:
  type: tmux
  layout: grid
  panes:
    - name: frontend
      dir: ~/projects/myproject/frontend
      cmd: npm run dev
    - name: backend
      dir: ~/projects/myproject/backend
      cmd: watchexec -e py python main.py
    - name: git
      dir: ~/projects/myproject
editor:
  cmd: cursor
  folders:
    - ~/projects/myproject/frontend
    - ~/projects/myproject/backend
browser:
  - http://localhost:3000
  - https://github.com/me/myproject
"#;
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);
        assert_eq!(config.name, "myproject");

        let terminal = config.terminal.unwrap();
        assert_eq!(terminal.terminal_type, "tmux");
        assert_eq!(terminal.layout, Some("grid".to_string()));
        assert_eq!(terminal.panes.len(), 3);
        assert_eq!(terminal.panes[0].cmd, Some("npm run dev".to_string()));
        assert_eq!(terminal.panes[2].cmd, None);

        let editor = config.editor.unwrap();
        assert_eq!(editor.cmd, Some("cursor".to_string()));
        assert_eq!(editor.folders.unwrap().len(), 2);

        assert_eq!(config.browser.unwrap().len(), 2);
    }

    #[test]
    fn parse_legacy_iterm_format() {
        let yaml = r#"
name: myproject
iterm:
  layout: grid
  panes:
    - name: frontend
      dir: ~/projects/myproject/frontend
      cmd: npm run dev
"#;
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);

        let terminal = config.terminal.unwrap();
        assert_eq!(terminal.terminal_type, "iterm");
        assert_eq!(terminal.layout, Some("grid".to_string()));
        assert_eq!(terminal.panes.len(), 1);
    }

    #[test]
    fn terminal_takes_priority_over_iterm() {
        let yaml = r#"
name: myproject
terminal:
  type: tmux
  panes:
    - name: dev
      dir: /tmp
iterm:
  panes:
    - name: old
      dir: /tmp
"#;
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);

        let terminal = config.terminal.unwrap();
        assert_eq!(terminal.terminal_type, "tmux");
        assert_eq!(terminal.panes[0].name, "dev");
    }

    #[test]
    fn parse_minimal_yaml() {
        let yaml = "name: simple\n";
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);
        assert_eq!(config.name, "simple");
        assert!(config.terminal.is_none());
        assert!(config.editor.is_none());
        assert!(config.browser.is_none());
    }

    #[test]
    fn default_terminal_type_on_current_os() {
        let t = default_terminal_type();
        if cfg!(target_os = "macos") {
            assert_eq!(t, "iterm");
        } else {
            assert_eq!(t, "tmux");
        }
    }

    #[test]
    fn expand_tilde_paths() {
        let mut config = Config {
            name: "test".to_string(),
            terminal: Some(TerminalConfig {
                terminal_type: "tmux".to_string(),
                layout: None,
                max_panes_per_tab: None,
                panes: vec![PaneConfig {
                    name: "dev".to_string(),
                    dir: "~/projects/test".to_string(),
                    cmd: None,
                }],
            }),
            editor: Some(EditorConfig {
                cmd: None,
                folders: Some(vec!["~/projects/test".to_string()]),
                workspace: None,
            }),
            browser: None,
        };

        expand_paths(&mut config);

        let home = dirs::home_dir().unwrap();
        let expected = home.join("projects/test").to_string_lossy().to_string();
        assert_eq!(config.terminal.unwrap().panes[0].dir, expected);
        assert_eq!(config.editor.unwrap().folders.unwrap()[0], expected);
    }

    #[test]
    fn build_pane_command_with_cmd() {
        let pane = PaneConfig {
            name: "dev".to_string(),
            dir: "/tmp/test".to_string(),
            cmd: Some("npm run dev".to_string()),
        };
        let cmd = pane.build_command("myproject");
        assert!(cmd.contains("cd /tmp/test"));
        assert!(cmd.contains("echo $$"));
        assert!(cmd.contains(".on_myproject_dev.pid"));
        assert!(cmd.contains("npm run dev"));
        assert!(!cmd.contains("exec"));
    }

    #[test]
    fn build_pane_command_without_cmd() {
        let pane = PaneConfig {
            name: "shell".to_string(),
            dir: "/tmp/test".to_string(),
            cmd: None,
        };
        let cmd = pane.build_command("myproject");
        assert_eq!(cmd, "cd /tmp/test");
    }

    #[test]
    fn base_dir_path() {
        let dir = base_dir();
        let home = dirs::home_dir().unwrap();
        assert_eq!(dir, home.join(".on"));
    }

    #[test]
    fn config_path_format() {
        let path = config_path("myproject");
        assert_eq!(path, base_dir().join("myproject.yaml"));
    }

    #[test]
    fn ensure_dirs_creates_directories() {
        ensure_dirs().unwrap();
        assert!(base_dir().exists());
        assert!(base_dir().join("state").exists());
    }

    #[test]
    fn create_and_load_template() {
        let name = "_on_test_tpl";
        let path = config_path(name);
        let _ = fs::remove_file(&path);

        ensure_dirs().unwrap();
        let created = create_template(name).unwrap();
        assert!(created.exists());

        let config = load(name).unwrap();
        assert_eq!(config.name, name);
        if let Some(terminal) = &config.terminal {
            for pane in &terminal.panes {
                assert!(!pane.dir.contains('~'));
            }
        }

        assert!(create_template(name).is_err());

        let _ = fs::remove_file(&path);
    }
}
