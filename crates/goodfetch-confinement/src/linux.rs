//! The Linux applier (ADR 0033): confines the calling process, and whatever
//! it starts, to a [`Policy`] with Landlock, then the seccomp list of the
//! policy's network level.
//!
//! - Landlock grants reading the policy's reads and the system's shared
//!   libraries, reading and starting its executables and the system's
//!   loaders, and every file access under its writes, and nothing
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
//! - Without Landlock, the namespaces add a mount namespace whose root holds
//!   only the policy's paths (`mounts`), as bubblewrap builds one.
//!
//! A host that allows none of these is refused rather than confined less
//! ([`ConfineError::Unavailable`]).

pub mod host;
pub mod limits;
mod mounts;
mod namespaces;
pub mod probe;
pub mod relay;

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

pub use crate::host::SIGNAL_SCOPE_ABI;
use crate::policy::Policy;
use crate::seccomp::{self, Call, Condition, Rule, Verdict};
pub use namespaces::probe_user_namespace;

/// The newest Landlock ABI whose rights this applier handles: what a newer
/// kernel adds is left alone until a probe tests it.
const HANDLED_ABI: ABI = ABI::V6;

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
    /// A user, PID and mount namespace of its own, whose root holds only
    /// the policy's paths, with the seccomp list.
    MountNamespace,
}

/// Which mechanism to confine with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanismKind {
    /// [`Mechanism::Landlock`].
    Landlock,
    /// [`Mechanism::PidNamespace`].
    PidNamespace,
    /// [`Mechanism::MountNamespace`].
    MountNamespace,
}

impl MechanismKind {
    /// The mechanism this kernel gives: Landlock from ABI 6, a PID
    /// namespace with an older Landlock, a mount namespace without it.
    pub fn of_kernel() -> Self {
        match kernel_abi() {
            0 => Self::MountNamespace,
            abi if abi < SIGNAL_SCOPE_ABI => Self::PidNamespace,
            _ => Self::Landlock,
        }
    }
}

/// Why a process was not confined.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The policy's network level has no seccomp list yet (#833, #855).
    #[error("network level {0:?} has no seccomp list yet")]
    LevelNotBuilt(NetworkLevel),
    /// This host lacks what the mechanism needs: Landlock at ABI 6, or an
    /// unprivileged user namespace, with Landlock for the PID namespace.
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
/// with the mechanism this kernel gives ([`MechanismKind::of_kernel`]).
pub fn confine_self(policy: &Policy) -> Result<Mechanism, ConfineError> {
    confine_self_with(policy, MechanismKind::of_kernel())
}

/// Confines the calling process to `policy`, then everything it starts,
/// with `kind`. Call it from a launcher's one thread, before it starts the
/// Plugin: with a namespace, the call returns in a child, the PID
/// namespace's first process, and the caller's former self waits for it
/// and ends as it ends. On an error, the process may be confined in part
/// and must start nothing.
pub fn confine_self_with(policy: &Policy, kind: MechanismKind) -> Result<Mechanism, ConfineError> {
    confine_self_with_relay(policy, kind, None).map(|(mechanism, _)| mechanism)
}

/// Confines the calling process to `policy` with `kind`, and when
/// `proxy_control_fd` is given at [`NetworkLevel::ProxyTcp`], starts the
/// loopback relay child inside the new network namespace before installing
/// Landlock and seccomp, returning the bound loopback port.
pub(crate) fn confine_self_with_relay(
    policy: &Policy,
    kind: MechanismKind,
    proxy_control_fd: Option<libc::c_int>,
) -> Result<(Mechanism, Option<u16>), ConfineError> {
    let rules = seccomp::rules(policy.network_level);
    if rules.is_empty() {
        return Err(ConfineError::LevelNotBuilt(policy.network_level));
    }
    let programs = programs(&rules)?;
    let isolate_network = policy.network_level == NetworkLevel::ProxyTcp;
    let bind_relay = isolate_network && proxy_control_fd.is_some();
    let (mechanism, bound) = match kind {
        MechanismKind::Landlock => {
            let bound = if isolate_network {
                namespaces::enter_network_only(bind_relay)?
            } else {
                None
            };
            (restrict_files(policy, true)?, bound)
        }
        MechanismKind::PidNamespace => {
            if kernel_abi() == 0 {
                return Err(not_enabled());
            }
            let bound = namespaces::enter(None, isolate_network, bind_relay)?;
            (restrict_files(policy, false)?, bound)
        }
        MechanismKind::MountNamespace => {
            let bound = namespaces::enter(Some(policy), isolate_network, bind_relay)?;
            (Mechanism::MountNamespace, bound)
        }
    };
    let relay_port = if let (Some(control_fd), Some((listener, port))) = (proxy_control_fd, bound) {
        relay::spawn_in_netns(control_fd, listener).map_err(|error| ConfineError::Namespace {
            call: "fork(relay)",
            error,
        })?;
        Some(port)
    } else {
        None
    };
    // The allows refuse `seccomp` itself, so they go last.
    for program in &programs {
        seccompiler::apply_filter(program)?;
    }
    Ok((mechanism, relay_port))
}

