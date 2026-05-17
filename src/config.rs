use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// Raw config as deserialized from YAML (supports both `terminal:` and legacy `iterm:`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct RawConfig {
    pub name: String,
    pub extends: Option<String>,
    pub terminal: Option<TerminalConfig>,
    pub iterm: Option<LegacyItermConfig>,
    pub editor: Option<EditorConfig>,
    pub browser: Option<Vec<String>>,
    pub checks: Option<ChecksConfig>,
    pub hooks: Option<HooksConfig>,
}

/// Resolved config used by the rest of the application
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub terminal: Option<TerminalConfig>,
    pub editor: Option<EditorConfig>,
    pub browser: Option<Vec<String>>,
    pub checks: Option<ChecksConfig>,
    pub hooks: Option<HooksConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChecksConfig {
    /// Warn and prompt when repos have uncommitted changes (default: false)
    pub dirty_git: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HooksConfig {
    pub pre_launch: Option<Vec<String>>,
    pub post_launch: Option<Vec<String>>,
    pub pre_stop: Option<Vec<String>>,
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
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

impl PaneConfig {
    /// Build the shell command string for a pane.
    /// When `logging` is true, output is tee'd to a log file (used by iTerm backend).
    pub fn build_command(&self, project: &str, logging: bool) -> String {
        let mut parts = vec![format!("cd {}", shell_escape(&self.dir))];

        let mut keys: Vec<&String> = self.env.keys().collect();
        keys.sort();
        for key in keys {
            let value = &self.env[key];
            parts.push(format!("export {key}={}", shell_escape(value)));
        }

        if let Some(cmd) = &self.cmd {
            let pid_file = pid_file_path(project, &self.name);
            let pid_file = pid_file.display();
            if logging {
                let log_file = log_path(project, &self.name);
                let log_file = log_file.display();
                parts.push(format!(
                    "echo $$ > {pid_file} && {cmd} 2>&1 | tee -a {log_file}"
                ));
            } else {
                parts.push(format!("echo $$ > {pid_file} && {cmd}"));
            }
        }

        parts.join(" && ")
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

/// Pure helper: pick a home directory, falling back to `.` when none.
fn home_or_fallback(home: Option<PathBuf>) -> PathBuf {
    home.unwrap_or_else(|| PathBuf::from("."))
}

/// Returns the base directory: ~/.on/. Falls back to ./.on when no home
/// directory is available (some sandboxes / containers strip `HOME`).
/// Never panics.
pub fn base_dir() -> PathBuf {
    home_or_fallback(dirs::home_dir()).join(".on")
}

/// Returns the config file path for a project: ~/.on/<name>.yaml
pub fn config_path(name: &str) -> PathBuf {
    base_dir().join(format!("{name}.yaml"))
}

/// Returns the logs directory: ~/.on/logs/
pub fn logs_dir() -> PathBuf {
    base_dir().join("logs")
}

/// Returns the log file path for a project pane
pub fn log_path(project: &str, pane: &str) -> PathBuf {
    logs_dir().join(format!("{project}_{pane}.log"))
}

/// Returns the PID file path for a project pane.
///
/// Lives under `~/.on/state/pids/` rather than `/tmp` to avoid the
/// symlink-attack surface of a world-writable directory and to prevent
/// cross-user collisions on shared hosts.
pub fn pid_file_path(project: &str, pane: &str) -> PathBuf {
    base_dir()
        .join("state")
        .join("pids")
        .join(format!("{project}_{pane}.pid"))
}

/// Ensure ~/.on/ and its `state/`, `state/pids/`, and `logs/` subdirectories exist
pub fn ensure_dirs() -> Result<()> {
    let base = base_dir();
    fs::create_dir_all(&base).context("Failed to create ~/.on/")?;
    fs::create_dir_all(base.join("state")).context("Failed to create ~/.on/state/")?;
    fs::create_dir_all(base.join("state").join("pids"))
        .context("Failed to create ~/.on/state/pids/")?;
    fs::create_dir_all(base.join("logs")).context("Failed to create ~/.on/logs/")?;
    Ok(())
}

fn parse_raw(name: &str) -> Result<RawConfig> {
    let path = config_path(name);
    if !path.exists() {
        bail!(
            "Config file not found: {}\nRun `on new {name}` to create one.",
            path.display(),
        );
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    serde_yaml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

fn merge_configs(base: Config, current: Config) -> Config {
    Config {
        name: current.name,
        terminal: current.terminal.or(base.terminal),
        editor: current.editor.or(base.editor),
        browser: current.browser.or(base.browser),
        checks: current.checks.or(base.checks),
        hooks: current.hooks.or(base.hooks),
    }
}

/// Load and parse a project config, expanding ~ paths and resolving
/// `extends:` chains of arbitrary depth. Cycles return an error rather
/// than recursing forever.
pub fn load(name: &str) -> Result<Config> {
    let mut visiting = Vec::new();
    let mut config = load_with_chain(name, &mut visiting)?;
    expand_paths(&mut config);
    validate_config(&config)?;
    Ok(config)
}

fn load_with_chain(name: &str, visiting: &mut Vec<String>) -> Result<Config> {
    if visiting.iter().any(|n| n == name) {
        visiting.push(name.to_string());
        bail!("extends cycle detected: {}", visiting.join(" -> "));
    }
    visiting.push(name.to_string());

    let raw = parse_raw(name)?;
    let current = resolve_config(raw.clone());
    let result = if let Some(ref base_name) = raw.extends {
        let base = load_with_chain(base_name, visiting)
            .with_context(|| format!("Failed to load base config '{base_name}'"))?;
        merge_configs(base, current)
    } else {
        current
    };

    visiting.pop();
    Ok(result)
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
        hooks: raw.hooks,
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

/// A problem discovered while validating a project config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub project: String,
    pub message: String,
}

/// Keep only issues whose project matches `name`. `None` is a no-op.
pub fn filter_issues_by_project(issues: Vec<ConfigIssue>, name: Option<&str>) -> Vec<ConfigIssue> {
    match name {
        None => issues,
        Some(n) => issues.into_iter().filter(|i| i.project == n).collect(),
    }
}

/// Validate every `*.yaml` config in a directory. Returns a list of issues
/// (YAML parse failures, dangling `extends:` references). Used by `on doctor`.
pub fn validate_configs_in(dir: &std::path::Path) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return issues;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let Some(project) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                issues.push(ConfigIssue {
                    project,
                    message: format!("read failed: {e}"),
                });
                continue;
            }
        };
        let raw: RawConfig = match serde_yaml::from_str(&content) {
            Ok(r) => r,
            Err(e) => {
                issues.push(ConfigIssue {
                    project,
                    message: format!("yaml parse error: {e}"),
                });
                continue;
            }
        };
        if let Some(base) = raw.extends.as_deref() {
            let base_path = dir.join(format!("{base}.yaml"));
            if !base_path.exists() {
                issues.push(ConfigIssue {
                    project,
                    message: format!(
                        "extends '{base}' but {} does not exist",
                        base_path.display()
                    ),
                });
            }
        }
    }
    issues
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
      # env:                       # optional environment variables
      #   RUST_LOG: debug
      #   PORT: "3000"
    # - name: frontend
    #   dir: ~/projects/{name}/frontend
    #   cmd: pnpm dev
    # - name: backend
    #   dir: ~/projects/{name}/backend
    #   cmd: uv run python src/main.py
    #   env:
    #     DATABASE_URL: postgres://localhost/mydb
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

# Hooks — run commands at lifecycle stages
# hooks:
#   pre_launch:
#     - docker compose up -d
#   post_launch:
#     - echo "ready!"
#   pre_stop:
#     - docker compose down

# Inheritance — share common settings from another config
# extends: base
"#,
    );
    fs::write(&path, &template).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

/// A detected sub-project within a directory
#[derive(Debug, Clone)]
pub struct DetectedPane {
    pub name: String,
    pub dir: String,
    pub cmd: Option<String>,
    pub port: Option<u16>,
}

/// Result of scanning the current directory for project structure
#[derive(Debug, Clone)]
pub struct DetectedProject {
    pub name: String,
    pub panes: Vec<DetectedPane>,
}

fn detect_node_cmd(dir: &std::path::Path) -> &'static str {
    let Ok(content) = fs::read_to_string(dir.join("package.json")) else {
        return "npm run dev";
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return "npm run dev";
    };
    let scripts = json.get("scripts").and_then(serde_json::Value::as_object);
    let Some(scripts) = scripts else {
        return "npm run dev";
    };
    if scripts.contains_key("dev") {
        "npm run dev"
    } else if scripts.contains_key("start") {
        "npm start"
    } else {
        "npm run dev"
    }
}

fn detect_python_cmd(dir: &std::path::Path) -> &'static str {
    if dir.join("manage.py").exists() {
        "python manage.py runserver"
    } else {
        "python main.py"
    }
}

