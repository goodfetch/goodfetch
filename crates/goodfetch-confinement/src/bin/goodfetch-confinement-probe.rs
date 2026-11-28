//! `goodfetch-confinement-probe` (ADR 0067): makes one probe of a probe
//! table, confined to a policy first when its request carries one, and
//! answers through its exit status alone (`linux::probe`). A test-only
//! executable: the confinement runner starts it, and no Product ships it.
//! Outside Linux it has no probe yet and fails.
//!
//! Run as `goodfetch-confinement-probe plugin-shim`, it is a Plugin's
//! launcher instead, as the Product executable's hidden mode is
//! (`goodfetch_confinement::launch`), so that a runner starts a probe
//! confined the way the engine starts a Plugin.

use std::process::ExitCode;

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, SHIM_COMMAND, shim};

fn main() -> ExitCode {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|first| first == SHIM_COMMAND)
    {
        #[expect(
            clippy::disallowed_methods,
            reason = "a launcher reads its launch from its own environment (ADR 0033)"
        )]
        let launch = std::env::var_os(LAUNCH_VARIABLE);
        return ExitCode::from(shim(launch.as_deref()).status());
    }
    #[cfg(target_os = "linux")]
    let status = linux::run();
    #[cfg(target_os = "macos")]
    let status = macos::run();
    #[cfg(windows)]
    let status = windows::run();
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let status = u8::MAX;
    ExitCode::from(status)
}

