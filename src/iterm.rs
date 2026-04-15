use anyhow::{bail, Context, Result};
use std::fmt::Write as _;
use std::process::Command;

use crate::config::PaneConfig;

/// Open iTerm2 tabs with panes arranged according to layout.
/// When `max_per_tab` is set, panes are split across multiple tabs.
pub fn open_panes(
    project: &str,
    panes: &[PaneConfig],
    layout: &str,
    max_per_tab: Option<usize>,
) -> Result<()> {
    if panes.is_empty() {
        return Ok(());
    }

    let limit = max_per_tab.unwrap_or(panes.len());
    let script = build_script(project, panes, layout, limit);
    run_applescript(&script)
}

/// Build a complete `AppleScript` that creates one or more tabs with panes.
fn build_script(project: &str, panes: &[PaneConfig], layout: &str, max_per_tab: usize) -> String {
    let chunks: Vec<&[PaneConfig]> = panes.chunks(max_per_tab).collect();
    let mut s = String::new();

    s.push_str("tell application \"iTerm2\"\n");
    s.push_str("  activate\n");

    for (tab_idx, chunk) in chunks.iter().enumerate() {
        append_tab_creation(&mut s, tab_idx == 0);
        s.push_str("  tell current window\n");

        match layout {
            "grid" => append_grid_splits(&mut s, chunk.len()),
            _ => append_vertical_splits(&mut s, chunk.len()),
        }

        for (i, pane) in chunk.iter().enumerate() {
            append_pane_config(&mut s, project, pane, i + 1);
        }

        s.push_str("  end tell\n");
    }

    s.push_str("end tell\n");
    s
}

/// Append `AppleScript` lines to create a new tab (or window if none exists).
fn append_tab_creation(s: &mut String, is_first: bool) {
    if is_first {
        s.push_str("  if (count of windows) = 0 then\n");
        s.push_str("    create window with default profile\n");
        s.push_str("    set theTab to current tab of current window\n");
        s.push_str("  else\n");
        s.push_str("    tell current window\n");
        s.push_str("      set theTab to (create tab with default profile)\n");
        s.push_str("    end tell\n");
        s.push_str("  end if\n");
    } else {
        s.push_str("  tell current window\n");
        s.push_str("    set theTab to (create tab with default profile)\n");
        s.push_str("  end tell\n");
    }
}

/// Append vertical (side-by-side) split commands for `n` panes.
fn append_vertical_splits(s: &mut String, n: usize) {
    for _ in 1..n {
        s.push_str("    tell first session of theTab\n");
        s.push_str("      split vertically with default profile\n");
        s.push_str("    end tell\n");
    }
}

