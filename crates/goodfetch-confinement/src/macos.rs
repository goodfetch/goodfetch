//! The macOS applier (ADR 0033, issue 867): generates a Seatbelt profile
//! (SBPL) from a [`Policy`] and optional loopback proxy port, applies it via
//! the in-house two-function `sandbox_init` / `sandbox_free_error` binding in
//! `seatbelt` (the one `unsafe` module for macOS), sets rlimits for CPU time
//! and open files (`limits`), and exposes the macOS probe tables checked on
//! every OS (ADR 0067).
//!
//! ## How a live confined process shows its sandbox on macOS
//!
//! On macOS, a live confined process shows its active Seatbelt sandbox through
//! `sandbox_check(pid, NULL, SANDBOX_FILTER_NONE)` (`libSystem`), which
//! returns `1` when a Seatbelt profile is active on `pid`, `0` when the
//! process is unconfined, and `-1` on error. Both the probe runner
//! (`Probe::Mechanism`) and external inspectors call [`is_sandboxed`] to assert
//! the mechanism rather than inferring it from a failed syscall alone.
//!
//! ## Memory limits on macOS
//!
//! An unprivileged macOS process cannot enforce a per-process memory cap via
//! rlimits, so `limits` sets `RLIMIT_CPU` and `RLIMIT_NOFILE` alone; memory is
//! sampled once per `governor.sample_interval` by the engine's Resource
//! governor (ADR 0040) via `proc_pid_rusage` (`ri_phys_footprint`), which
//! kills the instance with `plugin_limit_exceeded` once it passes the
//! manifest's memory limit.

#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
pub mod limits;
#[cfg(target_os = "macos")]
pub mod probe;
#[cfg(target_os = "macos")]
mod seatbelt;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use goodfetch_protocol::plugin::NetworkLevel;
use serde::{Deserialize, Serialize};

use crate::deny_list::Denied;
use crate::policy::Policy;
use crate::probes::{
    Access, Errno, Mismatch, OsOpCase, OsOpRule, OsPathCase, Outcome, Placed, check_os_table,
};
use crate::seccomp::Verdict;

/// Mach services macOS name resolution uses: excluded at `none` and
/// `proxy_tcp` so a Plugin cannot exfiltrate data via DNS queries (ADR 0033),
/// allowed at `direct`.
pub const DNS_MACH_SERVICES: &[&str] = &[
    "com.apple.dnssd.service",
    "com.apple.mDNSResponder",
    "com.apple.SystemConfiguration.DNSConfiguration",
];

/// What confines a process on macOS, asserted by every probe runner so a
/// silent fallback fails (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    /// Apple Seatbelt applied via `sandbox_init`, confirmed active on the live
    /// process via `sandbox_check(pid, NULL, SANDBOX_FILTER_NONE) == 1`.
    Seatbelt,
}

/// A Seatbelt operation probed by `probes/macos/*.toml` (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatbeltOperation {
    /// Outgoing IPv4/IPv6 TCP connection to a non-proxy destination.
    NetworkOutboundIp,
    /// Outgoing TCP connection to the instance's loopback proxy port.
    NetworkOutboundProxy,
    /// Incoming TCP connection acceptance.
    NetworkInbound,
    /// Binding a TCP or UDP listening socket.
    NetworkBind,
    /// Connecting or binding a non-system Unix-domain socket.
    NetworkUnixSocket,
    /// DNS name resolution through macOS's `mDNSResponder` / Mach service.
    MachLookupDns,
    /// Spawning a child process via `fork`.
    ProcessFork,
    /// Sending a signal to a process outside the confined instance.
    SignalOutside,
}

/// One declarative Seatbelt rule for a network level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatbeltRule {
    pub name: &'static str,
    pub operation: SeatbeltOperation,
    pub verdict: Verdict,
}

const fn rule(name: &'static str, operation: SeatbeltOperation, verdict: Verdict) -> SeatbeltRule {
    SeatbeltRule {
        name,
        operation,
        verdict,
    }
}