#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "a probe makes each system call raw, as a confined Plugin's runtime would"
)]
mod linux {
    use std::ffi::{CString, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use goodfetch_confinement::linux::limits::Resource;
    use goodfetch_confinement::linux::number;
    use goodfetch_confinement::linux::probe::{Fault, Probe, Report, Request};
    use goodfetch_confinement::linux::{apply_sandbox_baseline, confine_self, confine_self_with};
    use goodfetch_confinement::probes::{Access, Arg, Outcome, ProcessArg};
    use goodfetch_confinement::seccomp::Call;

    /// `landlock_create_ruleset`'s flag that asks for the ABI.
    const LANDLOCK_CREATE_RULESET_VERSION: i64 = 1;

    pub(super) fn run() -> u8 {
        report().status()
    }

    fn report() -> Report {
        let Some(text) = std::env::args().nth(1) else {
            return Report::Fault(Fault::Request);
        };
        let Ok(request) = serde_json::from_str::<Request>(&text) else {
            return Report::Fault(Fault::Request);
        };
        // The runner's id, read before a PID namespace hides it.
        let parent = i64::from(std::os::unix::process::parent_id());
        let mechanism = match &request.policy {
            None => {
                if request.sandbox_baseline
                    && let Err(error) = apply_sandbox_baseline()
                {
                    return Report::Fault(Fault::from(&error));
                }
                None
            }
            Some(policy) => match request.mechanism.map_or_else(
                || confine_self(policy),
                |kind| confine_self_with(policy, kind),
            ) {
                Ok(mechanism) => Some(mechanism),
                Err(error) => return Report::Fault(Fault::from(&error)),
            },
        };
        match (request.probe, mechanism) {
            (Probe::Call { call, args }, _) => made(call, &args, parent),
            (Probe::Path { access, path }, _) => opened(access, &path),
            (Probe::Mechanism, Some(mechanism)) => Report::Mechanism(mechanism),
            (Probe::KernelAbi, None) => kernel_abi(),
            (Probe::UserNamespace, None) => user_namespace(),
            (Probe::Raise { resource, to }, _) => raised(resource, to),
            (Probe::Capabilities, _) => capabilities(),
            (Probe::Mechanism, None) | (Probe::KernelAbi | Probe::UserNamespace, Some(_)) => {
                Report::Fault(Fault::Request)
            }
        }
    }

    /// The report of `call` made with `args`, `parent` standing for the
    /// runner.
    fn made(call: Call, args: &[Arg], parent: i64) -> Report {
        if call == Call::RtSigreturn {
            return signal_returned();
        }
        #[cfg(target_arch = "x86_64")]
        if call == Call::Vfork {
            return forked(vfork());
        }
        let Some(number) = number(call) else {
            return Report::Fault(Fault::NoSuchCall);
        };
        let mut values = [0_i64; 6];
        for (value, arg) in values.iter_mut().zip(args) {
            *value = match arg {
                Arg::Number(number) => *number,
                Arg::Process(ProcessArg::Own) => i64::from(std::process::id()),
                Arg::Process(ProcessArg::Parent) => parent,
            };
        }
        // SAFETY: a probe's arguments are numbers, null pointers, lengths of
        // 0 and descriptors of -1, so the kernel writes no memory of this
        // process through them; the calls that end it (`exit`) or start a
        // child (`fork`, `clone`) are handled below.
        let result = unsafe {
            libc::syscall(
                number, values[0], values[1], values[2], values[3], values[4], values[5],
            )
        };
        if result == -1 {
            return last_errno();
        }
        if matches!(call, Call::Fork | Call::Clone | Call::Clone3) {
            return forked(result);
        }
        Report::Outcome(Outcome::Ok)
    }

    /// The report of a call that may have started a child: 0 in the child,
    /// which ends at once; its id in the parent, which waits for it.
    fn forked(result: i64) -> Report {
        if result < 0 {
            return Report::errno(c_int::try_from(-result).unwrap_or(c_int::MAX));
        }
        if result == 0 {
            // SAFETY: the child ends at once, running no handler and
            // flushing nothing it shares with its parent.
            unsafe { libc::_exit(0) }
        }
        let child =
            libc::pid_t::try_from(result).expect("invariant: the kernel's process ids fit pid_t");
        // SAFETY: waits for the child just started, with no status to write.
        if unsafe { libc::waitpid(child, std::ptr::null_mut(), 0) } == -1 {
            return last_errno();
        }
        Report::Outcome(Outcome::Ok)
    }

    /// `vfork`, made where its child cannot touch the stack it shares with
    /// its parent: the child ends inside the same instructions. The result
    /// is the call's, a negated error on failure.
    #[cfg(target_arch = "x86_64")]
    fn vfork() -> i64 {
        let result: i64;
        // SAFETY: the parent is suspended until the child ends, and the
        // child runs only the `exit_group` below, touching no memory; the
        // parent resumes with the call's result in `rax`.
        unsafe {
            std::arch::asm!(
                "syscall",
                "test rax, rax",
                "jnz 2f",
                "mov eax, {exit_group}",
                "xor edi, edi",
                "syscall",
                "2:",
                exit_group = const libc::SYS_exit_group,
                inlateout("rax") libc::SYS_vfork => result,
                out("rdi") _,
                out("rcx") _,
                out("r11") _,
                options(nostack),
            );
        }
        result
    }

    extern "C" fn on_signal(_: c_int) {}

    /// `rt_sigreturn`, which only a signal handler's return makes: the
    /// handler of a raised signal returns through it.
    fn signal_returned() -> Report {
        let handler = on_signal as extern "C" fn(c_int) as *const () as libc::sighandler_t;
        // SAFETY: the handler does nothing, so it is safe in a signal.
        let previous = unsafe { libc::signal(libc::SIGUSR1, handler) };
        if previous == libc::SIG_ERR {
            return last_errno();
        }
        // SAFETY: sends this thread the signal whose handler was just
        // installed, with `tgkill` as glibc's `raise` does: musl's makes
        // `tkill`, which the list refuses.
        let raised = unsafe {
            libc::syscall(
                libc::SYS_tgkill,
                libc::getpid(),
                libc::gettid(),
                libc::SIGUSR1,
            )
        };
        if raised == -1 {
            return last_errno();
        }
        Report::Outcome(Outcome::Ok)
    }

    /// The report of opening `path` for reading, or creating it for writing.
    fn opened(access: Access, path: &Path) -> Report {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return Report::Fault(Fault::Request);
        };
        let flags = match access {
            Access::Read => libc::O_RDONLY,
            Access::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        } | libc::O_CLOEXEC;
        let mode: c_uint = 0o600;
        // SAFETY: `path` is a string ending in NUL that outlives the call.
        let descriptor = unsafe { libc::open(path.as_ptr(), flags, mode) };
        if descriptor == -1 {
            return last_errno();
        }
        // SAFETY: closes the descriptor just opened, which nothing else holds.
        unsafe { libc::close(descriptor) };
        Report::Outcome(Outcome::Ok)
    }

