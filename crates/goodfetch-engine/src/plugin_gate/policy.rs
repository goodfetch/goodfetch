//! Pure decision rules for a Plugin's Effects, `page_script`, and navigation
//! scope (ADRs 0010, 0014 and 0033; issue 845).
//!
//! Every rule here is a pure function with zero I/O, property-tested over
//! actions, Hook stages, login state, domains, manifest grants and Operator
//! policy approvals.

use std::collections::BTreeSet;

use goodfetch_protocol::plugin::approvals::{Approvals, Permission};
use goodfetch_protocol::plugin::{Grant, HookStageName};

use crate::egress::EgressClass;
use crate::host_name;
use crate::secrets;

/// The category of an action a Plugin requests back into the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginAction {
    /// A `net.fetch` HTTP request, with whether its method or headers ask for
    /// a write (`is_effect`) and its destination's [`EgressClass`].
    NetFetch {
        /// True when the method is other than `GET`/`HEAD` or a method-override
        /// header is present.
        is_effect: bool,
        /// Whether the target is an Operator-configured endpoint or a
        /// page-reachable site of the run.
        egress_class: EgressClass,
    },
    /// A read-only `page.*` / `locator.*` inspection that runs no page script
    /// and reads no form values, storage or cookies.
    PageRead,
    /// Running script in a page (`page.evaluate`, `locator.evaluate_all`,
    /// `page.add_init_script`, `page.wait_for_function`,
    /// `locator.wait_for_function`) or reading form values, storage or cookies.
    PageScript,
    /// A `page.*` / `locator.*` action that changes the page (`click`, `fill`,
    /// `press`, `submit`).
    PageMutate,
    /// Navigating the page (`page.goto`, `page.reload`, `page.go_back`,
    /// `page.go_forward`).
    PageNavigate,
}

impl PluginAction {
    /// Whether this action operates on a browser page.
    #[must_use]
    pub const fn is_page(self) -> bool {
        matches!(
            self,
            Self::PageRead | Self::PageScript | Self::PageMutate | Self::PageNavigate
        )
    }
}

/// Classifies a `page.*` or `locator.*` method name into its [`PluginAction`],
/// returning `None` for any method outside the page operations a Plugin may
/// request.
#[must_use]
pub fn classify_page_method(method: &str) -> Option<PluginAction> {
    match method {
        "page.evaluate"
        | "locator.evaluate_all"
        | "page.add_init_script"
        | "page.wait_for_function"
        | "locator.wait_for_function"
        | "page.cookies"
        | "page.storage"
        | "locator.input_value" => Some(PluginAction::PageScript),
        "locator.click" | "locator.fill" | "locator.press" | "locator.submit" => {
            Some(PluginAction::PageMutate)
        }
        "page.goto" | "page.reload" | "page.go_back" | "page.go_forward" => {
            Some(PluginAction::PageNavigate)
        }
        "page.read"
        | "page.formats"
        | "page.body"
        | "page.wait_for_load_state"
        | "page.wait_for_url"
        | "locator.wait_for"
        | "locator.wait_for_count" => Some(PluginAction::PageRead),
        _ => None,
    }
}

/// Whether `stage` is one of the three page-stage Hook stages that may call
/// `page.*` operations (ADR 0033).
#[must_use]
pub fn is_page_stage(stage: HookStageName) -> bool {
    matches!(
        stage,
        HookStageName::BEFORE_NAVIGATION
            | HookStageName::AFTER_NAVIGATION
            | HookStageName::PAGE_READY
    )
}

/// Whether `stage` is an observe-only Hook stage (no Effects toward the run's
/// sites).
#[must_use]
pub fn is_observe_only_stage(stage: HookStageName) -> bool {
    matches!(
        stage.as_str(),
        "request_ended"
            | "run_started"
            | "checkpoint_taken"
            | "run_migrated"
            | "limit_reached"
            | "run_ended"
    )
}

/// The registrable domain of `host` (canonicalized via [`host_name`]), falling
/// back to the canonicalized host itself for IP addresses or single-label
/// hosts.
#[must_use]
pub fn registrable_or_host(host: &str) -> String {
    let key = host_name::lookup_key(host);
    host_name::registrable(&key).unwrap_or(&key).to_owned()
}

/// Whether `host` is covered by any domain entry in `approved`.
#[must_use]
pub fn domain_approved(host: &str, approved: &BTreeSet<String>) -> bool {
    if approved.is_empty() {
        return false;
    }
    let domains: Vec<String> = approved.iter().cloned().collect();
    secrets::allowed(host, &domains)
}

/// Inputs to the pure Effects / `page_script` / navigation-scope decision.
#[derive(Clone, Debug)]
pub struct ActionContext<'a> {
    /// What the Plugin is asking to do.
    pub action: PluginAction,
    /// The Hook stage being served, or `None` for non-Hook kinds.
    pub stage: Option<HookStageName>,
    /// Whether the active Session is logged in (carries a Secret or a seeded
    /// identity).
    pub logged_in: bool,
    /// The host of the page the Plugin was called for (when serving a page
    /// call).
    pub page_host: Option<&'a str>,
    /// The host the action targets (destination host for navigation /
    /// `net.fetch`; current page host for in-page reads, scripts and
    /// mutations).
    pub target_host: &'a str,
    /// The domains declared in `manifest.permissions.domains`.
    pub declared_domains: &'a [String],
    /// The grants declared in `manifest.permissions.grants`.
    pub grants: &'a BTreeSet<Grant>,
    /// The Operator policy's approvals (`plugins.<name>.approved`).
    pub approved: &'a Approvals,
}

