use regex::Regex;
use std::collections::HashSet;
use std::process::Command;

pub struct PortConflict {
    pub port: u16,
    pub pid: u32,
    pub process_name: String,
}

/// Extract ports from browser URLs and pane commands.
pub fn extract_ports(urls: &[String], cmds: &[String]) -> Vec<u16> {
    let mut ports = HashSet::new();

    // From browser URLs: localhost:<port> or 127.0.0.1:<port>
    let url_re = Regex::new(r"(?:localhost|127\.0\.0\.1):(\d+)").unwrap();
    for url in urls {
        for cap in url_re.captures_iter(url) {
            if let Ok(port) = cap[1].parse::<u16>() {
                ports.insert(port);
            }
        }
    }

    // From commands: --port <N>, --port=<N>, -p <N>, -p<N>
    let cmd_re = Regex::new(r"(?:--port[=\s]|(?:^|\s)-p\s?)(\d+)").unwrap();
    for cmd in cmds {
        for cap in cmd_re.captures_iter(cmd) {
            if let Ok(port) = cap[1].parse::<u16>() {
                ports.insert(port);
            }
        }
    }

    let mut result: Vec<u16> = ports.into_iter().collect();
    result.sort_unstable();
    result
}

/// Check if a port is in use. Returns conflict info if occupied.
pub fn check_port(port: u16) -> Option<PortConflict> {
    let output = Command::new("lsof")
        .args(["-i", &format!(":{port}"), "-t"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None; // port is free
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid: u32 = stdout.lines().next()?.trim().parse().ok()?;
    let process_name = get_process_name(pid);

    Some(PortConflict {
        port,
        pid,
        process_name,
    })
}

/// Get process name from PID
fn get_process_name(pid: u32) -> String {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Kill a process by PID
pub fn kill_pid(pid: u32) -> bool {
    Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
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
    fn check_unused_port_returns_none() {
        // Port 59999 is very unlikely to be in use
        assert!(check_port(59999).is_none());
    }
}