    /// The kernel's Landlock ABI, 0 without Landlock, asked without the
    /// crate the applier uses.
    fn kernel_abi() -> Report {
        // SAFETY: with no attribute, a size of 0 and the version flag, the
        // kernel reads no memory and answers the ABI.
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<libc::c_void>(),
                0_usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        Report::KernelAbi(u8::try_from(abi).unwrap_or(0))
    }

    /// The outcome of entering a user and PID namespace and mapping the
    /// user there, asked without the applier's code.
    #[expect(
        clippy::disallowed_methods,
        reason = "a probe is one thread, never an async worker"
    )]
    fn user_namespace() -> Report {
        // SAFETY: `getuid` reads no memory.
        let user = unsafe { libc::getuid() };
        // SAFETY: `unshare` takes flags only.
        if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWPID) } == -1 {
            return last_errno();
        }
        match std::fs::write("/proc/self/uid_map", format!("{user} {user} 1")) {
            Ok(()) => Report::Outcome(Outcome::Ok),
            Err(error) => error
                .raw_os_error()
                .map_or(Report::Fault(Fault::ErrnoOutOfRange), Report::errno),
        }
    }

    /// The report of setting `resource`'s soft and hard rlimit to `to`.
    /// Its own mapping to the rlimit, apart from the launcher's, so that a
    /// wrong one in either shows.
    fn raised(resource: Resource, to: u64) -> Report {
        let number = i64::from(match resource {
            Resource::Memory => libc::RLIMIT_DATA,
            Resource::CpuTime => libc::RLIMIT_CPU,
            Resource::OpenFiles => libc::RLIMIT_NOFILE,
        });
        let limit = [to, to];
        // SAFETY: the kernel reads the limit pair from `limit`, which
        // outlives the call, and writes nothing, as the old pair's pointer
        // is null.
        let result = unsafe {
            libc::syscall(
                libc::SYS_prlimit64,
                0_i64,
                number,
                limit.as_ptr(),
                std::ptr::null_mut::<u64>(),
            )
        };
        if result == -1 {
            return last_errno();
        }
        Report::Outcome(Outcome::Ok)
    }

    /// Probes that all effective, permitted and inheritable capabilities are
    /// zero (via raw `capget`) and `PR_GET_NO_NEW_PRIVS` is 1.
    fn capabilities() -> Report {
        const CAPABILITY_VERSION_3: u32 = 0x2008_0522;
        #[repr(C)]
        struct CapHeader {
            version: u32,
            pid: c_int,
        }
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct CapData {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }
        let mut header = CapHeader {
            version: CAPABILITY_VERSION_3,
            pid: 0,
        };
        let mut data = [CapData::default(); 2];
        // SAFETY: `header` and `data` match Linux's version-3 `capget` layout
        // and outlive the syscall; pid 0 queries the calling thread.
        if unsafe { libc::syscall(libc::SYS_capget, &raw mut header, data.as_mut_ptr()) } == -1 {
            return last_errno();
        }
        let any_cap = data
            .iter()
            .any(|word| word.effective != 0 || word.permitted != 0 || word.inheritable != 0);
        if any_cap {
            return Report::Outcome(Outcome::Exit(1));
        }
        // SAFETY: `prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0)` takes integer arguments only.
        let nnp = unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) };
        if nnp == -1 {
            return last_errno();
        }
        if nnp != 1 {
            return Report::Outcome(Outcome::Exit(2));
        }
        Report::Outcome(Outcome::Ok)
    }

    fn last_errno() -> Report {
        std::io::Error::last_os_error()
            .raw_os_error()
            .map_or(Report::Fault(Fault::ErrnoOutOfRange), Report::errno)
    }
}

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "macOS confinement probe invokes raw POSIX syscalls (open, kill, fork) to verify exact Seatbelt errno outcomes"
)]
mod macos {
    use std::ffi::{CString, c_int, c_uint};
    use std::net::{TcpListener, TcpStream, ToSocketAddrs as _};
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    use goodfetch_confinement::macos::probe::{Fault, Probe, Report, Request};
    use goodfetch_confinement::macos::{Mechanism, SeatbeltOperation, confine_self, is_sandboxed};
    use goodfetch_confinement::probes::{Access, Errno, Outcome};

