use regex::Regex;
use std::collections::HashSet;
use std::process::Command;
use std::sync::LazyLock;

#[allow(clippy::expect_used)] // constant regex, validated by unit tests
static URL_PORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:localhost|127\.0\.0\.1):(\d+)").expect("URL_PORT_RE is a constant valid regex")
});
#[allow(clippy::expect_used)] // constant regex, validated by unit tests
static CMD_PORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:--port[=\s]|(?:^|\s)-p\s?)(\d+)")
        .expect("CMD_PORT_RE is a constant valid regex")
});

pub struct PortConflict {
    pub port: u16,
    pub pid: u32,
    pub process_name: String,
}

/// Extract ports from browser URLs and pane commands.
pub fn extract_ports(urls: &[String], cmds: &[String]) -> Vec<u16> {
    let mut ports = HashSet::new();

    // From browser URLs: localhost:<port> or 127.0.0.1:<port>
    for url in urls {
        for cap in URL_PORT_RE.captures_iter(url) {
            if let Ok(port) = cap[1].parse::<u16>() {
                ports.insert(port);
            }
        }
    }

    // From commands: --port <N>, --port=<N>, -p <N>, -p<N>
    for cmd in cmds {
        for cap in CMD_PORT_RE.captures_iter(cmd) {
            if let Ok(port) = cap[1].parse::<u16>() {
                ports.insert(port);
            }
        }
    }

    let mut result: Vec<u16> = ports.into_iter().collect();
    result.sort_unstable();
    result
}

/// Parse the output of `lsof -F pcn -i -P -n -sTCP:LISTEN`.
///
/// Format: each process is a `p<pid>` line followed by `c<command>` and
/// one or more `n<addr>` lines. `<addr>` looks like `*:80`, `127.0.0.1:3000`,
/// or `[::1]:8443`. Returns one `PortConflict` per (process, port) pair.
fn parse_lsof_listeners(output: &str) -> Vec<PortConflict> {
    let mut listeners = Vec::new();
    let mut pid: Option<u32> = None;
    let mut name = String::new();

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            pid = rest.trim().parse().ok();
            name.clear();
        } else if let Some(rest) = line.strip_prefix('c') {
            name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix('n') {
            let Some(pid) = pid else { continue };
            // Port is whatever follows the last ':' in the address.
            if let Some(colon) = rest.rfind(':') {
                if let Ok(port) = rest[colon + 1..].parse::<u16>() {
                    listeners.push(PortConflict {
                        port,
                        pid,
                        process_name: name.clone(),
                    });
                }
            }
        }
    }
    listeners
}

