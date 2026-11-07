//! Chromium's sandbox, probed at startup (ADRs 0017, 0033 and 0049; spec
//! #564's `sandbox_probe`): whether this host lets a process create the
//! user namespace Chromium's Linux sandbox starts in, which ADR 0033's
//! Plugin confinement reads too. The probe runs once per process, in a
//! child of its own, and its answer decides each browser launch:
//!
//! - the namespace starts, or the OS is not Linux, whose sandboxes need
//!   none: the browser launches sandboxed;
//! - a default seccomp profile refuses it, Docker's or containerd's, or
//!   Kubernetes' `RuntimeDefault` in a pod: browser runs are refused with
//!   `browser_sandbox_unavailable`, naming the environment and its fix;
//! - Ubuntu's AppArmor restriction on unprivileged user namespaces holds
//!   it: a profile that grants `userns` to the browser's executable lifts
//!   it for that executable alone, so each launch reads the label its
//!   browser runs under and refuses one the restriction holds;
//! - a platform that allows no sandbox and offers no fix (AWS Lambda,
//!   Fargate, Azure Container Apps): `security.browser_sandbox` is off by
//!   default on this host, which every Run then reports as a switched-off
//!   control (one Audit log entry, `weakened`, one warning);
//! - any other refusal: browser runs are refused with
//!   `browser_sandbox_unavailable`, environment `unknown`, whose message
//!   names what the probe met.

#[cfg(target_os = "linux")]
mod linux;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::OnceLock;

use goodfetch_protocol::security::{
    BROWSER_SANDBOX_UNAVAILABLE, BrowserSandboxUnavailableDetails, SECURITY_BROWSER_SANDBOX,
    SandboxEnvironment,
};
use serde_json::Value;

use crate::call_error::CallError;
use crate::settings::HostDefaults;

/// What the probe found of the user namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserNamespace {
    /// Created, and this user mapped into it.
    Created,
    /// `unshare(CLONE_NEWUSER)` refused, with its `errno`.
    Refused(i32),
    /// Created, but this user could not be mapped into it, with the
    /// `errno` of the write to `uid_map`: AppArmor's restriction leaves the
    /// namespace without the capabilities to map itself.
    Unmapped(i32),
}

/// A platform that allows no sandbox and offers no fix (ADR 0049).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoSandboxPlatform {
    /// AWS Lambda.
    AwsLambda,
    /// AWS Fargate under ECS.
    Fargate,
    /// Azure Container Apps.
    AzureContainerApps,
}

impl NoSandboxPlatform {
    /// Its name in logs and messages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwsLambda => "AWS Lambda",
            Self::Fargate => "AWS Fargate",
            Self::AzureContainerApps => "Azure Container Apps",
        }
    }
}

/// What the probe read of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    /// The user namespace a child of this process tried to make.
    pub user_namespace: UserNamespace,
    /// Whether this process runs under a seccomp filter (`Seccomp: 2`).
    pub seccomp_filtered: bool,
    /// Whether AppArmor restricts unprivileged user namespaces
    /// (`kernel.apparmor_restrict_unprivileged_userns=1`).
    pub apparmor_restricts: bool,
    /// Whether this process runs in a Kubernetes pod
    /// (`KUBERNETES_SERVICE_HOST`).
    pub kubernetes: bool,
    /// The platform that allows no sandbox, when this is one.
    pub platform: Option<NoSandboxPlatform>,
}

/// What the probe decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probed {
    /// The sandbox starts.
    Starts,
    /// AppArmor's restriction holds: each browser executable's profile
    /// decides.
    Restricted,
    /// Refused where a fix exists: browser runs are refused.
    Refused {
        /// What refuses it.
        environment: SandboxEnvironment,
        /// What the probe met.
        failure: String,
    },
    /// The platform allows no sandbox: it is switched off by default.
    NoSandbox(NoSandboxPlatform),
    /// Refused for a reason the probe does not know.
    Unknown {
        /// What the probe met.
        failure: String,
    },
}

