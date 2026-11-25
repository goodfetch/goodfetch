//! The Linux applier (ADR 0033): confines the calling process, and whatever
//! it starts, to a [`Policy`] with Landlock, then the seccomp list of the
//! policy's network level.
//!
//! - Landlock grants reading the policy's reads, reading and starting its
//!   executables, and every file access under its writes, and nothing
//!   else; from ABI 6 (kernel 6.12) it also scopes signals and abstract
//!   Unix sockets to the confined processes.
//! - Before ABI 6, a user and PID namespace of its own scope signals
//!   instead (`namespaces`), and Landlock keeps the files at the kernel's
//!   ABI. Abstract Unix sockets need a socket, which `none` refuses.
//! - seccomp stacks three filters, since a filter has one action for the
//!   calls it matches: the allows (any other call `EPERM`), the refusals
//!   (`EPERM`), and the calls answered `ENOSYS`. Where filters disagree,
//!   the kernel keeps the error over the allow, and the newest filter's
//!   error over an older one's.
//!
//! Without Landlock, a mount namespace must hide the files, still to come,
//! so such a host is refused rather than confined less
//! ([`ConfineError::Unavailable`]).

mod namespaces;
pub mod probe;

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use goodfetch_protocol::plugin::NetworkLevel;
use landlock::{
    ABI, Access, AccessFs, BitFlags, CompatLevel, Compatible, LandlockStatus, PathBeneath, PathFd,
    Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope, path_beneath_rules,
};
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};
use serde::{Deserialize, Serialize};

use crate::policy::Policy;
use crate::seccomp::{self, Call, Condition, Rule, Verdict};

/// The newest Landlock ABI whose rights this applier handles: what a newer
/// kernel adds is left alone until a probe tests it.
const HANDLED_ABI: ABI = ABI::V6;

/// The Landlock ABI that scopes signals (kernel 6.12).
pub const SIGNAL_SCOPE_ABI: u8 = 6;

/// What confines a process, which a runner asserts so that a silent fallback
/// fails (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    /// Landlock at this ABI, 6 or more, with the seccomp list.
    Landlock { abi: u8 },
    /// Landlock for files at this ABI, 1 to 5 (5 on a newer kernel, whose
    /// signal scope is left unused), a user and PID namespace of its own
    /// for signals, and the seccomp list.
    PidNamespace { abi: u8 },
}

/// What keeps a confined process's signals among the confined processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalScope {
    /// Landlock's own scope, from ABI 6.
    Landlock,
    /// A user and PID namespace of its own.
    PidNamespace,
}

impl SignalScope {
    /// The scope this kernel gives: Landlock's from ABI 6, else a PID
    /// namespace.
    pub fn of_kernel() -> Self {
        if kernel_abi() >= SIGNAL_SCOPE_ABI {
            Self::Landlock
        } else {
            Self::PidNamespace
        }
    }
}

