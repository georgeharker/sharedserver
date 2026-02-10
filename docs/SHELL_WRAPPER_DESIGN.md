# Shell Wrapper Design Discussion

## The Problem

Enable third-party tools to launch servers through shareserver's refcounting system while maintaining:
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

## Conclusion

The fork-watch-exec pattern provides a **transparent, efficient, and robust** solution for integrating shell-launched servers with shareserver's refcounting system.

Key insight: By using `exec()` to replace the wrapper process, we achieve true transparency - the calling process never knows a wrapper existed. Combined with a detached watcher for cleanup, this gives us the best of both worlds: transparency during runtime, automatic cleanup on exit.

The implementation is simple enough to maintain (~100 LOC bash) yet robust enough for production use, with optional C optimization for maximum efficiency.