fn detect_in_dir(dir: &std::path::Path, name: &str) -> Option<DetectedPane> {
    if dir.join("Cargo.toml").exists() {
        Some(DetectedPane {
            name: name.to_string(),
            dir: dir.to_string_lossy().to_string(),
            cmd: Some("cargo run".to_string()),
            port: None,
        })
    } else if dir.join("package.json").exists() {
        Some(DetectedPane {
            name: name.to_string(),
            dir: dir.to_string_lossy().to_string(),
            cmd: Some(detect_node_cmd(dir).to_string()),
            port: Some(3000),
        })
    } else if dir.join("pyproject.toml").exists() || dir.join("requirements.txt").exists() {
        Some(DetectedPane {
            name: name.to_string(),
            dir: dir.to_string_lossy().to_string(),
            cmd: Some(detect_python_cmd(dir).to_string()),
            port: Some(8000),
        })
    } else if dir.join("go.mod").exists() {
        Some(DetectedPane {
            name: name.to_string(),
            dir: dir.to_string_lossy().to_string(),
            cmd: Some("go run .".to_string()),
            port: None,
        })
    } else if dir.join("docker-compose.yml").exists() || dir.join("docker-compose.yaml").exists() {
        Some(DetectedPane {
            name: name.to_string(),
            dir: dir.to_string_lossy().to_string(),
            cmd: Some("docker compose up".to_string()),
            port: None,
        })
    } else {
        None
    }
}