/// Why a process was not confined.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The policy's network level has no seccomp list yet (#833, #855).
    #[error("network level {0:?} has no seccomp list yet")]
    LevelNotBuilt(NetworkLevel),
    /// This host lacks what the built mechanisms need: Landlock, and, before
    /// its ABI 6, an unprivileged user namespace.
    #[error("this host cannot confine a Plugin: {0}")]
    Unavailable(&'static str),
    /// A call that sets up the namespaces failed for another reason.
    #[error("`{call}` failed while entering the namespaces: {error}")]
    Namespace {
        call: &'static str,
        #[source]
        error: io::Error,
    },
    /// A granted path cannot be opened for its Landlock rule.
    #[error("cannot grant `{path}`: {reason}")]
    Grant { path: PathBuf, reason: String },
    #[error("Landlock refused the ruleset: {0}")]
    Landlock(#[from] landlock::RulesetError),
    #[error("cannot build the seccomp filters: {0}")]
    Filter(#[from] seccompiler::BackendError),
    #[error("cannot install the seccomp filters: {0}")]
    Install(#[from] seccompiler::Error),
}

/// Confines the calling process to `policy`, then everything it starts,
/// with the signal scope this kernel gives ([`SignalScope::of_kernel`]).
pub fn confine_self(policy: &Policy) -> Result<Mechanism, ConfineError> {
    confine_self_with(policy, SignalScope::of_kernel())
}

/// Confines the calling process to `policy`, then everything it starts,
/// scoping its signals with `scope`. Call it from a launcher's one thread,
/// before it starts the Plugin: with [`SignalScope::PidNamespace`], the
/// call returns in a child, the PID namespace's first process, and the
/// caller's former self waits for it and ends as it ends. On an error, the
/// process may be confined in part and must start nothing.
pub fn confine_self_with(policy: &Policy, scope: SignalScope) -> Result<Mechanism, ConfineError> {
    let rules = seccomp::rules(policy.network_level);
    if rules.is_empty() {
        return Err(ConfineError::LevelNotBuilt(policy.network_level));
    }
    let programs = programs(&rules)?;
    if scope == SignalScope::PidNamespace {
        if kernel_abi() == 0 {
            return Err(not_enabled());
        }
        namespaces::enter()?;
    }
    let mechanism = restrict_files(policy, scope)?;
    // The allows refuse `seccomp` itself, so they go last.
    for program in &programs {
        seccompiler::apply_filter(program)?;
    }
    Ok(mechanism)
}

fn restrict_files(policy: &Policy, scope: SignalScope) -> Result<Mechanism, ConfineError> {
    let read = AccessFs::ReadFile | AccessFs::ReadDir;
    let start = AccessFs::ReadFile | AccessFs::Execute;
    let ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(HANDLED_ABI))?;
    let ruleset = match scope {
        SignalScope::Landlock => ruleset.scope(Scope::from_all(HANDLED_ABI))?,
        SignalScope::PidNamespace => ruleset,
    };
    let status = ruleset
        .create()?
        .add_rules(granted(&policy.reads, read))?
        .add_rules(granted(&policy.executables, start))?
        .add_rules(granted(&policy.writes, AccessFs::from_all(HANDLED_ABI)))?
        .restrict_self()?;
    let LandlockStatus::Available { effective_abi, .. } = status.landlock else {
        return Err(not_enabled());
    };
    let abi = abi_number(effective_abi);
    match scope {
        SignalScope::Landlock
            if abi < SIGNAL_SCOPE_ABI || status.ruleset != RulesetStatus::FullyEnforced =>
        {
            Err(ConfineError::Unavailable(
                "Landlock lacks ABI 6, which scopes signals",
            ))
        }
        SignalScope::Landlock => Ok(Mechanism::Landlock { abi }),
        // Rights an older ABI lacks are kept another way: seccomp refuses
        // `truncate` and allows `ioctl` only for its few requests, and ABI
        // 1 refuses any rename across folders.
        SignalScope::PidNamespace if status.ruleset == RulesetStatus::NotEnforced => {
            Err(not_enabled())
        }
        SignalScope::PidNamespace => Ok(Mechanism::PidNamespace {
            abi: abi.min(SIGNAL_SCOPE_ABI - 1),
        }),
    }
}

fn not_enabled() -> ConfineError {
    ConfineError::Unavailable(
        "Landlock is not enabled in this kernel, and no mount namespace is built yet",
    )
}

/// The Landlock ABI the kernel gives, 0 without Landlock: the crate asks it
/// only while it builds a ruleset, too late to choose the signal scope.
#[expect(unsafe_code, reason = "the kernel's Landlock ABI is a raw system call")]
fn kernel_abi() -> u8 {
    /// `landlock_create_ruleset`'s flag that asks for the ABI.
    const VERSION: libc::c_uint = 1;
    // SAFETY: with no attribute, a size of 0 and the version flag, the
    // kernel reads no memory and answers the ABI, or an error.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0_usize,
            VERSION,
        )
    };
    u8::try_from(abi).unwrap_or(0)
}

