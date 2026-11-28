//! Approving a Plugin (ADRs 0033 and 0035; issue 900): a third-party
//! Plugin loads only when the Operator policy allows it, by name and
//! optionally pinned to a digest, and each permission its manifest asks
//! for is approved on its own, as `plugins.<name>.approved`. Each
//! permission states in plain words what it lets the Plugin do and what the
//! engine cannot check, which the person reads before approving it.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, SharedType, SharedTypeValue,
};
use crate::plugin::{Grant, NetworkLevel, PluginManifest};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// One permission a Plugin's manifest asks for, which the Operator policy
/// approves on its own.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "permission", rename_all = "snake_case", deny_unknown_fields)]
pub enum Permission {
    /// Causing Effects on one domain.
    Effects {
        /// The domain.
        #[schemars(extend("arrived_in" = "0.1"))]
        domain: String,
    },
    /// Running script in the pages of one domain.
    PageScript {
        /// The domain.
        #[schemars(extend("arrived_in" = "0.1"))]
        domain: String,
    },
    /// The logged-in Session's credentials for one domain.
    SessionCredentials {
        /// The domain.
        #[schemars(extend("arrived_in" = "0.1"))]
        domain: String,
    },
    /// One Secret reference, whose value the engine inserts on the way out.
    Secret {
        /// The Secret reference, such as `env:TOKEN`.
        #[schemars(extend("arrived_in" = "0.1"))]
        reference: String,
    },
    /// One Secret reference's value itself, released to the Plugin.
    SecretValue {
        /// The Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        reference: String,
    },
    /// A browser the Plugin adds, for one platform, by its digest.
    Browser {
        /// The browser's name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
        /// The platform, such as `linux-x64`.
        #[schemars(extend("arrived_in" = "0.1"))]
        platform: String,
        /// The download's digest, `sha256:<hex>`.
        #[schemars(extend("arrived_in" = "0.1"))]
        digest: String,
    },
    /// The network directly, at the `direct` level.
    Direct,
}

impl Permission {
    /// Each permission `manifest` asks for, in this order: `effects`,
    /// `page_script` and `session_credentials` for each of its domains, its
    /// Secret references, the value of each when it asks for
    /// `secret_value`, each browser's download per platform, then
    /// `direct`.
    #[must_use]
    pub fn needed_by(manifest: &PluginManifest) -> Vec<Self> {
        let permissions = &manifest.permissions;
        let granted = |grant| permissions.grants.contains(&grant);
        let mut needed = Vec::new();
        for (grant, each) in [
            (
                Grant::Effects,
                (|domain| Self::Effects { domain }) as fn(String) -> Self,
            ),
            (Grant::PageScript, |domain| Self::PageScript { domain }),
            (Grant::SessionCredentials, |domain| {
                Self::SessionCredentials { domain }
            }),
        ] {
            if granted(grant) {
                needed.extend(permissions.domains.iter().cloned().map(each));
            }
        }
        needed.extend(permissions.secrets.iter().map(|reference| Self::Secret {
            reference: reference.clone(),
        }));
        if granted(Grant::SecretValue) {
            needed.extend(
                permissions
                    .secrets
                    .iter()
                    .map(|reference| Self::SecretValue {
                        reference: reference.clone(),
                    }),
            );
        }
        for browser in &manifest.browsers {
            needed.extend(
                browser
                    .platforms
                    .iter()
                    .map(|(platform, download)| Self::Browser {
                        name: browser.name.clone(),
                        platform: platform.clone(),
                        digest: download.digest.clone(),
                    }),
            );
        }
        if manifest.network_level == NetworkLevel::Direct {
            needed.push(Self::Direct);
        }
        needed
    }

    /// Its key under `plugins.<name>.approved`.
    #[must_use]
    pub fn field(&self) -> &'static str {
        match self {
            Self::Effects { .. } => "effects",
            Self::PageScript { .. } => "page_script",
            Self::SessionCredentials { .. } => "session_credentials",
            Self::Secret { .. } => "secrets",
            Self::SecretValue { .. } => "secret_values",
            Self::Browser { .. } => "browsers",
            Self::Direct => "direct",
        }
    }

    /// What it names: its domain, Secret reference or digest, none for
    /// `direct`.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Effects { domain }
            | Self::PageScript { domain }
            | Self::SessionCredentials { domain } => Some(domain),
            Self::Secret { reference } | Self::SecretValue { reference } => Some(reference),
            Self::Browser { digest, .. } => Some(digest),
            Self::Direct => None,
        }
    }
}

/// A permission with what it lets the Plugin do and what the engine cannot
/// check, in plain words, as an approval states them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatedPermission {
    /// The permission.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub permission: Permission,
    /// What it lets the Plugin do.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lets: String,
    /// What the engine cannot check once it is approved.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cannot_check: String,
}

