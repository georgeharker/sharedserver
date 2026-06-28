# Shell Wrapper Design Discussion

## The Problem

Enable third-party tools to launch servers through sharedserver's refcounting system while maintaining:
1. **PID transparency**: Calling process gets the real server PID
2. **stdio transparency**: stdin/stdout/stderr connect directly to server
3. **Refcount integrity**: Automatic increment/decrement with proper cleanup
4. **Efficiency**: Minimal overhead, no tight polling loops

## The Solution: Fork-Watch-Exec Pattern

```
Initial state:
  Process A calls: fork() + exec("sharedserver-wrapper server args")
  
Wrapper execution (PID 1234):
  1. Acquire lock on lockfile
  2. Read lockfile:
     - If exists: increment refcount
     - If new: create with refcount=1, pid=1234
  3. Fork watcher subprocess (PID 1235)
     - Watcher detaches with setsid()
     - Watcher waits for PID 1234 to exit
     - Watcher decrements refcount on exit
  4. Update lockfile with watcher_pid=1235
  5. Release lock
  6. exec("server", "args")  // PID 1234 becomes server
  
Final state:
  - Process A has PID 1234 (the server, not wrapper)
  - stdio(Process A) ⟷ stdio(PID 1234 = server)
  - Watcher (PID 1235) monitors PID 1234
```

## Key Design Decisions

### 1. Why exec() instead of fork()?

**Rejected approach**: Fork server as child, wrapper waits
```bash
server_command &
SERVER_PID=$!
wait $SERVER_PID
```

**Problems**:
- Process A gets wrapper PID, not server PID
- stdio goes through wrapper (extra pipe/pty needed)
- Wrapper must stay alive entire time (wasted process)

**Our approach**: exec() to replace wrapper with server
- PID stays the same (1234)
- stdio file descriptors inherited directly
- Zero runtime overhead

### 2. Why separate watcher process?

**Why not let wrapper watch before exec?**
- Can't watch after exec (we ARE the server)
- Can't watch before exec (race: server exits before watch starts)

**Why fork watcher?**
- Watcher can outlive wrapper (continues after exec)
- Watcher is independent of server process
- Clean separation: server=main work, watcher=cleanup

### 3. How does watcher efficiently wait?

**Rejected: Tight polling**
```bash
while kill -0 $PID; do sleep 0.1; done  # 10 checks/second = wasteful
```

**Rejected: Parent-child waitpid**
```bash
wait $PID  # Only works if PID is our child
```
But watcher and server are siblings (both children of wrapper, which is gone).

**Solution A: Exponential backoff polling**
```bash
sleep_time=0.1
while kill -0 $PID; do
    sleep $sleep_time
    sleep_time=$(awk "BEGIN {print $sleep_time * 1.5}")  # 0.1 → 0.15 → 0.22 → ... → 5.0
done
```
- Starts fast (100ms) for quick exits
- Backs off to 5s for long-running servers
- ~0.01% CPU overhead

**Solution B: C program with waitpid/kqueue/inotify**
```c
// Try waitpid first (might work if we're reparented to init)
if (waitpid(pid, &status, 0) == -1 && errno == ECHILD) {
    // Fall back to polling
    wait_polling(pid);
}
```
- Zero CPU if waitpid works
- Fallback to efficient polling if not
- Requires compilation

### 4. Lockfile race conditions

**Race 1: Concurrent wrapper starts**
```
Time 0: Wrapper A reads lockfile (doesn't exist)
Time 1: Wrapper B reads lockfile (doesn't exist)
Time 2: Wrapper A creates lockfile (refcount=1)
Time 3: Wrapper B creates lockfile (refcount=1)  ← WRONG! Should be 2
```

**Solution: flock() for atomic read-modify-write**
```bash
exec 200>"$LOCKFILE.lock"
flock -x 200  # Exclusive lock

if [ -f "$LOCKFILE" ]; then
    jq '.refcount += 1' "$LOCKFILE" > tmp && mv tmp "$LOCKFILE"
else
    echo '{"refcount":1}' > "$LOCKFILE"
fi

flock -u 200
```

