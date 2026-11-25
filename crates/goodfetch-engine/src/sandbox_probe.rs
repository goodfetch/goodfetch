//! Chromium's sandbox, probed at startup (ADRs 0017, 0033 and 0049; spec
//! #564's `sandbox_probe`): whether this host lets a process create the
//! user namespace Chromium's Linux sandbox starts in, which ADR 0033's
//! Plugin confinement reads too. What the host allows is read once per
//! process, in a child of its own; each engine judges it against its own
//! environment map, and its answer decides each of its browser launches:
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
//!   Fargate under ECS or EKS, Azure Container Apps), even in a pod under
//!   Kubernetes' `RuntimeDefault`: `security.browser_sandbox` is off by
//!   default on this host, which every Run then reports as a switched-off
//!   control (one Audit log entry, `weakened`, one warning);
//! - WSL1, which offers neither seccomp nor user namespaces: browser runs
//!   are refused, environment `wsl1`;
//! - any other refusal: browser runs are refused with
//!   `browser_sandbox_unavailable`, environment `unknown`, whose message
//!   names what the probe met.
//!
//! The same reading decides a process Plugin's confinement (ADR 0033):
//! where `goodfetch-confinement` finds the host confines none, neither by
//! Landlock ABI 6 nor by a user namespace, or on WSL1, every Plugin's start
//! is refused with `confinement_unavailable`, naming the environment the
//! browser's refusal names and its fix for a Plugin
//! ([`SandboxProbe::plugin_refusal`]).

#[cfg(target_os = "linux")]
mod linux;

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::sync::{Arc, OnceLock};

use goodfetch_confinement::host::SIGNAL_SCOPE_ABI;
pub use goodfetch_confinement::host::{HostFacts, UserNamespace};
use goodfetch_protocol::plugin::{CONFINEMENT_UNAVAILABLE, ConfinementUnavailableDetails};
use goodfetch_protocol::security::{
    BROWSER_SANDBOX_UNAVAILABLE, BrowserSandboxUnavailableDetails, SECURITY_BROWSER_SANDBOX,
    SandboxEnvironment,
};
use serde_json::Value;

use goodfetch_protocol::settings::EKS_FARGATE_PROFILE_VARIABLE;

use crate::call_error::CallError;
use crate::settings::HostDefaults;

/// A platform that allows no sandbox and offers no fix (ADR 0049).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoSandboxPlatform {
    /// AWS Lambda.
    AwsLambda,
    /// AWS Fargate under ECS.
    Fargate,
    /// AWS Fargate under EKS, which a pod reads from its own Fargate
    /// profile label.
    EksFargate,
    /// Azure Container Apps.
    AzureContainerApps,
}

impl NoSandboxPlatform {
    /// Its name in logs and messages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwsLambda => "AWS Lambda",
            Self::Fargate => "AWS Fargate",
            Self::EksFargate => "EKS on Fargate",
            Self::AzureContainerApps => "Azure Container Apps",
        }
    }
}

