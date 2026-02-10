use anyhow::{bail, Context, Result};
use nix::fcntl::{flock, FlockArg};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerLock {
    pub pid: i32,
    pub command: Vec<String>,
    pub grace_period: String,
    pub watcher_pid: Option<i32>,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub attached_at: chrono::DateTime<chrono::Utc>,
    pub metadata: Option<String>,
}

impl ClientInfo {
    pub fn new(metadata: Option<String>) -> Self {
        Self {
            attached_at: chrono::Utc::now(),
            metadata,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientsLock {
    pub refcount: u32,
    pub clients: HashMap<i32, ClientInfo>,
}

impl ClientsLock {
    pub fn new() -> Self {
        Self {
            refcount: 0,
            clients: HashMap::new(),
        }
    }
}

/// Get the lockfile directory
pub fn lockfile_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("SHAREDSERVER_LOCKDIR") {
        return Ok(PathBuf::from(dir));
    }

    if let Ok(xdg_runtime) = std::env::var("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(xdg_runtime).join("sharedserver");
        return Ok(path);
    }

    Ok(PathBuf::from("/tmp/sharedserver"))
}

/// Ensure lockfile directory exists
pub fn ensure_lockfile_dir() -> Result<PathBuf> {
    let dir = lockfile_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create lockfile directory: {:?}", dir))?;
    Ok(dir)
}

/// Get path to server lockfile
pub fn server_lockfile_path(name: &str) -> Result<PathBuf> {
    Ok(ensure_lockfile_dir()?.join(format!("{}.server.json", name)))
}

/// Get path to clients lockfile
pub fn clients_lockfile_path(name: &str) -> Result<PathBuf> {
    Ok(ensure_lockfile_dir()?.join(format!("{}.clients.json", name)))
}

/// Perform operation on file with exclusive lock
pub fn with_lock<F, R>(path: &Path, operation: F) -> Result<R>
where
    F: FnOnce(&mut File) -> Result<R>,
{
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .with_context(|| format!("Failed to open lockfile: {:?}", path))?;

    // Acquire exclusive lock
    flock(file.as_raw_fd(), FlockArg::LockExclusive)
        .with_context(|| format!("Failed to acquire lock on: {:?}", path))?;

    let result = operation(&mut file);

    // Lock is automatically released when file is dropped
    result
}

/// Read JSON from file
pub fn read_json<T>(file: &mut File) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    file.seek(SeekFrom::Start(0))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    if contents.trim().is_empty() {
        bail!("Lockfile is empty");
    }

    serde_json::from_str(&contents).context("Failed to parse JSON")
}

/// Write JSON to file (truncates)
pub fn write_json<T>(file: &mut File, data: &T) -> Result<()>
where
    T: Serialize,
{
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    let json = serde_json::to_string_pretty(data)?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Read server lockfile
pub fn read_server_lock(name: &str) -> Result<ServerLock> {
    let path = server_lockfile_path(name)?;
    with_lock(&path, |file| read_json(file))
}

/// Write server lockfile
pub fn write_server_lock(name: &str, lock: &ServerLock) -> Result<()> {
    let path = server_lockfile_path(name)?;
    with_lock(&path, |file| write_json(file, lock))
}

/// Read clients lockfile
pub fn read_clients_lock(name: &str) -> Result<ClientsLock> {
    let path = clients_lockfile_path(name)?;
    with_lock(&path, |file| read_json(file))
}

/// Write clients lockfile
pub fn write_clients_lock(name: &str, lock: &ClientsLock) -> Result<()> {
    let path = clients_lockfile_path(name)?;
    with_lock(&path, |file| write_json(file, lock))
}

/// Delete server lockfile
pub fn delete_server_lock(name: &str) -> Result<()> {
    let path = server_lockfile_path(name)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to delete server lockfile: {:?}", path))?;
    }
    Ok(())
}

/// Delete clients lockfile
pub fn delete_clients_lock(name: &str) -> Result<()> {
    let path = clients_lockfile_path(name)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to delete clients lockfile: {:?}", path))?;
    }
    Ok(())
}

/// Check if server lockfile exists
pub fn server_lock_exists(name: &str) -> bool {
    server_lockfile_path(name)
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// Check if clients lockfile exists
pub fn clients_lock_exists(name: &str) -> bool {
    clients_lockfile_path(name)
        .map(|p| p.exists())
        .unwrap_or(false)
}
