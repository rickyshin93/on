use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::PaneConfig;

/// Open a new iTerm2 tab with panes arranged according to layout.
pub fn open_panes(project: &str, panes: &[PaneConfig], layout: &str) -> Result<()> {
    if panes.is_empty() {
        return Ok(());
    }

    let script = match layout {
        "grid" => build_grid_script(project, panes),
        _ => build_vertical_script(project, panes),
    };

    run_applescript(&script)
}

/// Build `AppleScript` for vertical (side-by-side) layout
fn build_vertical_script(project: &str, panes: &[PaneConfig]) -> String {
    let n = panes.len();
    let mut s = String::new();

    s.push_str("tell application \"iTerm2\"\n");
    s.push_str("  activate\n");
    s.push_str("  if (count of windows) = 0 then\n");
    s.push_str("    create window with default profile\n");
    s.push_str("    set theTab to current tab of current window\n");
    s.push_str("  else\n");
    s.push_str("    tell current window\n");
    s.push_str("      set theTab to (create tab with default profile)\n");
    s.push_str("    end tell\n");
    s.push_str("  end if\n");
    s.push_str("  tell current window\n");

    for _ in 1..n {
        s.push_str("    tell first session of theTab\n");
        s.push_str("      split vertically with default profile\n");
        s.push_str("    end tell\n");
    }

    for (i, pane) in panes.iter().enumerate() {
        append_pane_config(&mut s, project, pane, i + 1);
    }

    s.push_str("  end tell\n");
    s.push_str("end tell\n");
    s
}

/// Build `AppleScript` for grid (2x2) layout
fn build_grid_script(project: &str, panes: &[PaneConfig]) -> String {
    let n = panes.len();

    if n <= 2 {
        return build_vertical_script(project, panes);
    }

    let mut s = String::new();

    s.push_str("tell application \"iTerm2\"\n");
    s.push_str("  activate\n");
    s.push_str("  if (count of windows) = 0 then\n");
    s.push_str("    create window with default profile\n");
    s.push_str("    set theTab to current tab of current window\n");
    s.push_str("  else\n");
    s.push_str("    tell current window\n");
    s.push_str("      set theTab to (create tab with default profile)\n");
    s.push_str("    end tell\n");
    s.push_str("  end if\n");
    s.push_str("  tell current window\n");

    s.push_str("    tell first session of theTab\n");
    s.push_str("      split vertically with default profile\n");
    s.push_str("    end tell\n");

    s.push_str("    tell first session of theTab\n");
    s.push_str("      split horizontally with default profile\n");
    s.push_str("    end tell\n");

    if n >= 4 {
        s.push_str("    tell last session of theTab\n");
        s.push_str("      split horizontally with default profile\n");
        s.push_str("    end tell\n");
    }

    for _ in 4..n {
        s.push_str("    tell last session of theTab\n");
        s.push_str("      split vertically with default profile\n");
        s.push_str("    end tell\n");
    }

    for (i, pane) in panes.iter().enumerate() {
        append_pane_config(&mut s, project, pane, i + 1);
    }

    s.push_str("  end tell\n");
    s.push_str("end tell\n");
    s
}

/// Append `AppleScript` lines to configure a single pane session
#[allow(clippy::format_push_string)]
fn append_pane_config(s: &mut String, project: &str, pane: &PaneConfig, index: usize) {
    let title = format!("[{project}] {}", pane.name);
    let cmd = build_pane_command(project, pane);
    let escaped_cmd = cmd.replace('\\', "\\\\").replace('"', "\\\"");
    s.push_str(&format!("    tell item {index} of sessions of theTab\n"));
    s.push_str(&format!("      set name to \"{title}\"\n"));
    s.push_str(&format!("      write text \"{escaped_cmd}\"\n"));
    s.push_str("    end tell\n");
}

/// Build the shell command string for a pane.
fn build_pane_command(project: &str, pane: &PaneConfig) -> String {
    match &pane.cmd {
        Some(cmd) => {
            let pid_file = format!("/tmp/.on_{project}_{}.pid", pane.name);
            format!("cd {} && echo $$ > {pid_file} && exec {cmd}", pane.dir)
        }
        None => format!("cd {}", pane.dir),
    }
}

/// Close iTerm2 tabs whose sessions have names starting with "[project]"
pub fn close_tabs(project: &str) {
    let prefix = format!("[{project}]");
    let script = format!(
        r#"tell application "iTerm2"
  if (count of windows) > 0 then
    tell current window
      set tabsToClose to {{}}
      repeat with t in tabs
        repeat with s in sessions of t
          if name of s starts with "{prefix}" then
            set end of tabsToClose to t
            exit repeat
          end if
        end repeat
      end repeat
      repeat with t in tabsToClose
        close t
      end repeat
    end tell
  end if
end tell"#,
    );

    let _ = run_applescript(&script);
}

/// Execute an `AppleScript` string via osascript
fn run_applescript(script: &str) -> Result<()> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .context("Failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("AppleScript error: {stderr}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pane(name: &str, dir: &str, cmd: Option<&str>) -> PaneConfig {
        PaneConfig {
            name: name.to_string(),
            dir: dir.to_string(),
            cmd: cmd.map(String::from),
        }
    }

    #[test]
    fn pane_command_with_cmd() {
        let pane = test_pane("dev", "/tmp/test", Some("npm run dev"));
        let cmd = build_pane_command("myproject", &pane);
        assert!(cmd.contains("cd /tmp/test"));
        assert!(cmd.contains("echo $$"));
        assert!(cmd.contains(".on_myproject_dev.pid"));
        assert!(cmd.contains("exec npm run dev"));
    }

    #[test]
    fn pane_command_without_cmd() {
        let pane = test_pane("shell", "/tmp/test", None);
        let cmd = build_pane_command("myproject", &pane);
        assert_eq!(cmd, "cd /tmp/test");
    }

    #[test]
    fn vertical_script_contains_split() {
        let panes = vec![
            test_pane("a", "/tmp/a", Some("echo a")),
            test_pane("b", "/tmp/b", Some("echo b")),
            test_pane("c", "/tmp/c", None),
        ];
        let script = build_vertical_script("proj", &panes);
        assert_eq!(script.matches("split vertically").count(), 2);
        assert!(script.contains("[proj] a"));
        assert!(script.contains("[proj] b"));
        assert!(script.contains("[proj] c"));
    }

    #[test]
    fn grid_script_for_4_panes() {
        let panes = vec![
            test_pane("a", "/tmp/a", Some("echo a")),
            test_pane("b", "/tmp/b", Some("echo b")),
            test_pane("c", "/tmp/c", Some("echo c")),
            test_pane("d", "/tmp/d", Some("echo d")),
        ];
        let script = build_grid_script("proj", &panes);
        assert_eq!(script.matches("split vertically").count(), 1);
        assert_eq!(script.matches("split horizontally").count(), 2);
    }

    #[test]
    fn grid_falls_back_to_vertical_for_2() {
        let panes = vec![
            test_pane("a", "/tmp/a", Some("echo a")),
            test_pane("b", "/tmp/b", Some("echo b")),
        ];
        let script = build_grid_script("proj", &panes);
        assert_eq!(script.matches("split vertically").count(), 1);
        assert_eq!(script.matches("split horizontally").count(), 0);
    }

    #[test]
    fn empty_panes_is_ok() {
        assert!(open_panes("proj", &[], "vertical").is_ok());
    }
}
