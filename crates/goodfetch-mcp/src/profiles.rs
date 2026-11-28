//! MCP tool profiles (ADRs 0011, 0090, 0095, 0099; spec #608): each profile
//! selects a deterministic set of catalog operations, identical in Collie and
//! Foxhound executables. Granting, person-only and SDK-only operations are
//! kept off every profile.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::{Operation, OperationKind, Product, Reach, SDK_ONLY};
use goodfetch_protocol::entries::operations_of;

use crate::projection::{ProjectedTool, project_operation, tool_name};

/// Operations kept off every MCP profile regardless of Product (ADRs 0011,
/// 0029, 0041, 0042, 0089, 0092, 0099, 0101, 0106; spec #608).
pub const KEPT_OFF_OPERATIONS: &[&str] = &[
    // Granting operations (plugin management & index import)
    "plugin.add",
    "plugin.update",
    "plugin.remove",
    "plugin.setup",
    "plugin.index_import",
    // Limit raises
    "usage.raise",
    // Plan approvals
    "plan.accept",
    "plan.reject",
    // Page lending & raw storage/cookies
    "page.lend",
    "session.cookies",
    "session.add_cookies",
    "session.clear_cookies",
    "page.local_storage",
    "page.session_storage",
    // Effect journal boundaries
    "effect.open",
    "effect.close",
    // Notification subscriptions (HTTP listener & CLI only)
    "notification.subscribe",
    "notification.unsubscribe",
    "notifications.subscribe",
    "notifications.unsubscribe",
    // Account, token & step-up writes
    "account.create",
    "account.delete",
    "account.update",
    "token.create",
    "token.revoke",
    "token.rotate",
    "step_up.start",
    "step_up.verify",
    // Live view grants (#1014)
    "live_view.grant",
    "live_view.revoke",
    // Config & policy writes, certs, consent/wall/pay widening, prices
    "config.user_write",
    "config.policy_write",
    "cert.accept",
    "cert.forget",
    "certs.accept",
    "certs.forget",
    "needs_you.accept_consent_wall",
    "needs_you.accept_wall",
    "needs_you.accept_pay",
    "needs_you.allow_consent_domain",
    "needs_you.deny_wall_domain",
    "needs_you.reconcile",
    "model.set_price",
    "model.write_prices",
];

/// Collie's five MCP profiles (spec #608).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProfileName {
    /// Collie's default: Run lifecycle/progress/report, Session and page
    /// opening/closing, navigation and reading, locator actions, Saved
    /// identity list (and AI verbs / `ai` snapshot / `expect` as they land).
    #[default]
    Browser,
    /// `browser` plus `page_agent`.
    Agent,
    /// Runs, Run reports, `recipe_start`, `catalog_run_start`, `schedule_*`,
    /// `usage_*`, Run index reads; no page tools.
    Run,
    /// `code_mode_run` alone plus docs resources; no per-operation tools.
    Code,
    /// Every agent-surface operation of the Product.
    Full,
}

impl ProfileName {
    /// All five Collie profiles in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Browser,
        Self::Agent,
        Self::Run,
        Self::Code,
        Self::Full,
    ];

    /// Its canonical wire/flag spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Agent => "agent",
            Self::Run => "run",
            Self::Code => "code",
            Self::Full => "full",
        }
    }

    /// Parses a profile name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "browser" => Some(Self::Browser),
            "agent" => Some(Self::Agent),
            "run" => Some(Self::Run),
            "code" => Some(Self::Code),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// Whether `operation` is eligible to appear on any MCP profile.
#[must_use]
pub fn is_mcp_eligible(operation: &Operation) -> bool {
    if operation.reach != Reach::AllSurfaces {
        return false;
    }
    if SDK_ONLY.contains(&operation.name)
        || KEPT_OFF_OPERATIONS.contains(&operation.name)
        || crate::projection::is_live_view_grant(operation.name)
    {
        return false;
    }
    matches!(
        operation.kind,
        OperationKind::RoundTrip
            | OperationKind::Query
            | OperationKind::Opener
            | OperationKind::Followed
    )
}

