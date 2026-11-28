//! The Windows applier (ADR 0033, issue 873): confines process Plugins via
//! `windows-sys` using an `AppContainer` profile with zero capabilities at
//! `none`, a restricted token (`DISABLE_MAX_PRIVILEGE`), a Job Object with
//! memory, CPU time, active-process and UI limits plus handle-count
//! enforcement, and an alternate desktop (`CreateDesktopW`), isolating all
//! Win32 `unsafe` calls in `windows::win32`.
//!
//! ## Network levels on Windows
//!
//! - [`NetworkLevel::None`]: an `AppContainer` with zero network capabilities,
//!   so Windows' default outbound rule blocks all sockets (`WSAEACCES`), and
//!   minimal NTFS ACL grants on the policy's read, write and executable paths
//!   alone.
//! - [`NetworkLevel::ProxyTcp`] and [`NetworkLevel::Direct`]: require the
//!   Windows filter broker (`goodfetch-filter-broker`, issue 879) to install
//!   persistent Windows Filtering Platform filters scoped to the instance's
//!   `AppContainer` SID before start. Without a [`FilterBrokerGrant`], the
//!   Windows applier refuses both levels with [`ConfineError::Unavailable`]
//!   rather than weakening confinement.
//!
//! ## How a live confined process shows its sandbox on Windows
//!
//! Inside a confined child, `GetTokenInformation(current_token,
//! TokenIsAppContainer, ...)` returns a non-zero `DWORD` (`is_appcontainer`),
//! which the probe runner (`Probe::Mechanism`) asserts as
//! [`Mechanism::AppContainer`] before checking refusals (ADR 0067).

#[cfg(windows)]
pub mod host;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
mod win32;

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::plugin::{NetworkLevel, PluginResourceLimit};
use serde::{Deserialize, Serialize};

use crate::deny_list::Denied;
use crate::policy::Policy;
use crate::probes::{
    Access, Errno, Mismatch, OsOpCase, OsOpRule, OsPathCase, Outcome, Placed, check_os_table,
};
use crate::seccomp::Verdict;

/// What confines a process on Windows, asserted by every probe runner so a
/// silent fallback fails (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    /// Windows `AppContainer` + restricted token + Job Object + alternate
    /// desktop, confirmed active on the live process via
    /// `GetTokenInformation(..., TokenIsAppContainer, ...) != 0`.
    AppContainer,
}

/// Filter broker grant supplied for `proxy_tcp` or `direct` instances on
/// Windows once `goodfetch-filter-broker` (#879) has created the instance's
/// persistent WFP filters scoped to `app_container_name`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterBrokerGrant {
    /// Pre-registered `AppContainer` profile name whose SID the broker's
    /// persistent WFP filters match.
    pub app_container_name: String,
    /// Optional loopback proxy port permitted by the broker at `proxy_tcp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
}

/// Parameters for launching one confined process Plugin on Windows.
#[derive(Clone, Copy, Debug)]
pub struct ProcessSpec<'a> {
    pub policy: &'a Policy,
    pub program: &'a Path,
    pub args: &'a [String],
    pub folder: &'a Path,
    pub env: &'a [(&'a str, &'a str)],
    pub broker_grant: Option<&'a FilterBrokerGrant>,
}

/// A Windows confinement operation probed by `probes/windows/*.toml`
/// (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsOperation {
    /// Outgoing IPv4 TCP connection (`connect`).
    NetworkOutboundIp,
    /// Incoming TCP connection acceptance / listening socket (`listen`).
    NetworkInbound,
    /// Binding a TCP listening socket (`bind`).
    NetworkBind,
    /// Opening the user's interactive `"Default"` desktop from the alternate
    /// desktop (`OpenDesktopW`).
    DesktopEscape,
    /// Opening the unconfined parent process with `PROCESS_TERMINATE`
    /// (`signals.outside`).
    ProcessOpenParent,
}

/// One declarative Windows confinement rule for a network level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsRule {
    pub name: &'static str,
    pub operation: WindowsOperation,
    pub verdict: Verdict,
}

const fn rule(name: &'static str, operation: WindowsOperation, verdict: Verdict) -> WindowsRule {
    WindowsRule {
        name,
        operation,
        verdict,
    }
}

