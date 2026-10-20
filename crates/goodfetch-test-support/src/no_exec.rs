//! `NoExec` (ADR 0067): a child started under a seccomp filter that refuses
//! `execve` and `execveat` with `EPERM` once its own program runs, so a test
//! proves that what the child does starts no other program, such as no
//! process spawned to compare page content. Linux only, on x86-64 and
//! 64-bit Arm.
//!
//! The filter must let the child's own program start, so the forked child
//! opens the program, installs the filter and starts it itself, with
//! `execveat` on that file descriptor. The filter allows that one call: an
//! `execveat` of that descriptor number with an empty path. The descriptor
//! closes when the program starts, so a later call of the same shape would
//! need the program to open a file onto that same number and start it.
#![expect(
    unsafe_code,
    reason = "the forked child installs a seccomp filter and starts its program with raw system calls, which Rust's standard library does not offer (ADR 0067)"
)]

use std::ffi::{CString, OsString, c_char};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::env::TestEnv;
use crate::process::Process;

/// A program to start under the filter, with its arguments, in a test's
/// folder with the test's cleared environment.
#[derive(Clone, Debug)]
pub struct NoExec {
    program: PathBuf,
    args: Vec<OsString>,
}

impl NoExec {
    /// The executable file at `program`, a path: `NoExec` searches no
    /// `PATH`, and a script, which the kernel starts through its
    /// interpreter's path, cannot start this way.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Adds an argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Runs the program under the filter until it exits, and returns its
    /// status and output.
    ///
    /// # Errors
    ///
    /// The program could not be opened, the filter could not be installed or
    /// the program could not start, as [`Command::output`] reports.
    pub fn output(&self, env: &TestEnv) -> io::Result<Output> {
        self.command(env)?.output()
    }

    /// Starts the program under the filter through [`TestEnv::spawn`], which
    /// records its process for the test's teardown.
    ///
    /// # Errors
    ///
    /// As [`NoExec::output`].
    pub fn spawn(&self, env: &TestEnv) -> io::Result<Process> {
        env.spawn(&mut self.command(env)?)
    }

    fn command(&self, env: &TestEnv) -> io::Result<Command> {
        let launch = Launch::new(&self.program, &self.args, env)?;
        let mut command = env.process_command(&self.program);
        command.args(&self.args);
        // SAFETY: the closure runs in the forked child before its program
        // starts. It allocates nothing and takes no lock: it makes system
        // calls on memory `launch` prepared before the fork and on its own
        // stack, and builds its error from the raw OS error.
        unsafe {
            command.pre_exec(move || launch.start());
        }
        Ok(command)
    }
}

/// What the forked child starts its program with, prepared before the
/// fork.
struct Launch {
    path: CString,
    /// The storage `argv` and `envp` point into.
    _strings: Vec<CString>,
    argv: Pointers,
    envp: Pointers,
}

/// A null-terminated array of pointers to C strings.
struct Pointers(Vec<*const c_char>);

// SAFETY: the pointers point into the `CString`s of the `Launch` that holds
// them, which is never changed after it is built, and only the forked child
// reads them.
unsafe impl Send for Pointers {}
// SAFETY: as for `Send`: nothing writes through the pointers.
unsafe impl Sync for Pointers {}