**Race 2: Update lockfile after exec**
```
Time 0: Wrapper at PID 1234, lockfile has pid=1234
Time 1: Wrapper forks watcher (PID 1235)
Time 2: Wrapper tries to update lockfile with watcher_pid=1235
Time 3: Wrapper exec's into server (PID 1234 now = server)
Time 4: ⚠️  Lock file write completes... but we're already exec'd!
```

**Solution: Update before exec, hold lock across fork**
```bash
exec 200>"$LOCKFILE.lock"
flock -x 200

# Increment refcount
jq '.refcount += 1' "$LOCKFILE" > tmp && mv tmp "$LOCKFILE"

# Fork watcher (still holding lock)
fork_watcher &
WATCHER_PID=$!

# Update with watcher PID (still holding lock)
jq ".watcher_pid = $WATCHER_PID" "$LOCKFILE" > tmp && mv tmp "$LOCKFILE"

# Release lock
flock -u 200
exec 200>&-  # Close FD (happens automatically on exec anyway)

# Now safe to exec
exec server args
```

### 5. Error handling

**What if exec fails?**
```bash
fork_watcher &
WATCHER_PID=$!

exec server args

# If we're here, exec failed
# But watcher already started! Need to clean up:
kill $WATCHER_PID
decrement_refcount
```

**What if command not found?**
```bash
if ! command -v "$SERVER_COMMAND" &>/dev/null; then
    echo "Error: command not found" >&2
    # Don't increment refcount
    exit 127
fi
```

## Comparison with Alternatives

### Alternative 1: Client-Server Daemon

**Architecture**: Persistent daemon manages all servers
```
sharedserver-daemon (always running)
    ├─ Tracks refcounts
    ├─ Spawns servers
    └─ Listens on socket
    
sharedserver-client (CLI tool)
    └─ Sends attach/detach to daemon
```

**Pros**:
- No races (daemon serializes all operations)
- Could add rich features (status UI, logging)
- Multiple client types (CLI, Neovim, VS Code)

**Cons**:
- Requires daemon management (systemd/launchd)
- Not transparent (client blocks, not server)
- More complex (~1000 LOC vs ~100 LOC)

### Alternative 2: Wrapper with Config File

**Architecture**: Wrapper reads config, no Neovim integration
```
~/.config/sharedserver/config.json:
{
    "chroma": {
        "command": "chroma",
        "args": ["run", "--path", "..."]
    }
}

sharedserver-attach chroma  # Reads config, launches
```

**Pros**:
- User-friendly CLI
- Centralized configuration

**Cons**:
- Config drift (Neovim config vs shell config)
- Can't wrap arbitrary commands
- Not transparent

### Alternative 3: LD_PRELOAD Hook

**Architecture**: Preload library intercepts process start
```
LD_PRELOAD=/path/to/sharedserver-hook.so server args
    → Hook intercepts main()
    → Increments refcount
    → Calls real main()
    → Decrements on exit
```

**Pros**:
- Truly transparent (no wrapper visible)
- Zero overhead

**Cons**:
- Linux-only (no macOS)
- Requires compilation
- Fragile (depends on libc internals)
- Can't handle statically-linked binaries

## Our Choice: Fork-Watch-Exec

**Why this design wins:**

1. ✅ **Transparency**: Process gets real PID, real stdio
2. ✅ **Simplicity**: ~100 lines of bash (or +200 for C watcher)
3. ✅ **Portability**: Works on Linux/macOS/BSD
4. ✅ **Compatibility**: Works with any third-party tool
5. ✅ **Efficiency**: Zero runtime overhead, efficient watching
6. ✅ **Robustness**: flock prevents races, watcher handles cleanup

**Tradeoffs accepted:**

1. ⚠️  **Orphan watchers**: If server is SIGKILL'd, watcher stays until next poll
   - Mitigation: Watcher is tiny (~1MB), exits eventually
   
2. ⚠️  **Platform-specific**: Requires Unix (fork/exec/flock)
   - Acceptable: Neovim is primarily used on Unix
   
