/// Liveness of a process, distinguishing a still-running process from one that
/// has died but not yet been reaped by its parent.
///
/// A zombie (defunct) process keeps its `/proc/<pid>/` entry (Linux) / is still
/// reported by `proc_pidinfo` (macOS) until its parent reaps it, but it is
/// already dead and cannot be signalled. We surface that as a distinct
/// [`Liveness::Zombie`] rather than folding it into "alive" or "gone" so callers
/// can react precisely: a stop in progress treats `Zombie` as "death succeeded,
/// now waiting for the reap", while `is_process_alive` treats it as not alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Running (or sleeping, stopped, etc.) — signallable.
    Alive,
    /// Dead but not yet reaped by its parent (Linux state `Z`, macOS `SZOMB`).
    Zombie,
    /// No such process — fully gone (and reaped, if it ever existed).
    Gone,
}

/// Determine the [`Liveness`] of a process using platform-specific APIs.
#[cfg(target_os = "linux")]
pub fn process_liveness(pid: i32) -> Liveness {
    match std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
        Ok(stat) => liveness_from_proc_stat(&stat),
        Err(_) => Liveness::Gone,
    }
}

/// Decide liveness from the contents of `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
fn liveness_from_proc_stat(stat: &str) -> Liveness {
    // Layout: "<pid> (<comm>) <state> ...". `comm` may itself contain spaces
    // and parens, so split on the *last* ')' and take the next whitespace token.
    match stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next())
    {
        Some("Z") => Liveness::Zombie, // dead, just not yet reaped by its parent
        Some(_) => Liveness::Alive,
        None => Liveness::Gone, // malformed/empty: treat as gone
    }
}

/// Determine the [`Liveness`] of a process using platform-specific APIs.
#[cfg(target_os = "macos")]
pub fn process_liveness(pid: i32) -> Liveness {
    use libc::{c_int, proc_pidinfo, PROC_PIDTBSDINFO};
    use std::mem;

    unsafe {
        let mut info: libc::proc_bsdinfo = mem::zeroed();
        let size = mem::size_of::<libc::proc_bsdinfo>() as c_int;

        let result = proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        );

        liveness_from_bsd_status(result, info.pbi_status)
    }
}

/// Decide liveness from a `proc_pidinfo(PROC_PIDTBSDINFO)` call.
///
/// `result` is the byte count it returned (`<= 0` means the pid is gone or the
/// call failed); `status` is `proc_bsdinfo::pbi_status`. `libc::SZOMB`
/// ("awaiting collection by its parent") maps to [`Liveness::Zombie`].
#[cfg(target_os = "macos")]
fn liveness_from_bsd_status(result: i32, status: u32) -> Liveness {
    if result <= 0 {
        Liveness::Gone
    } else if status == libc::SZOMB {
        Liveness::Zombie
    } else {
        Liveness::Alive
    }
}

/// Determine the [`Liveness`] of a process.
///
/// Fallback for platforms without a richer API: `kill(pid, 0)` cannot
/// distinguish a zombie from a running process (it succeeds on both), so this
/// can only ever report [`Liveness::Alive`] or [`Liveness::Gone`] — a zombie is
/// reported as `Alive`.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process_liveness(pid: i32) -> Liveness {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let p = Pid::from_raw(pid);
    if kill(p, Signal::SIGCONT).is_ok() || kill(p, None).is_ok() {
        Liveness::Alive
    } else {
        Liveness::Gone
    }
}

/// Check if a process is alive (running and signallable).
///
/// A zombie ([`Liveness::Zombie`]) is **not** alive: it is already dead, just
/// not yet reaped. Callers that need to tell a zombie apart from a fully-gone
/// process should use [`process_liveness`] directly.
pub fn is_process_alive(pid: i32) -> bool {
    process_liveness(pid) == Liveness::Alive
}