/// The Landlock rules granting `access` beneath each of `paths`, narrowed to
/// a file's rights for a file; an error for a path that cannot be opened,
/// which the crate's own iterator would skip.
fn granted(
    paths: &[PathBuf],
    access: BitFlags<AccessFs>,
) -> impl Iterator<Item = Result<PathBeneath<PathFd>, ConfineError>> {
    paths.iter().map(move |path| {
        path_beneath_rules([path], access)
            .next()
            .ok_or_else(|| ConfineError::Grant {
                path: path.clone(),
                reason: "it cannot be opened".to_owned(),
            })?
            .map_err(ConfineError::from)
    })
}

fn abi_number(abi: ABI) -> u8 {
    match abi {
        ABI::Unsupported => 0,
        ABI::V1 => 1,
        ABI::V2 => 2,
        ABI::V3 => 3,
        ABI::V4 => 4,
        ABI::V5 => 5,
        ABI::V6 => 6,
        ABI::V7 => 7,
        ABI::V8 => 8,
        ABI::V9 => 9,
        // ABI 10 and what the crate learns after it.
        _ => 10,
    }
}

/// The three filters of `rules`, in the order they are installed: the
/// calls answered `ENOSYS`, the refusals, then the allows. A call answered
/// `ENOSYS` is matched by the allows too, so their `EPERM` for any other
/// call does not override it.
fn programs(rules: &[Rule]) -> Result<Vec<BpfProgram>, ConfineError> {
    let arch = TargetArch::try_from(std::env::consts::ARCH)?;
    let eperm = SeccompAction::Errno(libc::EPERM.unsigned_abs());
    let enosys = SeccompAction::Errno(libc::ENOSYS.unsigned_abs());
    let filters = [
        (Verdict::NotImplemented, enosys, SeccompAction::Allow),
        (Verdict::Refuse, eperm.clone(), SeccompAction::Allow),
        (Verdict::Allow, SeccompAction::Allow, eperm),
    ];
    let mut programs = Vec::new();
    for (verdict, on_match, otherwise) in filters {
        let matched = rules.iter().filter(|rule| {
            rule.verdict == verdict
                || (verdict == Verdict::Allow && rule.verdict == Verdict::NotImplemented)
        });
        let chains = chains(matched)?;
        if chains.is_empty() {
            continue;
        }
        let filter = SeccompFilter::new(chains, otherwise, on_match, arch)?;
        programs.push(BpfProgram::try_from(filter)?);
    }
    Ok(programs)
}

/// The rules of one filter by system call number; an empty chain matches
/// every call to it. A call this architecture lacks has no rule.
fn chains<'a>(
    rules: impl Iterator<Item = &'a Rule>,
) -> Result<BTreeMap<i64, Vec<SeccompRule>>, ConfineError> {
    // `None` once an unconditional rule matches every call to the number.
    let mut conditional: BTreeMap<i64, Option<Vec<SeccompRule>>> = BTreeMap::new();
    for rule in rules {
        let Some(number) = number(rule.call) else {
            continue;
        };
        let entry = conditional
            .entry(number)
            .or_insert_with(|| Some(Vec::new()));
        match (rule.when, entry.as_mut()) {
            (None, _) => *entry = None,
            (Some(when), Some(chain)) => chain.push(SeccompRule::new(vec![condition(when)?])?),
            (Some(_), None) => {}
        }
    }
    Ok(conditional
        .into_iter()
        .map(|(number, chain)| (number, chain.unwrap_or_default()))
        .collect())
}

fn condition(when: Condition) -> Result<SeccompCondition, ConfineError> {
    let (arg, op, value) = match when {
        Condition::Equals { arg, value } => (arg, SeccompCmpOp::Eq, value),
        Condition::Differs { arg, value } => (arg, SeccompCmpOp::Ne, value),
        Condition::HasAll { arg, mask } => (arg, SeccompCmpOp::MaskedEq(mask), mask),
        Condition::HasNone { arg, mask } => (arg, SeccompCmpOp::MaskedEq(mask), 0),
    };
    Ok(SeccompCondition::new(
        arg,
        SeccompCmpArgLen::Qword,
        op,
        value,
    )?)
}