/// What the probe read of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    /// What this process's host allows, alike for every engine in it.
    pub host: HostFacts,
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
    let host = &facts.host;
    if host.wsl1 {
        return Probed::Refused {
            environment: SandboxEnvironment::Wsl1,
            failure: "this is WSL1, which offers neither seccomp nor user namespaces".to_owned(),
        };
    }
    let failure = match host.user_namespace {
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
    let refused = matches!(host.user_namespace, UserNamespace::Refused(_));
    if refused && host.seccomp_filtered {
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
    if host.apparmor_restricts {
        return Probed::Restricted;
    }
    Probed::Unknown { failure }
}

impl Facts {
    /// The refusal of every process Plugin's start, where this host cannot
    /// confine one: it names the environment that refuses the browser's
    /// sandbox too, whatever platform this is, and the fix for a Plugin.
    pub fn plugin_refusal(&self) -> Option<CallError> {
        if self.host.confines() {
            return None;
        }
        let (environment, failure) = match judge(&Self {
            platform: None,
            ..*self
        }) {
            Probed::Refused {
                environment,
                failure,
            } => (environment, failure),
            Probed::Restricted => (
                SandboxEnvironment::ApparmorUserns,
                "AppArmor restricts unprivileged user namespaces".to_owned(),
            ),
            Probed::Unknown { failure } => (SandboxEnvironment::Unknown, failure),
            // A namespace made confines a Plugin, and only WSL1 refuses one
            // then, which `judge` names.
            Probed::Starts | Probed::NoSandbox(_) => (
                SandboxEnvironment::Unknown,
                "the probe met no refusal it names".to_owned(),
            ),
        };
        let failure = match environment {
            SandboxEnvironment::Wsl1 => failure,
            _ => format!(
                "{failure}, and the kernel gives Landlock ABI {}, below the {SIGNAL_SCOPE_ABI} that confines without one",
                self.host.landlock_abi
            ),
        };
        Some(CallError::new(
            &CONFINEMENT_UNAVAILABLE,
            format!(
                "this host cannot confine a process Plugin ({}): {failure}",
                serde_name(environment)
            ),
            &ConfinementUnavailableDetails {
                environment,
                fix: plugin_fix(environment),
            },
        ))
    }
}

/// One engine's probe, which its settings' host defaults and its browser
/// launches read: its answer is found once, from what the host allows and
/// the engine's `environment` map, which `main` read (ADR 0083). Clones
/// share it.
#[derive(Clone)]
pub struct SandboxProbe(Arc<Probing>);

struct Probing {
    environment: BTreeMap<OsString, OsString>,
    host: fn() -> HostFacts,
    answer: OnceLock<(Facts, Probed)>,
}

impl SandboxProbe {
    /// The probe of an engine started with `environment`, on the host
    /// `host` reads: [`this_host`], for a real engine.
    pub(crate) fn of(host: fn() -> HostFacts, environment: BTreeMap<OsString, OsString>) -> Self {
        Self(Arc::new(Probing {
            environment,
            host,
            answer: OnceLock::new(),
        }))
    }

    /// The engine's answer for the browser's sandbox. The process's first
    /// call blocks while a child tries the user namespace, so it runs on
    /// the blocking policy.
    pub fn probed(&self) -> &Probed {
        &self.answer().1
    }

    /// The refusal of every process Plugin's start where this host cannot
    /// confine one ([`Facts::plugin_refusal`]), from the same reading as
    /// [`Self::probed`]; it blocks as that does.
    pub fn plugin_refusal(&self) -> Option<CallError> {
        self.answer().0.plugin_refusal()
    }

    fn answer(&self) -> &(Facts, Probed) {
        self.0.answer.get_or_init(|| {
            let facts = facts((self.0.host)(), &self.0.environment);
            let probed = judge(&facts);
            log(&probed);
            (facts, probed)
        })
    }
}

/// What this process's host allows, read once: no engine's environment
/// changes it, and the child's fork costs each read.
pub(crate) fn this_host() -> HostFacts {
    static HOST: OnceLock<HostFacts> = OnceLock::new();
    *HOST.get_or_init(read_host)
}

#[cfg(target_os = "linux")]
fn read_host() -> HostFacts {
    goodfetch_confinement::linux::host::read()
}

/// Outside Linux, Chromium's sandboxes and a Plugin's confinement need no
/// user namespace.
#[cfg(not(target_os = "linux"))]
fn read_host() -> HostFacts {
    HostFacts::NO_NAMESPACE_NEEDED
}

/// What the probe judges: what `host` allows, and where `environment`, an
/// engine's map, says it runs.
fn facts(host: HostFacts, environment: &BTreeMap<OsString, OsString>) -> Facts {
    let variable = |name: &str| {
        environment
            .get(OsStr::new(name))
            .and_then(|value| value.to_str())
            .map(str::to_owned)
    };
    Facts {
        host,
        kubernetes: variable("KUBERNETES_SERVICE_HOST").is_some(),
        platform: platform(variable),
    }
}

/// The platform that allows no sandbox, from the variables `variable`
/// reads, as each platform sets them.
fn platform(variable: impl Fn(&str) -> Option<String>) -> Option<NoSandboxPlatform> {
    if variable("AWS_LAMBDA_FUNCTION_NAME").is_some() {
        return Some(NoSandboxPlatform::AwsLambda);
    }
    if variable("AWS_EXECUTION_ENV").is_some_and(|value| value == "AWS_ECS_FARGATE") {
        return Some(NoSandboxPlatform::Fargate);
    }
    // EKS sets no variable: the pod passes its Fargate profile label, empty
    // on any other node.
    if variable(EKS_FARGATE_PROFILE_VARIABLE).is_some_and(|profile| !profile.is_empty()) {
        return Some(NoSandboxPlatform::EksFargate);
    }
    if variable("CONTAINER_APP_NAME").is_some() {
        return Some(NoSandboxPlatform::AzureContainerApps);
    }
    None
}

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
        SandboxEnvironment::Wsl1 => "wsl1",
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
        SandboxEnvironment::Wsl1 => format!(
            "WSL1 offers neither the seccomp filters nor the user namespace Chromium's sandbox needs: convert the distribution to WSL 2 (`wsl --set-version <distribution> 2`), {off}."
        ),
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

/// The fix for `environment` when it keeps a process Plugin from being
/// confined: from Linux 6.12, Landlock ABI 6 confines one without a user
/// namespace.
fn plugin_fix(environment: SandboxEnvironment) -> String {
    let kernel = "or run on a host with Linux 6.12 or later, whose Landlock ABI 6 confines a Plugin without a user namespace";
    match environment {
        SandboxEnvironment::Wsl1 => "WSL1 offers neither seccomp nor user namespaces, so no Plugin can be confined there: convert the distribution to WSL 2 (`wsl --set-version <distribution> 2`).".to_owned(),
        SandboxEnvironment::DockerDefaultSeccomp => format!(
            "Docker's and containerd's default seccomp profiles refuse the user namespace a Plugin is confined in on this kernel: run the container with a seccomp profile that allows `clone` and `unshare` with `CLONE_NEWUSER` (`docker run --security-opt seccomp=<profile.json>`), {kernel}."
        ),
        SandboxEnvironment::KubernetesRuntimeDefault => format!(
            "Kubernetes' `RuntimeDefault` seccomp profile refuses the user namespace a Plugin is confined in on this kernel: give the pod a `Localhost` seccomp profile that allows `clone` and `unshare` with `CLONE_NEWUSER` (`securityContext.seccompProfile: {{type: Localhost, localhostProfile: <profile.json>}}`), {kernel}."
        ),
        SandboxEnvironment::ApparmorUserns => format!(
            "Ubuntu's AppArmor restricts unprivileged user namespaces (`kernel.apparmor_restrict_unprivileged_userns=1`), and no AppArmor profile gives one to Goodfetch's executable, which confines each Plugin: load a profile that grants it `userns`, {kernel}."
        ),
        SandboxEnvironment::Unknown => format!(
            "This host refuses the user namespace a Plugin is confined in, for a reason the probe does not recognise: check that `unshare --user --map-root-user true` works for this user (`user.max_user_namespaces` above 0, no seccomp or LSM rule refusing `CLONE_NEWUSER`), {kernel}."
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