/// An opaque, platform-specific stamp identifying a *specific* run of a process.
///
/// Two processes that happen to share a PID (because the OS recycled it) will
/// have different stamps, so comparing a stored stamp against the live one
/// detects PID reuse. The value is only comparable to other stamps from the
/// same platform; never interpret it as a wall-clock time.
///
/// Linux: `starttime` (field 22 of `/proc/<pid>/stat`, clock ticks since boot —
/// typically 100 Hz, so ~10 ms resolution).
/// macOS: process start time in microseconds, `pbi_start_tvsec * 1_000_000 +
/// pbi_start_tvusec` from `proc_bsdinfo`. Folding in the µsec field keeps the
/// stamp from collapsing to whole seconds, so two processes that reuse a PID
/// within the same second still get distinct stamps.
/// Other platforms: always `None` (reuse can't be detected there).
#[cfg(target_os = "linux")]
pub fn process_start_stamp(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    // After the comm field's closing ')', the remaining whitespace-separated
    // fields start at `state` (field 3). starttime is field 22, i.e. index 19
    // among those post-')' tokens (state == index 0).
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(target_os = "macos")]
pub fn process_start_stamp(pid: i32) -> Option<u64> {
    use libc::{c_int, proc_pidinfo, PROC_PIDTBSDINFO};
    use std::mem;

    unsafe {
        let mut info: libc::proc_bsdinfo = mem::zeroed();
        let size = mem::size_of::<libc::proc_bsdinfo>() as c_int;
        let result = proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        );
        if result <= 0 {
            None
        } else {
            // Combine seconds and microseconds into one opaque µsec-resolution
            // stamp. The value is only ever equality-compared (PID-reuse guard),
            // never read as wall-clock, so this re-encoding is safe and gives a
            // finer-grained identity than whole seconds alone.
            Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process_start_stamp(_pid: i32) -> Option<u64> {
    None
}

/// Like [`process_liveness`], but guards against PID reuse using a previously
/// recorded start stamp.
///
/// If `expected_stamp` is `Some` and the live process's current stamp differs,
/// the PID has been recycled by an unrelated process, so the process we care
/// about is reported as [`Liveness::Gone`] (and must not be signalled). If
/// `expected_stamp` is `None` (a legacy lock) or the current stamp can't be
/// read, this falls back to the plain liveness result.
pub fn process_liveness_checked(pid: i32, expected_stamp: Option<u64>) -> Liveness {
    let liveness = process_liveness(pid);
    match (liveness, expected_stamp) {
        (Liveness::Alive | Liveness::Zombie, Some(expected)) => match process_start_stamp(pid) {
            Some(current) if current != expected => Liveness::Gone, // PID reused
            _ => liveness,
        },
        _ => liveness,
    }
}

// Platform-specific parsing tests (the raw stat/bsd-status decoders).
#[cfg(all(test, target_os = "linux"))]
mod tests_linux {
    use super::*;

    #[test]
    fn running_process_is_alive() {
        // A real "(comm)" with a space and parens, state R (running).
        let stat = "1234 (my proc (x)) R 1 1234 1234 0 -1 4194560 0";
        assert_eq!(liveness_from_proc_stat(stat), Liveness::Alive);
    }

    #[test]
    fn zombie_process_is_zombie() {
        // State Z must be reported as Zombie even though /proc/<pid>/stat exists.
        let stat = "1234 (combiner) Z 1 1234 1234 0 -1 4194560 0";
        assert_eq!(liveness_from_proc_stat(stat), Liveness::Zombie);
    }

    #[test]
    fn malformed_stat_is_gone() {
        assert_eq!(
            liveness_from_proc_stat("garbage with no paren"),
            Liveness::Gone
        );
        assert_eq!(liveness_from_proc_stat(""), Liveness::Gone);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests_macos {
    use super::*;

    #[test]
    fn running_process_is_alive() {
        // result > 0 (bytes written) and a live status (SRUN == 2).
        assert_eq!(liveness_from_bsd_status(648, 2), Liveness::Alive);
    }

    #[test]
    fn zombie_process_is_zombie() {
        assert_eq!(liveness_from_bsd_status(648, libc::SZOMB), Liveness::Zombie);
    }

    #[test]
    fn no_such_process_is_gone() {
        // proc_pidinfo returns <= 0 when the pid is gone or the call fails.
        assert_eq!(liveness_from_bsd_status(0, 2), Liveness::Gone);
        assert_eq!(liveness_from_bsd_status(-1, 2), Liveness::Gone);
    }
}

// Cross-platform behaviour — runs on every OS, so the macOS liveness/start-stamp
// implementations are exercised by `cargo test` there too.
#[cfg(test)]
mod tests_common {
    use super::*;

    #[test]
    fn self_pid_is_alive() {
        assert!(is_process_alive(std::process::id() as i32));
        assert_eq!(process_liveness(std::process::id() as i32), Liveness::Alive);
    }

    #[test]
    fn unused_pid_is_gone() {
        assert_eq!(process_liveness(0), Liveness::Gone);
        assert!(!is_process_alive(0));
    }

    #[test]
    fn start_stamp_is_readable_for_self() {
        assert!(process_start_stamp(std::process::id() as i32).is_some());
        assert_eq!(process_start_stamp(0), None);
    }

    #[test]
    fn checked_liveness_matches_real_stamp() {
        let pid = std::process::id() as i32;
        let stamp = process_start_stamp(pid);
        assert!(stamp.is_some());
        // Same pid, same stamp -> still alive.
        assert_eq!(process_liveness_checked(pid, stamp), Liveness::Alive);
    }

    #[test]
    fn checked_liveness_detects_pid_reuse() {
        // Same pid is alive, but a different recorded stamp means the original
        // process is gone and the pid was recycled.
        let pid = std::process::id() as i32;
        let real = process_start_stamp(pid).unwrap();
        let wrong = real.wrapping_add(1);
        assert_eq!(process_liveness_checked(pid, Some(wrong)), Liveness::Gone);
    }

    #[test]
    fn checked_liveness_falls_back_without_stamp() {
        // Legacy lock (no recorded stamp) -> plain liveness, no false "gone".
        let pid = std::process::id() as i32;
        assert_eq!(process_liveness_checked(pid, None), Liveness::Alive);
        assert_eq!(process_liveness_checked(0, Some(123)), Liveness::Gone);
    }
}
