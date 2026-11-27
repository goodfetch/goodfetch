//! Updating an installed Plugin (ADRs 0033, 0052 and 0099; issue 924):
//! `plugin.update` (person-only) previews or applies an update to an installed
//! Plugin, returning the new digest, signer identity, whether the signer
//! changed, and the [`PermissionDiff`] listing added, widened and removed
//! permissions. Any added or widened permission requires explicit approval,
//! and a changed signer identity is refused with `plugin_signer_changed` until
//! the operator approves it again.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::plugin::add::PluginApproval;
use crate::plugin::approvals::{Permission, StatedPermission, is_false, stated};
use crate::plugin::index::SignerIdentity;
use crate::plugin::manifest::{NetworkLevel, PluginManifest};
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A permission or capability that widened between the installed manifest and
/// the target release (ADR 0033, issue 924).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WidenedPermission {
    /// What widened (`network_level`, `effects`, `page_script`,
    /// `session_credentials`, `secret_value`, or `browser`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// The previous narrower value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub from: String,
    /// The new wider value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub to: String,
    /// Plain-word statement of the widened permission when it maps to a
    /// [`Permission`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub statement: Option<StatedPermission>,
}

/// The diff of permissions between an installed Plugin manifest and a target
/// release manifest (ADR 0033, issue 924).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PermissionDiff {
    /// Permissions declared by the new release that were not in the installed
    /// manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: Vec<StatedPermission>,
    /// Permissions or capabilities that became wider in the new release (for
    /// example a higher `network_level`, a broader wildcard domain, a Secret
    /// escalated from injected reference to raw `secret_value`, or a changed
    /// browser download digest).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub widened: Vec<WidenedPermission>,
    /// Permissions declared by the installed manifest that are no longer
    /// requested by the new release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed: Vec<StatedPermission>,
    /// Whether this diff adds or widens any permission and therefore requires
    /// explicit person approval before applying.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub requires_approval: bool,
}

impl PermissionDiff {
    /// Computes the permission diff from `old` to `new`.
    #[must_use]
    pub fn between(old: &PluginManifest, new: &PluginManifest) -> Self {
        let old_perms: Vec<Permission> = Permission::needed_by(old);
        let new_perms: Vec<Permission> = Permission::needed_by(new);
        let old_set: BTreeSet<&Permission> = old_perms.iter().collect();
        let new_set: BTreeSet<&Permission> = new_perms.iter().collect();

        let mut added = Vec::new();
        let mut widened = Vec::new();
        let mut superseded: BTreeSet<Permission> = BTreeSet::new();

        if new.network_level > old.network_level {
            widened.push(WidenedPermission {
                kind: "network_level".to_owned(),
                from: old.network_level.as_str().to_owned(),
                to: new.network_level.as_str().to_owned(),
                statement: (new.network_level == NetworkLevel::Direct)
                    .then(|| stated(Permission::Direct)),
            });
        }

        for perm in &new_perms {
            if old_set.contains(perm) {
                continue;
            }
            if matches!(perm, Permission::Direct) && new.network_level > old.network_level {
                continue;
            }
            if let Some((item, replaced)) = find_widening(perm, &old_perms) {
                widened.push(item);
                if let Some(old_perm) = replaced {
                    superseded.insert(old_perm);
                }
            } else {
                added.push(stated(perm.clone()));
            }
        }

        let removed = old_perms
            .into_iter()
            .filter(|perm| !new_set.contains(perm) && !superseded.contains(perm))
            .map(stated)
            .collect();

        let requires_approval = !added.is_empty() || !widened.is_empty();
        Self {
            added,
            widened,
            removed,
            requires_approval,
        }
    }

    /// Whether no permissions were added, widened or removed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.widened.is_empty() && self.removed.is_empty()
    }
}