    pub(super) fn run() -> u8 {
        report().status()
    }

    fn report() -> Report {
        let Some(text) = std::env::args().nth(1) else {
            return Report::Fault(Fault::Request);
        };
        let Ok(request) = serde_json::from_str::<Request>(&text) else {
            return Report::Fault(Fault::Request);
        };
        let parent = std::os::unix::process::parent_id();
        let mechanism = match &request.policy {
            None => None,
            Some(policy) => match confine_self(policy, request.proxy_port) {
                Ok(mechanism) => Some(mechanism),
                Err(error) => return Report::Fault(Fault::from(&error)),
            },
        };
        match (request.probe, mechanism) {
            (
                Probe::Operation {
                    operation,
                    port,
                    unix_path,
                },
                _,
            ) => run_operation(operation, port, unix_path.as_deref(), parent),
            (Probe::Path { access, path }, _) => opened(access, &path),
            (Probe::Mechanism, Some(Mechanism::Seatbelt)) => {
                if is_sandboxed(std::process::id()) {
                    Report::Mechanism(Mechanism::Seatbelt)
                } else {
                    Report::Fault(Fault::NotConfined)
                }
            }
            (Probe::Mechanism, None) => Report::Fault(Fault::Request),
            (Probe::Raise { resource, to }, _) => match resource.set_both(to) {
                Ok(()) => Report::Outcome(Outcome::Ok),
                Err(error) => error
                    .raw_os_error()
                    .map_or(Report::Fault(Fault::ErrnoOutOfRange), Report::errno),
            },
        }
    }

