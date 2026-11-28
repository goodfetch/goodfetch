//! Runtime enforcement of a Plugin's Effects, `page_script`, and navigation
//! scope (ADRs 0010, 0014 and 0033; issue 845): bridges the pure decision in
//! [`super::policy`] to the Run's Audit log, Effect journal and domain policy.

use std::fmt;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::plugin::approvals::{Approvals, PLUGIN_PERMISSION_USED, Permission};
use goodfetch_protocol::plugin::{HookStageName, PluginManifest};
use goodfetch_protocol::run::RunId;
use serde_json::json;
use url::Url;

use super::policy::{
    ActionContext, PluginAction, PolicyDecision, PolicyRefusal, classify_page_method, decide,
};
use super::{permission_denied, refused};
use crate::audit::{AuditLog, Origin};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::egress::{self, EgressClass, Judgment, Policy, Request, RequestKind};

/// One in-flight Plugin Effect recorded in the Run's Effect journal.
pub trait PluginEffectEntry: Send + Sync {
    /// Marks the Effect `done` once the operation returned cleanly.
    fn done(self: Box<Self>) -> BoxFuture<'static, Result<(), CallError>>;

    /// Marks the Effect `uncertain` if the operation was cancelled or failed
    /// after being sent.
    fn uncertain(self: Box<Self>, reason: UncertainReason) -> BoxFuture<'static, ()>;
}

/// Persists Plugin Effects into the Run's Effect journal (`effect_journal`
/// table in `run.sqlite`).
pub trait PluginEffectJournal: Send + Sync + fmt::Debug {
    /// Records a new Effect entry as `sent` (synced) before the action goes
    /// out, returning its handle for closing as `done` or `uncertain`.
    fn begin<'a>(
        &'a self,
        plugin: &'a str,
        method: &'a str,
        target: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn PluginEffectEntry>, CallError>>;
}

/// Enforces `effects`, `page_script`, and navigation scope for one Plugin
/// instance inside a live Run, recording approved permission uses in the Audit
/// log and journaling Effects through [`PluginEffectJournal`].
#[derive(Clone)]
pub struct EffectsGate {
    plugin: String,
    manifest: PluginManifest,
    approved: Approvals,
    page_policy: Policy,
    audit: Option<AuditLog>,
    run_id: RunId,
    clock: Clock,
    journal: Arc<dyn PluginEffectJournal>,
}

impl fmt::Debug for EffectsGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectsGate")
            .field("plugin", &self.plugin)
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

