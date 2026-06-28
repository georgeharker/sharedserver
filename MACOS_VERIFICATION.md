# macOS verification — branch `fix/zombie-process-alive`

This branch reworks teardown, refcounting, and PID-reuse handling. **All of it was
developed and tested on Linux.** The liveness and process-identity layer has
**platform-specific code that never ran on Linux**, so it needs a pass on a Mac
before merge. This file is that checklist (delete it before merging).

macOS-specific paths to validate:
- `core/health.rs` → `process_liveness` (via `proc_pidinfo(PROC_PIDTBSDINFO)`, `SZOMB` → `Zombie`)
- `core/health.rs` → `process_start_stamp` (via `proc_bsdinfo::pbi_start_tvsec`)

> Known macOS caveat: `pbi_start_tvsec` is **whole seconds**, so the PID-reuse
> guard is coarser than Linux's tick-based `starttime` — two processes that reuse
> a PID within the same second could share a stamp. It's a best-effort guard; if
> that matters we can mix in `pbi_start_tvusec`. Note it, not a blocker.

## 1. Build + automated tests

```sh
cd rust
cargo build && cargo test            # debug: runs tests_macos + tests_common (H2 logic on the bsd path)
cargo build --release && cargo test --release   # integration suite (spawns real daemons)
```

Expect everything green. The macOS-only coverage that matters:
- `tests_macos::zombie_process_is_zombie` — `SZOMB` decode.
- `tests_common::start_stamp_is_readable_for_self`, `checked_liveness_*` — exercise the real `pbi_start_tvsec` path and the reuse guard.

## 2. Manual lifecycle smoke (real daemons)

On a normal Mac terminal the daemon survives fine — no launcher trick needed.
Use an isolated lockdir so you don't touch real servers:

```sh
export SHAREDSERVER_LOCKDIR=/tmp/ss-mac-test && rm -rf "$SHAREDSERVER_LOCKDIR"
BIN="$PWD/target/release/sharedserver"
```

Checklist:

- [ ] **start/use:** `$BIN use t1 --pid $$ --grace-period 1h -- sleep 99777` → `$BIN list` shows `t1` Active, refcount 1.
- [ ] **info has stamps:** `$BIN info t1 --json` → `pid`, `watcher_pid`, **`start_time` and `watcher_start_time` non-null** (this is the macOS `pbi_start_tvsec` path working), refcount 1.
- [ ] **H1 idempotent incref:** `$BIN admin incref t1 --pid $$` then `$BIN info t1 --json` → refcount **still 1**.
- [ ] **M3 require --pid:** `$BIN admin incref t1` (no `--pid`) → fails with a usage error mentioning `--pid`.
- [ ] **H3 grace keeps file:** `$BIN admin decref t1 --pid $$` → `$BIN check t1`; `echo $?` is **1** (grace, alive); `ls "$SHAREDSERVER_LOCKDIR"/t1.clients.json` still **exists**.
- [ ] **rescue from grace:** `$BIN use t1 --pid $$ --grace-period 1h -- sleep 99777` → Active again.
- [ ] **graceful stop:** `$BIN admin stop t1 --timeout 8s` → success; lockfiles gone; `pgrep -f 'sleep 99777'` empty; `ps -o pid,stat,comm | grep -E ' Z'` shows no zombie from ours.
- [ ] **stop --force on a SIGTERM-ignorer:** `$BIN use t2 --pid $$ --grace-period 1h -- "$PWD/../tests/test_helpers/ignore_sigterm.sh"`; then `$BIN admin stop t2 --timeout 1s` → **fails** ("did not stop … --force"); then `$BIN admin stop t2 --force --timeout 5s` → **success**, locks gone.
- [ ] **kill (the floor):** start t3; `$BIN admin kill t3` → "forcefully terminated and cleaned up"; watcher + server gone.
- [ ] **M2 / restart race:** `use` t4; `$BIN admin stop --force t4`; immediately `use` t4 again; `$BIN list` shows t4 healthy (not clobbered), `t4.server.json` present.
- [ ] **M4/M1 corrupt lock:** `echo 'garbage{{{' > "$SHAREDSERVER_LOCKDIR"/x.server.json`; `$BIN check x` → exit **2** (not a crash); `$BIN admin doctor x` → succeeds and removes it.
- [ ] **L3 orphan clients sweep:** `printf '{"refcount":0,"clients":{}}' > "$SHAREDSERVER_LOCKDIR"/y.clients.json`; `$BIN admin doctor` (all) → discovers `y` and removes the stray file.

## 3. Hard to force on macOS (covered elsewhere, not blockers)

- **Defunct/zombie state:** needs a server that dies but isn't reaped within the
  500 ms watcher tick — hard to stage by hand. Covered by the `SZOMB` unit test
  and the Linux integration run.
- **Actual PID reuse:** not deterministically forceable. Covered by
  `checked_liveness_detects_pid_reuse`.

## 4. Report back

Note any failures, paste an `info --json` showing non-null `start_time`/
`watcher_start_time` on macOS, and whether the checklist passes.

## 5. Remove this file

This file is a throwaway verification aid for the branch — **delete it once the
checklist passes, before (or as part of) merging:**

```sh
git rm MACOS_VERIFICATION.md && git commit -m "Remove macOS verification note"
```

It must not survive into `main`.
