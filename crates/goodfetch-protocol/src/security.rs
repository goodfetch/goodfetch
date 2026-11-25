//! The switchable security controls (ADRs 0016, 0028 and 0102): one
//! `security.*` setting for each security control that costs speed or
//! resources, on by default, which the program's own code, configuration
//! and an Operator policy may switch off where an Operator policy or a
//! Recipe does not lock or cap it; a Recipe and a Client acting for a model
//! never do. A run that runs with one off lists it in its Run report's
//! `weakened`, records the switch once in the Audit log and logs one
//! warning. Controls with no run-time cost have no setting: they cannot be
//! switched off.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn on() -> Value {
    Value::Bool(true)
}

fn switched_off(value: &Value) -> bool {
    value == &Value::Bool(false)
}

/// A switchable control's setting: a boolean, on by default, whose `false`
/// weakens the run, so that a Recipe or a Client acting for a model only
/// switches it on (#2445). `unbuilt` names the feature its `false` asks for
/// while the engine does not build its switch: the engine then does what the
/// control says whatever its value, so off is refused where it would be
/// reported as a weakened Run that was not one.
const fn control(
    name: &'static str,
    description: &'static str,
    unbuilt: Option<&'static str>,
) -> Setting {
    Setting {
        name,
        description,
        kind: ValueKind::Boolean,
        default: on,
        clearable: false,
        authority: Authority::NotWeakenedByRecipeOrModel,
        weakens: Some(switched_off),
        fixed_for_run: false,
        unbuilt,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }
}

/// `security.field_encryption` (ADRs 0015 and 0016).
pub const SECURITY_FIELD_ENCRYPTION: Setting = control(
    "security.field_encryption",
    "Whether the run's state seals its secret fields with the run's data key (ADR 0015). On by default; off, they are written in the clear, and the run says so in its report's `weakened`, in the Audit log and in one warning. A Recipe or a Client acting for a model never switches it off: its `false` is `setting_locked`.",
    Some("unsealed_fields"),
);

/// `security.secure_delete` (ADRs 0015 and 0016).
pub const SECURITY_SECURE_DELETE: Setting = control(
    "security.secure_delete",
    "Whether the run's secret tables overwrite what they delete, SQLite's `secure_delete` (ADR 0015). On by default; off, deleted values may stay on disk until overwritten, and the run says so in its report's `weakened`, in the Audit log and in one warning. A Recipe or a Client acting for a model never switches it off: its `false` is `setting_locked`.",
    Some("plain_delete"),
);

/// `security.resolved_ip_check` (ADRs 0016 and 0017).
pub const SECURITY_RESOLVED_IP_CHECK: Setting = control(
    "security.resolved_ip_check",
    "Whether every address a name resolves to is checked against the egress policy before a connection is made (ADR 0017). On by default; off, the domain policy alone applies, and the run says so in its report's `weakened`, in the Audit log and in one warning. A Recipe or a Client acting for a model never switches it off: its `false` is `setting_locked`.",
    None,
);

/// `security.raw_cdp_egress_proxy` (ADRs 0016 and 0017).
pub const SECURITY_RAW_CDP_EGRESS_PROXY: Setting = control(
    "security.raw_cdp_egress_proxy",
    "Whether every browser Session the engine launches reaches the network through the egress relay, which holds a raw CDP client or a Lent page at the network layer (ADR 0017). On by default; off, a Session with no certificate exception and no local address connects directly, held by interception alone, and the run says so in its report's `weakened`, in the Audit log and in one warning. It never removes the relay that carries certificate exceptions or local addresses. A Recipe or a Client acting for a model never switches it off: its `false` is `setting_locked`.",
    None,
);

