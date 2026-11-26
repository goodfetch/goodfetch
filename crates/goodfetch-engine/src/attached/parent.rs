//! The watch on an Attached engine's parent, the Client that started it
//! (ADR 0024): on Linux `PR_SET_PDEATHSIG` has the kernel kill the engine;
//! on Windows the engine joins a kill-on-close Job Object whose only handle
//! it moves into its parent, so the kernel kills it when that handle
//! closes with the parent; on macOS a kqueue `NOTE_EXIT` watch on the
//! parent wakes the engine, which stops as at the end of its stdin. System
//! calls that Miri cannot run: the multi-process tests on each real OS
//! cover them (STYLE.md, `unsafe`).
#![cfg_attr(
    any(target_os = "linux", target_os = "macos", windows),
    expect(
        unsafe_code,
        reason = "the parent's death is only seen through the OS's C calls: prctl, kqueue, Job Objects (ADR 0024)"
    )
)]

/// The watch, started once at an Attached engine's start.
pub(crate) struct ParentWatch(Watch);

enum Watch {
    /// The kernel ends the engine with its parent: nothing to wait for.
    #[cfg_attr(
        target_os = "macos",
        expect(
            dead_code,
            reason = "macOS's kernel ends no orphan: its watch wakes the engine"
        )
    )]
    ByTheKernel,
    /// The parent was already gone at the start.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(
            dead_code,
            reason = "only macOS's watch finds its parent gone at the start"
        )
    )]
    Gone,
    /// No watch could be set up; the end of stdin still ends the engine.
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos", windows)),
        expect(dead_code, reason = "every OS with a watch builds its own")
    )]
    Unwatched,
    /// A kqueue holding the parent's `NOTE_EXIT`.
    #[cfg(target_os = "macos")]
    Kqueue(std::os::fd::OwnedFd),
}

impl ParentWatch {
    /// Starts watching this process's parent.
    pub(crate) fn start() -> Self {
        Self(start())
    }

    /// Resolves once the parent is gone and the engine must stop on its
    /// own; never where the kernel kills the engine itself.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future ends only the wait.
    pub(crate) async fn gone(self) {
        match self.0 {
            Watch::Gone => {}
            Watch::ByTheKernel | Watch::Unwatched => std::future::pending().await,
            #[cfg(target_os = "macos")]
            Watch::Kqueue(kqueue) => {
                use tokio::io::Interest;
                use tokio::io::unix::AsyncFd;
                // SAFETY: an `OwnedFd` is an open descriptor that stays the
                // same until the `AsyncFd` owning it drops.
                let registered =
                    unsafe { AsyncFd::register_with_interest(kqueue, Interest::READABLE) };
                match registered {
                    // The kqueue turns readable once it holds the parent's
                    // exit, the one event it watches.
                    Ok(kqueue) => {
                        let _ = kqueue.readable().await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the parent's exit cannot be awaited; the end of stdin still stops the engine");
                        std::future::pending::<()>().await;
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn start() -> Watch {
    // SAFETY: `prctl` with `PR_SET_PDEATHSIG` takes a signal number and no
    // pointer; it changes only this process's own death signal.
    let set = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) };
    if set == 0 {
        Watch::ByTheKernel
    } else {
        let error = std::io::Error::last_os_error();
        tracing::warn!(%error, "the parent-death signal cannot be set; the end of stdin still stops the engine");
        Watch::Unwatched
    }
}

#[cfg(target_os = "macos")]
fn start() -> Watch {
    use std::os::fd::{FromRawFd as _, OwnedFd};
    // SAFETY: `getppid` takes nothing and cannot fail.
    let parent = unsafe { libc::getppid() };
    // launchd adopts an orphan: the parent that started the engine is gone.
    if parent == 1 {
        return Watch::Gone;
    }
    // SAFETY: `kqueue` takes nothing; its descriptor is checked below.
    let descriptor = unsafe { libc::kqueue() };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        tracing::warn!(%error, "no kqueue watches the parent; the end of stdin still stops the engine");
        return Watch::Unwatched;
    }
    // SAFETY: the descriptor was just opened by `kqueue` and nothing else
    // owns it.
    let kqueue = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let change = libc::kevent {
        ident: usize::try_from(parent).expect("invariant: a process ID is positive"),
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: one change read from a live local, no event list, a null
    // timeout pointer that the call accepts as none.
    let added = unsafe {
        libc::kevent(
            descriptor,
            &raw const change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if added < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Watch::Gone;
        }
        tracing::warn!(%error, "the parent's exit cannot be watched; the end of stdin still stops the engine");
        return Watch::Unwatched;
    }
    Watch::Kqueue(kqueue)
}

#[cfg(windows)]
fn start() -> Watch {
    match windows::kill_with_parent() {
        Ok(()) => Watch::ByTheKernel,
        Err(error) => {
            tracing::warn!(%error, "no Job Object ties the engine to its parent; the end of stdin still stops the engine");
            Watch::Unwatched
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn start() -> Watch {
    Watch::ByTheKernel
}

#[cfg(windows)]
mod windows {
    use std::io;

    use windows_sys::Win32::Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, FALSE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcess, PROCESS_DUP_HANDLE,
    };

    use crate::children::Job;

    /// A handle this process owns, closed when it drops.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by this process and is closed
            // once, here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Puts this process in a new kill-on-close Job Object whose one handle
    /// its parent holds, so the parent's end kills it.
    pub(super) fn kill_with_parent() -> io::Result<()> {
        let parent = parent_id()?;
        // Its handle closes as `job` drops, at the end, once the parent
        // holds its own copy.
        let job = Job::new()?;
        // SAFETY: `PROCESS_DUP_HANDLE` on a process ID, checked below.
        let opened = unsafe { OpenProcess(PROCESS_DUP_HANDLE, FALSE, parent) };
        if opened.is_null() {
            return Err(io::Error::last_os_error());
        }
        let parent = Owned(opened);
        // SAFETY: both handles are live; the pseudo-handle of this process
        // needs no closing.
        if unsafe { AssignProcessToJobObject(job.handle(), GetCurrentProcess()) } == FALSE {
            return Err(io::Error::last_os_error());
        }
        let mut theirs: HANDLE = std::ptr::null_mut();
        // SAFETY: the source handle is this process's own and live, the
        // target process handle allows duplicating into it, and `theirs`
        // is a live local the call writes once.
        let duplicated = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                job.handle(),
                parent.0,
                &raw mut theirs,
                0,
                FALSE,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if duplicated == FALSE {
            return Err(io::Error::last_os_error());
        }
        // `job` drops here: the parent's handle is the job's last.
        Ok(())
    }

    /// The ID of this process's parent, from a snapshot of the processes.
    fn parent_id() -> io::Result<u32> {
        // SAFETY: `GetCurrentProcessId` takes nothing and cannot fail.
        let own = unsafe { GetCurrentProcessId() };
        // SAFETY: a snapshot of every process, checked below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Owned(snapshot);
        // SAFETY: a zeroed entry is valid once its size is set.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize =
            u32::try_from(size_of::<PROCESSENTRY32W>()).expect("invariant: the entry is small");
        // SAFETY: the snapshot is live and `entry` a sized local.
        let mut found = unsafe { Process32FirstW(snapshot.0, &raw mut entry) };
        while found != FALSE {
            if entry.th32ProcessID == own {
                return Ok(entry.th32ParentProcessID);
            }
            // SAFETY: as above.
            found = unsafe { Process32NextW(snapshot.0, &raw mut entry) };
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "this process is not in the snapshot of processes",
        ))
    }
}