/// Drops every capability, sets `no_new_privs`, and installs
/// `goodfetch-sandbox`'s unconditional baseline seccomp filter (no further
/// namespaces, no netlink) (ADR 0033, issue 884).
pub fn apply_sandbox_baseline() -> Result<(), ConfineError> {
    namespaces::drop_capabilities()?;
    let baseline = baseline_programs(&seccomp::sandbox_baseline_rules())?;
    for program in &baseline {
        seccompiler::apply_filter(program)?;
    }
    Ok(())
}

/// Confines the calling process inside `goodfetch-sandbox`: enters namespaces,
/// drops every capability, sets `no_new_privs`, applies the helper's baseline
/// seccomp filter (no further namespaces, no netlink), then applies Landlock
/// and the caller's policy seccomp filter (ADR 0033, issue 884).
pub(crate) fn confine_for_sandbox(
    policy: &Policy,
    kind: MechanismKind,
    proxy_control_fd: Option<libc::c_int>,
) -> Result<(Mechanism, Option<u16>), ConfineError> {
    let rules = seccomp::rules(policy.network_level);
    if rules.is_empty() {
        return Err(ConfineError::LevelNotBuilt(policy.network_level));
    }
    let baseline = baseline_programs(&seccomp::sandbox_baseline_rules())?;
    let caller_programs = programs(&rules)?;
    let isolate_network = policy.network_level == NetworkLevel::ProxyTcp;
    let bind_relay = isolate_network && proxy_control_fd.is_some();
    let bound = match kind {
        MechanismKind::Landlock => {
            if isolate_network {
                namespaces::enter_network_only(bind_relay)?
            } else {
                None
            }
        }
        MechanismKind::PidNamespace => {
            if kernel_abi() == 0 {
                return Err(not_enabled());
            }
            namespaces::enter(None, isolate_network, bind_relay)?
        }
        MechanismKind::MountNamespace => {
            namespaces::enter(Some(policy), isolate_network, bind_relay)?
        }
    };
    // Always drop every capability and set `no_new_privs`, even when Landlock
    // ABI 6 ran without entering a user namespace.
    namespaces::drop_capabilities()?;
    for program in &baseline {
        seccompiler::apply_filter(program)?;
    }
    let relay_port = if let (Some(control_fd), Some((listener, port))) = (proxy_control_fd, bound) {
        relay::spawn_in_netns(control_fd, listener).map_err(|error| ConfineError::Namespace {
            call: "fork(relay)",
            error,
        })?;
        Some(port)
    } else {
        None
    };
    let mechanism = match kind {
        MechanismKind::Landlock => restrict_files(policy, true)?,
        MechanismKind::PidNamespace => restrict_files(policy, false)?,
        MechanismKind::MountNamespace => Mechanism::MountNamespace,
    };
    // The caller's allows refuse `seccomp` itself, so they go last.
    for program in &caller_programs {
        seccompiler::apply_filter(program)?;
    }
    Ok((mechanism, relay_port))
}

/// Applies every confinement layer this host offers for a first-party Plugin
/// running under Operator-approved `reduced_confinement` (ADR 0033): uses
/// Landlock when enabled (scoping signals from ABI 6), falls back to a host
/// loopback relay when a network namespace cannot be created at `proxy_tcp`,
/// and applies seccomp filters whenever the kernel accepts them.
pub(crate) fn confine_self_reduced(
    policy: &Policy,
    proxy_control_fd: Option<libc::c_int>,
) -> Result<Option<u16>, ConfineError> {
    let rules = seccomp::rules(policy.network_level);
    let programs = if rules.is_empty() {
        Vec::new()
    } else {
        programs(&rules)?
    };
    let relay_port = if policy.network_level == NetworkLevel::ProxyTcp
        && let Some(control_fd) = proxy_control_fd
    {
        let bound = match namespaces::enter_network_only(true) {
            Ok(Some(bound)) => bound,
            Ok(None) | Err(ConfineError::Unavailable(_)) => {
                #[expect(
                    clippy::disallowed_types,
                    reason = "plugin-shim runs single-threaded outside the engine runtime and binds a local loopback relay socket for proxy_tcp reduced confinement (ADR 0033)"
                )]
                let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| {
                    ConfineError::Namespace {
                        call: "bind(127.0.0.1:0)",
                        error,
                    }
                })?;
                let port = listener
                    .local_addr()
                    .map_err(|error| ConfineError::Namespace {
                        call: "local_addr",
                        error,
                    })?
                    .port();
                (listener, port)
            }
            Err(error) => return Err(error),
        };
        relay::spawn_in_netns(control_fd, bound.0).map_err(|error| ConfineError::Namespace {
            call: "fork(relay)",
            error,
        })?;
        Some(bound.1)
    } else {
        None
    };
    let abi = kernel_abi();
    if abi > 0 {
        let _ = restrict_files(policy, abi >= SIGNAL_SCOPE_ABI);
    }
    for program in &programs {
        let _ = seccompiler::apply_filter(program);
    }
    Ok(relay_port)
}