/// `security.browser_sandbox` (ADRs 0016, 0017 and 0049): switched off
/// only by Operator policy or configuration, never by a Recipe or a run,
/// and off by default only on a host whose platform allows no sandbox.
pub const SECURITY_BROWSER_SANDBOX: Setting = Setting {
    authority: Authority::Configuration,
    ..control(
        "security.browser_sandbox",
        "Whether Chromium's renderers run in their sandbox (ADRs 0017 and 0049): the engine never passes `--no-sandbox` while it is on. On by default, except on a host whose platform allows no sandbox (AWS Lambda, Fargate, Azure Container Apps), where the engine switches it off. Only an Operator policy or configuration switches it off; a Recipe's or a run's value is `setting_locked`. Off, the renderers run unsandboxed, and the run says so in its report's `weakened`, in the Audit log and in one warning.",
        None,
    )
};

/// A host where Chromium's sandbox cannot start, and a fix exists (ADR
/// 0049).
pub const BROWSER_SANDBOX_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "browser_sandbox_unavailable",
    description: "Chromium's sandbox cannot start on this host, so the engine launches no browser. `environment` names what refuses it (Docker's or containerd's default seccomp profile, Kubernetes' `RuntimeDefault`, Ubuntu's AppArmor restriction on unprivileged user namespaces, or `unknown` for a refusal the probe does not recognise, which the message names), and `fix` the fix; an Operator policy may instead switch `security.browser_sandbox` off, which the run reports.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What refuses the user namespace Chromium's sandbox starts in, on a host
/// where a fix exists or the probe does not recognise the refusal (ADR
/// 0049).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEnvironment {
    /// Docker's or containerd's default seccomp profile, which refuses
    /// `CLONE_NEWUSER`.
    DockerDefaultSeccomp,
    /// Kubernetes' `RuntimeDefault` seccomp profile, the container
    /// runtime's default.
    KubernetesRuntimeDefault,
    /// Ubuntu's AppArmor restriction on unprivileged user namespaces, with
    /// no profile that gives the browser one.
    ApparmorUserns,
    /// A refusal the probe does not recognise, such as
    /// `user.max_user_namespaces` at 0; the message names what it met.
    Unknown,
}

/// What `browser_sandbox_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserSandboxUnavailableDetails {
    /// What refuses the sandbox.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub environment: SandboxEnvironment,
    /// The fix for that environment.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// `security.audit_chain` (ADRs 0016 and 0055).
pub const SECURITY_AUDIT_CHAIN: Setting = control(
    "security.audit_chain",
    "Whether the run's Audit log entries are hash-chained (ADR 0016). On by default; off, that run's entries are written unchained, each marked `chained: false`, which `audit verify` steps over while it verifies the chain around them; the switch itself is a chained entry, and the run says so in its report's `weakened` and in one warning. A Recipe or a Client acting for a model never switches it off: its `false` is `setting_locked`.",
    None,
);

/// `security.redaction` (ADRs 0014 and 0016).
pub const SECURITY_REDACTION: Setting = control(
    "security.redaction",
    "Whether logs, traces and HAR files are redacted and screenshots and recordings kept as outputs are masked (ADR 0014). On by default; off lifts that guarantee for those outputs only: secrets still never reach the model, Plans, caches, the Effect journal or observations. The run says so in its report's `weakened`, in the Audit log and in one warning. A Recipe or a Client acting for a model never switches it off: its `false` is `setting_locked`.",
    None,
);

/// The switchable security controls, which the engine's own contribution
/// registers so that each one switched off is reported.
pub const SECURITY_CONTROLS: [&Setting; 7] = [
    &SECURITY_FIELD_ENCRYPTION,
    &SECURITY_SECURE_DELETE,
    &SECURITY_RESOLVED_IP_CHECK,
    &SECURITY_RAW_CDP_EGRESS_PROXY,
    &SECURITY_BROWSER_SANDBOX,
    &SECURITY_AUDIT_CHAIN,
    &SECURITY_REDACTION,
];

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[BROWSER_SANDBOX_UNAVAILABLE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        SECURITY_FIELD_ENCRYPTION,
        SECURITY_SECURE_DELETE,
        SECURITY_RESOLVED_IP_CHECK,
        SECURITY_RAW_CDP_EGRESS_PROXY,
        SECURITY_BROWSER_SANDBOX,
        SECURITY_AUDIT_CHAIN,
        SECURITY_REDACTION,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