/// What `facts` decide.
pub fn judge(facts: &Facts) -> Probed {
    let failure = match facts.user_namespace {
        UserNamespace::Created => return Probed::Starts,
        UserNamespace::Refused(errno) => format!(
            "creating a user namespace was refused ({})",
            std::io::Error::from_raw_os_error(errno)
        ),
        UserNamespace::Unmapped(errno) => format!(
            "mapping this user into a new user namespace was refused ({})",
            std::io::Error::from_raw_os_error(errno)
        ),
    };
    if let Some(platform) = facts.platform {
        return Probed::NoSandbox(platform);
    }
    let refused = matches!(facts.user_namespace, UserNamespace::Refused(_));
    if refused && facts.seccomp_filtered {
        let environment = if facts.kubernetes {
            SandboxEnvironment::KubernetesRuntimeDefault
        } else {
            SandboxEnvironment::DockerDefaultSeccomp
        };
        return Probed::Refused {
            environment,
            failure,
        };
    }
    if facts.apparmor_restricts {
        return Probed::Restricted;
    }
    Probed::Unknown { failure }
}

/// The probe's answer for this process, found once: by the engine at
/// startup, in its `environment` map, which `main` read (ADR 0083). The
/// first call blocks while a child tries the user namespace.
pub fn probed_in(environment: &BTreeMap<OsString, OsString>) -> &'static Probed {
    PROBED.get_or_init(|| {
        let probed = probe(environment);
        log(&probed);
        probed
    })
}

/// The probe's answer for this process: the engine's, found at its
/// startup; probed now, in an empty environment, where no engine of this
/// process has.
pub fn probed() -> &'static Probed {
    PROBED.get().unwrap_or_else(|| probed_in(&BTreeMap::new()))
}

static PROBED: OnceLock<Probed> = OnceLock::new();

fn log(probed: &Probed) {
    match probed {
        Probed::Starts => tracing::debug!("Chromium's sandbox can start on this host"),
        Probed::Restricted => tracing::debug!(
            "AppArmor restricts unprivileged user namespaces on this host: each browser's own AppArmor profile decides whether its sandbox starts"
        ),
        Probed::Refused {
            environment,
            failure,
        } => tracing::warn!(
            environment = serde_name(*environment),
            "Chromium's sandbox cannot start on this host, so browser runs are refused: {failure}"
        ),
        // Each Run reports the switch with the one warning of a switched-off
        // control.
        Probed::NoSandbox(platform) => tracing::info!(
            platform = platform.as_str(),
            "{} allows no browser sandbox, so `security.browser_sandbox` is off by default on this host",
            platform.as_str()
        ),
        Probed::Unknown { failure } => tracing::warn!(
            environment = serde_name(SandboxEnvironment::Unknown),
            "Chromium's sandbox cannot start on this host, for a reason the probe does not recognise, so browser runs are refused: {failure}"
        ),
    }
}

#[cfg(target_os = "linux")]
fn probe(environment: &BTreeMap<OsString, OsString>) -> Probed {
    judge(&linux::facts(environment))
}

/// Outside Linux, Chromium's sandboxes need no user namespace.
#[cfg(not(target_os = "linux"))]
fn probe(_environment: &BTreeMap<OsString, OsString>) -> Probed {
    Probed::Starts
}

/// `defaults`, with `security.browser_sandbox` off where `probed` found a
/// platform that allows none.
#[must_use]
pub fn host_defaults(defaults: HostDefaults, probed: &Probed) -> HostDefaults {
    match probed {
        Probed::NoSandbox(_) => defaults.with(&SECURITY_BROWSER_SANDBOX, Value::Bool(false)),
        _ => defaults,
    }
}

impl Probed {
    /// The refusal of a browser launch with the sandbox on, where the probe
    /// refused it, whether or not it recognised why.
    pub fn refusal(&self) -> Option<CallError> {
        match self {
            Self::Refused {
                environment,
                failure,
            } => Some(refused(*environment, failure, None)),
            Self::Unknown { failure } => Some(refused(SandboxEnvironment::Unknown, failure, None)),
            Self::Starts | Self::Restricted | Self::NoSandbox(_) => None,
        }
    }
}

