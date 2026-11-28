//! What `goodfetch-confinement-probe` is asked and answers on macOS (ADR 0067,
//! issue 867): mirrors `linux::probe`'s status-code protocol so a runner reads
//! the exact outcome, mechanism (`Mechanism::Seatbelt`), or fault from the
//! exit status alone.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::limits::Resource;
use super::{ConfineError, Mechanism, SeatbeltOperation};
use crate::policy::Policy;
use crate::probes::{Access, Errno, Outcome};

/// The macOS probe request passed as JSON to `goodfetch-confinement-probe`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub probe: Probe,
    pub policy: Option<Policy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
}

/// One macOS probe action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    /// Exercise one Seatbelt operation (`network.*`, `mach.dns`, `process.fork`,
    /// `signals.outside`).
    Operation {
        operation: SeatbeltOperation,
        /// Optional loopback target port for `NetworkOutboundProxy` /
        /// `NetworkOutboundIp` probes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// Optional Unix socket path for `NetworkUnixSocket` probes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unix_path: Option<PathBuf>,
    },
    /// Open `path` for reading or create it for writing.
    Path { access: Access, path: PathBuf },
    /// Report the active mechanism verified via `sandbox_check(pid, NULL, 0)`.
    Mechanism,
    /// Attempt to set `resource`'s soft and hard rlimit to `to` (oracle for
    /// `macos::limits::limit_self`).
    Raise { resource: Resource, to: u64 },
}

/// What a macOS probe reports through its exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Outcome(Outcome),
    OtherErrno(u8),
    Mechanism(Mechanism),
    Fault(Fault),
}

/// Why a macOS probe could not produce an outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Request,
    Unavailable,
    Confine,
    NotConfined,
    ErrnoOutOfRange,
}

const ERRNO_BASE: u8 = 100;
const MECHANISM_SEATBELT: u8 = 200;
const FAULTS: &[(Fault, u8)] = &[
    (Fault::Request, 240),
    (Fault::Unavailable, 242),
    (Fault::Confine, 243),
    (Fault::NotConfined, 244),
    (Fault::ErrnoOutOfRange, 245),
];

impl Report {
    /// Maps a macOS `errno` value into a [`Report`].
    #[must_use]
    pub fn errno(number: i32) -> Self {
        let Ok(number) = u8::try_from(number) else {
            return Self::Fault(Fault::ErrnoOutOfRange);
        };
        if number >= MECHANISM_SEATBELT - ERRNO_BASE {
            return Self::Fault(Fault::ErrnoOutOfRange);
        }
        ERRNOS
            .iter()
            .find(|(_, known)| i32::from(number) == *known)
            .map_or(Self::OtherErrno(number), |(errno, _)| {
                Self::Outcome(Outcome::Errno(*errno))
            })
    }

    /// Encodes this report into an exit status byte below `LaunchError`'s
    /// 246..=254 range.
    #[must_use]
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
            Self::Mechanism(Mechanism::Seatbelt) => MECHANISM_SEATBELT,
            Self::Fault(fault) => FAULTS
                .iter()
                .find(|(known, _)| *known == fault)
                .map_or(u8::MAX, |(_, status)| *status),
        }
    }

    /// Decodes an exit status byte back into a [`Report`].
    #[must_use]
    pub fn of_status(status: u8) -> Option<Self> {
        if let Some((fault, _)) = FAULTS.iter().find(|(_, known)| *known == status) {
            return Some(Self::Fault(*fault));
        }
        match status {
            0 => Some(Self::Outcome(Outcome::Ok)),
            1..ERRNO_BASE => Some(Self::Outcome(Outcome::Exit(status))),
            ERRNO_BASE..MECHANISM_SEATBELT => Some(Self::errno(i32::from(status - ERRNO_BASE))),
            MECHANISM_SEATBELT => Some(Self::Mechanism(Mechanism::Seatbelt)),
            _ => None,
        }
    }
}

impl From<&ConfineError> for Fault {
    fn from(error: &ConfineError) -> Self {
        match error {
            ConfineError::Unavailable(_) => Self::Unavailable,
            ConfineError::InvalidPath(_)
            | ConfineError::MissingProxyPort
            | ConfineError::Apply(_) => Self::Confine,
        }
    }
}

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
