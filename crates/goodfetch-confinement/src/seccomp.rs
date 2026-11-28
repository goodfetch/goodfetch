//! The seccomp lists (ADR 0033, spec #596): one allowlist per network level,
//! built from the baseline every supported runtime needs (Node and Python),
//! as data. A call no rule allows is refused with `EPERM`, never by killing
//! the Plugin, so it fails visibly. Each rule has a name, which one probe of
//! the probe table names (ADR 0067).
//!
//! Some refusals are rules too, though the default refuses them already:
//! ADR 0033 names them, and a rule of their own gives each its probe, so a
//! later allow rule that lets one through fails that probe. `none` (#813),
//! `proxy_tcp` (#833) and `direct` (#855) are built.

use goodfetch_protocol::plugin::NetworkLevel;
use serde::{Deserialize, Serialize};

/// A system call, by its Linux name. A call some architectures lack (`open`
/// on 64-bit Arm) is in the list for those that have it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Call {
    Accept,
    Accept4,
    Access,
    AddKey,
    ArchPrctl,
    Bind,
    Brk,
    Capget,
    ClockGetres,
    ClockGettime,
    ClockNanosleep,
    Clone,
    Clone3,
    Close,
    CloseRange,
    Connect,
    Dup,
    Dup2,
    Dup3,
    EpollCreate1,
    EpollCtl,
    EpollPwait,
    EpollPwait2,
    EpollWait,
    Eventfd2,
    Execve,
    Exit,
    ExitGroup,
    Faccessat,
    Faccessat2,
    Fcntl,
    Fdatasync,
    Fork,
    Fstat,
    Fstatfs,
    Fsync,
    Ftruncate,
    Futex,
    Getcwd,
    Getdents64,
    Getegid,
    Geteuid,
    Getgid,
    Getpeername,
    Getpid,
    Getppid,
    Getrandom,
    Getrusage,
    Getsockname,
    Getsockopt,
    Gettid,
    Gettimeofday,
    Getuid,
    IoUringEnter,
    IoUringRegister,
    IoUringSetup,
    Ioctl,
    Keyctl,
    Kill,
    Listen,
    Lseek,
    Lstat,
    Madvise,
    Mkdir,
    Mkdirat,
    Mmap,
    Mprotect,
    Mremap,
    Msgget,
    Munmap,
    Nanosleep,
    Newfstatat,
    Open,
    Openat,
    Pipe2,
    Poll,
    Ppoll,
    Prctl,
    Pread64,
    Prlimit64,
    ProcessVmReadv,
    ProcessVmWritev,
    Pselect6,
    Ptrace,
    Pwrite64,
    Read,
    Readlink,
    Readlinkat,
    Readv,
    Recvfrom,
    Recvmsg,
    Rename,
    Renameat,
    Renameat2,
    RequestKey,
    Rmdir,
    RtSigaction,
    RtSigprocmask,
    RtSigreturn,
    Rseq,
    SchedGetaffinity,
    SchedYield,
    Semget,
    Sendmsg,
    Sendto,
    SetRobustList,
    SetTidAddress,
    Setns,
    Setsockopt,
    Shmget,
    Shutdown,
    Sigaltstack,
    Socket,
    Socketpair,
    Stat,
    Statfs,
    Statx,
    Sysinfo,
    Tgkill,
    Uname,
    Unlink,
    Unlinkat,
    Unshare,
    Vfork,
    Wait4,
    Write,
    Writev,
}

/// A condition on one argument of a call, as seccomp compares it: the
/// argument's 64 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// The argument equals `value`.
    Equals { arg: u8, value: u64 },
    /// Every bit of `mask` is set in the argument.
    HasAll { arg: u8, mask: u64 },
    /// No bit of `mask` is set in the argument.
    HasNone { arg: u8, mask: u64 },
    /// The argument differs from `value`.
    Differs { arg: u8, value: u64 },
}

/// What a rule does with a call that matches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    /// Refused with `EPERM`.
    Refuse,
    /// Answered `ENOSYS`, as a kernel without the call would: no refusal,
    /// but the answer that makes a runtime use the older call a rule
    /// allows (`clone3`, whose flags sit in memory seccomp cannot read,
    /// for `clone`, whose flags it can).
    NotImplemented,
}