/// Whether `operation` belongs to Collie's `browser` profile.
#[must_use]
fn in_browser_profile(operation: &Operation) -> bool {
    let name = operation.name;
    let namespace = name.split('.').next().unwrap_or_default();
    match namespace {
        "run" => matches!(
            name,
            "run.start"
                | "run.finish"
                | "run.stop"
                | "run.progress"
                | "run.report"
                | "run.new_session"
        ),
        "session" => matches!(name, "session.close" | "session.new_page" | "session.pages"),
        "page" => {
            name != "page.agent"
                && name != "page.site_tools"
                && name != "page.invoke_site_tool"
                && name != "page.cancel_site_tool"
        }
        "locator" | "expect" => true,
        "identity" | "identities" => name
            .split_once('.')
            .is_some_and(|(_, member)| member == "list"),
        "success_check" => name == "success_check.evaluate",
        _ => false,
    }
}

/// Whether `operation` belongs to Collie's `run` profile (no page tools).
#[must_use]
fn in_run_profile(operation: &Operation) -> bool {
    let name = operation.name;
    let namespace = name.split('.').next().unwrap_or_default();
    matches!(
        namespace,
        "run" | "runs" | "recipe" | "catalog_run" | "schedule" | "usage"
    ) && name != "run.new_session"
        && name != "run.research"
}

/// A resolved MCP profile ready to serve `tools/list` and dispatch calls.
#[derive(Clone, Debug)]
pub struct ResolvedProfile {
    /// The profile's name (e.g. `"browser"`).
    pub name: String,
    /// Projected tools sorted by tool name for deterministic snapshots.
    pub tools: Vec<ProjectedTool>,
    /// Lookup from MCP tool name to index in `tools`.
    index: BTreeMap<String, usize>,
}

impl ResolvedProfile {
    /// Builds one of Collie's five profiles for `product`.
    ///
    /// For Collie's five profiles (`browser`, `agent`, `run`, `code`, `full`),
    /// `browser`, `agent`, `run` and `code` evaluate over Collie's catalog so
    /// they are byte-for-byte identical in both Collie and Foxhound servers
    /// (ADRs 0090, 0095). `full` includes every eligible operation of `product`
    /// unless restricted to Collie.
    #[must_use]
    pub fn for_product(product: Product, profile: ProfileName) -> Self {
        let catalog_product = match profile {
            ProfileName::Full => product,
            ProfileName::Browser | ProfileName::Agent | ProfileName::Run | ProfileName::Code => {
                Product::Collie
            }
        };
        let mut ops: Vec<&'static Operation> = operations_of(catalog_product)
            .filter(|op| is_mcp_eligible(op))
            .filter(|op| match profile {
                ProfileName::Browser => in_browser_profile(op),
                ProfileName::Agent => in_browser_profile(op) || op.name == "page.agent",
                ProfileName::Run => in_run_profile(op),
                ProfileName::Code => false,
                ProfileName::Full => true,
            })
            .collect();
        ops.sort_by_key(|op| tool_name(op.name));
        Self::from_operations(profile.as_str().to_owned(), ops)
    }

    /// Builds a profile from an explicit list of operations (used by Foxhound
    /// profiles or fixture servers).
    #[must_use]
    pub fn from_operations(name: String, operations: Vec<&'static Operation>) -> Self {
        let mut tools: Vec<ProjectedTool> = operations.into_iter().map(project_operation).collect();
        tools.sort_by(|a, b| a.tool.name.cmp(&b.tool.name));
        let index = tools
            .iter()
            .enumerate()
            .map(|(i, projected)| (projected.tool.name.to_string(), i))
            .collect();
        Self { name, tools, index }
    }

    /// Looks up a projected tool by its MCP tool name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ProjectedTool> {
        self.index.get(name).map(|&idx| &self.tools[idx])
    }
}