/// Checks whether `new_perm` widens an existing permission in `old_perms`,
/// returning the [`WidenedPermission`] and optionally the superseded old
/// permission so it is not also listed as removed.
fn find_widening(
    new_perm: &Permission,
    old_perms: &[Permission],
) -> Option<(WidenedPermission, Option<Permission>)> {
    match new_perm {
        Permission::Effects { domain } => {
            widened_domain("effects", domain, old_perms, new_perm, |p| match p {
                Permission::Effects { domain } => Some(domain.as_str()),
                _ => None,
            })
        }
        Permission::PageScript { domain } => {
            widened_domain("page_script", domain, old_perms, new_perm, |p| match p {
                Permission::PageScript { domain } => Some(domain.as_str()),
                _ => None,
            })
        }
        Permission::SessionCredentials { domain } => widened_domain(
            "session_credentials",
            domain,
            old_perms,
            new_perm,
            |p| match p {
                Permission::SessionCredentials { domain } => Some(domain.as_str()),
                _ => None,
            },
        ),
        Permission::SecretValue { reference } => {
            let had_injected = old_perms
                .iter()
                .any(|p| matches!(p, Permission::Secret { reference: r } if r == reference));
            had_injected.then(|| {
                (
                    WidenedPermission {
                        kind: "secret_value".to_owned(),
                        from: format!("secret:{reference} (injected on egress)"),
                        to: format!("secret_value:{reference} (raw value released)"),
                        statement: Some(stated(new_perm.clone())),
                    },
                    None,
                )
            })
        }
        Permission::Browser {
            name,
            platform,
            digest,
        } => old_perms.iter().find_map(|p| match p {
            Permission::Browser {
                name: old_name,
                platform: old_platform,
                digest: old_digest,
            } if old_name == name && old_platform == platform && old_digest != digest => Some((
                WidenedPermission {
                    kind: "browser".to_owned(),
                    from: format!("{old_name} ({old_platform}): {old_digest}"),
                    to: format!("{name} ({platform}): {digest}"),
                    statement: Some(stated(new_perm.clone())),
                },
                Some(p.clone()),
            )),
            _ => None,
        }),
        Permission::Secret { .. } | Permission::Direct => None,
    }
}

fn widened_domain(
    kind: &str,
    new_domain: &str,
    old_perms: &[Permission],
    new_perm: &Permission,
    extract: impl Fn(&Permission) -> Option<&str>,
) -> Option<(WidenedPermission, Option<Permission>)> {
    old_perms.iter().find_map(|old| {
        let old_domain = extract(old)?;
        domain_widens(old_domain, new_domain).then(|| {
            (
                WidenedPermission {
                    kind: kind.to_owned(),
                    from: old_domain.to_owned(),
                    to: new_domain.to_owned(),
                    statement: Some(stated(new_perm.clone())),
                },
                Some(old.clone()),
            )
        })
    })
}

/// Whether `new_domain` strictly widens `old_domain` (for example
/// `shop.test` -> `*.shop.test`, `api.shop.test` -> `*.shop.test`, or any
/// specific domain -> `*`).
fn domain_widens(old_domain: &str, new_domain: &str) -> bool {
    let old = old_domain.to_ascii_lowercase();
    let new = new_domain.to_ascii_lowercase();
    if old == new {
        return false;
    }
    if new == "*" {
        return true;
    }
    if let Some(root) = new.strip_prefix("*.") {
        if let Some(old_root) = old.strip_prefix("*.") {
            return old_root.ends_with(&format!(".{root}"));
        }
        return old == root || old.ends_with(&format!(".{root}"));
    }
    false
}

/// What `plugin.update` takes (ADR 0033, issue 924).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginUpdateParams {
    /// The target release: either a digest-pinned OCI reference
    /// `<registry>/<repository>@sha256:<hex>`, or a Plugin name listed in the
    /// cached Plugin index (which resolves to its pinned digest).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The project configuration the Client found, layer 3 (ADR 0083): the
    /// lockfile beside it is updated, else the Goodfetch home's lockfile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
    /// When `true`, inspect the target release and return its new digest,
    /// signer identity and permission diff without modifying the Plugin store,
    /// lockfile or Operator policy.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub preview: bool,
    /// Whether the person approved any added or widened permissions shown in
    /// the diff (writes the updated Operator policy entry).
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approve: bool,
    /// Whether the person explicitly approved a changed signer identity when
    /// the new release's verified signer differs from the previously recorded
    /// signer.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approve_signer_change: bool,
}

