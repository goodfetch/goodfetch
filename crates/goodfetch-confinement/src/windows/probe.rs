//! What `goodfetch-confinement-probe` is asked and answers on Windows
//! (ADR 0067, issue 873): mirrors `linux::probe` and `macos::probe`'s
//! status-code protocol so a runner reads the exact outcome, mechanism
//! (`Mechanism::AppContainer`), or fault from the exit status alone.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{ConfineError, Mechanism, WindowsOperation};
use crate::probes::{Access, Errno, Outcome};

/// The Windows probe request passed as JSON to `goodfetch-confinement-probe`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub probe: Probe,
    /// Optional parent PID supplied by the runner for `ProcessOpenParent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
}

/// One Windows probe action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    /// Exercise one Windows confinement operation (`network.*`,
    /// `desktop.default`, `signals.outside`).
    Operation {
        operation: WindowsOperation,
        /// Optional loopback target port for `NetworkOutboundIp` probes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    /// Open `path` for reading or create it for writing.
    Path { access: Access, path: PathBuf },
    /// Report the active mechanism verified via
    /// `GetTokenInformation(..., TokenIsAppContainer, ...) != 0`.
    Mechanism,
    /// Allocate `bytes` of heap memory and touch every page (oracle for Job
    /// Object memory limit).
    AllocateMemory { bytes: usize },
    /// Spin CPU until terminated (oracle for Job Object CPU-time limit).
    BurnCpu,
    /// Spawn a child process (oracle for Job Object active-process limit).
    SpawnChild,
    /// Open `count` file handles in `dir` and hold them open briefly (oracle
    /// for open-files handle-count limit).
    HoldHandles { dir: PathBuf, count: u32 },
}

/// What a Windows probe reports through its exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Outcome(Outcome),
    OtherError(u8),
    Mechanism(Mechanism),
    Fault(Fault),
}

/// Why a Windows probe could not produce an outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Request,
    Unavailable,
    Confine,
    NotConfined,
    ErrorOutOfRange,
}

const ERRNO_BASE: u8 = 100;
const MECHANISM_APPCONTAINER: u8 = 200;
const FAULTS: &[(Fault, u8)] = &[
    (Fault::Request, 240),
    (Fault::Unavailable, 242),
    (Fault::Confine, 243),
    (Fault::NotConfined, 244),
    (Fault::ErrorOutOfRange, 245),
];

/// Win32 / `WinSock` error codes mapped to the canonical probe `Errno` values.
const WIN32_ERROR_FILE_NOT_FOUND: i32 = 2;
const WIN32_ERROR_PATH_NOT_FOUND: i32 = 3;
const WIN32_ERROR_ACCESS_DENIED: i32 = 5;
const WIN32_ERROR_INVALID_HANDLE: i32 = 6;
const WIN32_ERROR_INVALID_PARAMETER: i32 = 87;
const WIN32_ERROR_NOT_ENOUGH_QUOTA: i32 = 1816;
const WSAEACCES: i32 = 10013;
const WSAEOPNOTSUPP: i32 = 10045;

impl Report {
    /// Maps a Win32 or `WinSock` OS error code into a [`Report`].
    #[must_use]
    pub fn win32_error(code: i32) -> Self {
        match code {
            WIN32_ERROR_ACCESS_DENIED | WIN32_ERROR_NOT_ENOUGH_QUOTA | WSAEACCES => {
                Self::Outcome(Outcome::Errno(Errno::Eacces))
            }
            WIN32_ERROR_FILE_NOT_FOUND | WIN32_ERROR_PATH_NOT_FOUND => {
                Self::Outcome(Outcome::Errno(Errno::Enoent))
            }
            WIN32_ERROR_INVALID_HANDLE => Self::Outcome(Outcome::Errno(Errno::Ebadf)),
            WIN32_ERROR_INVALID_PARAMETER => Self::Outcome(Outcome::Errno(Errno::Einval)),
            WSAEOPNOTSUPP => Self::Outcome(Outcome::Errno(Errno::Eopnotsupp)),
            other => u8::try_from(other)
                .ok()
                .filter(|&n| n < MECHANISM_APPCONTAINER - ERRNO_BASE)
                .map_or(Self::Fault(Fault::ErrorOutOfRange), Self::OtherError),
        }
    }

