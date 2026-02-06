# Shell Wrapper for sharedserver.nvim

This directory contains shell tools that allow non-Neovim processes to participate in the sharedserver.nvim refcounting system.

## Overview

The `sharedserver-wrapper` is a transparent process wrapper that:
- Increments the refcount when a server starts
- Execs into the real server (preserving PID and stdio)
- Spawns a watcher process that decrements refcount when server exits
- Handles cleanup when the last reference is removed

## Files

- **sharedserver-wrapper** - Main wrapper script (bash)
- **sharedserver-watcher.c** - Optional optimized watcher (C)
- **Makefile** - Build system for C watcher
- **test-wrapper.sh** - Test suite

## Quick Start

### Basic Usage

```bash
# Instead of running:
chroma run --path ~/.local/share/chromadb

# Run:
sharedserver-wrapper chroma chroma run --path ~/.local/share/chromadb
```

The wrapper will:
1. Create/increment a lockfile at `$LOCKDIR/chroma.lock.json`
2. Fork a watcher process
3. Execute into `chroma` (your process becomes the server)
4. When the server exits, the watcher decrements the refcount

### With Third-Party Tools

Many tools let you specify a command wrapper. For example:

```bash
# If a tool launches: server --port 8080
# Configure it to launch: sharedserver-wrapper myserver server --port 8080
```

Your tool will:
- Get the real server's PID (not the wrapper's)
- Have direct stdin/stdout/stderr to the server
- Not know the wrapper exists

## Installation

### Install wrapper script only

```bash
cd bin
make install-wrapper
```

This installs `sharedserver-wrapper` to `~/.local/bin/` (or `$PREFIX/bin/`).

### Install with optimized watcher

```bash
cd bin
make
make install
```

This compiles `sharedserver-watcher` (C program) and installs both tools.

The C watcher uses `waitpid()` for efficient blocking waits instead of polling.

## Configuration

### Environment Variables

- `SHAREDSERVER_LOCKDIR` - Directory for lockfiles (default: `$XDG_RUNTIME_DIR/sharedserver` or `/tmp/sharedserver`)
- `SHAREDSERVER_DEBUG` - Set to `1` for debug output

### Lockfile Format

```json
{
    "pid": 12345,
    "server_name": "chroma",
    "refcount": 2,
    "started_at": 1708123456,
    "watcher_pid": 12346
}
```

## How It Works

### The Fork-Watch-Exec Pattern

```
Wrapper Process (PID 1234):
  1. Increment refcount in lockfile
  2. Fork watcher child process
     └─ Watcher (PID 1235) detaches and waits for parent
  3. Update lockfile with watcher PID
  4. exec() into real server (PID 1234 becomes the server)

Result:
  - Calling process sees PID 1234 (the server)
  - stdio flows directly to PID 1234 (the server)
  - Watcher (PID 1235) waits efficiently for PID 1234 to exit
```

### Efficient Waiting

The watcher uses one of two methods:

1. **C watcher** (if compiled): Uses `waitpid()` syscall - blocks with zero CPU usage
2. **Bash watcher** (fallback): Uses `kill -0` polling with exponential backoff (100ms → 5s)

Both are much more efficient than tight polling loops.

### Race Condition Handling

The wrapper uses `flock()` for atomic lockfile operations, preventing:
- Multiple processes creating lockfiles simultaneously
- Refcount corruption from concurrent updates
- Lost decrements when multiple instances exit

## Integration with Neovim Plugin

The wrapper shares the same lockfile format as the Neovim plugin. This means:

- Neovim instances and shell processes share the same refcount
- A server started by Neovim can be attached to by shell tools (and vice versa)
- The server stays alive as long as ANY instance (Neovim or shell) is using it

### Example: Mixed Usage

```bash
# Terminal 1: Start Neovim
nvim
# Plugin starts chroma server, refcount=1

# Terminal 2: Attach from shell
sharedserver-wrapper chroma chroma run &
# Refcount becomes 2

# Terminal 1: Exit Neovim
:q
# Refcount becomes 1, server stays running

# Terminal 2: Kill server
kill %1
# Refcount becomes 0, lockfile removed
```

## Testing

Run the test suite:

```bash
cd bin
./test-wrapper.sh
```

Tests cover:
- Basic wrapper functionality (exec correctness)
- Refcount increment/decrement
- Watcher lifecycle
- Lockfile cleanup
- stdio passthrough

## Troubleshooting

### "command not found" error

The wrapper checks if the command exists before exec. If you see this error:
- Ensure the command is in your `$PATH`
- Or use an absolute path: `sharedserver-wrapper myserver /usr/local/bin/myserver`

### Orphaned lockfiles

If a process is killed with `SIGKILL`, the watcher might not clean up immediately.

To manually clean:

```bash
rm -rf $SHAREDSERVER_LOCKDIR/*.lock.json
```

Or check for stale locks:

```bash
for lock in $SHAREDSERVER_LOCKDIR/*.lock.json; do
    pid=$(jq -r .pid "$lock")
    if ! kill -0 $pid 2>/dev/null; then
        echo "Stale lock: $lock (PID $pid is dead)"
        rm "$lock"
    fi
done
```

### Debug mode

Enable debug output:

```bash
export SHAREDSERVER_DEBUG=1
sharedserver-wrapper myserver mycommand
```

This shows:
- When refcount is incremented
- Watcher PID
- Exec call
- Errors

## Advanced Usage

### Custom lockfile directory

```bash
export SHAREDSERVER_LOCKDIR=/var/run/myapp
sharedserver-wrapper ...
```

### Monitoring refcounts

```bash
# Watch refcount in real-time
watch -n1 'jq . $SHAREDSERVER_LOCKDIR/chroma.lock.json'
```

### Multiple servers

```bash
# Each server gets its own lockfile based on name
sharedserver-wrapper db1 postgres -D /data/db1 &
sharedserver-wrapper db2 postgres -D /data/db2 &
sharedserver-wrapper redis redis-server &

# Check all
ls $SHAREDSERVER_LOCKDIR/
# db1.lock.json  db2.lock.json  redis.lock.json
```

## Performance

### Wrapper Overhead

- **Startup**: ~10-50ms (lockfile operations + fork)
- **Runtime**: Zero (wrapper exec's into server)
- **Shutdown**: Zero (watcher is separate process)

### Watcher Overhead

- **C watcher**: Zero CPU (blocked in waitpid)
- **Bash watcher**: ~0.01% CPU (wakes every 0.1-5s)
- **Memory**: ~1-2 MB per watcher process

## Platform Support

- **Linux**: Fully supported
- **macOS**: Fully supported
- **BSD**: Should work (untested)
- **Windows**: Not supported (uses Unix-specific features: fork, exec, flock)

## License

Same as sharedserver.nvim (MIT)