/// Append grid split commands for `n` panes.
///
/// Creates rows by splitting horizontally, then splits each row vertically
/// into columns. Panes are ordered left-to-right, top-to-bottom.
fn append_grid_splits(s: &mut String, n: usize) {
    if n <= 2 {
        append_vertical_splits(s, n);
        return;
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let cols = ((n as f64).sqrt().ceil()) as usize;
    let rows = n.div_ceil(cols);

    // Distribute panes across rows: first `extra` rows get one more column
    let base = n / rows;
    let extra = n % rows;
    let cols_per_row: Vec<usize> = (0..rows)
        .map(|i| if i < extra { base + 1 } else { base })
        .collect();

    // Create rows by splitting first session horizontally
    for _ in 1..rows {
        s.push_str("    tell first session of theTab\n");
        s.push_str("      split horizontally with default profile\n");
        s.push_str("    end tell\n");
    }

    // For each row, create columns by splitting vertically
    for i in 0..rows {
        let target: usize = 1 + cols_per_row[..i].iter().sum::<usize>();
        for _ in 1..cols_per_row[i] {
            let _ = writeln!(s, "    tell item {target} of sessions of theTab");
            s.push_str("      split vertically with default profile\n");
            s.push_str("    end tell\n");
        }
    }
}

/// Append `AppleScript` lines to configure a single pane session
#[allow(clippy::format_push_string)]
fn append_pane_config(s: &mut String, project: &str, pane: &PaneConfig, index: usize) {
    let title = format!("[{project}] {}", pane.name);
    let cmd = pane.build_command(project);
    // \e]1; sets the tab title (to project name), \e]2; sets the pane title (to pane name)
    let full_cmd = format!("printf '\\e]1;{project}\\a\\e]2;{}\\a'; {cmd}", pane.name);
    let escaped_cmd = full_cmd.replace('\\', "\\\\").replace('"', "\\\"");
    s.push_str(&format!("    tell item {index} of sessions of theTab\n"));
    s.push_str(&format!("      set name to \"{title}\"\n"));
    s.push_str(&format!("      write text \"{escaped_cmd}\"\n"));
    s.push_str("    end tell\n");
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

    fn test_panes(n: usize) -> Vec<PaneConfig> {
        (0..n)
            .map(|i| test_pane(&format!("p{i}"), "/tmp", Some(&format!("echo {i}"))))
            .collect()
    }

    #[test]
    fn vertical_script_contains_split() {
        let panes = vec![
            test_pane("a", "/tmp/a", Some("echo a")),
            test_pane("b", "/tmp/b", Some("echo b")),
            test_pane("c", "/tmp/c", None),
        ];
        let script = build_script("proj", &panes, "vertical", panes.len());
        assert_eq!(script.matches("split vertically").count(), 2);
        assert!(script.contains("[proj] a"));
        assert!(script.contains("[proj] b"));
        assert!(script.contains("[proj] c"));
    }

    #[test]
    fn grid_script_for_3_panes() {
        let panes = test_panes(3);
        let script = build_script("proj", &panes, "grid", panes.len());
        // 2 rows, cols_per_row=[2,1]: 1 horizontal + 1 vertical
        assert_eq!(script.matches("split horizontally").count(), 1);
        assert_eq!(script.matches("split vertically").count(), 1);
    }

    #[test]
    fn grid_script_for_4_panes() {
        let panes = test_panes(4);
        let script = build_script("proj", &panes, "grid", panes.len());
        // 2x2 grid: 1 horizontal + 2 vertical
        assert_eq!(script.matches("split horizontally").count(), 1);
        assert_eq!(script.matches("split vertically").count(), 2);
    }

    #[test]
    fn grid_script_for_7_panes() {
        let panes = test_panes(7);
        let script = build_script("proj", &panes, "grid", panes.len());
        // 3 rows, cols_per_row=[3,2,2]: 2 horizontal + 4 vertical
        assert_eq!(script.matches("split horizontally").count(), 2);
        assert_eq!(script.matches("split vertically").count(), 4);
        // Verify indexed session targeting (not all "last session")
        assert!(script.contains("item 1 of sessions"));
        assert!(script.contains("item 4 of sessions"));
        assert!(script.contains("item 6 of sessions"));
    }

    #[test]
    fn grid_script_for_9_panes() {
        let panes = test_panes(9);
        let script = build_script("proj", &panes, "grid", panes.len());
        // 3x3 grid: 2 horizontal + 6 vertical
        assert_eq!(script.matches("split horizontally").count(), 2);
        assert_eq!(script.matches("split vertically").count(), 6);
    }

    #[test]
    fn grid_falls_back_to_vertical_for_2() {
        let panes = test_panes(2);
        let script = build_script("proj", &panes, "grid", panes.len());
        assert_eq!(script.matches("split vertically").count(), 1);
        assert_eq!(script.matches("split horizontally").count(), 0);
    }

    #[test]
    fn multi_tab_vertical() {
        let panes = test_panes(5);
        let script = build_script("proj", &panes, "vertical", 3);
        // Tab 1: 3 panes (2 splits), Tab 2: 2 panes (1 split)
        assert_eq!(script.matches("split vertically").count(), 3);
        assert_eq!(script.matches("create tab with default profile").count(), 2);
        // First tab may create window, second always creates tab
        assert!(script.contains("create window with default profile"));
    }

    #[test]
    fn multi_tab_grid() {
        let panes = test_panes(7);
        let script = build_script("proj", &panes, "grid", 3);
        // Tab 1: 3 panes (grid 2+1), Tab 2: 3 panes (grid 2+1), Tab 3: 1 pane (no splits)
        assert_eq!(script.matches("create tab with default profile").count(), 3);
        // Each 3-pane grid tab: 1 horizontal + 1 vertical = 2+2=4 total grid splits
        assert_eq!(script.matches("split horizontally").count(), 2);
        assert_eq!(script.matches("split vertically").count(), 2);
    }

    #[test]
    fn single_pane_no_splits() {
        let panes = test_panes(1);
        let script = build_script("proj", &panes, "grid", 3);
        assert_eq!(script.matches("split").count(), 0);
        assert!(script.contains("[proj] p0"));
    }

    #[test]
    fn empty_panes_is_ok() {
        assert!(open_panes("proj", &[], "vertical", None).is_ok());
    }
}