fn restrict_files(policy: &Policy, scoped: bool) -> Result<Mechanism, ConfineError> {
    let read = AccessFs::ReadFile | AccessFs::ReadDir;
    let start = AccessFs::ReadFile | AccessFs::Execute;
    let ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(HANDLED_ABI))?;
    let ruleset = if scoped {
        ruleset.scope(Scope::from_all(HANDLED_ABI))?
    } else {
        ruleset
    };
    let status = ruleset
        .create()?
        .add_rules(granted(&policy.reads, read))?
        .add_rules(granted(&libraries(), read))?
        .add_rules(granted(&policy.executables, start))?
        .add_rules(granted(&loaders(), start))?
        .add_rules(granted(&policy.writes, AccessFs::from_all(HANDLED_ABI)))?
        .restrict_self()?;
    let LandlockStatus::Available { effective_abi, .. } = status.landlock else {
        return Err(not_enabled());
    };
    let abi = abi_number(effective_abi);
    match scoped {
        true if abi < SIGNAL_SCOPE_ABI || status.ruleset != RulesetStatus::FullyEnforced => Err(
            ConfineError::Unavailable("Landlock lacks ABI 6, which scopes signals"),
        ),
        true => Ok(Mechanism::Landlock { abi }),
        // Rights an older ABI lacks are kept another way: seccomp refuses
        // `truncate` and allows `ioctl` only for its few requests, and ABI
        // 1 refuses any rename across folders.
        false if status.ruleset == RulesetStatus::NotEnforced => Err(not_enabled()),
        false => Ok(Mechanism::PidNamespace {
            abi: abi.min(SIGNAL_SCOPE_ABI - 1),
        }),
    }
}

fn not_enabled() -> ConfineError {
    ConfineError::Unavailable("Landlock is not enabled in this kernel")
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

/// Marks every handle beyond stdio to close when the process starts
/// another program, so a Plugin inherits none its launcher was given.
///
/// # Errors
///
/// When the kernel refuses: `close_range`'s flag needs Linux 5.11.
#[expect(
    unsafe_code,
    reason = "marking a range of handles close-on-exec is a raw system call"
)]
pub fn close_beyond_stdio() -> io::Result<()> {
    // SAFETY: `close_range` takes two handle numbers and a flag, and reads
    // no memory.
    let result = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3_u32,
            u32::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Where a dynamically linked program's loader reads the system's shared
/// libraries: a Plugin's entrypoint or runtime starts nowhere without them.
/// They hold no secret, and the deny list covers none of them.
const LIBRARIES: &[&str] = &[
    "/etc/ld.so.cache",
    "/lib",
    "/lib64",
    "/usr/lib",
    "/usr/lib64",
];

/// The start of the name of each loader, glibc's and musl's, in `/lib` or
/// `/lib64`.
const LOADERS: &[&str] = &["ld-linux", "ld-musl"];

/// The places of [`LIBRARIES`] this host has, which every instance reads.
pub(crate) fn libraries() -> Vec<PathBuf> {
    LIBRARIES
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect()
}

/// This host's loaders, which every instance may start, as the kernel
/// does for each dynamically linked program it starts. A loader maps any
/// library it reads executable, which Landlock does not check: the
/// libraries it reads are only [`libraries`].
#[expect(
    clippy::disallowed_methods,
    reason = "the applier runs in a launcher's one thread before the Plugin starts, never on an async worker"
)]
fn loaders() -> Vec<PathBuf> {
    ["/lib", "/lib64"]
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| LOADERS.iter().any(|start| name.starts_with(start)))
        })
        .collect()
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

/// Builds denylist BPF filters for `goodfetch-sandbox`'s unconditional
/// baseline rules (`ENOSYS` for `NotImplemented`, `EPERM` for `Refuse`,
/// default `Allow`), so the caller's policy allowlist can stack after it.
fn baseline_programs(rules: &[Rule]) -> Result<Vec<BpfProgram>, ConfineError> {
    let arch = TargetArch::try_from(std::env::consts::ARCH)?;
    let eperm = SeccompAction::Errno(libc::EPERM.unsigned_abs());
    let enosys = SeccompAction::Errno(libc::ENOSYS.unsigned_abs());
    let filters = [(Verdict::NotImplemented, enosys), (Verdict::Refuse, eperm)];
    let mut programs = Vec::new();
    for (verdict, on_match) in filters {
        let matched = rules.iter().filter(|rule| rule.verdict == verdict);
        let chains = chains(matched)?;
        if chains.is_empty() {
            continue;
        }
        let filter = SeccompFilter::new(chains, SeccompAction::Allow, on_match, arch)?;
        programs.push(BpfProgram::try_from(filter)?);
    }
    Ok(programs)
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
