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
    /// Opaque, platform-specific process start stamp for `pid`, captured when the
    /// server was launched. Used to detect PID reuse: if the OS recycles `pid`
    /// for an unrelated process, its start stamp won't match and we must not
    /// treat it as the server or signal it. `None` for locks written before this
    /// field existed (the identity check is then skipped, falling back to a plain
    /// liveness probe).
    #[serde(default)]
    pub start_time: Option<u64>,
    /// Same as `start_time`, but for `watcher_pid` — so a recycled watcher PID
    /// isn't mistaken for a live watcher (e.g. by `stop`'s convergence wait or
    /// `kill`). `None` on older locks.
    #[serde(default)]
    pub watcher_start_time: Option<u64>,
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

impl Default for ClientsLock {
    fn default() -> Self {
        Self::new()
    }
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

/// Get path to the per-server launch lock (see [`LaunchLock`]).
pub fn launch_lockfile_path(name: &str) -> Result<PathBuf> {
    Ok(ensure_lockfile_dir()?.join(format!("{}.launch.lock", name)))
}

/// Exclusive per-server lock serializing the start-or-attach decision.
///
/// Without it, `use` reads the state and writes the lockfiles with nothing held
/// in between, so concurrent `use` calls on a Stopped server could each start
/// their own instance (last writer wins the lockfiles, the loser can't bind its
/// port) and lose references. `use` and `admin start` hold this from before the
/// state check until they have attached, or until the instance they started has
/// published its PIDs; others block, then see Active and attach.
///
/// Lock order: the launch lock is OUTERMOST. The `server.json` / `clients.json`
/// locks are only ever taken briefly (one read or write) inside it, and nothing
/// takes the launch lock while holding either of them. The watcher, `unuse`,
/// `stop`, `kill` and teardown never take it. `admin incref` / `admin decref`
/// are break-glass tools and deliberately do NOT take it: they act on the
/// refcount directly, so they can still race a concurrent start.
///
/// Lifetime: the file exists only while a launcher holds or awaits the lock.
/// The holder deletes it just before releasing (on drop). That is only safe
/// together with the re-check in [`LaunchLock::acquire`]: a flock belongs to the
/// file, not the name, so a waiter that was queued on the deleted file wakes
/// holding a lock nobody else can see, while a newcomer creates and locks a
/// fresh file. The re-check notices that the name no longer refers to the file
/// it locked, and retries on the current one.
///
/// Fork: a flock belongs to the open file description, which `fork` shares. A
/// forked child must call [`LaunchLock::close_in_child`] (close its copy, never
/// `LOCK_UN`, never delete), or the long-lived watcher would hold it forever
/// (and an unlock in the child would release it for the parent too). Exec'd
/// children are covered by `O_CLOEXEC`, which Rust sets on every open.
pub struct LaunchLock {
    /// `None` once handed off by [`LaunchLock::close_in_child`].
    file: Option<File>,
    path: PathBuf,
}

impl LaunchLock {
    /// Block until this process holds the launch lock for `name`.
    pub fn acquire(name: &str) -> Result<Self> {
        Self::acquire_at(launch_lockfile_path(name)?)
    }

    fn acquire_at(path: PathBuf) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        loop {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .with_context(|| format!("Failed to open launch lock: {:?}", path))?;
            flock(file.as_raw_fd(), FlockArg::LockExclusive)
                .with_context(|| format!("Failed to acquire launch lock on: {:?}", path))?;

            // Re-check: is the file we locked still the one at `path`? If the
            // previous holder deleted it while we waited, we hold a lock on a
            // nameless file, so drop it and lock the current one instead.
            let ours = file
                .metadata()
                .with_context(|| format!("Failed to stat launch lock: {:?}", path))?;
            match std::fs::metadata(&path) {
                Ok(now) if now.dev() == ours.dev() && now.ino() == ours.ino() => {
                    return Ok(Self {
                        file: Some(file),
                        path,
                    });
                }
                // Deleted or replaced while we waited: retry.
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("Failed to stat launch lock: {:?}", path))
                }
            }
        }
    }

    /// In a forked child: close the inherited descriptor WITHOUT unlocking or
    /// deleting, so the parent keeps the lock and the child doesn't extend it.
    pub fn close_in_child(mut self) {
        drop(self.file.take());
    }
}

impl Drop for LaunchLock {
    /// Delete the file while still holding the lock, then close (which
    /// releases it). Waiters queued on this file re-check, see it's gone, and
    /// retry on whatever file the name refers to next.
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        if let Some(file) = self.file.take() {
            // Only delete the file WE locked. Under the protocol it always is,
            // but if something outside sharedserver deleted it and a newcomer
            // locked a fresh one, removing that would let a third caller in.
            let same = match (file.metadata(), std::fs::metadata(&self.path)) {
                (Ok(ours), Ok(now)) => ours.dev() == now.dev() && ours.ino() == now.ino(),
                _ => false,
            };
            if same {
                let _ = std::fs::remove_file(&self.path);
            }
            drop(file);
        }
    }
}