impl EffectsGate {
    /// Builds an [`EffectsGate`] for `plugin` with its approved manifest,
    /// Operator policy `approved`, run domain policy `page_policy`, optional
    /// `audit`, `run_id`, `clock`, and `journal`.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "packs the Run's policy, audit and journal handles for one Plugin instance"
    )]
    pub fn new(
        plugin: String,
        manifest: PluginManifest,
        approved: Approvals,
        page_policy: Policy,
        audit: Option<AuditLog>,
        run_id: RunId,
        clock: Clock,
        journal: Arc<dyn PluginEffectJournal>,
    ) -> Self {
        Self {
            plugin,
            manifest,
            approved,
            page_policy,
            audit,
            run_id,
            clock,
            journal,
        }
    }

    /// Authorizes a `net.fetch` request toward `target_url` with
    /// `egress_class` and `is_effect`, auditing any used permission and
    /// opening an Effect journal entry when required.
    ///
    /// # Errors
    ///
    /// `plugin_call_back_refused` when the manifest does not declare
    /// `Grant::Effects` (or stage forbids it), `plugin_permission_denied` when
    /// Operator policy has not approved `effects` for the target domain, or an
    /// internal error if journaling fails.
    pub async fn authorize_net_fetch(
        &self,
        method: &str,
        target_url: &Url,
        is_effect: bool,
        egress_class: EgressClass,
        stage: Option<HookStageName>,
    ) -> Result<Option<Box<dyn PluginEffectEntry>>, CallError> {
        let host = target_url.host_str().unwrap_or_default();
        let ctx = ActionContext {
            action: PluginAction::NetFetch {
                is_effect,
                egress_class,
            },
            stage,
            logged_in: false,
            page_host: None,
            target_host: host,
            declared_domains: &self.manifest.permissions.domains,
            grants: &self.manifest.permissions.grants,
            approved: &self.approved,
        };
        let label = format!("net.fetch {method}");
        self.apply_decision(&label, target_url.as_str(), decide(&ctx))
            .await
    }

    /// Authorizes a `page.*` / `locator.*` call back from this Plugin: checks
    /// stage, `page_script`, `effects`, navigation scope (page registrable
    /// domain + declared domains only when not logged in), and the Run's
    /// domain policy (declared domains never widen it); audits approved
    /// permission use and opens an Effect journal entry when required.
    ///
    /// # Errors
    ///
    /// `plugin_call_back_refused` when `method` is unknown, the stage forbids
    /// it, the manifest lacks the required grant, or navigation leaves scope;
    /// `plugin_permission_denied` when Operator policy has not approved
    /// `page_script` or `effects` for the domain; or `egress_refused` when
    /// navigation violates the Run's domain policy.
    pub async fn authorize_page_call(
        &self,
        method: &str,
        stage: Option<HookStageName>,
        logged_in: bool,
        page_url: &Url,
        target_url: Option<&Url>,
    ) -> Result<Option<Box<dyn PluginEffectEntry>>, CallError> {
        let Some(action) = classify_page_method(method) else {
            return Err(refused(&format!(
                "the Plugin `{}` called `{method}`, which is not an admitted page operation",
                self.plugin
            )));
        };
        let effective_url = match (action, target_url) {
            (PluginAction::PageNavigate, Some(target)) => {
                let request = Request {
                    url: target.as_str(),
                    kind: RequestKind::Fetch,
                };
                if let Judgment::Refused(failure) = egress::judge(&request, &self.page_policy) {
                    return Err(failure.error());
                }
                target
            }
            _ => page_url,
        };
        let page_host = page_url.host_str();
        let target_host = effective_url.host_str().unwrap_or_default();
        let ctx = ActionContext {
            action,
            stage,
            logged_in,
            page_host,
            target_host,
            declared_domains: &self.manifest.permissions.domains,
            grants: &self.manifest.permissions.grants,
            approved: &self.approved,
        };
        self.apply_decision(method, effective_url.as_str(), decide(&ctx))
            .await
    }

    async fn apply_decision(
        &self,
        method: &str,
        target: &str,
        decision: PolicyDecision,
    ) -> Result<Option<Box<dyn PluginEffectEntry>>, CallError> {
        match decision {
            PolicyDecision::Allow(plan) => {
                for permission in &plan.audit {
                    self.record_audit(permission, method).await;
                }
                if plan.journal_effect {
                    let entry = self.journal.begin(&self.plugin, method, target).await?;
                    Ok(Some(entry))
                } else {
                    Ok(None)
                }
            }
            PolicyDecision::DenyPermission(permission) => {
                Err(permission_denied(&self.plugin, permission))
            }
            PolicyDecision::Refuse(reason) => Err(refusal_error(&self.plugin, method, reason)),
        }
    }

    async fn record_audit(&self, permission: &Permission, method: &str) {
        let Some(audit) = &self.audit else {
            return;
        };
        let origin = Origin {
            actor: Actor::Engine,
            surface: None,
            layer: None,
            run_id: Some(self.run_id.clone()),
        };
        let details = json!({
            "plugin": self.plugin,
            "permission": permission.field(),
            "domain": permission.value(),
            "method": method,
        });
        if let Err(error) = audit
            .record(
                &origin,
                &PLUGIN_PERMISSION_USED,
                details,
                self.clock.timestamp(),
            )
            .await
        {
            tracing::warn!(
                plugin = %self.plugin,
                permission = permission.field(),
                %error,
                "could not record `plugin_permission_used` in Audit log"
            );
        }
    }
}

fn refusal_error(plugin: &str, method: &str, reason: PolicyRefusal) -> CallError {
    let message = match reason {
        PolicyRefusal::StageForbidsPageCall(stage) => {
            format!("the Plugin `{plugin}` cannot call `{method}` during the `{stage}` Hook stage")
        }
        PolicyRefusal::StageForbidsEffect(stage) => format!(
            "the Plugin `{plugin}` cannot cause an Effect (`{method}`) during the observe-only `{stage}` Hook stage"
        ),
        PolicyRefusal::GrantUndeclared(grant) => {
            format!("the Plugin `{plugin}`'s manifest does not declare `{grant:?}` for `{method}`")
        }
        PolicyRefusal::NavigationOutOfScope {
            target_domain,
            page_domain,
            logged_in,
        } => {
            let page = page_domain.unwrap_or_else(|| "none".to_owned());
            if logged_in {
                format!(
                    "the Plugin `{plugin}` cannot navigate to `{target_domain}` while the Session is logged in: logged-in navigations stay within `{page}`"
                )
            } else {
                format!(
                    "the Plugin `{plugin}` cannot navigate to `{target_domain}`, outside `{page}` and its declared domains"
                )
            }
        }
    };
    refused(&message)
}