/// The declarative Seatbelt operation rules enforced at `level` (ADR 0033).
#[must_use]
pub fn rules(level: NetworkLevel) -> Vec<SeatbeltRule> {
    match level {
        NetworkLevel::None => vec![
            rule(
                "network.outbound_ip",
                SeatbeltOperation::NetworkOutboundIp,
                Verdict::Refuse,
            ),
            rule(
                "network.inbound",
                SeatbeltOperation::NetworkInbound,
                Verdict::Refuse,
            ),
            rule(
                "network.bind",
                SeatbeltOperation::NetworkBind,
                Verdict::Refuse,
            ),
            rule(
                "network.unix_socket",
                SeatbeltOperation::NetworkUnixSocket,
                Verdict::Refuse,
            ),
            rule(
                "mach.dns",
                SeatbeltOperation::MachLookupDns,
                Verdict::Refuse,
            ),
            rule(
                "process.fork",
                SeatbeltOperation::ProcessFork,
                Verdict::Refuse,
            ),
        ],
        NetworkLevel::ProxyTcp => vec![
            rule(
                "network.outbound_proxy",
                SeatbeltOperation::NetworkOutboundProxy,
                Verdict::Allow,
            ),
            rule(
                "network.outbound_ip",
                SeatbeltOperation::NetworkOutboundIp,
                Verdict::Refuse,
            ),
            rule(
                "network.inbound",
                SeatbeltOperation::NetworkInbound,
                Verdict::Refuse,
            ),
            rule(
                "network.bind",
                SeatbeltOperation::NetworkBind,
                Verdict::Refuse,
            ),
            rule(
                "network.unix_socket",
                SeatbeltOperation::NetworkUnixSocket,
                Verdict::Refuse,
            ),
            rule(
                "mach.dns",
                SeatbeltOperation::MachLookupDns,
                Verdict::Refuse,
            ),
            rule(
                "process.fork",
                SeatbeltOperation::ProcessFork,
                Verdict::Allow,
            ),
        ],
        NetworkLevel::Direct => vec![
            rule(
                "network.outbound_ip",
                SeatbeltOperation::NetworkOutboundIp,
                Verdict::Allow,
            ),
            rule(
                "network.inbound",
                SeatbeltOperation::NetworkInbound,
                Verdict::Refuse,
            ),
            rule(
                "network.bind",
                SeatbeltOperation::NetworkBind,
                Verdict::Refuse,
            ),
            rule(
                "network.unix_socket",
                SeatbeltOperation::NetworkUnixSocket,
                Verdict::Refuse,
            ),
            rule("mach.dns", SeatbeltOperation::MachLookupDns, Verdict::Allow),
            rule(
                "process.fork",
                SeatbeltOperation::ProcessFork,
                Verdict::Allow,
            ),
        ],
    }
}

/// Why a macOS process was not confined.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The host refused Seatbelt confinement (`confinement_unavailable`),
    /// never weakened.
    #[error("this macOS host cannot confine a Plugin with Seatbelt: {0}")]
    Unavailable(String),
    /// A policy path contains a NUL byte or cannot be represented in SBPL.
    #[error("path `{}` cannot be encoded in a Seatbelt profile", .0.display())]
    InvalidPath(PathBuf),
    /// `proxy_tcp` requires the engine's loopback proxy port so Seatbelt can
    /// restrict outgoing traffic to that port alone.
    #[error("`proxy_tcp` on macOS requires a loopback proxy port")]
    MissingProxyPort,
    /// Seatbelt failed to compile or install the generated profile.
    #[error("Seatbelt refused the generated profile: {0}")]
    Apply(String),
}