/// Query all TCP listeners in one `lsof` invocation.
fn query_listeners() -> Vec<PortConflict> {
    let output = Command::new("lsof")
        .args(["-F", "pcn", "-i", "-P", "-n", "-sTCP:LISTEN"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    // lsof exits non-zero when no matches found; treat that as "no listeners".
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_lsof_listeners(&stdout)
}

/// Check a set of ports in a single `lsof` invocation. Returns conflicts only
/// for ports that are actually in use.
pub fn check_ports(ports: &[u16]) -> Vec<PortConflict> {
    if ports.is_empty() {
        return Vec::new();
    }
    let wanted: HashSet<u16> = ports.iter().copied().collect();
    query_listeners()
        .into_iter()
        .filter(|c| wanted.contains(&c.port))
        .collect()
}

/// Get the full command line for a PID (not just the comm name).
/// Returns `None` if the process is gone or `ps` fails.
pub fn process_cmdline(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
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

/// Kill a process by PID: SIGTERM first, wait briefly, SIGKILL if still alive.
/// This gives the process a chance to clean up (close sockets, flush buffers)
/// instead of the kernel ripping it out from under it.
///
/// Returns `true` if the SIGTERM was delivered successfully. The caller's
/// parent process (not us) is responsible for reaping the dead child, so we
/// don't try to verify the PID is fully gone — a zombie that's been signalled
/// still shows up as "alive" to `kill(pid, 0)`.
pub fn kill_pid(pid: u32) -> bool {
    let pid_arg = pid.to_string();

    let term_ok = Command::new("kill")
        .args(["-TERM", &pid_arg])
        .output()
        .is_ok_and(|o| o.status.success());
    if !term_ok {
        return false;
    }

    std::thread::sleep(std::time::Duration::from_millis(200));

    if crate::state::is_pid_alive(pid) {
        let _ = Command::new("kill").args(["-9", &pid_arg]).output();
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn extract_from_localhost_urls() {
        let urls = vec![
            "http://localhost:3000".to_string(),
            "https://github.com/test".to_string(),
            "http://127.0.0.1:8080/api".to_string(),
        ];
        let ports = extract_ports(&urls, &[]);
        assert!(ports.contains(&3000));
        assert!(ports.contains(&8080));
        assert_eq!(ports.len(), 2);
    }

    #[test]
    fn extract_from_commands() {
        let cmds = vec![
            "python main.py --port 5000".to_string(),
            "server --port=9090".to_string(),
            "redis-server -p 6379".to_string(),
            "npm run dev".to_string(), // no port
        ];
        let ports = extract_ports(&[], &cmds);
        assert!(ports.contains(&5000));
        assert!(ports.contains(&9090));
        assert!(ports.contains(&6379));
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn extract_combined_and_deduped() {
        let urls = vec!["http://localhost:3000".to_string()];
        let cmds = vec!["server --port 3000".to_string()];
        let ports = extract_ports(&urls, &cmds);
        // Port 3000 appears in both but should be deduped
        assert_eq!(ports, vec![3000]);
    }

    #[test]
    fn extract_empty_inputs() {
        let ports = extract_ports(&[], &[]);
        assert!(ports.is_empty());
    }

    #[test]
    fn extract_no_port_urls() {
        let urls = vec![
            "https://github.com/foo".to_string(),
            "https://example.com".to_string(),
        ];
        let ports = extract_ports(&urls, &[]);
        assert!(ports.is_empty());
    }

    #[test]
    fn process_cmdline_for_self_is_some() {
        let cmd = process_cmdline(std::process::id());
        assert!(cmd.is_some());
        assert!(!cmd.unwrap().is_empty());
    }

    #[test]
    fn process_cmdline_for_dead_pid_is_none() {
        assert!(process_cmdline(99999).is_none());
    }

    #[test]
    fn kill_pid_terminates_sleep_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");
        let pid = child.id();
        assert!(kill_pid(pid), "kill_pid returned false");
        // Reap so the test cleans up (and to confirm the child actually exited
        // from a signal rather than aging out).
        let status = child.wait().expect("wait failed");
        // Should have been signalled, not exited normally.
        assert!(
            !status.success(),
            "expected sleep to be killed, got success status"
        );
    }

    #[test]
    fn parse_single_listener() {
        let output = "p1234\ncnginx\nn*:8080\n";
        let listeners = parse_lsof_listeners(output);
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].port, 8080);
        assert_eq!(listeners[0].pid, 1234);
        assert_eq!(listeners[0].process_name, "nginx");
    }

    #[test]
    fn parse_multiple_ports_one_process() {
        // A process listening on several ports has multiple "n" lines.
        let output = "p2000\ncnode\nn*:3000\nn127.0.0.1:3001\n";
        let listeners = parse_lsof_listeners(output);
        let mut ports: Vec<u16> = listeners.iter().map(|l| l.port).collect();
        ports.sort_unstable();
        assert_eq!(ports, vec![3000, 3001]);
        assert!(listeners
            .iter()
            .all(|l| l.pid == 2000 && l.process_name == "node"));
    }

    #[test]
    fn parse_multiple_processes() {
        let output = "p1000\ncfoo\nn*:80\np2000\ncbar\nn*:443\n";
        let listeners = parse_lsof_listeners(output);
        assert_eq!(listeners.len(), 2);
        let foo = listeners.iter().find(|l| l.port == 80).unwrap();
        assert_eq!(foo.process_name, "foo");
        assert_eq!(foo.pid, 1000);
        let bar = listeners.iter().find(|l| l.port == 443).unwrap();
        assert_eq!(bar.process_name, "bar");
        assert_eq!(bar.pid, 2000);
    }

    #[test]
    fn parse_ipv6_listener() {
        // lsof writes IPv6 as `[::1]:port` or `[::]:port`
        let output = "p3000\ncfoo\nn[::1]:8443\n";
        let listeners = parse_lsof_listeners(output);
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].port, 8443);
    }

    #[test]
    fn parse_empty_output() {
        assert!(parse_lsof_listeners("").is_empty());
    }

    #[test]
    fn check_ports_filters_to_requested() {
        // Smoke test: requesting an unused port returns empty result.
        let conflicts = check_ports(&[59998, 59999]);
        assert!(conflicts.is_empty());
    }
}