/// `permission`, with what it lets the Plugin do and what the engine cannot
/// check (ADR 0033): a Secret reference's approval states both of its
/// limits.
#[must_use]
pub fn stated(permission: Permission) -> StatedPermission {
    let (lets, cannot_check) = match &permission {
        Permission::Effects { domain } => (
            format!("cause Effects on {domain}: page actions that change a page, navigations while logged in, and requests other than GET or HEAD, each through the Effect journal"),
            "whether an Effect is the one you meant: the engine holds it to the Effects rule and journals it, but does not judge its purpose".to_owned(),
        ),
        Permission::PageScript { domain } => (
            format!("run script in the pages of {domain}, and read their form values, storage and cookies"),
            "what the script does with what it reads inside the page".to_owned(),
        ),
        Permission::SessionCredentials { domain } => (
            format!("use the logged-in Session's credentials for {domain}"),
            "what the Plugin does with the credentials once they are released; the Audit log records each release".to_owned(),
        ),
        Permission::Secret { reference } => (
            format!("have the engine insert the value of {reference} into its requests, only toward the domains the run binds the Secret to"),
            "a bound service that stores or publishes what it receives (an issue body, a profile field) keeps the value where the Plugin's author might read it, and an encoding the scrub does not know returns it".to_owned(),
        ),
        Permission::SecretValue { reference } => (
            format!("receive the value of {reference} itself"),
            "where the Plugin sends the value: a released value leaves its domain binding; the Audit log records each release".to_owned(),
        ),
        Permission::Browser { name, platform, digest } => (
            format!("add the browser {name} for {platform}, whose download must match {digest}"),
            "what the browser does: the engine checks its download against the digest, not its behaviour".to_owned(),
        ),
        Permission::Direct => (
            "reach the network directly rather than through the engine".to_owned(),
            "its writes: the engine cannot hold what it sends directly to the Effects rule or the egress policy".to_owned(),
        ),
    };
    StatedPermission {
        permission,
        lets,
        cannot_check,
    }
}

/// The permissions the Operator policy approved for one Plugin,
/// `plugins.<name>.approved`: each domain, Secret reference and digest on
/// its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Approvals {
    /// The domains it may cause Effects on.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub effects: BTreeSet<String>,
    /// The domains whose pages it may run script in.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_script: BTreeSet<String>,
    /// The domains whose logged-in Session's credentials it may use.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_credentials: BTreeSet<String>,
    /// The Secret references it may use.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secrets: BTreeSet<String>,
    /// The Secret references whose value it may receive.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secret_values: BTreeSet<String>,
    /// The digests of the browser downloads it may add.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browsers: BTreeSet<String>,
    /// Whether it may reach the network directly.
    #[serde(skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub direct: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
pub(crate) fn is_false(flag: &bool) -> bool {
    !flag
}

impl Approvals {
    /// The set that approves `permission`'s kind, none for `direct`.
    fn set(&self, permission: &Permission) -> Option<&BTreeSet<String>> {
        Some(match permission {
            Permission::Effects { .. } => &self.effects,
            Permission::PageScript { .. } => &self.page_script,
            Permission::SessionCredentials { .. } => &self.session_credentials,
            Permission::Secret { .. } => &self.secrets,
            Permission::SecretValue { .. } => &self.secret_values,
            Permission::Browser { .. } => &self.browsers,
            Permission::Direct => return None,
        })
    }

    /// Whether `permission` is approved.
    #[must_use]
    pub fn covers(&self, permission: &Permission) -> bool {
        match (self.set(permission), permission.value()) {
            (Some(set), Some(value)) => set.contains(value),
            _ => self.direct,
        }
    }

    /// Approves `permission`, and nothing else.
    pub fn approve(&mut self, permission: &Permission) {
        let Some(value) = permission.value().map(str::to_owned) else {
            self.direct = true;
            return;
        };
        let set = match permission {
            Permission::Effects { .. } => &mut self.effects,
            Permission::PageScript { .. } => &mut self.page_script,
            Permission::SessionCredentials { .. } => &mut self.session_credentials,
            Permission::Secret { .. } => &mut self.secrets,
            Permission::SecretValue { .. } => &mut self.secret_values,
            Permission::Browser { .. } => &mut self.browsers,
            Permission::Direct => return,
        };
        set.insert(value);
    }

    /// Whether it approves nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Where a Plugin comes from, which sets whether it loads by default (ADRs
/// 0033 and 0035).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Shipped in the engine's own release: allowed by default.
    FirstParty,
    /// Anyone else's: loads only when the Operator policy allows it.
    #[default]
    ThirdParty,
    /// A Private component, which is never first-party: loads only when the
    /// Operator policy allows it.
    Private,
}

/// Why the Operator policy refuses a Plugin.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PluginRefusal {
    /// A Plugin that is not first-party, which the policy does not allow.
    #[error("the Operator policy does not allow it")]
    NotAllowed,
    /// A Plugin the policy denies.
    #[error("the Operator policy denies it")]
    Denied,
    /// A Plugin the policy pins to another digest.
    #[error("the Operator policy pins it to {pinned}")]
    Digest {
        /// The digest the policy pins.
        pinned: String,
    },
}

impl PluginRefusal {
    /// Its reason, as `plugin_not_allowed`'s details name it.
    #[must_use]
    pub fn reason(&self) -> NotAllowedReason {
        match self {
            Self::NotAllowed => NotAllowedReason::NotAllowed,
            Self::Denied => NotAllowedReason::Denied,
            Self::Digest { .. } => NotAllowedReason::Digest,
        }
    }
}

