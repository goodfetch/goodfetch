//! What the probe executable, `goodfetch-confinement-probe`, is asked and
//! answers (ADR 0067): a runner passes a [`Request`] as JSON, its one
//! argument, and reads the [`Report`] from its exit status, since a probe
//! writes nothing. A seccomp kill ends it by a signal instead, which no
//! report stands for, so a runner fails on it.
//!
//! The exit statuses: 0 for a call that returned without an error; 1 to 99
//! for a call that ended the process with that status; 100 plus the error's
//! number for an error; 200 plus the ABI for the mechanism (200 the mount
//! namespace, 201 to 205 the PID-namespace mechanism, 206 and up Landlock
//! alone); 216 plus the
//! kernel's Landlock ABI; 240 and up for a probe that could not run.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::limits::Resource;
use super::{ConfineError, Mechanism, MechanismKind, SIGNAL_SCOPE_ABI};
use crate::policy::Policy;
use crate::probes::{Access, Arg, Errno, Outcome};
use crate::seccomp::Call;

/// The probe to make, confined to `policy` first when there is one, with
/// `mechanism`, or the kernel's without one. When `sandbox_baseline` is true
/// (with `policy: None`), applies only `goodfetch-sandbox`'s unconditional
/// capability drop, `no_new_privs`, and baseline seccomp filter before running
/// the probe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub probe: Probe,
    pub policy: Option<Policy>,
    pub mechanism: Option<MechanismKind>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sandbox_baseline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    /// Make `call` with `args`.
    Call { call: Call, args: Vec<Arg> },
    /// Open `path` for reading, or create it for writing.
    Path { access: Access, path: PathBuf },
    /// Report the mechanism that confines it, and make no call.
    Mechanism,
    /// Report the Landlock ABI the kernel gives, asked with the raw call and
    /// never confined: the runner's oracle for the mechanism to expect.
    KernelAbi,
    /// Enter a user and PID namespace and map its user, with the raw calls
    /// and never confined: the runner's oracle for the PID-namespace
    /// mechanism. It reports the outcome.
    UserNamespace,
    /// Set `resource`'s soft and hard limit to `to` with `prlimit64`, which
    /// an unprivileged process may only where `to` is not past its hard
    /// limit: the oracle for the limits its launcher set.
    Raise { resource: Resource, to: u64 },
    /// Verify every capability (effective, permitted, inheritable) is zero and
    /// `PR_GET_NO_NEW_PRIVS` is 1: returns `Outcome::Ok` when dropped and set,
    /// `Outcome::Exit(1)` when any capability bit remains set, or
    /// `Outcome::Exit(2)` when `no_new_privs` is not set (issue 884).
    Capabilities,
}

/// What a probe reports through its exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    /// Its call gave this.
    Outcome(Outcome),
    /// Its call failed with an error no probe table names.
    OtherErrno(u8),
    Mechanism(Mechanism),
    /// The kernel's Landlock ABI, 0 without Landlock.
    KernelAbi(u8),
    Fault(Fault),
}

/// Why a probe made no call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Its request cannot be read.
    Request,
    /// [`ConfineError::LevelNotBuilt`].
    LevelNotBuilt,
    /// [`ConfineError::Unavailable`]: the host lacks the mechanism, which a
    /// runner asserts where its oracle says so.
    Unavailable,
    /// Any other [`ConfineError`].
    Confine,
    /// This architecture has no such call.
    NoSuchCall,
    /// Its call failed with an error whose number has no status.
    ErrnoOutOfRange,
}

const ERRNO_BASE: u8 = 100;
const MECHANISM_BASE: u8 = 200;
const KERNEL_ABI_BASE: u8 = 216;
const FAULT_BASE: u8 = 240;
const FAULTS: &[(Fault, u8)] = &[
    (Fault::Request, 240),
    (Fault::LevelNotBuilt, 241),
    (Fault::Unavailable, 242),
    (Fault::Confine, 243),
    (Fault::NoSuchCall, 244),
    (Fault::ErrnoOutOfRange, 245),
];