/// One rule of a seccomp list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The rule's name, which the probe table names.
    pub name: &'static str,
    pub call: Call,
    /// The condition the call's arguments meet, none for every call.
    pub when: Option<Condition>,
    pub verdict: Verdict,
}

const fn allow(name: &'static str, call: Call) -> Rule {
    Rule {
        name,
        call,
        when: None,
        verdict: Verdict::Allow,
    }
}

const fn refuse(name: &'static str, call: Call) -> Rule {
    Rule {
        name,
        call,
        when: None,
        verdict: Verdict::Refuse,
    }
}

const fn refuse_when(name: &'static str, call: Call, when: Condition) -> Rule {
    Rule {
        name,
        call,
        when: Some(when),
        verdict: Verdict::Refuse,
    }
}

const fn allow_when(name: &'static str, call: Call, when: Condition) -> Rule {
    Rule {
        name,
        call,
        when: Some(when),
        verdict: Verdict::Allow,
    }
}

/// `CLONE_THREAD`: a `clone` that makes a thread, not a process.
pub const CLONE_THREAD: u64 = 0x0001_0000;
/// `AF_UNIX`, the one family `socketpair` makes at any level; refused to
/// `socket` at `none` and `proxy_tcp`.
pub const AF_UNIX: u64 = 1;
/// `AF_INET`, IPv4 sockets allowed inside `proxy_tcp`'s network namespace.
pub const AF_INET: u64 = 2;
/// `AF_INET6`, IPv6 sockets allowed inside `proxy_tcp`'s network namespace.
pub const AF_INET6: u64 = 10;
/// `AF_NETLINK`, refused to `socket` at `proxy_tcp` (and every level).
pub const AF_NETLINK: u64 = 16;

/// The `ioctl` requests a runtime makes on its standard streams and pipes:
/// `FIONBIO`, `FIOCLEX`, `FIONCLEX`, `FIONREAD`, `TCGETS` and `TIOCGWINSZ`.
/// Any other, `TIOCSTI` among them, is refused.
pub const IOCTLS: &[(&str, u64)] = &[
    ("ioctl.fionbio", 0x5421),
    ("ioctl.fioclex", 0x5451),
    ("ioctl.fionclex", 0x5450),
    ("ioctl.fionread", 0x541b),
    ("ioctl.tcgets", 0x5401),
    ("ioctl.tiocgwinsz", 0x5413),
];

/// `TIOCSTI`, which pushes input into a terminal: refused by a rule of its
/// own, so a later `ioctl` rule that lets it through fails its probe.
pub const TIOCSTI: u64 = 0x5412;