/// Whether the Operator policy lets a Plugin of `origin` and `digest` load,
/// as its entry's `allow` and `digest` say: a denied Plugin never loads, a
/// pinned one only at its pinned digest, and one the entry does not allow
/// only when it is first-party.
///
/// # Errors
///
/// [`PluginRefusal::Denied`] when `allow` is `false`;
/// [`PluginRefusal::Digest`] when `pinned` is not `digest`, or the digest
/// is unknown; otherwise [`PluginRefusal::NotAllowed`] for a Plugin that is
/// not first-party, which `allow` does not allow.
pub fn admit(
    origin: Origin,
    allow: Option<bool>,
    pinned: Option<&str>,
    digest: Option<&str>,
) -> Result<(), PluginRefusal> {
    if allow == Some(false) {
        return Err(PluginRefusal::Denied);
    }
    if let Some(pinned) = pinned
        && Some(pinned) != digest
    {
        return Err(PluginRefusal::Digest {
            pinned: pinned.to_owned(),
        });
    }
    if allow == Some(true) || origin == Origin::FirstParty {
        Ok(())
    } else {
        Err(PluginRefusal::NotAllowed)
    }
}

/// Why `plugin_not_allowed` refused a Plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotAllowedReason {
    /// The Operator policy does not allow it.
    NotAllowed,
    /// The Operator policy denies it.
    Denied,
    /// The Operator policy pins it to another digest.
    Digest,
}

/// A Plugin the Operator policy does not let load (ADRs 0033 and 0035).
pub const PLUGIN_NOT_ALLOWED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_not_allowed",
    description: "The Operator policy does not let the Plugin load: it is not first-party and `plugins.<name>.allow` is not `true`, `plugins.<name>.allow` is `false`, or `plugins.<name>.digest` pins it to another digest than the one installed. A Private component is never first-party. The details name the Plugin, the reason and, for a pin, both digests; the `fix` names the Operator policy entry to set, or `goodfetch plugin add <reference>`, which writes it after a person approves it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_not_allowed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotAllowedDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugin: String,
    /// Why it is refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: NotAllowedReason,
    /// The digest the Operator policy pins it to, for a pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pinned: Option<String>,
    /// The digest installed, for a pin, when the store records one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found: Option<String>,
    /// What lifts the refusal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A Plugin's call that needs a permission the Operator policy has not
/// approved (ADR 0033).
pub const PLUGIN_PERMISSION_DENIED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_permission_denied",
    description: "The Plugin needs a permission the Operator policy has not approved for it: `effects`, `page_script` or `session_credentials` for a domain, a Secret reference, a Secret reference's value, a browser's digest, or `direct`. Each is approved on its own, under `plugins.<name>.approved`. The details name the Plugin and the permission; the `fix` names the Operator policy entry to set, or `goodfetch plugin add <reference>`, which writes it after a person approves it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_permission_denied` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginPermissionDeniedDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugin: String,
    /// The permission it needs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub permission: Permission,
    /// What lifts the refusal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// The `fix` of a refusal of the Plugin `name`: the Operator policy entry
/// `entry` sets, `plugins.<name>.<entry>`, or the command that writes it.
#[must_use]
pub fn fix(name: &str, entry: &str) -> String {
    format!(
        "set `plugins.{name}.{entry}` in the Operator policy, or run `goodfetch plugin add <reference>` as a person to approve it"
    )
}

/// A kind of Audit log entry: a value of the shared type `audit_kind`.
const fn kind(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::AuditKind,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// A person approved a Plugin.
pub const PLUGIN_APPROVED: SharedTypeValue = kind(
    "plugin_approved",
    "A person approved a Plugin through `plugin.add`, which wrote `plugins.<name>` into the Operator policy: its name, its digest and each permission approved, with what it lets the Plugin do and what the engine cannot check.",
);

/// A Secret value released to a Plugin.
pub const PLUGIN_SECRET_VALUE_RELEASED: SharedTypeValue = kind(
    "plugin_secret_value_released",
    "The engine released a Secret reference's value to a Plugin, as the Operator policy's `plugins.<name>.approved.secret_values` allows: the Plugin, the reference and the Run, never the value.",
);

/// A Plugin used an Operator-approved permission (`effects`, `page_script` or
/// `session_credentials`) on a domain (ADR 0033).
pub const PLUGIN_PERMISSION_USED: SharedTypeValue = kind(
    "plugin_permission_used",
    "A Plugin performed an action under an Operator-approved permission (`effects`, `page_script` or `session_credentials`) on a domain: the Plugin, the permission, the domain, the method and the Run.",
);

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[PLUGIN_NOT_ALLOWED, PLUGIN_PERMISSION_DENIED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[
        PLUGIN_APPROVED,
        PLUGIN_SECRET_VALUE_RELEASED,
        PLUGIN_PERMISSION_USED,
    ],
};

#[cfg(test)]
mod tests;
