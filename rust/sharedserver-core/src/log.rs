use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationLog {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub command: String,
    pub args: Vec<String>,
    pub result: String,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl InvocationLog {
    pub fn success(command: &str, args: &[String], metadata: Option<serde_json::Value>) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            command: command.to_string(),
            args: args.to_vec(),
            result: "success".to_string(),
            error: None,
            metadata,
        }
    }

    pub fn error(command: &str, args: &[String], error: String) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            command: command.to_string(),
            args: args.to_vec(),
            result: "error".to_string(),
            error: Some(error),
            metadata: None,
        }
    }
}

/// Get path to invocation log
pub fn invocation_log_path(name: &str) -> Result<PathBuf> {
    let dir = crate::lockfile::ensure_lockfile_dir()?;
    Ok(dir.join(format!("{}.invocations.log", name)))
}

/// Append invocation to log
pub fn log_invocation(name: &str, log: &InvocationLog) -> Result<()> {
    let path = invocation_log_path(name)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open invocation log: {:?}", path))?;

    let json = serde_json::to_string(log)?;
    writeln!(file, "{}", json)?;

    Ok(())
}

/// Read recent invocations (last N lines)
pub fn read_recent_invocations(name: &str, count: usize) -> Result<Vec<InvocationLog>> {
    let path = invocation_log_path(name)?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read invocation log: {:?}", path))?;

    let lines: Vec<&str> = contents.lines().collect();
    let start = lines.len().saturating_sub(count);

    let mut logs = Vec::new();
    for line in &lines[start..] {
        if let Ok(log) = serde_json::from_str::<InvocationLog>(line) {
            logs.push(log);
        }
    }

    Ok(logs)
}