/// Perform read-only operation with shared lock (allows multiple concurrent readers)
pub fn with_shared_lock<F, R>(path: &Path, operation: F) -> Result<R>
where
    F: FnOnce(&mut File) -> Result<R>,
{
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("Failed to open lockfile: {:?}", path))?;

    // Acquire shared lock (multiple readers allowed simultaneously)
    flock(file.as_raw_fd(), FlockArg::LockShared)
        .with_context(|| format!("Failed to acquire shared lock on: {:?}", path))?;

    let result = operation(&mut file);

    // Lock is automatically released when file is dropped
    result
}

/// Perform operation on file with exclusive lock (single writer, no readers)
pub fn with_lock<F, R>(path: &Path, operation: F) -> Result<R>
where
    F: FnOnce(&mut File) -> Result<R>,
{
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // Never truncate: the open happens BEFORE the flock below, so truncating
        // here would wipe the state out from under whoever currently holds the lock.
        .truncate(false)
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

/// Read server lockfile with shared lock (allows concurrent reads)
pub fn read_server_lock(name: &str) -> Result<ServerLock> {
    let path = server_lockfile_path(name)?;
    with_shared_lock(&path, read_json)
}

/// Write server lockfile
pub fn write_server_lock(name: &str, lock: &ServerLock) -> Result<()> {
    let path = server_lockfile_path(name)?;
    with_lock(&path, |file| write_json(file, lock))
}

/// Read clients lockfile with shared lock (allows concurrent reads)
pub fn read_clients_lock(name: &str) -> Result<ClientsLock> {
    let path = clients_lockfile_path(name)?;
    with_shared_lock(&path, read_json)
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

/// Delete both lockfiles for `name`, but only if the server lockfile still
/// refers to `pid`.
///
/// This guards against deleting a *newer* instance's lockfiles after a restart
/// reused the name: if the server lock now names a different pid, we leave both
/// files alone. If the lock is already gone/unreadable, deletion is a safe
/// no-op. Used by both the watcher and `stop` so teardown has a single,
/// race-free cleanup path.
///
/// Note: the pid check and the unlink are not one atomic step, so there is a
/// microsecond-wide window where a brand-new instance could publish between the
/// check and the unlink and have its `server.json` removed. In practice this is
/// unreachable: `stop` waits for full teardown before returning, so restarts are
/// sequential, and the watcher only calls this once its own server is dead.
pub fn delete_locks_owned_by(name: &str, pid: i32) {
    if let Ok(lock) = read_server_lock(name) {
        if lock.pid != pid {
            return;
        }
    }
    let _ = delete_server_lock(name);
    let _ = delete_clients_lock(name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp_lock(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ss-launch-{tag}-{}.lock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Can anyone else lock the file currently at `path` right now?
    fn lockable_now(path: &Path) -> bool {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        flock(f.as_raw_fd(), FlockArg::LockExclusiveNonblock).is_ok()
    }

    #[test]
    fn holder_deletes_the_file_on_release() {
        let p = tmp_lock("drop");
        let lock = LaunchLock::acquire_at(p.clone()).unwrap();
        assert!(p.exists());
        drop(lock);
        assert!(
            !p.exists(),
            "the holder deletes the lock file before releasing"
        );
    }

    #[test]
    fn waiter_on_a_deleted_file_relocks_the_current_one() {
        // A holds; B queues on A's file; A deletes + releases. B must not keep a
        // lock on the nameless file: it has to end up holding whatever file the
        // name refers to, so a newcomer still blocks.
        let p = tmp_lock("requeue");
        let a = LaunchLock::acquire_at(p.clone()).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let pb = p.clone();
        let waiter = std::thread::spawn(move || {
            let b = LaunchLock::acquire_at(pb).unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300)); // hold while main probes
            drop(b);
        });
        std::thread::sleep(Duration::from_millis(100)); // let B block on A's file
        drop(a);
        rx.recv_timeout(Duration::from_secs(5)).expect("B acquired");
        assert!(p.exists(), "B re-created the file at the name");
        assert!(
            !lockable_now(&p),
            "B holds the file at the name: a newcomer must block"
        );
        waiter.join().unwrap();
        assert!(!p.exists(), "B deleted it on release");
    }

    #[test]
    fn close_in_child_never_deletes() {
        let p = tmp_lock("child");
        LaunchLock::acquire_at(p.clone()).unwrap().close_in_child();
        assert!(
            p.exists(),
            "a forked child's close must not delete the parent's lock file"
        );
        let _ = std::fs::remove_file(&p);
    }
}
