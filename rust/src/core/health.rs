/// Check if a process is alive using platform-specific APIs
#[cfg(target_os = "linux")]
pub fn is_process_alive(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{}/stat", pid)).exists()
}

#[cfg(target_os = "macos")]
pub fn is_process_alive(pid: i32) -> bool {
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

        result > 0
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn is_process_alive(pid: i32) -> bool {
    // Fallback: use kill(pid, 0)
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    kill(Pid::from_raw(pid), Signal::SIGCONT).is_ok() || kill(Pid::from_raw(pid), None).is_ok()
}