/// The calls every supported runtime needs, at every level: memory,
/// threads, signals, files (which Landlock then limits), polling, time,
/// what a process reads about itself, and the calls on the `socketpair`
/// sockets runtimes use inside the process.
const BASELINE: &[Rule] = &[
    allow("brk", Call::Brk),
    allow("mmap", Call::Mmap),
    allow("munmap", Call::Munmap),
    allow("mprotect", Call::Mprotect),
    allow("mremap", Call::Mremap),
    allow("madvise", Call::Madvise),
    allow("futex", Call::Futex),
    allow("set_robust_list", Call::SetRobustList),
    allow("set_tid_address", Call::SetTidAddress),
    allow("rseq", Call::Rseq),
    allow_when(
        "clone.thread",
        Call::Clone,
        Condition::HasAll {
            arg: 0,
            mask: CLONE_THREAD,
        },
    ),
    Rule {
        name: "clone3",
        call: Call::Clone3,
        when: None,
        verdict: Verdict::NotImplemented,
    },
    allow("gettid", Call::Gettid),
    allow("sched_yield", Call::SchedYield),
    allow("sched_getaffinity", Call::SchedGetaffinity),
    allow("rt_sigaction", Call::RtSigaction),
    allow("rt_sigprocmask", Call::RtSigprocmask),
    allow("rt_sigreturn", Call::RtSigreturn),
    allow("sigaltstack", Call::Sigaltstack),
    // Signals to another process are Landlock's to refuse (ABI 6), or the
    // PID namespace's before it.
    allow("kill", Call::Kill),
    allow("tgkill", Call::Tgkill),
    allow("read", Call::Read),
    allow("write", Call::Write),
    allow("readv", Call::Readv),
    allow("writev", Call::Writev),
    allow("pread64", Call::Pread64),
    allow("pwrite64", Call::Pwrite64),
    allow("open", Call::Open),
    allow("openat", Call::Openat),
    allow("close", Call::Close),
    allow("close_range", Call::CloseRange),
    allow("lseek", Call::Lseek),
    allow("fstat", Call::Fstat),
    allow("stat", Call::Stat),
    allow("lstat", Call::Lstat),
    allow("newfstatat", Call::Newfstatat),
    allow("statx", Call::Statx),
    allow("statfs", Call::Statfs),
    allow("fstatfs", Call::Fstatfs),
    allow("access", Call::Access),
    allow("faccessat", Call::Faccessat),
    allow("faccessat2", Call::Faccessat2),
    allow("getdents64", Call::Getdents64),
    allow("getcwd", Call::Getcwd),
    allow("readlink", Call::Readlink),
    allow("readlinkat", Call::Readlinkat),
    allow("mkdir", Call::Mkdir),
    allow("mkdirat", Call::Mkdirat),
    allow("rmdir", Call::Rmdir),
    allow("unlink", Call::Unlink),
    allow("unlinkat", Call::Unlinkat),
    allow("rename", Call::Rename),
    allow("renameat", Call::Renameat),
    allow("renameat2", Call::Renameat2),
    allow("fsync", Call::Fsync),
    allow("fdatasync", Call::Fdatasync),
    allow("ftruncate", Call::Ftruncate),
    allow("fcntl", Call::Fcntl),
    allow("dup", Call::Dup),
    allow("dup2", Call::Dup2),
    allow("dup3", Call::Dup3),
    allow("pipe2", Call::Pipe2),
    allow("epoll_create1", Call::EpollCreate1),
    allow("epoll_ctl", Call::EpollCtl),
    allow("epoll_wait", Call::EpollWait),
    allow("epoll_pwait", Call::EpollPwait),
    allow("epoll_pwait2", Call::EpollPwait2),
    allow("eventfd2", Call::Eventfd2),
    allow("poll", Call::Poll),
    allow("ppoll", Call::Ppoll),
    allow("pselect6", Call::Pselect6),
    allow("clock_nanosleep", Call::ClockNanosleep),
    allow("nanosleep", Call::Nanosleep),
    allow("clock_gettime", Call::ClockGettime),
    allow("clock_getres", Call::ClockGetres),
    allow("gettimeofday", Call::Gettimeofday),
    allow("getpid", Call::Getpid),
    allow("getppid", Call::Getppid),
    allow("getuid", Call::Getuid),
    allow("geteuid", Call::Geteuid),
    allow("getgid", Call::Getgid),
    allow("getegid", Call::Getegid),
    allow("getrandom", Call::Getrandom),
    allow("uname", Call::Uname),
    allow("sysinfo", Call::Sysinfo),
    allow("getrusage", Call::Getrusage),
    allow("prlimit64", Call::Prlimit64),
    allow("capget", Call::Capget),
    allow("prctl", Call::Prctl),
    allow("arch_prctl", Call::ArchPrctl),
    allow("exit", Call::Exit),
    allow("exit_group", Call::ExitGroup),
    allow("wait4", Call::Wait4),
    // The launcher starts the Plugin's entrypoint once confined; Landlock
    // limits what may be started to the bundle and the runtime.
    allow("execve", Call::Execve),
    allow_when(
        "socketpair.unix",
        Call::Socketpair,
        Condition::Equals {
            arg: 0,
            value: AF_UNIX,
        },
    ),
    allow("sendto", Call::Sendto),
    allow("recvfrom", Call::Recvfrom),
    allow("sendmsg", Call::Sendmsg),
    allow("recvmsg", Call::Recvmsg),
    allow("getsockname", Call::Getsockname),
    allow("getpeername", Call::Getpeername),
    allow("getsockopt", Call::Getsockopt),
    allow("setsockopt", Call::Setsockopt),
    allow("shutdown", Call::Shutdown),
];

