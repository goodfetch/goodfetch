//! The child processes an engine starts, such as its browsers (ADR 0024):
//! each in a process group of its own on macOS and Linux, with `SIGKILL`
//! as its parent-death signal and the huge-page state the engine itself was
//! started with on Linux, which [`give_huge_pages_as_started`] also gives
//! a Daemon its executable starts, and in a kill-on-close Job Object
//! whose only handle the engine holds on Windows. Stopping the engine's
//! [`Children`] kills every group it started, or its Job Object; the
//! engine's own death kills them on Linux and Windows through the kernel,
//! and on macOS an Attached engine whose Client died stops first, through
//! its parent watch, and kills them on its way out. Every child the engine
//! starts goes through here: Clippy refuses `tokio::process::Command::spawn`
//! elsewhere. System calls that Miri cannot run: the tests on each real OS
//! cover them (STYLE.md, `unsafe`).
#![expect(
    clippy::disallowed_methods,
    reason = "the one module that starts the engine's child processes (ADR 0024)"
)]
#![cfg_attr(
    any(target_os = "linux", windows),
    expect(
        unsafe_code,
        reason = "a child's parent-death signal, its huge-page flag and its Job Object are only set through the OS's C calls, and the flag is read from a constructor's link section (ADR 0024)"
    )
)]

use std::io;

/// Whether the process was started with transparent huge pages turned
/// off for it, read before the executable's allocator turns them off for
/// the engine itself (`governor.huge_pages`, ADR 0138), so that the
/// engine's children keep the host's state.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_types,
    reason = "a constructor's static is built at compile time, which loom's atomics cannot be; no loom model starts a child (ADR 0060)"
)]
mod huge_pages {
    use std::ffi::c_ulong;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// What `PR_GET_THP_DISABLE` answered before `main`.
    static OFF_AT_START: AtomicBool = AtomicBool::new(false);

    /// Reads the flag. The entry's section name gives it priority 100, so
    /// the loader runs it before mimalloc's constructor, of priority 101,
    /// which is what sets the flag for the engine.
    #[used]
    #[unsafe(link_section = ".init_array.00100")]
    static READ_AT_START: extern "C" fn() = {
        extern "C" fn read_at_start() {
            // SAFETY: `prctl(PR_GET_THP_DISABLE, 0, 0, 0, 0)` takes no
            // pointer and changes nothing; a kernel without the flag
            // answers -1, read as not off.
            let off = unsafe { libc::prctl(libc::PR_GET_THP_DISABLE, 0, 0, 0, 0) } == 1;
            OFF_AT_START.store(off, Ordering::Relaxed);
        }
        read_at_start
    };

    /// The value `PR_SET_THP_DISABLE` takes to give a child that state.
    pub(super) fn started_with() -> c_ulong {
        c_ulong::from(OFF_AT_START.load(Ordering::Relaxed))
    }
}

/// Gives the process `command` starts, before `exec`, the huge-page state
/// this process was started with, for a process of the engine's executable
/// that another one starts outside [`Children`], such as a Daemon: the
/// executable's allocator turned them off for the starting process
/// (`governor.huge_pages`, ADR 0138), and the started one hands its own
/// start state on to its children. Linux only; elsewhere nothing changes.
pub fn give_huge_pages_as_started(command: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        let started_with = huge_pages::started_with();
        // SAFETY: the closure runs in the child between fork and exec, or
        // in this process right before an `exec` in place, and makes one
        // async-signal-safe system call, which takes no pointer.
        unsafe {
            command.pre_exec(move || {
                // A kernel without the flag refuses the call, which leaves
                // nothing to set.
                libc::prctl(libc::PR_SET_THP_DISABLE, started_with, 0, 0, 0);
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

/// The children an engine started; dropping it kills each still alive.
#[derive(Debug)]
pub struct Children {
    #[cfg(unix)]
    groups: Vec<i32>,
    #[cfg(windows)]
    job: windows::Job,
}

impl Children {
    /// None yet; on Windows, the Job Object that will hold them.
    ///
    /// # Errors
    ///
    /// When Windows refuses the Job Object.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            groups: Vec::new(),
            #[cfg(windows)]
            job: windows::Job::new()?,
        })
    }

    /// Starts `command` as one of them.
    ///
    /// On Linux, call it from a thread that lives as long as the engine,
    /// such as an async task's: the kernel sends the parent-death signal
    /// when the thread that started the child ends.
    ///
    /// # Errors
    ///
    /// When the OS refuses to start it, or to put it in its Job Object.
    pub fn spawn(
        &mut self,
        mut command: tokio::process::Command,
    ) -> io::Result<tokio::process::Child> {
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: the closure runs in the child between fork and exec, and
        // only makes async-signal-safe system calls.
        unsafe {
            let started_with = huge_pages::started_with();
            command.pre_exec(move || {
                // The engine's executable may have turned transparent huge
                // pages off for itself (`governor.huge_pages`), a flag a
                // child inherits through `exec`: the child gets the state
                // the engine was started with instead. A kernel without
                // the flag refuses the call, which leaves nothing to set.
                libc::prctl(libc::PR_SET_THP_DISABLE, started_with, 0, 0, 0);
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            });
        }
        let child = command.kill_on_drop(true).spawn()?;
        #[cfg(unix)]
        if let Some(id) = child.id().and_then(|id| i32::try_from(id).ok()) {
            self.groups.push(id);
        }
        #[cfg(windows)]
        self.job.assign(&child)?;
        Ok(child)
    }

    /// Kills every child it started, with every process in their groups or
    /// its Job Object.
    pub fn kill_all(&mut self) {
        #[cfg(unix)]
        for group in self.groups.drain(..) {
            // SAFETY: `killpg` takes a group ID and a signal, no pointer; a
            // group already gone answers `ESRCH`, which leaves nothing to do.
            #[expect(unsafe_code, reason = "killpg is a C call (ADR 0024)")]
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        self.job.terminate();
    }
}

impl Drop for Children {
    fn drop(&mut self) {
        self.kill_all();
    }
}

#[cfg(windows)]
mod windows {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    /// A kill-on-close Job Object whose only handle this value holds.
    #[derive(Debug)]
    pub(super) struct Job(HANDLE);

    // SAFETY: a Job Object's handle may be used from any thread.
    unsafe impl Send for Job {}
    // SAFETY: as above; each call on it is a single system call.
    unsafe impl Sync for Job {}

    impl Job {
        pub(super) fn new() -> io::Result<Self> {
            // SAFETY: no security attributes and no name: an anonymous job.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            // SAFETY: a zeroed limit information is valid; only its flags
            // are set below.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the handle is live, and the pointer and size describe
            // the local above.
            let set = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                        .expect("invariant: the structure is small"),
                )
            };
            if set == FALSE {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        pub(super) fn assign(&self, child: &tokio::process::Child) -> io::Result<()> {
            let Some(process) = child.raw_handle() else {
                return Ok(());
            };
            // SAFETY: both handles are live: the child's until it is waited
            // for, and the job's while this value lives.
            if unsafe { AssignProcessToJobObject(self.0, process) } == FALSE {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub(super) fn terminate(&self) {
            // SAFETY: the handle is live; a job whose processes all ended
            // terminates nothing.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `new` and is closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