/// Generates the Seatbelt profile (SBPL) for `policy` (ADR 0033).
///
/// At [`NetworkLevel::ProxyTcp`], `proxy_port` must name the engine's
/// per-instance loopback proxy port so outgoing traffic is permitted to that
/// loopback port alone; passing `None` returns [`ConfineError::MissingProxyPort`]
/// rather than weakening confinement.
///
/// # Errors
///
/// Returns [`ConfineError::MissingProxyPort`] when `policy.network_level` is
/// [`NetworkLevel::ProxyTcp`] and `proxy_port` is `None`, or
/// [`ConfineError::InvalidPath`] when any granted path contains a NUL byte or
/// non-UTF-8 characters.
pub fn profile(policy: &Policy, proxy_port: Option<u16>) -> Result<String, ConfineError> {
    if policy.network_level == NetworkLevel::ProxyTcp && proxy_port.is_none() {
        return Err(ConfineError::MissingProxyPort);
    }

    let mut out = String::from("(version 1)\n(deny default)\n");
    out.push_str("(allow process-info* (target self))\n");
    out.push_str("(allow signal (target self))\n");
    out.push_str("(allow sysctl-read)\n");
    if policy.network_level != NetworkLevel::None || policy.limits.processes > 1 {
        out.push_str("(allow process-fork)\n");
    }

    // Read-only system runtime libraries and basic device nodes needed to load
    // dynamic executables and language runtimes on macOS.
    out.push_str(
        "(allow file-read*\n\
         \x20 (subpath \"/usr/lib\")\n\
         \x20 (subpath \"/usr/share\")\n\
         \x20 (subpath \"/System/Library\")\n\
         \x20 (subpath \"/Library/Apple\")\n\
         \x20 (literal \"/dev/null\")\n\
         \x20 (literal \"/dev/urandom\")\n\
         \x20 (literal \"/dev/random\")\n\
         \x20 (literal \"/dev/zero\"))\n",
    );
    out.push_str("(allow file-write-data (literal \"/dev/null\"))\n");
    out.push_str("(allow process-exec (literal \"/usr/lib/dyld\"))\n");

    // Ancestor metadata traversal (stat/realpath only, never reading directory
    // contents or files) for granted paths.
    let mut ancestors = BTreeSet::new();
    for path in policy
        .reads
        .iter()
        .chain(&policy.writes)
        .chain(&policy.executables)
    {
        for variant in path_variants(path)? {
            let mut cursor = variant.as_path();
            while let Some(parent) = cursor.parent() {
                if let Some(text) = parent.to_str()
                    && !text.is_empty()
                {
                    ancestors.insert(text.to_owned());
                }
                cursor = parent;
            }
        }
    }
    if !ancestors.is_empty() {
        out.push_str("(allow file-read-metadata");
        for ancestor in &ancestors {
            let escaped = escape_sbpl(ancestor);
            let _ = write!(out, "\n  (literal \"{escaped}\")");
        }
        out.push_str(")\n");
    }

    // Granted reads (bundle, runtime, manifest files).
    for path in &policy.reads {
        for variant in path_variants(path)? {
            let escaped = escape_sbpl(&path_to_utf8(&variant)?);
            let _ = writeln!(out, "(allow file-read* (subpath \"{escaped}\"))");
        }
    }

    // Granted writes (instance private temporary folders).
    for path in &policy.writes {
        for variant in path_variants(path)? {
            let escaped = escape_sbpl(&path_to_utf8(&variant)?);
            let _ = writeln!(
                out,
                "(allow file-read* file-write* (subpath \"{escaped}\"))"
            );
        }
    }

    // Granted executables (bundle entrypoint, runtime, manifest processes).
    for path in &policy.executables {
        for variant in path_variants(path)? {
            let escaped = escape_sbpl(&path_to_utf8(&variant)?);
            let _ = writeln!(
                out,
                "(allow file-read* process-exec (subpath \"{escaped}\"))"
            );
        }
    }

    // Network and Mach service rules per network level (ADR 0033).
    out.push_str("(deny network*)\n");
    match policy.network_level {
        NetworkLevel::None => {
            emit_deny_dns_mach(&mut out);
        }
        NetworkLevel::ProxyTcp => {
            let port = proxy_port.expect("invariant: checked Some above for ProxyTcp");
            let _ = writeln!(
                out,
                "(allow network-outbound (remote tcp \"localhost:{port}\"))"
            );
            emit_deny_dns_mach(&mut out);
        }
        NetworkLevel::Direct => {
            out.push_str("(allow network-outbound (remote ip))\n");
            out.push_str(
                "(allow network-outbound (remote unix-socket (path-literal \"/private/var/run/mDNSResponder\")))\n",
            );
            out.push_str("(allow mach-lookup");
            for service in DNS_MACH_SERVICES {
                let _ = write!(out, "\n  (global-name \"{service}\")");
            }
            out.push_str(")\n");
        }
    }

    Ok(out)
}

fn emit_deny_dns_mach(out: &mut String) {
    out.push_str("(deny mach-lookup");
    for service in DNS_MACH_SERVICES {
        let _ = write!(out, "\n  (global-name \"{service}\")");
    }
    out.push_str(")\n");
}

fn path_to_utf8(path: &Path) -> Result<String, ConfineError> {
    let Some(text) = path.to_str() else {
        return Err(ConfineError::InvalidPath(path.to_path_buf()));
    };
    if text.contains('\0') {
        return Err(ConfineError::InvalidPath(path.to_path_buf()));
    }
    Ok(text.to_owned())
}

/// On macOS, `/var` and `/tmp` are symlinks to `/private/var` and
/// `/private/tmp`; emitting both ensures Seatbelt matches whichever spelling
/// the kernel resolves.
fn path_variants(path: &Path) -> Result<Vec<PathBuf>, ConfineError> {
    let text = path_to_utf8(path)?;
    let mut variants = vec![path.to_path_buf()];
    if let Some(rest) = text.strip_prefix("/var/") {
        variants.push(PathBuf::from(format!("/private/var/{rest}")));
    } else if text == "/var" {
        variants.push(PathBuf::from("/private/var"));
    } else if let Some(rest) = text.strip_prefix("/tmp/") {
        variants.push(PathBuf::from(format!("/private/tmp/{rest}")));
    } else if text == "/tmp" {
        variants.push(PathBuf::from("/private/tmp"));
    } else if let Some(rest) = text.strip_prefix("/private/var/") {
        variants.push(PathBuf::from(format!("/var/{rest}")));
    } else if let Some(rest) = text.strip_prefix("/private/tmp/") {
        variants.push(PathBuf::from(format!("/tmp/{rest}")));
    }
    Ok(variants)
}