/// The refusals ADR 0033 names, at every level so far: `io_uring`, tracing
/// and reading other processes, the kernel keyring, namespaces, System V
/// IPC, waiting for connections, and new processes, which only a manifest
/// that declares processes may start (the applier refuses such a policy
/// until its rules are built).
const REFUSED: &[Rule] = &[
    refuse_when(
        "ioctl.tiocsti",
        Call::Ioctl,
        Condition::Equals {
            arg: 1,
            value: TIOCSTI,
        },
    ),
    refuse("io_uring_setup", Call::IoUringSetup),
    refuse("io_uring_enter", Call::IoUringEnter),
    refuse("io_uring_register", Call::IoUringRegister),
    refuse("ptrace", Call::Ptrace),
    refuse("process_vm_readv", Call::ProcessVmReadv),
    refuse("process_vm_writev", Call::ProcessVmWritev),
    refuse("keyctl", Call::Keyctl),
    refuse("add_key", Call::AddKey),
    refuse("request_key", Call::RequestKey),
    refuse("unshare", Call::Unshare),
    refuse("setns", Call::Setns),
    refuse("msgget", Call::Msgget),
    refuse("semget", Call::Semget),
    refuse("shmget", Call::Shmget),
    refuse("listen", Call::Listen),
    refuse("accept", Call::Accept),
    refuse("accept4", Call::Accept4),
    refuse("bind", Call::Bind),
    refuse_when(
        "clone.process",
        Call::Clone,
        Condition::HasNone {
            arg: 0,
            mask: CLONE_THREAD,
        },
    ),
    refuse("fork", Call::Fork),
    refuse("vfork", Call::Vfork),
    refuse_when(
        "socketpair.other",
        Call::Socketpair,
        Condition::Differs {
            arg: 0,
            value: AF_UNIX,
        },
    ),
];

/// At `none`, no socket of any family and no `connect`: the probe table holds
/// one case per family number Linux has.
const NONE: &[Rule] = &[
    refuse("connect", Call::Connect),
    refuse("socket", Call::Socket),
];

/// At `proxy_tcp` (inside a loopback-only network namespace with `plugin-shim`)
/// and `direct` (directly on the host network), outgoing IP sockets (`AF_INET`,
/// `AF_INET6`) and `connect` are allowed, while Unix-domain and netlink sockets
/// (plus `bind`/`listen`/`accept` in [`REFUSED`]) are refused (ADR 0033,
/// issues 833 and 855).
const OUTGOING_IP: &[Rule] = &[
    allow("connect", Call::Connect),
    allow_when(
        "socket.inet",
        Call::Socket,
        Condition::Equals {
            arg: 0,
            value: AF_INET,
        },
    ),
    allow_when(
        "socket.inet6",
        Call::Socket,
        Condition::Equals {
            arg: 0,
            value: AF_INET6,
        },
    ),
    refuse_when(
        "socket.unix",
        Call::Socket,
        Condition::Equals {
            arg: 0,
            value: AF_UNIX,
        },
    ),
    refuse_when(
        "socket.netlink",
        Call::Socket,
        Condition::Equals {
            arg: 0,
            value: AF_NETLINK,
        },
    ),
];

/// The rules of the seccomp list at `level`.
pub fn rules(level: NetworkLevel) -> Vec<Rule> {
    let extra = match level {
        NetworkLevel::None => NONE,
        NetworkLevel::ProxyTcp | NetworkLevel::Direct => OUTGOING_IP,
    };
    let ioctls = IOCTLS.iter().map(|&(name, request)| {
        allow_when(
            name,
            Call::Ioctl,
            Condition::Equals {
                arg: 1,
                value: request,
            },
        )
    });
    BASELINE
        .iter()
        .copied()
        .chain(ioctls)
        .chain(REFUSED.iter().copied())
        .chain(extra.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests;