    /// Encodes this report into an exit status byte disjoint from
    /// `LaunchError`'s ranges (`232..=236` and `246..=254`).
    #[must_use]
    pub fn status(self) -> u8 {
        match self {
            Self::Outcome(Outcome::Ok) => 0,
            Self::Outcome(Outcome::Exit(status)) => status,
            Self::Outcome(Outcome::Errno(errno)) => ERRNO_BASE.saturating_add(errno_slot(errno)),
            Self::OtherError(number) => ERRNO_BASE.saturating_add(number),
            Self::Mechanism(Mechanism::AppContainer) => MECHANISM_APPCONTAINER,
            Self::Fault(fault) => fault_status(fault),
        }
    }

    /// Decodes an exit status byte back into a [`Report`].
    #[must_use]
    pub fn of_status(status: u8) -> Option<Self> {
        if let Some(fault) = fault_of_status(status) {
            return Some(Self::Fault(fault));
        }
        match status {
            0 => Some(Self::Outcome(Outcome::Ok)),
            1..ERRNO_BASE => Some(Self::Outcome(Outcome::Exit(status))),
            ERRNO_BASE..MECHANISM_APPCONTAINER => Some(errno_from_slot(status - ERRNO_BASE)),
            MECHANISM_APPCONTAINER => Some(Self::Mechanism(Mechanism::AppContainer)),
            _ => None,
        }
    }
}

fn errno_slot(errno: Errno) -> u8 {
    match errno {
        Errno::Eperm => 1,
        Errno::Enoent => 2,
        Errno::Esrch => 3,
        Errno::Ebadf => 9,
        Errno::Echild => 10,
        Errno::Eacces => 13,
        Errno::Efault => 14,
        Errno::Einval => 22,
        Errno::Erofs => 30,
        Errno::Erange => 34,
        Errno::Enosys => 38,
        Errno::Eopnotsupp => 95,
    }
}

fn errno_from_slot(slot: u8) -> Report {
    let errno = match slot {
        1 => Some(Errno::Eperm),
        2 => Some(Errno::Enoent),
        3 => Some(Errno::Esrch),
        9 => Some(Errno::Ebadf),
        10 => Some(Errno::Echild),
        13 => Some(Errno::Eacces),
        14 => Some(Errno::Efault),
        22 => Some(Errno::Einval),
        30 => Some(Errno::Erofs),
        34 => Some(Errno::Erange),
        38 => Some(Errno::Enosys),
        95 => Some(Errno::Eopnotsupp),
        _ => None,
    };
    errno.map_or(Report::OtherError(slot), |e| {
        Report::Outcome(Outcome::Errno(e))
    })
}

fn fault_status(fault: Fault) -> u8 {
    FAULTS
        .iter()
        .find_map(|&(known, code)| (known == fault).then_some(code))
        .unwrap_or(u8::MAX)
}

fn fault_of_status(status: u8) -> Option<Fault> {
    FAULTS
        .iter()
        .find_map(|&(fault, code)| (code == status).then_some(fault))
}

impl From<&ConfineError> for Fault {
    fn from(error: &ConfineError) -> Self {
        match error {
            ConfineError::Unavailable(_) => Self::Unavailable,
            ConfineError::Confine(_)
            | ConfineError::Folder(_)
            | ConfineError::Limits(_)
            | ConfineError::Start(_)
            | ConfineError::LimitExceeded(_)
            | ConfineError::BlockedBySmartAppControl(_) => Self::Confine,
        }
    }
}
