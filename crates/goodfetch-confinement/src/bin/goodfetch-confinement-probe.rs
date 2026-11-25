//! `goodfetch-confinement-probe` (ADR 0067): makes one probe of a probe
//! table, confined to a policy first when its request carries one, and
//! answers through its exit status alone (`linux::probe`). A test-only
//! executable: the confinement runner starts it, and no Product ships it.
//! Outside Linux it has no probe yet and fails.

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    let status = linux::run();
    #[cfg(not(target_os = "linux"))]
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

    use goodfetch_confinement::linux::number;
    use goodfetch_confinement::linux::probe::{Fault, Probe, Report, Request};
    use goodfetch_confinement::linux::{confine_self, confine_self_with};
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
            None => None,
            Some(policy) => match request.scope.map_or_else(
                || confine_self(policy),
                |scope| confine_self_with(policy, scope),
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

    fn last_errno() -> Report {
        std::io::Error::last_os_error()
            .raw_os_error()
            .map_or(Report::Fault(Fault::ErrnoOutOfRange), Report::errno)
    }
}