fn c_string(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

impl Launch {
    fn new(program: &Path, arguments: &[OsString], env: &TestEnv) -> io::Result<Self> {
        let path = c_string(program.as_os_str().as_bytes())?;
        let mut strings = vec![path.clone()];
        for arg in arguments {
            strings.push(c_string(arg.as_bytes())?);
        }
        let arg_count = strings.len();
        for (name, value) in env.env() {
            strings.push(c_string(format!("{name}={value}").as_bytes())?);
        }
        let pointers = |range: &[CString]| {
            let mut pointers: Vec<*const c_char> = range.iter().map(|text| text.as_ptr()).collect();
            pointers.push(std::ptr::null());
            Pointers(pointers)
        };
        let (argv, envp) = strings.split_at(arg_count);
        let (argv, envp) = (pointers(argv), pointers(envp));
        Ok(Self {
            path,
            _strings: strings,
            argv,
            envp,
        })
    }

    /// In the forked child: opens the program, installs the filter and
    /// starts the program. Returns only when one of them failed.
    fn start(&self) -> io::Result<()> {
        // SAFETY: `path` is a NUL-terminated string `self` owns.
        let descriptor = unsafe { libc::open(self.path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let filter = filter(descriptor.cast_unsigned());
        let program = libc::sock_fprog {
            len: FILTER_LENGTH,
            filter: filter.as_ptr().cast_mut(),
        };
        // SAFETY: `prctl` with integer arguments only.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // Integer arguments go through `syscall`'s variadic list at the width
        // of a `long`, which it reads them as.
        let no_flags: libc::c_long = 0;
        // SAFETY: `program` points at `filter`, on this frame for the call,
        // and the kernel copies it.
        let installed = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                libc::c_long::from(libc::SECCOMP_SET_MODE_FILTER),
                no_flags,
                &raw const program,
            )
        };
        if installed != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `argv` and `envp` are null-terminated arrays of pointers
        // to NUL-terminated strings `self` owns, and the path is empty.
        unsafe {
            libc::syscall(
                libc::SYS_execveat,
                libc::c_long::from(descriptor),
                c"".as_ptr(),
                self.argv.0.as_ptr(),
                self.envp.0.as_ptr(),
                libc::c_long::from(libc::AT_EMPTY_PATH),
            );
        }
        Err(io::Error::last_os_error())
    }
}

/// `AUDIT_ARCH_*` of the architecture the filter is built for, which
/// `seccomp_data.arch` holds (`linux/audit.h`).
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xC000_003E;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xC000_00B7;

/// The bit of the x32 ABI's system call numbers on x86-64; no 64-bit Arm
/// system call reaches it.
const X32_SYSCALL_BIT: u32 = 0x4000_0000;

/// Offsets into `struct seccomp_data`: `nr`, `arch`, and the low 32 bits of
/// `args[0]` and `args[4]` on a little-endian machine.
const NR: u32 = 0;
const ARCH: u32 = 4;
const ARG0_LOW: u32 = 16;
const ARG4_LOW: u32 = 16 + 4 * 8;

const FILTER_LENGTH: u16 = 13;

fn statement(code: u32, k: u32) -> libc::sock_filter {
    jump(code, k, 0, 0)
}

fn jump(code: u32, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter {
        code: u16::try_from(code).expect("invariant: a BPF opcode fits 16 bits"),
        jt,
        jf,
        k,
    }
}

/// The filter: refuse `execve`, refuse `execveat` except the launch's own
/// (`descriptor` with an empty path), and refuse every call of another ABI;
/// allow the rest. Each jump counts the instructions it skips.
fn filter(descriptor: u32) -> [libc::sock_filter; FILTER_LENGTH as usize] {
    let load = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
    let equal = libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K;
    let at_least = libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K;
    let ret = libc::BPF_RET | libc::BPF_K;
    let deny = libc::SECCOMP_RET_ERRNO | libc::EPERM.cast_unsigned();
    let execve = u32::try_from(libc::SYS_execve).expect("invariant: a system call number fits");
    let execveat = u32::try_from(libc::SYS_execveat).expect("invariant: a system call number fits");
    let empty_path = libc::AT_EMPTY_PATH.cast_unsigned();
    [
        /* 0 */ statement(load, ARCH),
        /* 1 */ jump(equal, AUDIT_ARCH, 1, 0),
        /* 2 */ statement(ret, deny),
        /* 3 */ statement(load, NR),
        /* 4 */ jump(at_least, X32_SYSCALL_BIT, 7, 0),
        /* 5 */ jump(equal, execve, 6, 0),
        /* 6 */ jump(equal, execveat, 0, 4),
        /* 7 */ statement(load, ARG0_LOW),
        /* 8 */ jump(equal, descriptor, 0, 3),
        /* 9 */ statement(load, ARG4_LOW),
        /* 10 */ jump(equal, empty_path, 0, 1),
        /* 11 */ statement(ret, libc::SECCOMP_RET_ALLOW),
        /* 12 */ statement(ret, deny),
    ]
}