fn escape_sbpl(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Confines the calling process on macOS to `policy` using Seatbelt
/// (`sandbox_init`).
///
/// # Errors
///
/// Returns [`ConfineError::Unavailable`] when this host refuses Seatbelt
/// confinement (e.g., nested inside an existing sandbox or `EPERM`), never
/// running weakened.
#[cfg(target_os = "macos")]
pub fn confine_self(policy: &Policy, proxy_port: Option<u16>) -> Result<Mechanism, ConfineError> {
    let text = profile(policy, proxy_port)?;
    let Ok(c_profile) = std::ffi::CString::new(text) else {
        return Err(ConfineError::Apply(
            "generated Seatbelt profile contained an interior NUL".to_owned(),
        ));
    };
    match seatbelt::apply(&c_profile) {
        Ok(()) => Ok(Mechanism::Seatbelt),
        Err(seatbelt::ApplyFailure::Unavailable(reason)) => Err(ConfineError::Unavailable(reason)),
        Err(seatbelt::ApplyFailure::Other(reason)) => Err(ConfineError::Apply(reason)),
    }
}

/// Whether the live macOS process `pid` is confined under a Seatbelt sandbox
/// (`sandbox_check(pid, NULL, SANDBOX_FILTER_NONE) == 1`).
#[cfg(target_os = "macos")]
#[must_use]
pub fn is_sandboxed(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(seatbelt::is_sandboxed)
}

/// Closes every open file descriptor above stderr (FD >= 3) before replacing
/// the launcher with the Plugin.
#[cfg(target_os = "macos")]
pub(crate) fn close_beyond_stdio() -> std::io::Result<()> {
    seatbelt::close_beyond_stdio()
}

/// The committed macOS Seatbelt probe table for `level` (ADR 0067).
#[must_use]
pub fn committed_probes(level: NetworkLevel) -> &'static str {
    match level {
        NetworkLevel::None => include_str!("../probes/macos/none.toml"),
        NetworkLevel::ProxyTcp => include_str!("../probes/macos/proxy_tcp.toml"),
        NetworkLevel::Direct => include_str!("../probes/macos/direct.toml"),
    }
}

/// One macOS network level's probe cases (ADR 0067): Seatbelt operations, the
/// signal rule, every macOS deny-list entry, and every policy file rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacOsProbeTable {
    pub operation: Vec<OperationCase>,
    pub path: Vec<MacOsPathCase>,
}

impl MacOsProbeTable {
    /// Parses a committed macOS probe table from TOML.
    ///
    /// # Errors
    ///
    /// Returns a TOML parse error if the table is malformed.
    pub fn read(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

/// One Seatbelt operation probe case with its unconfined positive control and
/// exact confined outcome under Seatbelt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCase {
    pub rule: String,
    pub operation: SeatbeltOperation,
    pub unconfined: Outcome,
    pub confined: Outcome,
}

/// One macOS filesystem probe case (deny-list entry or file rule) with its
/// unconfined positive control and exact confined outcome under Seatbelt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacOsPathCase {
    pub rule: String,
    pub access: Access,
    pub path: Placed,
    pub unconfined: Outcome,
    pub seatbelt: Outcome,
}

/// Checks `table` against `rules`, `denied` (`deny_list::on(Host::MacOs)`),
/// `FILE_RULES`, and `SIGNAL_RULE` (ADR 0067): returns empty when every rule
/// has a probe, no probe lacks a rule, and every refusal keeps a positive
/// control.
#[must_use]
pub fn check_probes(
    table: &MacOsProbeTable,
    level_rules: &[SeatbeltRule],
    denied: &[&Denied],
) -> Vec<Mismatch> {
    let op_cases: Vec<OsOpCase<'_>> = table
        .operation
        .iter()
        .map(|case| OsOpCase {
            rule: &case.rule,
            is_signal_op: case.operation == SeatbeltOperation::SignalOutside,
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
            confined: case.seatbelt,
        })
        .collect();
    let op_rules: Vec<OsOpRule<'_>> = level_rules
        .iter()
        .map(|rule| OsOpRule {
            name: rule.name,
            verdict: rule.verdict,
        })
        .collect();
    check_os_table(&op_cases, &path_cases, &op_rules, denied, Errno::Eperm)
}

#[cfg(test)]
mod tests;