/// What `plugin.update` answers (ADR 0033, issue 924).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginUpdated {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The previously installed version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub previous_version: String,
    /// The target release's version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The previously installed image index digest, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub previous_digest: Option<String>,
    /// The target release's image index digest, `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The lockfile that pins this Plugin.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lockfile: String,
    /// The signer identity previously recorded in the lockfile, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub previous_signer: Option<String>,
    /// The verified signer identity of the target release, when attested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: Option<SignerIdentity>,
    /// Whether the target release's signer differs from the previously
    /// recorded signer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer_changed: bool,
    /// The added, widened and removed permissions between the installed
    /// release and the target release.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub diff: PermissionDiff,
    /// Whether the update was applied to the Plugin store and lockfile
    /// (`false` on a `preview` call).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub applied: bool,
    /// Where the person's updated approval went, when written or managed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approval: Option<PluginApproval>,
}

/// `plugin.update` (ADRs 0033, 0052 and 0099; issue 924).
pub const PLUGIN_UPDATE: Operation = Operation {
    name: "plugin.update",
    description: "Updates an installed Plugin to a new digest-pinned release (a person's answer, ADR 0099). With `preview: true`, pulls and inspects the target release without changing the Plugin store, lockfile or Operator policy, returning the new digest, verified signer identity, whether the signer changed, and the permission diff (added, widened and removed permissions). Applying an update whose diff adds or widens any permission requires `approve: true` (else refused with `plugin_permission_denied`); applying a release whose verified signer differs from the previously recorded signer requires `approve_signer_change: true` (else refused with `plugin_signer_changed`). Plugins are never updated automatically on engine start, Run start or index refresh. Refused with `person_required` on any non-person surface.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PluginUpdateParams>,
    result: schema_of::<PluginUpdated>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A Plugin update whose verified signer differs from the previously installed
/// signer identity (ADR 0033, issue 924).
pub const PLUGIN_SIGNER_CHANGED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_signer_changed",
    description: "The target Plugin release was signed by a different signer identity than the previously installed release, so nothing was updated. Review the previous and new signer identities and re-run `plugin update` interactively to approve the signer change.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_signer_changed` details (ADR 0033, issue 924).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSignerChangedDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The signer identity previously recorded for the installed Plugin.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub previous_signer: String,
    /// The verified signer identity of the target release.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found_signer: SignerIdentity,
    /// The target release's image index digest, `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// What to do about it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// This module's entries for the protocol catalog.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[PLUGIN_UPDATE],
    events: &[],
    handler_requests: &[],
    error_codes: &[PLUGIN_SIGNER_CHANGED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::plugin::{
        Download, Grant, MANIFEST_VERSION, ManifestBrowser, NetworkLevel, PLUGIN_PROTOCOL,
        Permissions, PluginKindName, PluginManifest,
    };

    use super::*;

    fn manifest_with(
        network_level: NetworkLevel,
        domains: &[&str],
        secrets: &[&str],
        grants: &[Grant],
        browsers: Vec<ManifestBrowser>,
    ) -> PluginManifest {
        PluginManifest {
            manifest_version: MANIFEST_VERSION,
            name: "kettle".to_owned(),
            version: "1.0.0".to_owned(),
            kinds: BTreeSet::from([PluginKindName::NOTIFICATION]),
            hook_stages: BTreeSet::new(),
            protocol: PLUGIN_PROTOCOL,
            settings_schema: None,
            runtime: crate::plugin::Runtime::None,
            entrypoint: "kettle".to_owned(),
            network_level,
            permissions: Permissions {
                domains: domains.iter().map(|s| (*s).to_owned()).collect(),
                secrets: secrets.iter().map(|s| (*s).to_owned()).collect(),
                grants: grants.iter().copied().collect(),
            },
            env: BTreeMap::new(),
            files: Vec::new(),
            processes: Vec::new(),
            limits: crate::plugin::Limits::default(),
            browsers,
            spend: BTreeMap::new(),
            provider_name: None,
            models: Vec::new(),
            scheme: None,
        }
    }

    #[test]
    fn permission_diff_classifies_added_widened_and_removed_permissions() {
        let old = manifest_with(
            NetworkLevel::None,
            &["api.shop.test", "old.example"],
            &["env:TOKEN", "env:OLD"],
            &[Grant::Effects],
            vec![ManifestBrowser {
                name: "kettle-browser".to_owned(),
                platforms: BTreeMap::from([(
                    "linux-x64".to_owned(),
                    Download {
                        url: "https://downloads.example/b1.tar.gz".to_owned(),
                        digest:
                            "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                                .to_owned(),
                    },
                )]),
            }],
        );
        let new = manifest_with(
            NetworkLevel::ProxyTcp,
            &["*.shop.test", "new.example"],
            &["env:TOKEN", "env:EXTRA"],
            &[Grant::Effects, Grant::SecretValue],
            vec![ManifestBrowser {
                name: "kettle-browser".to_owned(),
                platforms: BTreeMap::from([(
                    "linux-x64".to_owned(),
                    Download {
                        url: "https://downloads.example/b2.tar.gz".to_owned(),
                        digest:
                            "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                                .to_owned(),
                    },
                )]),
            }],
        );

        let diff = PermissionDiff::between(&old, &new);
        assert!(diff.requires_approval);

        // Widened: network_level (none -> proxy_tcp), effects (api.shop.test -> *.shop.test),
        // secret_value (env:TOKEN), browser digest change.
        let widened_kinds: Vec<&str> = diff.widened.iter().map(|w| w.kind.as_str()).collect();
        assert!(widened_kinds.contains(&"network_level"), "{diff:?}");
        assert!(widened_kinds.contains(&"effects"), "{diff:?}");
        assert!(widened_kinds.contains(&"secret_value"), "{diff:?}");
        assert!(widened_kinds.contains(&"browser"), "{diff:?}");

        // Added: effects on new.example, secret env:EXTRA, secret_value env:EXTRA.
        assert!(
            diff.added.iter().any(|s| s.permission
                == Permission::Effects {
                    domain: "new.example".to_owned()
                }),
            "{diff:?}"
        );
        assert!(
            diff.added.iter().any(|s| s.permission
                == Permission::Secret {
                    reference: "env:EXTRA".to_owned()
                }),
            "{diff:?}"
        );

        // Removed: effects on old.example, secret env:OLD (not superseded api.shop.test or browser).
        assert!(
            diff.removed.iter().any(|s| s.permission
                == Permission::Effects {
                    domain: "old.example".to_owned()
                }),
            "{diff:?}"
        );
        assert!(
            diff.removed.iter().any(|s| s.permission
                == Permission::Secret {
                    reference: "env:OLD".to_owned()
                }),
            "{diff:?}"
        );
        assert!(
            !diff.removed.iter().any(|s| s.permission
                == Permission::Effects {
                    domain: "api.shop.test".to_owned()
                }),
            "widened domain should not also appear as removed: {diff:?}"
        );
    }

    #[test]
    fn shrinking_only_diff_requires_no_new_approval() {
        let old = manifest_with(
            NetworkLevel::ProxyTcp,
            &["shop.test", "extra.test"],
            &["env:TOKEN"],
            &[Grant::Effects],
            Vec::new(),
        );
        let new = manifest_with(
            NetworkLevel::None,
            &["shop.test"],
            &[],
            &[Grant::Effects],
            Vec::new(),
        );

        let diff = PermissionDiff::between(&old, &new);
        assert!(!diff.requires_approval);
        assert_eq!(diff.added, Vec::<StatedPermission>::new());
        assert_eq!(diff.widened, Vec::<WidenedPermission>::new());
        assert_eq!(diff.removed.len(), 2);
    }
}