/// The declarative Windows confinement operation rules enforced at `level`
/// (ADR 0033). `proxy_tcp` and `direct` wait for the filter broker (#879).
#[must_use]
pub fn rules(level: NetworkLevel) -> Option<Vec<WindowsRule>> {
    match level {
        NetworkLevel::None => Some(vec![
            rule(
                "network.outbound_ip",
                WindowsOperation::NetworkOutboundIp,
                Verdict::Refuse,
            ),
            rule(
                "network.inbound",
                WindowsOperation::NetworkInbound,
                Verdict::Refuse,
            ),
            rule(
                "network.bind",
                WindowsOperation::NetworkBind,
                Verdict::Refuse,
            ),
            rule(
                "desktop.default",
                WindowsOperation::DesktopEscape,
                Verdict::Refuse,
            ),
        ]),
        NetworkLevel::ProxyTcp | NetworkLevel::Direct => None,
    }
}

/// Windows Job Object notification IDs from `WinNT.h` / `Win32_System_SystemServices`,
/// classified into [`PluginResourceLimit`] on every OS so unit tests cover the
/// mapping without Windows headers.
pub const JOB_MSG_END_OF_JOB_TIME: u32 = 1;
pub const JOB_MSG_END_OF_PROCESS_TIME: u32 = 2;
pub const JOB_MSG_ACTIVE_PROCESS_LIMIT: u32 = 3;
pub const JOB_MSG_ACTIVE_PROCESS_ZERO: u32 = 4;
pub const JOB_MSG_PROCESS_MEMORY_LIMIT: u32 = 9;
pub const JOB_MSG_JOB_MEMORY_LIMIT: u32 = 10;

/// Maps a Windows Job Object I/O Completion Port notification message ID to
/// the [`PluginResourceLimit`] it enforces, or `None` for non-limit
/// lifecycle notifications (`ACTIVE_PROCESS_ZERO`, `NEW_PROCESS`, etc.).
#[must_use]
pub const fn classify_job_notification(msg_id: u32) -> Option<PluginResourceLimit> {
    match msg_id {
        JOB_MSG_END_OF_JOB_TIME | JOB_MSG_END_OF_PROCESS_TIME => Some(PluginResourceLimit::CpuTime),
        JOB_MSG_ACTIVE_PROCESS_LIMIT => Some(PluginResourceLimit::Processes),
        JOB_MSG_PROCESS_MEMORY_LIMIT | JOB_MSG_JOB_MEMORY_LIMIT => {
            Some(PluginResourceLimit::Memory)
        }
        _ => None,
    }
}

/// Converts a manifest CPU-time limit in whole seconds into Windows Job Object
/// `PerJobUserTimeLimit` units (100-nanosecond ticks), saturating at `i64::MAX`.
#[must_use]
pub const fn cpu_time_to_100ns(seconds: u64) -> i64 {
    let ticks = seconds.saturating_mul(10_000_000);
    if ticks > i64::MAX as u64 {
        i64::MAX
    } else {
        #[expect(
            clippy::cast_possible_wrap,
            reason = "guarded above by `ticks <= i64::MAX as u64`"
        )]
        {
            ticks as i64
        }
    }
}

/// Why a Windows process was not confined or terminated by its Job Object.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The host lacks `AppContainer` or the Windows filter broker
    /// (`confinement_unavailable`), never weakened.
    #[error("this Windows host cannot confine a Plugin: {0}")]
    Unavailable(String),
    /// Confining failed while setting up `AppContainer`, ACLs, restricted token,
    /// or alternate desktop.
    #[error("Windows confinement failed: {0}")]
    Confine(String),
    /// The working folder cannot be resolved.
    #[error("cannot prepare the instance's folder: {0}")]
    Folder(io::Error),
    /// Setting or enforcing Job Object resource limits failed.
    #[error("cannot set the Plugin's Job Object limits: {0}")]
    Limits(io::Error),
    /// Starting the confined executable failed.
    #[error("cannot start the confined Plugin: {0}")]
    Start(io::Error),
    /// A Job Object or handle-count limit ended the confined Plugin.
    #[error("the confined Plugin exceeded its `{0:?}` Job Object limit")]
    LimitExceeded(PluginResourceLimit),
    /// Smart App Control or another App Control policy blocked the executable
    /// from starting (ADR 0039).
    #[error("Smart App Control blocked `{}` from starting", .0.display())]
    BlockedBySmartAppControl(PathBuf),
}

/// Validates that `policy` and `broker_grant` can be confined on Windows:
/// `proxy_tcp` and `direct` require a [`FilterBrokerGrant`] from
/// `goodfetch-filter-broker` (#879) and are refused with
/// [`ConfineError::Unavailable`] without one.
///
/// # Errors
///
/// Returns [`ConfineError::Unavailable`] when `policy.network_level` is
/// `proxy_tcp` or `direct` and `broker_grant` is `None`.
pub fn check_network_level(
    level: NetworkLevel,
    broker_grant: Option<&FilterBrokerGrant>,
) -> Result<(), ConfineError> {
    match (level, broker_grant) {
        (NetworkLevel::None, _) | (NetworkLevel::ProxyTcp | NetworkLevel::Direct, Some(_)) => {
            Ok(())
        }
        (NetworkLevel::ProxyTcp | NetworkLevel::Direct, None) => {
            Err(ConfineError::Unavailable(format!(
                "`{}` on Windows requires the filter broker; run `goodfetch plugins setup`",
                level.as_str()
            )))
        }
    }
}