/// Whether a browser running under the AppArmor `label`, as
/// `/proc/<pid>/attr/current` reads, is held by the restriction on
/// unprivileged user namespaces: unconfined, or stacked with the
/// `unprivileged_userns` profile the restriction moves it to.
pub fn label_holds_user_namespaces(label: &str) -> bool {
    let label = label.trim().trim_end_matches(char::from(0)).trim();
    let profiles = label
        .split_once(" (")
        .map_or(label, |(profiles, _)| profiles);
    profiles == "unconfined"
        || profiles
            .split("//&")
            .any(|profile| profile == "unprivileged_userns")
}

/// The AppArmor label the process `pid` runs under, where AppArmor is on.
pub fn label_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::label_of(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// The refusal of the browser at `executable`, which AppArmor's restriction
/// holds.
pub fn apparmor_refusal(executable: &std::path::Path, label: &str) -> CallError {
    refused(
        SandboxEnvironment::ApparmorUserns,
        &format!(
            "AppArmor restricts unprivileged user namespaces, and the browser runs under `{}`",
            label.trim().trim_end_matches(char::from(0))
        ),
        Some(executable),
    )
}

fn serde_name(environment: SandboxEnvironment) -> &'static str {
    match environment {
        SandboxEnvironment::DockerDefaultSeccomp => "docker_default_seccomp",
        SandboxEnvironment::KubernetesRuntimeDefault => "kubernetes_runtime_default",
        SandboxEnvironment::ApparmorUserns => "apparmor_userns",
        SandboxEnvironment::Unknown => "unknown",
    }
}

/// The fix for `environment`, naming `executable` where one is known.
fn fix(environment: SandboxEnvironment, executable: Option<&std::path::Path>) -> String {
    let off =
        "or switch `security.browser_sandbox` off in Operator policy, which every run then reports";
    match environment {
        SandboxEnvironment::DockerDefaultSeccomp => format!(
            "Docker's and containerd's default seccomp profiles refuse the user namespace Chromium's sandbox starts in: run the container with a seccomp profile that allows `clone` and `unshare` with `CLONE_NEWUSER` (`docker run --security-opt seccomp=<profile.json>`), {off}."
        ),
        SandboxEnvironment::KubernetesRuntimeDefault => format!(
            "Kubernetes' `RuntimeDefault` seccomp profile refuses the user namespace Chromium's sandbox starts in: give the pod a `Localhost` seccomp profile that allows `clone` and `unshare` with `CLONE_NEWUSER` (`securityContext.seccompProfile: {{type: Localhost, localhostProfile: <profile.json>}}`), {off}."
        ),
        SandboxEnvironment::ApparmorUserns => format!(
            "Ubuntu's AppArmor restricts unprivileged user namespaces (`kernel.apparmor_restrict_unprivileged_userns=1`), and no AppArmor profile gives one to {}: load a profile that grants it `userns`, as Chromium's docs/security/apparmor-userns-restrictions.md shows, {off}.",
            executable.map_or_else(
                || "the browser".to_owned(),
                |executable| format!("`{}`", executable.display())
            )
        ),
        SandboxEnvironment::Unknown => format!(
            "This host refuses the user namespace Chromium's sandbox starts in, for a reason the probe does not recognise: check that `unshare --user --map-root-user true` works for this user (`user.max_user_namespaces` above 0, no seccomp or LSM rule refusing `CLONE_NEWUSER`), {off}."
        ),
    }
}

fn refused(
    environment: SandboxEnvironment,
    failure: &str,
    executable: Option<&std::path::Path>,
) -> CallError {
    CallError::new(
        &BROWSER_SANDBOX_UNAVAILABLE,
        format!(
            "Chromium's sandbox cannot start on this host ({}): {failure}",
            serde_name(environment)
        ),
        &BrowserSandboxUnavailableDetails {
            environment,
            fix: fix(environment, executable),
        },
    )
}

#[cfg(test)]
mod tests;