/// `number`, one exhaustive match from each call to its `libc` number: a
/// call added to [`Call`] without its number does not build.
macro_rules! numbers {
    (
        $(Call::$call:ident => libc::$number:ident,)*
        x86_64 only:
        $(Call::$x86_64_call:ident => libc::$x86_64_number:ident,)*
    ) => {
        /// `call`'s number on this architecture, none where it lacks the
        /// call (64-bit Arm has no `open`, `fork` or `stat`: their `*at`
        /// forms stand in).
        pub fn number(call: Call) -> Option<i64> {
            match call {
                $(Call::$call => Some(libc::$number),)*
                $(
                    #[cfg(target_arch = "x86_64")]
                    Call::$x86_64_call => Some(libc::$x86_64_number),
                )*
                #[cfg(not(target_arch = "x86_64"))]
                $(Call::$x86_64_call)|* => None,
            }
        }
    };
}

numbers! {
        Call::Accept => libc::SYS_accept,
        Call::Accept4 => libc::SYS_accept4,
        Call::AddKey => libc::SYS_add_key,
        Call::Bind => libc::SYS_bind,
        Call::Brk => libc::SYS_brk,
        Call::Capget => libc::SYS_capget,
        Call::ClockGetres => libc::SYS_clock_getres,
        Call::ClockGettime => libc::SYS_clock_gettime,
        Call::ClockNanosleep => libc::SYS_clock_nanosleep,
        Call::Clone => libc::SYS_clone,
        Call::Clone3 => libc::SYS_clone3,
        Call::Close => libc::SYS_close,
        Call::CloseRange => libc::SYS_close_range,
        Call::Connect => libc::SYS_connect,
        Call::Dup => libc::SYS_dup,
        Call::Dup3 => libc::SYS_dup3,
        Call::EpollCreate1 => libc::SYS_epoll_create1,
        Call::EpollCtl => libc::SYS_epoll_ctl,
        Call::EpollPwait => libc::SYS_epoll_pwait,
        Call::EpollPwait2 => libc::SYS_epoll_pwait2,
        Call::Eventfd2 => libc::SYS_eventfd2,
        Call::Execve => libc::SYS_execve,
        Call::Exit => libc::SYS_exit,
        Call::ExitGroup => libc::SYS_exit_group,
        Call::Faccessat => libc::SYS_faccessat,
        Call::Faccessat2 => libc::SYS_faccessat2,
        Call::Fcntl => libc::SYS_fcntl,
        Call::Fdatasync => libc::SYS_fdatasync,
        Call::Fstat => libc::SYS_fstat,
        Call::Fstatfs => libc::SYS_fstatfs,
        Call::Fsync => libc::SYS_fsync,
        Call::Ftruncate => libc::SYS_ftruncate,
        Call::Futex => libc::SYS_futex,
        Call::Getcwd => libc::SYS_getcwd,
        Call::Getdents64 => libc::SYS_getdents64,
        Call::Getegid => libc::SYS_getegid,
        Call::Geteuid => libc::SYS_geteuid,
        Call::Getgid => libc::SYS_getgid,
        Call::Getpeername => libc::SYS_getpeername,
        Call::Getpid => libc::SYS_getpid,
        Call::Getppid => libc::SYS_getppid,
        Call::Getrandom => libc::SYS_getrandom,
        Call::Getrusage => libc::SYS_getrusage,
        Call::Getsockname => libc::SYS_getsockname,
        Call::Getsockopt => libc::SYS_getsockopt,
        Call::Gettid => libc::SYS_gettid,
        Call::Gettimeofday => libc::SYS_gettimeofday,
        Call::Getuid => libc::SYS_getuid,
        Call::IoUringEnter => libc::SYS_io_uring_enter,
        Call::IoUringRegister => libc::SYS_io_uring_register,
        Call::IoUringSetup => libc::SYS_io_uring_setup,
        Call::Ioctl => libc::SYS_ioctl,
        Call::Keyctl => libc::SYS_keyctl,
        Call::Kill => libc::SYS_kill,
        Call::Listen => libc::SYS_listen,
        Call::Lseek => libc::SYS_lseek,
        Call::Madvise => libc::SYS_madvise,
        Call::Mkdirat => libc::SYS_mkdirat,
        Call::Mmap => libc::SYS_mmap,
        Call::Mprotect => libc::SYS_mprotect,
        Call::Mremap => libc::SYS_mremap,
        Call::Msgget => libc::SYS_msgget,
        Call::Munmap => libc::SYS_munmap,
        Call::Nanosleep => libc::SYS_nanosleep,
        Call::Newfstatat => libc::SYS_newfstatat,
        Call::Openat => libc::SYS_openat,
        Call::Pipe2 => libc::SYS_pipe2,
        Call::Ppoll => libc::SYS_ppoll,
        Call::Prctl => libc::SYS_prctl,
        Call::Pread64 => libc::SYS_pread64,
        Call::Prlimit64 => libc::SYS_prlimit64,
        Call::ProcessVmReadv => libc::SYS_process_vm_readv,
        Call::ProcessVmWritev => libc::SYS_process_vm_writev,
        Call::Pselect6 => libc::SYS_pselect6,
        Call::Ptrace => libc::SYS_ptrace,
        Call::Pwrite64 => libc::SYS_pwrite64,
        Call::Read => libc::SYS_read,
        Call::Readlinkat => libc::SYS_readlinkat,
        Call::Readv => libc::SYS_readv,
        Call::Recvfrom => libc::SYS_recvfrom,
        Call::Recvmsg => libc::SYS_recvmsg,
        Call::Renameat => libc::SYS_renameat,
        Call::Renameat2 => libc::SYS_renameat2,
        Call::RequestKey => libc::SYS_request_key,
        Call::RtSigaction => libc::SYS_rt_sigaction,
        Call::RtSigprocmask => libc::SYS_rt_sigprocmask,
        Call::RtSigreturn => libc::SYS_rt_sigreturn,
        Call::Rseq => libc::SYS_rseq,
        Call::SchedGetaffinity => libc::SYS_sched_getaffinity,
        Call::SchedYield => libc::SYS_sched_yield,
        Call::Semget => libc::SYS_semget,
        Call::Sendmsg => libc::SYS_sendmsg,
        Call::Sendto => libc::SYS_sendto,
        Call::SetRobustList => libc::SYS_set_robust_list,
        Call::SetTidAddress => libc::SYS_set_tid_address,
        Call::Setns => libc::SYS_setns,
        Call::Setsockopt => libc::SYS_setsockopt,
        Call::Shmget => libc::SYS_shmget,
        Call::Shutdown => libc::SYS_shutdown,
        Call::Sigaltstack => libc::SYS_sigaltstack,
        Call::Socket => libc::SYS_socket,
        Call::Socketpair => libc::SYS_socketpair,
        Call::Statfs => libc::SYS_statfs,
        Call::Statx => libc::SYS_statx,
        Call::Sysinfo => libc::SYS_sysinfo,
        Call::Tgkill => libc::SYS_tgkill,
        Call::Uname => libc::SYS_uname,
        Call::Unlinkat => libc::SYS_unlinkat,
        Call::Unshare => libc::SYS_unshare,
        Call::Wait4 => libc::SYS_wait4,
        Call::Write => libc::SYS_write,
        Call::Writev => libc::SYS_writev,
    x86_64 only:
        Call::Access => libc::SYS_access,
        Call::ArchPrctl => libc::SYS_arch_prctl,
        Call::Dup2 => libc::SYS_dup2,
        Call::EpollWait => libc::SYS_epoll_wait,
        Call::Fork => libc::SYS_fork,
        Call::Lstat => libc::SYS_lstat,
        Call::Mkdir => libc::SYS_mkdir,
        Call::Open => libc::SYS_open,
        Call::Poll => libc::SYS_poll,
        Call::Readlink => libc::SYS_readlink,
        Call::Rename => libc::SYS_rename,
        Call::Rmdir => libc::SYS_rmdir,
        Call::Stat => libc::SYS_stat,
        Call::Unlink => libc::SYS_unlink,
        Call::Vfork => libc::SYS_vfork,
}