/// What an allowed action must record before running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowPlan {
    /// Whether the action must be recorded through the Effect journal.
    pub journal_effect: bool,
    /// Permissions whose use must be recorded in the Audit log.
    pub audit: Vec<Permission>,
}

/// Why an action is refused as a call-back policy violation
/// (`plugin_call_back_refused`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyRefusal {
    /// A `page.*` action was requested during a Hook stage that does not allow
    /// page operations.
    StageForbidsPageCall(HookStageName),
    /// An Effect toward the run's sites was requested during an observe-only
    /// Hook stage.
    StageForbidsEffect(HookStageName),
    /// The Plugin's manifest does not declare the required [`Grant`].
    GrantUndeclared(Grant),
    /// A navigation targets a host outside the page's registrable domain and
    /// (when not logged in) the Plugin's declared domains.
    NavigationOutOfScope {
        /// Registrable domain (or host) of the requested target.
        target_domain: String,
        /// Registrable domain (or host) of the page the Plugin was called for.
        page_domain: Option<String>,
        /// Whether the Session is logged in (in which case declared domains do
        /// not widen scope).
        logged_in: bool,
    },
}

/// The pure decision for one Plugin action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The action may proceed with the required journaling and auditing.
    Allow(AllowPlan),
    /// The manifest declared the grant, but Operator policy has not approved
    /// this permission for the target domain (`plugin_permission_denied`).
    DenyPermission(Permission),
    /// The action is refused outright (`plugin_call_back_refused`).
    Refuse(PolicyRefusal),
}

/// Evaluates ADR 0033's Effects, `page_script` and navigation-scope rules for
/// `ctx` without performing any I/O.
#[must_use]
pub fn decide(ctx: &ActionContext<'_>) -> PolicyDecision {
    if let Some(stage) = ctx.stage {
        if ctx.action.is_page() && !is_page_stage(stage) {
            return PolicyDecision::Refuse(PolicyRefusal::StageForbidsPageCall(stage));
        }
        if is_observe_only_stage(stage)
            && matches!(
                ctx.action,
                PluginAction::NetFetch {
                    is_effect: true,
                    egress_class: EgressClass::PageReachable,
                }
            )
        {
            return PolicyDecision::Refuse(PolicyRefusal::StageForbidsEffect(stage));
        }
    }

    let target_domain = registrable_or_host(ctx.target_host);

    match ctx.action {
        PluginAction::NetFetch {
            egress_class: EgressClass::OperatorConfigured,
            ..
        }
        | PluginAction::NetFetch {
            is_effect: false,
            egress_class: EgressClass::PageReachable,
        }
        | PluginAction::PageRead => PolicyDecision::Allow(AllowPlan {
            journal_effect: false,
            audit: Vec::new(),
        }),

        PluginAction::NetFetch {
            is_effect: true,
            egress_class: EgressClass::PageReachable,
        }
        | PluginAction::PageMutate => require_effects(ctx, target_domain),

        PluginAction::PageScript => {
            if !ctx.grants.contains(&Grant::PageScript) {
                return PolicyDecision::Refuse(PolicyRefusal::GrantUndeclared(Grant::PageScript));
            }
            if !domain_approved(ctx.target_host, &ctx.approved.page_script) {
                return PolicyDecision::DenyPermission(Permission::PageScript {
                    domain: target_domain,
                });
            }
            PolicyDecision::Allow(AllowPlan {
                journal_effect: ctx.logged_in,
                audit: vec![Permission::PageScript {
                    domain: target_domain,
                }],
            })
        }

        PluginAction::PageNavigate => {
            let page_domain = ctx.page_host.map(registrable_or_host);
            let same_registrable = page_domain.as_deref() == Some(target_domain.as_str());
            let declared_allows =
                !ctx.logged_in && secrets::allowed(ctx.target_host, ctx.declared_domains);
            if !same_registrable && !declared_allows {
                return PolicyDecision::Refuse(PolicyRefusal::NavigationOutOfScope {
                    target_domain,
                    page_domain,
                    logged_in: ctx.logged_in,
                });
            }
            if ctx.logged_in {
                require_effects(ctx, target_domain)
            } else {
                PolicyDecision::Allow(AllowPlan {
                    journal_effect: false,
                    audit: Vec::new(),
                })
            }
        }
    }
}

fn require_effects(ctx: &ActionContext<'_>, target_domain: String) -> PolicyDecision {
    if !ctx.grants.contains(&Grant::Effects) {
        return PolicyDecision::Refuse(PolicyRefusal::GrantUndeclared(Grant::Effects));
    }
    if !domain_approved(ctx.target_host, &ctx.approved.effects) {
        return PolicyDecision::DenyPermission(Permission::Effects {
            domain: target_domain,
        });
    }
    PolicyDecision::Allow(AllowPlan {
        journal_effect: true,
        audit: vec![Permission::Effects {
            domain: target_domain,
        }],
    })
}

#[cfg(test)]
mod tests;