    fn run_operation(
        operation: SeatbeltOperation,
        port: Option<u16>,
        unix_path: Option<&Path>,
        parent: u32,
    ) -> Report {
        match operation {
            SeatbeltOperation::NetworkOutboundIp | SeatbeltOperation::NetworkOutboundProxy => {
                let Some(port) = port else {
                    return Report::Fault(Fault::Request);
                };
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(_) => Report::Outcome(Outcome::Ok),
                    Err(error) => io_errno(error),
                }
            }
            SeatbeltOperation::NetworkInbound => match TcpListener::bind(("127.0.0.1", 0)) {
                Ok(listener) => {
                    let _ = listener.set_nonblocking(true);
                    Report::Outcome(Outcome::Ok)
                }
                Err(error) => io_errno(error),
            },
            SeatbeltOperation::NetworkBind => match TcpListener::bind(("127.0.0.1", 0)) {
                Ok(_) => Report::Outcome(Outcome::Ok),
                Err(error) => io_errno(error),
            },
            SeatbeltOperation::NetworkUnixSocket => {
                let Some(path) = unix_path else {
                    return Report::Fault(Fault::Request);
                };
                match UnixStream::connect(path) {
                    Ok(_) => Report::Outcome(Outcome::Ok),
                    Err(error) => io_errno(error),
                }
            }
            SeatbeltOperation::MachLookupDns => match ("localhost", 80).to_socket_addrs() {
                Ok(mut addrs) if addrs.next().is_some() => Report::Outcome(Outcome::Ok),
                Ok(_) => Report::Outcome(Outcome::Errno(Errno::Eperm)),
                Err(_) => Report::Outcome(Outcome::Errno(Errno::Eperm)),
            },
            SeatbeltOperation::ProcessFork => {
                // SAFETY: single-threaded probe process forks and child exits immediately.
                let pid = unsafe { libc::fork() };
                if pid < 0 {
                    return last_errno();
                }
                if pid == 0 {
                    // SAFETY: child exits immediately without flushing shared buffers.
                    unsafe { libc::_exit(0) }
                }
                // SAFETY: waits for the child just started.
                if unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) } == -1 {
                    return last_errno();
                }
                Report::Outcome(Outcome::Ok)
            }
            SeatbeltOperation::SignalOutside => {
                let Ok(pid) = c_int::try_from(parent) else {
                    return Report::Fault(Fault::Request);
                };
                // SAFETY: signal 0 checks permission without delivering a signal.
                if unsafe { libc::kill(pid, 0) } == -1 {
                    last_errno()
                } else {
                    Report::Outcome(Outcome::Ok)
                }
            }
        }
    }

    fn opened(access: Access, path: &Path) -> Report {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return Report::Fault(Fault::Request);
        };
        let flags = match access {
            Access::Read => libc::O_RDONLY,
            Access::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        } | libc::O_CLOEXEC;
        let mode: c_uint = 0o600;
        // SAFETY: `path` is a valid NUL-terminated C string outliving the call.
        let fd = unsafe { libc::open(path.as_ptr(), flags, mode) };
        if fd == -1 {
            return last_errno();
        }
        // SAFETY: closes the descriptor just opened.
        unsafe { libc::close(fd) };
        Report::Outcome(Outcome::Ok)
    }

    fn io_errno(error: std::io::Error) -> Report {
        error
            .raw_os_error()
            .map_or(Report::Fault(Fault::ErrnoOutOfRange), Report::errno)
    }

    fn last_errno() -> Report {
        std::io::Error::last_os_error()
            .raw_os_error()
            .map_or(Report::Fault(Fault::ErrnoOutOfRange), Report::errno)
    }
}

