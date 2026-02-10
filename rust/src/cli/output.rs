use colored::*;
use sharedserver::core::ServerState;
use std::time::{Duration, SystemTime};

/// Print a success message with a green checkmark
pub fn print_success(msg: &str) {
    println!("{} {}", "✓".green().bold(), msg);
}

/// Print a warning message with a yellow warning symbol
pub fn print_warning(msg: &str) {
    println!("{} {}", "⚠".yellow().bold(), msg);
}

/// Print an error message with a red X
pub fn print_error(msg: &str) {
    eprintln!("{} {}", "✗".red().bold(), msg);
}

/// Print an info message with a blue info symbol
pub fn print_info(msg: &str) {
    println!("{} {}", "ℹ".blue().bold(), msg);
}

/// Format a duration in a human-readable way
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();

    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        let secs = secs % 60;
        if secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m {}s", mins, secs)
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if mins == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h {}m", hours, mins)
        }
    } else {
        let days = secs / 86400;
        let hours = (secs % 86400) / 3600;
        if hours == 0 {
            format!("{}d", days)
        } else {
            format!("{}d {}h", days, hours)
        }
    }
}

/// Format a timestamp relative to now
pub fn format_timestamp(time: SystemTime) -> String {
    match time.elapsed() {
        Ok(elapsed) => {
            let secs = elapsed.as_secs();

            if secs < 60 {
                "just now".to_string()
            } else if secs < 3600 {
                let mins = secs / 60;
                format!("{}m ago", mins)
            } else if secs < 86400 {
                let hours = secs / 3600;
                if hours == 1 {
                    "1h ago".to_string()
                } else {
                    format!("{}h ago", hours)
                }
            } else {
                let days = secs / 86400;
                if days == 1 {
                    "1 day ago".to_string()
                } else {
                    format!("{} days ago", days)
                }
            }
        }
        Err(_) => "in the future".to_string(),
    }
}

/// Format a server state with color and symbol
pub fn format_server_state(state: &ServerState) -> ColoredString {
    match state {
        ServerState::Active => "● Active".green(),
        ServerState::Grace => "⚠ Grace".yellow(),
        ServerState::Stopped => "✗ Stopped".red(),
    }
}

/// Format a PID with cyan color
pub fn format_pid(pid: i32) -> ColoredString {
    pid.to_string().cyan()
}

/// Format a server name with cyan color
pub fn format_server_name(name: &str) -> ColoredString {
    name.cyan().bold()
}

/// Format a refcount with appropriate color (red if 0, green otherwise)
pub fn format_refcount(refcount: u32) -> ColoredString {
    if refcount == 0 {
        refcount.to_string().red()
    } else {
        refcount.to_string().green()
    }
}

/// Format a list of clients
pub fn format_clients(clients: &[String], max_display: usize) -> String {
    if clients.is_empty() {
        "(none)".dimmed().to_string()
    } else if clients.len() <= max_display {
        clients.join(", ")
    } else {
        let shown: Vec<_> = clients.iter().take(max_display).cloned().collect();
        format!(
            "{}, +{} more",
            shown.join(", "),
            clients.len() - max_display
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(3660)), "1h 1m");
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
        assert_eq!(format_duration(Duration::from_secs(90000)), "1d 1h");
    }

    #[test]
    fn test_format_clients() {
        assert!(format_clients(&[], 3).contains("none"));
        assert_eq!(format_clients(&["a".to_string()], 3), "a");
        assert_eq!(
            format_clients(&["a".to_string(), "b".to_string()], 3),
            "a, b"
        );
        assert!(format_clients(
            &[
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string()
            ],
            2
        )
        .contains("+2 more"));
    }
}