3. ⚠️  **Manual invocation**: User must call wrapper
   - Acceptable: Many tools support command wrappers

## Implementation Notes

### Lockfile Format

```json
{
    "pid": 1234,              // Current server PID
    "server_name": "chroma",  // Human-readable name
    "refcount": 2,            // Number of attached instances
    "started_at": 1708123456, // Unix timestamp
    "watcher_pid": 1235       // PID of watcher process (optional)
}
```

### Environment Variables

- `SHAREDSERVER_LOCKDIR`: Where to store lockfiles (default: `/tmp/sharedserver`)
- `SHAREDSERVER_DEBUG`: Enable debug output (default: `0`)

### Exit Codes

- `0`: Success (normal exit or attached to existing)
- `1`: General error (lockfile issues, etc.)
- `127`: Command not found

## Testing Strategy

### Unit Tests (test-wrapper.sh)

1. **Basic functionality**: Wrapper execs correctly
2. **Refcount increment**: Multiple instances increment
3. **Watcher cleanup**: Refcount decrements on exit
4. **Lockfile cleanup**: Last exit removes lockfile
5. **stdio passthrough**: stdin/stdout work correctly

### Integration Tests (manual)

1. **Neovim + Shell**: Start server in Neovim, attach from shell
2. **Shell + Shell**: Multiple shell instances
3. **Restart scenarios**: Kill and restart various combinations
4. **Error cases**: Invalid commands, missing permissions

### Performance Tests

1. **Startup overhead**: Measure wrapper startup time
2. **Watcher CPU**: Monitor watcher CPU usage over time
3. **Memory**: Check watcher memory footprint
4. **Scaling**: Test with 10+ concurrent instances

## Future Enhancements

### 1. Health Checks

Add port/socket checking to verify server is actually working:

```json
{
    "pid": 1234,
    "refcount": 2,
    "health_check": {
        "type": "tcp",
        "port": 8080
    }
}
```

Watcher could verify port is listening before considering server "alive".

### 2. Stale Lock Detection

Add timestamp-based cleanup:

```bash
# In watcher, before decrement:
LAST_ALIVE=$(date +%s)
while kill -0 $PID; do
    LAST_ALIVE=$(date +%s)
    sleep $INTERVAL
done

# If server disappeared without trace:
if [ $(($(date +%s) - LAST_ALIVE)) -gt 60 ]; then
    # Probably SIGKILL'd, force cleanup
    rm -f "$LOCKFILE"
fi
```

### 3. Graceful Shutdown

Support clean shutdown with timeout:

```bash
# Try SIGTERM first
kill -TERM $PID
sleep 5

# Force SIGKILL if still alive
if kill -0 $PID 2>/dev/null; then
    kill -KILL $PID
fi
```

### 4. Status Command

Add a companion tool:

```bash
sharedserver-status [server-name]
# Shows: PID, refcount, uptime, watchers
```

## Teardown & Reaping (current implementation)

> The sections above are the original design discussion. This section documents
> how teardown actually works in the Rust implementation — it is the canonical
> reference for `stop` / `stop --force` / `kill` and the watcher's role.

### Process tree

`sharedserver start` double-forks: the first child becomes the **watcher**
(`setsid`, its own session); the watcher forks the **server** (`setpgid`, its own
process group, then `exec`s the real command — e.g. `uv` → `python`). The
original `start` process returns once setup is confirmed, so the watcher is
reparented to init.

```
start ──fork──▶ watcher (setsid)  ──fork──▶ server (setpgid; exec cmd)
                    │ parent of server, reaps it, owns lockfile cleanup
```

### Lockfiles & locking

Two JSON files per server, each serving as *both* the data and its own `flock`
mutex (there is no separate lock file):

- `<name>.server.json` — the server side: `pid`, `command`, `grace_period`,
  `watcher_pid`, `started_at`, and `start_time` (a `/proc` start stamp used to
  detect PID reuse). Created at start, deleted at teardown.
- `<name>.clients.json` — the clients side: `refcount` plus a `pid → {attached_at,
  metadata}` map. Created at start and kept for the server's **whole life**.