/// Scan the given directory for project structure (root + one level of subdirs)
pub fn detect_project(dir: &std::path::Path) -> DetectedProject {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    let mut panes = Vec::new();

    // Scan subdirectories first (monorepo detection)
    if let Ok(entries) = fs::read_dir(dir) {
        let mut subdirs: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                !name.starts_with('.') && name != "node_modules" && name != "target"
            })
            .collect();
        subdirs.sort_by_key(std::fs::DirEntry::file_name);

        for entry in &subdirs {
            let subdir_name = entry.file_name().to_string_lossy().to_string();
            if let Some(pane) = detect_in_dir(&entry.path(), &subdir_name) {
                panes.push(pane);
            }
        }
    }

    // If no sub-projects found, detect root directory
    if panes.is_empty() {
        if let Some(pane) = detect_in_dir(dir, "server") {
            panes.push(pane);
        }
    }

    // Always add a shell pane
    panes.push(DetectedPane {
        name: "shell".to_string(),
        dir: dir.to_string_lossy().to_string(),
        cmd: None,
        port: None,
    });

    DetectedProject { name, panes }
}

/// Generate a YAML config string from a detected project
pub fn create_config_from_detection(
    name: &str,
    detected: &DetectedProject,
    editor_cmd: &str,
) -> String {
    let terminal_type = default_terminal_type();
    let home = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut pane_lines = String::new();
    let mut browser_urls: Vec<String> = Vec::new();

    for pane in &detected.panes {
        let dir = pane.dir.replace(&home, "~");
        let _ = write!(pane_lines, "    - name: {}\n      dir: {dir}\n", pane.name);
        if let Some(ref cmd) = pane.cmd {
            let _ = writeln!(pane_lines, "      cmd: {cmd}");
        }
        if let Some(port) = pane.port {
            browser_urls.push(format!("http://localhost:{port}"));
        }
    }

    let mut yaml = format!(
        "name: {name}\n\nterminal:\n  type: {terminal_type}\n  layout: vertical\n  panes:\n{pane_lines}\neditor:\n  cmd: {editor_cmd}\n  folders:\n    - {}\n",
        detected.panes[0].dir.replace(&home, "~"),
    );

    if !browser_urls.is_empty() {
        yaml.push_str("\nbrowser:\n");
        for url in &browser_urls {
            let _ = writeln!(yaml, "  - {url}");
        }
    }

    yaml
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn parse_new_terminal_format() {
        let yaml = r"
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
";
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
        let yaml = r"
name: myproject
iterm:
  layout: grid
  panes:
    - name: frontend
      dir: ~/projects/myproject/frontend
      cmd: npm run dev
";
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);

        let terminal = config.terminal.unwrap();
        assert_eq!(terminal.terminal_type, "iterm");
        assert_eq!(terminal.layout, Some("grid".to_string()));
        assert_eq!(terminal.panes.len(), 1);
    }

    #[test]
    fn terminal_takes_priority_over_iterm() {
        let yaml = r"
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
";
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
                    env: HashMap::new(),
                }],
            }),
            editor: Some(EditorConfig {
                cmd: None,
                folders: Some(vec!["~/projects/test".to_string()]),
                workspace: None,
            }),
            browser: None,
            checks: None,
            hooks: None,
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
            env: HashMap::new(),
        };
        let cmd = pane.build_command("myproject", false);
        assert!(cmd.contains("cd '/tmp/test'"));
        assert!(cmd.contains("echo $$"));
        assert!(cmd.contains("myproject_dev.pid"));
        assert!(cmd.contains("npm run dev"));
        assert!(!cmd.contains("exec"));
    }

    #[test]
    fn build_pane_command_without_cmd() {
        let pane = PaneConfig {
            name: "shell".to_string(),
            dir: "/tmp/test".to_string(),
            cmd: None,
            env: HashMap::new(),
        };
        let cmd = pane.build_command("myproject", false);
        assert_eq!(cmd, "cd '/tmp/test'");
    }

    #[test]
    fn build_pane_command_with_env() {
        let mut env = HashMap::new();
        env.insert("RUST_LOG".to_string(), "debug".to_string());
        env.insert("PORT".to_string(), "3000".to_string());
        let pane = PaneConfig {
            name: "server".to_string(),
            dir: "/tmp/test".to_string(),
            cmd: Some("cargo run".to_string()),
            env,
        };
        let cmd = pane.build_command("myproject", false);
        assert!(cmd.starts_with("cd '/tmp/test' && "));
        assert!(cmd.contains("export PORT='3000'"));
        assert!(cmd.contains("export RUST_LOG='debug'"));
        assert!(cmd.contains("cargo run"));
    }

    #[test]
    fn pid_file_path_under_state_pids() {
        let path = pid_file_path("myproj", "dev");
        let expected = base_dir().join("state").join("pids").join("myproj_dev.pid");
        assert_eq!(path, expected);
    }

    #[test]
    fn build_pane_command_pid_file_not_in_tmp() {
        let pane = PaneConfig {
            name: "dev".to_string(),
            dir: "/tmp/x".to_string(),
            cmd: Some("cargo run".to_string()),
            env: HashMap::new(),
        };
        let cmd = pane.build_command("myproj", false);
        assert!(
            !cmd.contains("/tmp/.on_"),
            "PID file must not be in /tmp (symlink attack risk): {cmd}"
        );
        let expected = pid_file_path("myproj", "dev");
        assert!(
            cmd.contains(expected.to_str().unwrap()),
            "expected PID file path {} in command: {cmd}",
            expected.display()
        );
    }

    #[test]
    fn ensure_dirs_creates_pids_dir() {
        ensure_dirs().unwrap();
        assert!(base_dir().join("state").join("pids").exists());
    }

    #[test]
    fn build_pane_command_escapes_dir_with_spaces() {
        let pane = PaneConfig {
            name: "dev".to_string(),
            dir: "/tmp/my project".to_string(),
            cmd: None,
            env: HashMap::new(),
        };
        let cmd = pane.build_command("proj", false);
        assert_eq!(cmd, "cd '/tmp/my project'");
    }

    #[test]
    fn build_pane_command_escapes_dir_with_single_quote() {
        let pane = PaneConfig {
            name: "dev".to_string(),
            dir: "/tmp/it's".to_string(),
            cmd: None,
            env: HashMap::new(),
        };
        let cmd = pane.build_command("proj", false);
        assert_eq!(cmd, "cd '/tmp/it'\\''s'");
    }

    #[test]
    fn build_pane_command_env_shell_escape() {
        let mut env = HashMap::new();
        env.insert("MSG".to_string(), "it's a test".to_string());
        let pane = PaneConfig {
            name: "dev".to_string(),
            dir: "/tmp".to_string(),
            cmd: Some("echo $MSG".to_string()),
            env,
        };
        let cmd = pane.build_command("proj", false);
        assert!(cmd.contains("export MSG='it'\\''s a test'"));
    }

    #[test]
    fn build_pane_command_with_logging() {
        let pane = PaneConfig {
            name: "server".to_string(),
            dir: "/tmp/test".to_string(),
            cmd: Some("cargo run".to_string()),
            env: HashMap::new(),
        };
        let cmd = pane.build_command("myproject", true);
        assert!(cmd.contains("tee -a"));
        assert!(cmd.contains("myproject_server.log"));
        assert!(cmd.contains("cargo run"));
    }

    #[test]
    fn parse_yaml_with_env() {
        let yaml = r"
name: envtest
terminal:
  type: tmux
  panes:
    - name: backend
      dir: ~/projects/app
      cmd: cargo run
      env:
        RUST_LOG: debug
        DATABASE_URL: postgres://localhost/mydb
    - name: shell
      dir: ~/projects/app
";
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);
        let terminal = config.terminal.unwrap();
        assert_eq!(terminal.panes[0].env.len(), 2);
        assert_eq!(terminal.panes[0].env["RUST_LOG"], "debug");
        assert!(terminal.panes[1].env.is_empty());
    }

    #[test]
    fn parse_yaml_with_hooks() {
        let yaml = r#"
name: hooktest
hooks:
  pre_launch:
    - docker compose up -d
    - echo "starting"
  post_launch:
    - open http://localhost:3000
  pre_stop:
    - docker compose down
"#;
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.pre_launch.unwrap().len(), 2);
        assert_eq!(hooks.post_launch.unwrap().len(), 1);
        assert_eq!(hooks.pre_stop.unwrap()[0], "docker compose down");
    }

    #[test]
    fn parse_yaml_without_hooks() {
        let yaml = "name: nohooks\n";
        let raw: RawConfig = serde_yaml::from_str(yaml).unwrap();
        let config = resolve_config(raw);
        assert!(config.hooks.is_none());
    }

    #[test]
    fn merge_current_overrides_base() {
        let base = Config {
            name: "base".to_string(),
            terminal: None,
            editor: Some(EditorConfig {
                cmd: Some("vim".to_string()),
                folders: None,
                workspace: None,
            }),
            browser: Some(vec!["http://github.com".to_string()]),
            checks: Some(ChecksConfig {
                dirty_git: Some(true),
            }),
            hooks: None,
        };
        let current = Config {
            name: "myproject".to_string(),
            terminal: None,
            editor: Some(EditorConfig {
                cmd: Some("cursor".to_string()),
                folders: None,
                workspace: None,
            }),
            browser: None,
            checks: None,
            hooks: None,
        };
        let merged = merge_configs(base, current);
        assert_eq!(merged.name, "myproject");
        assert_eq!(merged.editor.unwrap().cmd, Some("cursor".to_string()));
        assert_eq!(merged.browser.unwrap(), vec!["http://github.com"]);
        assert!(merged.checks.unwrap().dirty_git.unwrap());
    }

    #[test]
    fn merge_inherits_from_base() {
        let base = Config {
            name: "base".to_string(),
            terminal: None,
            editor: Some(EditorConfig {
                cmd: Some("code".to_string()),
                folders: None,
                workspace: None,
            }),
            browser: None,
            checks: None,
            hooks: Some(HooksConfig {
                pre_launch: Some(vec!["docker compose up -d".to_string()]),
                post_launch: None,
                pre_stop: None,
            }),
        };
        let current = Config {
            name: "child".to_string(),
            terminal: None,
            editor: None,
            browser: None,
            checks: None,
            hooks: None,
        };
        let merged = merge_configs(base, current);
        assert_eq!(merged.name, "child");
        assert_eq!(merged.editor.unwrap().cmd, Some("code".to_string()));
        assert!(merged.hooks.unwrap().pre_launch.is_some());
    }

    #[test]
    fn extends_supports_multi_level_inheritance() {
        let grand = "_on_test_grand";
        let mid = "_on_test_mid";
        let leaf = "_on_test_leaf";
        let cleanup = || {
            let _ = fs::remove_file(config_path(grand));
            let _ = fs::remove_file(config_path(mid));
            let _ = fs::remove_file(config_path(leaf));
        };
        cleanup();

        ensure_dirs().unwrap();
        fs::write(
            config_path(grand),
            "name: _on_test_grand\neditor:\n  cmd: vim\n",
        )
        .unwrap();
        fs::write(
            config_path(mid),
            "name: _on_test_mid\nextends: _on_test_grand\nbrowser:\n  - http://localhost\n",
        )
        .unwrap();
        fs::write(
            config_path(leaf),
            "name: _on_test_leaf\nextends: _on_test_mid\n",
        )
        .unwrap();

        let config = load(leaf).unwrap();
        assert_eq!(config.name, "_on_test_leaf");
        // From grand
        assert_eq!(config.editor.unwrap().cmd, Some("vim".to_string()));
        // From mid
        assert_eq!(config.browser.unwrap(), vec!["http://localhost"]);

        cleanup();
    }

    #[test]
    fn extends_detects_cycle() {
        let a = "_on_test_cycle_a";
        let b = "_on_test_cycle_b";
        let cleanup = || {
            let _ = fs::remove_file(config_path(a));
            let _ = fs::remove_file(config_path(b));
        };
        cleanup();
        ensure_dirs().unwrap();
        fs::write(
            config_path(a),
            "name: _on_test_cycle_a\nextends: _on_test_cycle_b\n",
        )
        .unwrap();
        fs::write(
            config_path(b),
            "name: _on_test_cycle_b\nextends: _on_test_cycle_a\n",
        )
        .unwrap();

        let err = load(a).expect_err("expected cycle detection error");
        let msg = format!("{err:#}").to_lowercase();
        assert!(
            msg.contains("cycle") || msg.contains("circular") || msg.contains("loop"),
            "expected cycle-related error, got: {msg}"
        );

        cleanup();
    }

    #[test]
    fn extends_yaml_roundtrip() {
        let name_base = "_on_test_base";
        let name_child = "_on_test_child";
        let base_path = config_path(name_base);
        let child_path = config_path(name_child);
        let _ = fs::remove_file(&base_path);
        let _ = fs::remove_file(&child_path);

        ensure_dirs().unwrap();
        fs::write(
            &base_path,
            "name: _on_test_base\neditor:\n  cmd: cursor\nbrowser:\n  - http://github.com\n",
        )
        .unwrap();
        fs::write(
            &child_path,
            "name: _on_test_child\nextends: _on_test_base\nterminal:\n  type: tmux\n  panes:\n    - name: dev\n      dir: /tmp\n",
        )
        .unwrap();

        let config = load(name_child).unwrap();
        assert_eq!(config.name, "_on_test_child");
        assert!(config.terminal.is_some());
        assert_eq!(config.editor.unwrap().cmd, Some("cursor".to_string()));
        assert_eq!(config.browser.unwrap(), vec!["http://github.com"]);

        let _ = fs::remove_file(&base_path);
        let _ = fs::remove_file(&child_path);
    }

    #[test]
    fn home_or_fallback_uses_home_when_some() {
        assert_eq!(
            home_or_fallback(Some(PathBuf::from("/h"))),
            PathBuf::from("/h")
        );
    }

    #[test]
    fn home_or_fallback_falls_back_to_cwd() {
        // No home → must NOT panic. Falling back to `.` is acceptable; what
        // matters is we don't crash on sandboxes that hide HOME.
        let path = home_or_fallback(None);
        assert_eq!(path, PathBuf::from("."));
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

    #[test]
    fn detect_rust_project() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        let result = detect_project(dir.path());
        assert_eq!(result.panes.len(), 2);
        assert_eq!(result.panes[0].name, "server");
        assert_eq!(result.panes[0].cmd, Some("cargo run".to_string()));
        assert_eq!(result.panes[1].name, "shell");
    }

    #[test]
    fn detect_node_with_dev_script() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
        )
        .unwrap();
        let pane = detect_in_dir(dir.path(), "app").unwrap();
        assert_eq!(pane.cmd, Some("npm run dev".to_string()));
    }

    #[test]
    fn detect_node_with_start_only() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"start": "node index.js"}}"#,
        )
        .unwrap();
        let pane = detect_in_dir(dir.path(), "app").unwrap();
        assert_eq!(pane.cmd, Some("npm start".to_string()));
    }

    #[test]
    fn detect_node_ignores_devdependencies_substring() {
        // The previous implementation grep'd for "dev" — that matches
        // devDependencies even when there is no dev script. We should
        // pick `start` here, not `dev`.
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies": {"vite": "^4"}, "scripts": {"start": "node ."}}"#,
        )
        .unwrap();
        let pane = detect_in_dir(dir.path(), "app").unwrap();
        assert_eq!(pane.cmd, Some("npm start".to_string()));
    }

    #[test]
    fn detect_python_django_manage_py() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("manage.py"), "#!/usr/bin/env python\n").unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"app\"\n",
        )
        .unwrap();
        let pane = detect_in_dir(dir.path(), "app").unwrap();
        assert!(
            pane.cmd.as_deref().is_some_and(|c| c.contains("manage.py")),
            "expected manage.py command, got {:?}",
            pane.cmd
        );
    }

    #[test]
    fn detect_docker_compose() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("docker-compose.yml"), "services: {}\n").unwrap();
        let pane = detect_in_dir(dir.path(), "stack").unwrap();
        assert_eq!(pane.cmd, Some("docker compose up".to_string()));
    }

    #[test]
    fn detect_docker_compose_yaml_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("docker-compose.yaml"), "services: {}\n").unwrap();
        let pane = detect_in_dir(dir.path(), "stack").unwrap();
        assert_eq!(pane.cmd, Some("docker compose up".to_string()));
    }

    #[test]
    fn detect_node_project() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"dev": "next dev"}}"#,
        )
        .unwrap();
        let result = detect_project(dir.path());
        assert_eq!(result.panes[0].cmd, Some("npm run dev".to_string()));
        assert_eq!(result.panes[0].port, Some(3000));
    }

    #[test]
    fn detect_monorepo() {
        let dir = tempfile::tempdir().unwrap();
        let frontend = dir.path().join("frontend");
        let backend = dir.path().join("backend");
        fs::create_dir_all(&frontend).unwrap();
        fs::create_dir_all(&backend).unwrap();
        fs::write(
            frontend.join("package.json"),
            r#"{"scripts": {"dev": "vite"}}"#,
        )
        .unwrap();
        fs::write(backend.join("Cargo.toml"), "[package]\nname = \"api\"").unwrap();
        let result = detect_project(dir.path());
        // backend + frontend + shell = 3 panes
        assert_eq!(result.panes.len(), 3);
        let names: Vec<&str> = result.panes.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"frontend"));
        assert!(names.contains(&"backend"));
        assert!(names.contains(&"shell"));
    }

    #[test]
    fn detect_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = detect_project(dir.path());
        assert_eq!(result.panes.len(), 1);
        assert_eq!(result.panes[0].name, "shell");
    }

    #[test]
    fn validate_returns_empty_for_valid_configs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ok.yaml"), "name: ok\n").unwrap();
        let issues = validate_configs_in(dir.path());
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn validate_reports_yaml_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("broken.yaml"), "name: [unclosed\n").unwrap();
        let issues = validate_configs_in(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].project, "broken");
        assert!(
            issues[0].message.to_lowercase().contains("parse")
                || issues[0].message.to_lowercase().contains("yaml"),
            "expected parse error, got {}",
            issues[0].message
        );
    }

    #[test]
    fn validate_reports_missing_extends_base() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("child.yaml"),
            "name: child\nextends: nonexistent\n",
        )
        .unwrap();
        let issues = validate_configs_in(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].project, "child");
        assert!(
            issues[0].message.contains("nonexistent"),
            "expected message to mention the missing base, got {}",
            issues[0].message
        );
    }

    #[test]
    fn filter_issues_keeps_only_named_project() {
        let issues = vec![
            ConfigIssue {
                project: "a".to_string(),
                message: "x".to_string(),
            },
            ConfigIssue {
                project: "b".to_string(),
                message: "y".to_string(),
            },
            ConfigIssue {
                project: "a".to_string(),
                message: "z".to_string(),
            },
        ];
        let filtered = filter_issues_by_project(issues, Some("a"));
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|i| i.project == "a"));
    }

    #[test]
    fn filter_issues_none_returns_all() {
        let issues = vec![ConfigIssue {
            project: "a".to_string(),
            message: "x".to_string(),
        }];
        assert_eq!(filter_issues_by_project(issues.clone(), None).len(), 1);
    }

    #[test]
    fn validate_ignores_non_yaml_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "hello").unwrap();
        let issues = validate_configs_in(dir.path());
        assert!(issues.is_empty());
    }

    #[test]
    fn validate_returns_empty_for_present_extends() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("base.yaml"), "name: base\n").unwrap();
        fs::write(
            dir.path().join("child.yaml"),
            "name: child\nextends: base\n",
        )
        .unwrap();
        let issues = validate_configs_in(dir.path());
        assert!(issues.is_empty(), "expected no issues, got {issues:?}");
    }

    #[test]
    fn create_config_from_detection_roundtrip() {
        let detected = DetectedProject {
            name: "myapp".to_string(),
            panes: vec![
                DetectedPane {
                    name: "server".to_string(),
                    dir: "/Users/me/projects/myapp".to_string(),
                    cmd: Some("cargo run".to_string()),
                    port: None,
                },
                DetectedPane {
                    name: "shell".to_string(),
                    dir: "/Users/me/projects/myapp".to_string(),
                    cmd: None,
                    port: None,
                },
            ],
        };
        let yaml = create_config_from_detection("myapp", &detected, "cursor");
        assert!(yaml.contains("name: myapp"));
        assert!(yaml.contains("cmd: cargo run"));
        assert!(yaml.contains("cmd: cursor"));
    }
}