#[cfg(windows)]
#[expect(
    unsafe_code,
    reason = "Windows confinement probe invokes Win32 OpenProcess and OpenDesktopW to verify exact AppContainer/desktop access-denied outcomes"
)]
#[expect(
    clippy::disallowed_types,
    reason = "standalone probe executable tests real OS TCP bind/connect enforcement inside confinement"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "standalone probe executable performs direct synchronous file, process and sleep syscalls inside confinement"
)]
mod windows {
    use std::ffi::OsStr;
    use std::fs::OpenOptions;
    use std::net::{TcpListener, TcpStream};
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use goodfetch_confinement::probes::{Access, Outcome};
    use goodfetch_confinement::windows::probe::{Fault, Probe, Report, Request};
    use goodfetch_confinement::windows::{Mechanism, WindowsOperation, is_appcontainer};
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_READOBJECTS, OpenDesktopW,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE};

    pub(super) fn run() -> u8 {
        report().status()
    }

    fn report() -> Report {
        let Some(text) = std::env::args().nth(1) else {
            return Report::Fault(Fault::Request);
        };
        let Ok(request) = serde_json::from_str::<Request>(&text) else {
            return Report::Fault(Fault::Request);
        };
        match request.probe {
            Probe::Operation { operation, port } => {
                run_operation(operation, port, request.parent_pid)
            }
            Probe::Path { access, path } => opened(access, &path),
            Probe::Mechanism => {
                if is_appcontainer() {
                    Report::Mechanism(Mechanism::AppContainer)
                } else {
                    Report::Fault(Fault::NotConfined)
                }
            }
            Probe::AllocateMemory { bytes } => {
                let mut chunks: Vec<Vec<u8>> = Vec::new();
                let mut remaining = bytes;
                while remaining > 0 {
                    let step = remaining.min(4 * 1024 * 1024);
                    let mut buf = Vec::new();
                    if buf.try_reserve_exact(step).is_err() {
                        return Report::Outcome(Outcome::Exit(2));
                    }
                    buf.resize(step, 0xA5);
                    std::hint::black_box(&buf);
                    chunks.push(buf);
                    remaining -= step;
                }
                std::hint::black_box(&chunks);
                Report::Outcome(Outcome::Ok)
            }
            Probe::BurnCpu => {
                let mut acc = 0_u64;
                loop {
                    acc = acc.wrapping_add(1);
                    std::hint::black_box(acc);
                }
            }
            Probe::SpawnChild => {
                let Ok(exe) = std::env::current_exe() else {
                    return Report::Fault(Fault::Request);
                };
                match Command::new(exe)
                    .arg(r#"{"probe":{"kind":"mechanism"}}"#)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    Ok(_) => Report::Outcome(Outcome::Ok),
                    Err(error) => io_report(&error),
                }
            }
            Probe::HoldHandles { dir, count } => {
                let mut files = Vec::new();
                for i in 0..count {
                    let path = dir.join(format!("h-{i}.tmp"));
                    match OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(&path)
                    {
                        Ok(file) => files.push(file),
                        Err(error) => return io_report(&error),
                    }
                }
                // Hold briefly so the launcher's Job Object handle monitor
                // observes the count.
                std::thread::sleep(std::time::Duration::from_millis(300));
                std::hint::black_box(&files);
                Report::Outcome(Outcome::Ok)
            }
        }
    }

    fn run_operation(
        operation: WindowsOperation,
        port: Option<u16>,
        parent_pid: Option<u32>,
    ) -> Report {
        match operation {
            WindowsOperation::NetworkOutboundIp
            | WindowsOperation::NetworkOutboundProxy
            | WindowsOperation::NetworkOutboundConfigured
            | WindowsOperation::NetworkOutboundPrivate => {
                let Some(port) = port else {
                    return Report::Fault(Fault::Request);
                };
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(_) => Report::Outcome(Outcome::Ok),
                    Err(error) => io_report(&error),
                }
            }
            WindowsOperation::NetworkInbound => match TcpListener::bind(("127.0.0.1", 0)) {
                Ok(listener) => {
                    let _ = listener.set_nonblocking(true);
                    Report::Outcome(Outcome::Ok)
                }
                Err(error) => io_report(&error),
            },
            WindowsOperation::NetworkBind => match TcpListener::bind(("127.0.0.1", 0)) {
                Ok(_) => Report::Outcome(Outcome::Ok),
                Err(error) => io_report(&error),
            },
            WindowsOperation::DesktopEscape => {
                let wide: Vec<u16> = OsStr::new("Default")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                // SAFETY: `wide` is a NUL-terminated UTF-16 string.
                let desk = unsafe { OpenDesktopW(wide.as_ptr(), 0, FALSE, DESKTOP_READOBJECTS) };
                if desk.is_null() {
                    io_report(&std::io::Error::last_os_error())
                } else {
                    // SAFETY: closes the desktop handle just opened.
                    unsafe {
                        CloseDesktop(desk);
                    }
                    Report::Outcome(Outcome::Ok)
                }
            }
            WindowsOperation::ProcessOpenParent => {
                let Some(pid) = parent_pid else {
                    return Report::Fault(Fault::Request);
                };
                // SAFETY: checks whether we can open the unconfined parent for
                // termination (Windows equivalent of `kill(parent, 0)`).
                let handle = unsafe { OpenProcess(PROCESS_TERMINATE, FALSE, pid) };
                if handle.is_null() {
                    io_report(&std::io::Error::last_os_error())
                } else {
                    // SAFETY: closes the process handle just opened.
                    unsafe {
                        CloseHandle(handle);
                    }
                    Report::Outcome(Outcome::Ok)
                }
            }
        }
    }

    fn opened(access: Access, path: &Path) -> Report {
        let result = match access {
            Access::Read => OpenOptions::new().read(true).open(path),
            Access::Write => OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path),
        };
        match result {
            Ok(_) => Report::Outcome(Outcome::Ok),
            Err(error) => io_report(&error),
        }
    }

    fn io_report(error: &std::io::Error) -> Report {
        error
            .raw_os_error()
            .map_or(Report::Fault(Fault::ErrorOutOfRange), Report::win32_error)
    }
}