impl Report {
    /// An error by its number on this OS.
    pub fn errno(number: i32) -> Self {
        let Ok(number) = u8::try_from(number) else {
            return Self::Fault(Fault::ErrnoOutOfRange);
        };
        if number >= MECHANISM_BASE - ERRNO_BASE {
            return Self::Fault(Fault::ErrnoOutOfRange);
        }
        ERRNOS
            .iter()
            .find(|(_, known)| i32::from(number) == *known)
            .map_or(Self::OtherErrno(number), |(errno, _)| {
                Self::Outcome(Outcome::Errno(*errno))
            })
    }

    pub fn status(self) -> u8 {
        match self {
            Self::Outcome(Outcome::Ok) => 0,
            Self::Outcome(Outcome::Exit(status)) => status,
            Self::Outcome(Outcome::Errno(errno)) => ERRNO_BASE.saturating_add(
                ERRNOS
                    .iter()
                    .find(|(known, _)| *known == errno)
                    .and_then(|(_, number)| u8::try_from(*number).ok())
                    .expect("invariant: `ERRNOS` numbers every `Errno`, each below 100"),
            ),
            Self::OtherErrno(number) => ERRNO_BASE.saturating_add(number),
            Self::Mechanism(Mechanism::Landlock { abi } | Mechanism::PidNamespace { abi }) => {
                MECHANISM_BASE.saturating_add(abi)
            }
            Self::Mechanism(Mechanism::MountNamespace) => MECHANISM_BASE,
            Self::KernelAbi(abi) => KERNEL_ABI_BASE.saturating_add(abi),
            Self::Fault(fault) => FAULTS
                .iter()
                .find(|(known, _)| *known == fault)
                .map_or(u8::MAX, |(_, status)| *status),
        }
    }

    /// The report `status` stands for, none for a status no report has.
    pub fn of_status(status: u8) -> Option<Self> {
        if let Some((fault, _)) = FAULTS.iter().find(|(_, known)| *known == status) {
            return Some(Self::Fault(*fault));
        }
        Some(match status {
            0 => Self::Outcome(Outcome::Ok),
            1..ERRNO_BASE => Self::Outcome(Outcome::Exit(status)),
            ERRNO_BASE..MECHANISM_BASE => Self::errno(i32::from(status - ERRNO_BASE)),
            // The mount namespace has no Landlock, and the PID-namespace
            // mechanism's ABIs lie below Landlock's own.
            MECHANISM_BASE..KERNEL_ABI_BASE => Self::Mechanism(match status - MECHANISM_BASE {
                0 => Mechanism::MountNamespace,
                abi @ 1..SIGNAL_SCOPE_ABI => Mechanism::PidNamespace { abi },
                abi => Mechanism::Landlock { abi },
            }),
            KERNEL_ABI_BASE..FAULT_BASE => Self::KernelAbi(status - KERNEL_ABI_BASE),
            _ => return None,
        })
    }
}

impl From<&ConfineError> for Fault {
    fn from(error: &ConfineError) -> Self {
        match error {
            ConfineError::LevelNotBuilt(_) => Self::LevelNotBuilt,
            ConfineError::Unavailable(_) => Self::Unavailable,
            ConfineError::Namespace { .. }
            | ConfineError::Grant { .. }
            | ConfineError::Landlock(_)
            | ConfineError::Filter(_)
            | ConfineError::Install(_) => Self::Confine,
        }
    }
}

/// The probe tables' errors by their numbers on Linux.
const ERRNOS: &[(Errno, i32)] = &[
    (Errno::Eperm, libc::EPERM),
    (Errno::Enoent, libc::ENOENT),
    (Errno::Esrch, libc::ESRCH),
    (Errno::Ebadf, libc::EBADF),
    (Errno::Echild, libc::ECHILD),
    (Errno::Eacces, libc::EACCES),
    (Errno::Efault, libc::EFAULT),
    (Errno::Einval, libc::EINVAL),
    (Errno::Erofs, libc::EROFS),
    (Errno::Erange, libc::ERANGE),
    (Errno::Enosys, libc::ENOSYS),
    (Errno::Eopnotsupp, libc::EOPNOTSUPP),
];