Crucially, `clients.json` is **never deleted while the server lives**: when the
last client leaves, the file stays with an empty client map and `refcount 0`.
**Grace is refcount 0, not file absence.** This keeps the inode stable, so the
`flock` taken on the file is a real mutex — refcount changes are a single locked
read-modify-write, and `refcount` is always derived from the client-map size
(so a repeat attach from the same PID is idempotent). An earlier design deleted
`clients.json` at refcount 0 and recreated it on attach; because `flock` binds to
the inode, that delete/recreate broke mutual exclusion (two processes could lock
different inodes for the same path) and lost updates. Keeping the file fixes it.

### Single owner: the watcher

The watcher is the **sole owner** of the server lifecycle:

1. **Polls every 500 ms.** Removes dead client PIDs and re-derives the refcount;
   starts the grace timer when refcount hits 0 (the clients file persists, now
   holding an empty map); cancels it if a client re-attaches.
2. **Reaps the server** with `waitpid(WNOHANG)` — it is the server's parent, so
   it is the only process that can turn the exited server from a zombie into
   truly-gone. Without this the server would linger as a zombie (`/proc/<pid>`
   still present) until init adopted it.
3. **Deletes both lockfiles**, *pid-guarded*: it only removes them if the server
   lockfile still names the PID it was watching. This prevents a stale watcher
   (one whose server was stopped out of band) from deleting the lockfiles of a
   freshly-restarted instance that reused the same name.

On grace expiry the watcher does SIGTERM-group → wait → SIGKILL-group → reap →
delete → exit.

### Liveness is tri-state

`process_liveness(pid)` returns `Alive` / `Zombie` / `Gone` rather than a bool,
so callers can distinguish "still running" from "dead but not yet reaped" from
"fully gone". `is_process_alive()` is `== Alive`. A server lock whose PID is a
`Zombie` surfaces as the **DEFUNCT** server state (a transient, cleanup-pending
state). This is why a single misparse must never read as `Alive` — the
`/proc/<pid>/stat` parser splits on the *last* `)` because `comm` can contain
spaces and parens.

### The three stop paths

`stop` and `stop --force` are **signallers that wait for the watcher to
converge** — they never delete lockfiles themselves, which removes any
dual-deleter race:

- **`stop`** — SIGTERM the server group, then wait (up to `--timeout`, default
  10s) until the watcher has reaped the server, removed both lockfiles, and
  exited. If the server ignores SIGTERM it errors and leaves state intact.
- **`stop --force`** — same graceful path, then escalate to SIGKILL and wait
  again. On failure it reports exactly what survived (server / watcher /
  lockfile) and points at `kill`.
- **`kill`** — the **floor**, and the only command that does not depend on the
  watcher (use it when the watcher is wedged): SIGKILL the watcher first, then
  the server's process group, then delete the lockfiles itself. The orphaned
  server zombie is reaped by init.

Fallback safety net: if `stop` finds the server dead but no live watcher (it
crashed, or was never recorded), `stop` cleans up the orphaned lockfiles itself,
pid-guarded — because nothing else will. Likewise `doctor` only removes a
lockfile when the server is dead **and** there is no live watcher; otherwise it
defers to the watcher.

### Why this makes restart safe

Because `stop`/`--force` block until the old watcher has exited and the
lockfiles are gone, a restart with the same name immediately afterward starts
from a clean slate — there is no surviving watcher to delete the new instance's
lockfiles. (`kill` is restart-safe for the same reason: it leaves no watcher.)

## Conclusion

The fork-watch-exec pattern provides a **transparent, efficient, and robust** solution for integrating shell-launched servers with sharedserver's refcounting system.

Key insight: By using `exec()` to replace the wrapper process, we achieve true transparency - the calling process never knows a wrapper existed. Combined with a detached watcher for cleanup, this gives us the best of both worlds: transparency during runtime, automatic cleanup on exit.

The implementation is simple enough to maintain (~100 LOC bash) yet robust enough for production use, with optional C optimization for maximum efficiency.
