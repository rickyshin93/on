use std::collections::HashSet;
use std::process::Command;

pub struct DirtyRepo {
    pub dir: String,
    pub file_count: usize,
}

/// Check git status for a list of directories (deduplicated).
/// Returns list of directories with uncommitted changes.
pub fn check_status(dirs: &[String]) -> Vec<DirtyRepo> {
    let unique_dirs: HashSet<&String> = dirs.iter().collect();
    let mut dirty = Vec::new();

    for dir in unique_dirs {
        if let Some(count) = get_dirty_count(dir) {
            if count > 0 {
                dirty.push(DirtyRepo {
                    dir: dir.clone(),
                    file_count: count,
                });
            }
        }
    }
    dirty
}

/// Run `git status --porcelain` in a directory and count changed files.
/// Returns None if not a git repo or git not available.
fn get_dirty_count(dir: &str) -> Option<usize> {
    let output = Command::new("git")
        .args(["-C", dir, "status", "--porcelain"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout.lines().filter(|l| !l.is_empty()).count();
    Some(count)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::fs;

    fn git(args: &[&str], cwd: &std::path::Path) {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git failed");
    }

    #[test]
    fn clean_repo_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git(&["init"], dir);
        fs::write(dir.join("hello.txt"), "hello").unwrap();
        git(&["add", "."], dir);
        git(
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ],
            dir,
        );

        let dirs = vec![dir.to_string_lossy().to_string()];
        assert!(check_status(&dirs).is_empty());
    }

    #[test]
    fn dirty_repo_returns_count() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git(&["init"], dir);
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("b.txt"), "b").unwrap();

        let dirs = vec![dir.to_string_lossy().to_string()];
        let dirty = check_status(&dirs);
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].file_count, 2);
    }

    #[test]
    fn non_git_dir_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vec![tmp.path().to_string_lossy().to_string()];
        assert!(check_status(&dirs).is_empty());
    }

    #[test]
    fn deduplicates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git(&["init"], dir);
        fs::write(dir.join("a.txt"), "a").unwrap();

        let s = dir.to_string_lossy().to_string();
        let dirty = check_status(&[s.clone(), s.clone(), s]);
        assert_eq!(dirty.len(), 1);
    }
}