/// Runs `spec`'s program inside a Windows `AppContainer` with a restricted
/// token, alternate desktop, minimal NTFS ACL grants, and a Job Object
/// enforcing memory, CPU time, active-process and open-file limits, returning
/// the child's exit code on normal exit.
///
/// # Errors
///
/// Returns [`ConfineError::Unavailable`] when the network level requires the
/// filter broker or `AppContainer` is unavailable,
/// [`ConfineError::BlockedBySmartAppControl`] when Smart App Control refuses
/// to start `spec.program`, [`ConfineError::LimitExceeded`] when any Job
/// Object or handle limit fires, or another [`ConfineError`] on OS failure.
#[cfg(windows)]
pub fn run_confined(spec: &ProcessSpec<'_>) -> Result<i32, ConfineError> {
    check_network_level(spec.policy.network_level, spec.broker_grant)?;
    win32::run_confined(spec)
}

/// Whether the current Windows process is running inside an `AppContainer`
/// (`GetTokenInformation(current_token, TokenIsAppContainer, ...) != 0`).
#[cfg(windows)]
#[must_use]
pub fn is_appcontainer() -> bool {
    win32::is_current_process_appcontainer()
}

/// The committed Windows confinement probe table for `level` (ADR 0067);
/// `None` for levels that wait for the filter broker (#879).
#[must_use]
pub fn committed_probes(level: NetworkLevel) -> Option<&'static str> {
    match level {
        NetworkLevel::None => Some(include_str!("../probes/windows/none.toml")),
        NetworkLevel::ProxyTcp | NetworkLevel::Direct => None,
    }
}

/// One Windows network level's probe cases (ADR 0067): `AppContainer` / desktop
/// / process-isolation operations, the signal rule, every Windows deny-list
/// entry, and every policy file rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsProbeTable {
    pub operation: Vec<WindowsOperationCase>,
    pub path: Vec<WindowsPathCase>,
}

impl WindowsProbeTable {
    /// Parses a committed Windows probe table from TOML.
    ///
    /// # Errors
    ///
    /// Returns a TOML parse error if the table is malformed.
    pub fn read(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

/// One Windows operation probe case with its unconfined positive control and
/// exact confined outcome under `AppContainer`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsOperationCase {
    pub rule: String,
    pub operation: WindowsOperation,
    pub unconfined: Outcome,
    pub confined: Outcome,
}

/// One Windows filesystem probe case (deny-list entry or file rule) with its
/// unconfined positive control and exact confined outcome under `AppContainer`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsPathCase {
    pub rule: String,
    pub access: Access,
    pub path: Placed,
    pub unconfined: Outcome,
    pub appcontainer: Outcome,
}

/// Checks `table` against `level_rules`, `denied` (`deny_list::on(Host::Windows)`),
/// `FILE_RULES`, and `SIGNAL_RULE` (ADR 0067): returns empty when every rule
/// has a probe, no probe lacks a rule, and every refusal keeps a positive
/// control.
#[must_use]
pub fn check_probes(
    table: &WindowsProbeTable,
    level_rules: &[WindowsRule],
    denied: &[&Denied],
) -> Vec<Mismatch> {
    let op_cases: Vec<OsOpCase<'_>> = table
        .operation
        .iter()
        .map(|case| OsOpCase {
            rule: &case.rule,
            is_signal_op: case.operation == WindowsOperation::ProcessOpenParent,
            unconfined: case.unconfined,
            confined: case.confined,
        })
        .collect();
    let path_cases: Vec<OsPathCase<'_>> = table
        .path
        .iter()
        .map(|case| OsPathCase {
            rule: &case.rule,
            path: &case.path,
            unconfined: case.unconfined,
            confined: case.appcontainer,
        })
        .collect();
    let op_rules: Vec<OsOpRule<'_>> = level_rules
        .iter()
        .map(|rule| OsOpRule {
            name: rule.name,
            verdict: rule.verdict,
        })
        .collect();
    check_os_table(&op_cases, &path_cases, &op_rules, denied, Errno::Eacces)
}

#[cfg(test)]
mod tests;
