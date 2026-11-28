//! Canonical MCP tool projection and profile selection (ADRs 0011, 0065,
//! 0070, 0090, 0095, 0099; spec #608).
//!
//! Shared by `goodfetch-mcp` (runtime MCP server) and `xtask codegen`
//! (generated TypeScript and Python framework adapter tool tables), so every
//! agent surface exposes the exact same tool names, descriptions, input
//! schemas, operation classes, and profile membership.

use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::catalog::{Operation, OperationClass, OperationKind, Product, Reach, SDK_ONLY};
use crate::entries::operations_of;

/// Maximum length of an MCP tool name (ADR 0099, spec #608).
pub const MAX_TOOL_NAME_CHARS: usize = 39;

/// Maximum length of an MCP tool description (ADR 0065, spec #608).
pub const MAX_DESCRIPTION_CHARS: usize = 2048;

/// Parameter omitted from `run_start` and `recipe_start` input schemas on MCP
/// and Framework adapter surfaces because accepting a plugin change is a
/// granting operation (ADR 0011, ADR 0033, spec #608).
pub const OMITTED_GRANTING_PARAMS: &[(&str, &str)] = &[
    ("run.start", "accept_plugin_change"),
    ("recipe.start", "accept_plugin_change"),
];

/// Operations kept off every MCP profile and Framework adapter regardless of
/// Product (ADRs 0011, 0029, 0041, 0042, 0089, 0092, 0099, 0101, 0106; spec #608).
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

/// Why a catalog operation cannot be projected onto the MCP surface (ADRs 0065,
/// 0070, 0099; issues 1360 and 1014).
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProjectionViolation {
    /// The projected tool name is empty, longer than [`MAX_TOOL_NAME_CHARS`],
    /// or contains characters outside `[a-z0-9_]`.
    #[error(
        "MCP tool name `{tool_name}` (from `{operation}`) must be 1..={MAX_TOOL_NAME_CHARS} chars in [a-z0-9_]"
    )]
    InvalidToolName {
        /// Canonical catalog operation name.
        operation: String,
        /// Projected MCP tool name.
        tool_name: String,
    },
    /// The catalog description is empty.
    #[error("MCP tool description for `{operation}` must not be empty")]
    EmptyDescription {
        /// Canonical catalog operation name.
        operation: String,
    },
    /// The projected description exceeds [`MAX_DESCRIPTION_CHARS`].
    #[error(
        "MCP tool description for `{operation}` is {chars} chars (max {MAX_DESCRIPTION_CHARS})"
    )]
    DescriptionTooLong {
        /// Canonical catalog operation name.
        operation: String,
        /// Character count of the projected description.
        chars: usize,
    },
    /// Live view grant entries are refused on the MCP surface (issue 1014).
    #[error("Live view grant operation `{operation}` is not allowed on the MCP surface")]
    LiveViewGrantNotAllowed {
        /// Canonical catalog operation name.
        operation: String,
    },
}

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

/// A catalog operation projected onto its transport-neutral MCP tool spec.
#[derive(Clone, Debug)]
pub struct McpToolSpec {
    /// Projected MCP tool name (`namespace_member`).
    pub name: String,
    /// Canonical catalog operation name (`namespace.member`).
    pub operation: &'static str,
    /// Projected tool description (with deprecation prefix when deprecated).
    pub description: String,
    /// The operation's class (read, idempotent write or Effect).
    pub class: OperationClass,
    /// Whether the tool reaches external sites, search providers, or models.
    pub open_world: bool,
    /// Whether the engine cancels an in-flight call on `$/cancel`.
    pub cancellable: bool,
    /// The tool's JSON Schema object (`"type": "object"`).
    pub input_schema: Map<String, Value>,
}

impl McpToolSpec {
    /// Whether the tool is read-only.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        matches!(self.class, OperationClass::Read)
    }

    /// Whether the tool is destructive (Effects).
    #[must_use]
    pub const fn destructive(&self) -> bool {
        matches!(self.class, OperationClass::Effect)
    }

    /// Whether the tool is idempotent (idempotent writes).
    #[must_use]
    pub const fn idempotent(&self) -> bool {
        matches!(self.class, OperationClass::IdempotentWrite)
    }
}

/// Whether `canonical_name` is a Live view grant entry refused on agent
/// surfaces (issue 1014, ADR 0070).
#[must_use]
pub fn is_live_view_grant(canonical_name: &str) -> bool {
    canonical_name.starts_with("live_view.")
}

/// Converts a canonical catalog operation name (`namespace.member`) to its MCP
/// tool name (`namespace_member`).
#[must_use]
pub fn tool_name(canonical: &str) -> String {
    canonical.replace('.', "_")
}

/// Whether `name` satisfies the MCP tool-name rule: non-empty, at most
/// [`MAX_TOOL_NAME_CHARS`] characters, and every character in `[a-z0-9_]`.
#[must_use]
pub fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_TOOL_NAME_CHARS
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Validates that `operation` obeys every MCP projection rule: tool name in
/// `[a-z0-9_]` of at most [`MAX_TOOL_NAME_CHARS`] characters, non-empty
/// description of at most [`MAX_DESCRIPTION_CHARS`] characters, and never a
/// Live view grant entry (ADRs 0065, 0070, 0099; issues 1360 and 1014).
///
/// # Errors
///
/// Returns a [`ProjectionViolation`] naming the rule `operation` breaks.
pub fn validate_operation(operation: &Operation) -> Result<(), ProjectionViolation> {
    if is_live_view_grant(operation.name) {
        return Err(ProjectionViolation::LiveViewGrantNotAllowed {
            operation: operation.name.to_owned(),
        });
    }
    let name = tool_name(operation.name);
    if !is_valid_tool_name(&name) {
        return Err(ProjectionViolation::InvalidToolName {
            operation: operation.name.to_owned(),
            tool_name: name,
        });
    }
    let description = description_for(operation);
    let chars = description.chars().count();
    if chars == 0 {
        return Err(ProjectionViolation::EmptyDescription {
            operation: operation.name.to_owned(),
        });
    }
    if chars > MAX_DESCRIPTION_CHARS {
        return Err(ProjectionViolation::DescriptionTooLong {
            operation: operation.name.to_owned(),
            chars,
        });
    }
    Ok(())
}

/// Whether `operation` reaches a site, a search provider or a model, so its
/// MCP `openWorldHint` annotation is `true` (spec #608).
#[must_use]
pub fn is_open_world(operation: &Operation) -> bool {
    let name = operation.name;
    let namespace = name.split('.').next().unwrap_or_default();
    if matches!(namespace, "search" | "fetch" | "crawl" | "model") {
        return true;
    }
    if name == "run.research" || name == "catalog_run.start" || name == "recipe.start" {
        return true;
    }
    if namespace == "page" {
        return matches!(
            name,
            "page.goto"
                | "page.reload"
                | "page.go_back"
                | "page.go_forward"
                | "page.click"
                | "page.fill"
                | "page.press"
                | "page.type"
                | "page.check"
                | "page.uncheck"
                | "page.select_option"
                | "page.hover"
                | "page.focus"
                | "page.evaluate"
                | "page.act"
                | "page.observe"
                | "page.extract"
                | "page.agent"
                | "page.invoke_site_tool"
                | "page.site_tools"
                | "page.cancel_site_tool"
                | "page.wait_for_navigation"
                | "page.wait_for_response"
                | "page.wait_for_request"
        );
    }
    if namespace == "locator" {
        return operation.class == OperationClass::Effect;
    }
    false
}

/// Builds the MCP tool description from the catalog operation's one text,
/// prepending the standard deprecation line when deprecated (ADR 0099, #1360).
#[must_use]
pub fn description_for(operation: &Operation) -> String {
    match &operation.deprecated {
        Some(deprecation) => {
            let major = deprecation.earliest_removal.major;
            let prefix = match deprecation.replacement {
                Some(replacement) => {
                    format!(
                        "Deprecated: use `{}`, removed in {major}",
                        tool_name(replacement)
                    )
                }
                None => format!("Deprecated: removed in {major}"),
            };
            format!("{prefix}. {}", operation.description.trim())
        }
        None => operation.description.trim().to_owned(),
    }
}

/// Builds the JSON Schema object for an operation's `inputSchema`.
///
/// The root is always an object schema (`"type": "object"`) with any referenced
/// definitions carried in `"$defs"`. Granting parameters excluded from agent
/// surfaces are stripped, and deprecated operations carry `"deprecated": true`.
#[must_use]
pub fn input_schema_for(operation: &Operation) -> Map<String, Value> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let root_schema = (operation.params)(&mut generator);
    let defs = generator.definitions().clone();
    let mut object = resolve_root_object(root_schema.to_value(), &defs);

    if !defs.is_empty() && !object.contains_key("$defs") {
        object.insert("$defs".to_owned(), Value::Object(defs));
    }

    for &(op_name, param) in OMITTED_GRANTING_PARAMS {
        if operation.name == op_name {
            strip_property(&mut object, param);
        }
    }

    if operation.deprecated.is_some() {
        object.insert("deprecated".to_owned(), Value::Bool(true));
    }

    if !object.contains_key("type") {
        object.insert("type".to_owned(), Value::String("object".to_owned()));
    }
    if !object.contains_key("properties") {
        object.insert("properties".to_owned(), Value::Object(Map::new()));
    }

    object
}

/// Resolves a `$ref` root schema against `$defs` into an inline object schema
/// so every MCP client and framework adapter receives a top-level
/// `"type": "object"` schema.
fn resolve_root_object(root: Value, defs: &Map<String, Value>) -> Map<String, Value> {
    let Value::Object(mut map) = root else {
        return Map::new();
    };
    if let Some(reference) = map.get("$ref").and_then(Value::as_str)
        && let Some(def_name) = reference.strip_prefix("#/$defs/")
        && let Some(Value::Object(resolved)) = defs.get(def_name)
    {
        let mut merged = resolved.clone();
        for (key, value) in map {
            if key != "$ref" {
                merged.insert(key, value);
            }
        }
        map = merged;
    }
    map
}

/// Removes `param` from `properties` and `required` of an object schema.
fn strip_property(schema: &mut Map<String, Value>, param: &str) {
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.remove(param);
    }
    if let Some(Value::Array(required)) = schema.get_mut("required") {
        required.retain(|item| item.as_str() != Some(param));
        if required.is_empty() {
            schema.remove("required");
        }
    }
}

/// Whether `operation` is eligible to appear on any MCP profile or adapter.
#[must_use]
pub fn is_mcp_eligible(operation: &Operation) -> bool {
    if operation.reach != Reach::AllSurfaces {
        return false;
    }
    if SDK_ONLY.contains(&operation.name)
        || KEPT_OFF_OPERATIONS.contains(&operation.name)
        || is_live_view_grant(operation.name)
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
        && name != "run.new_page"
        && name != "run.research"
}

/// Projects a single catalog [`Operation`] onto its [`McpToolSpec`].
#[must_use]
pub fn project_operation(operation: &'static Operation) -> McpToolSpec {
    McpToolSpec {
        name: tool_name(operation.name),
        operation: operation.name,
        description: description_for(operation),
        class: operation.class,
        open_world: is_open_world(operation),
        cancellable: operation.cancellable,
        input_schema: input_schema_for(operation),
    }
}

/// Returns the sorted catalog operations selected by `profile` for `product`.
#[must_use]
pub fn profile_operations(product: Product, profile: ProfileName) -> Vec<&'static Operation> {
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
    ops
}

/// Returns the projected [`McpToolSpec`] list for `profile` and `product`,
/// sorted by MCP tool name.
#[must_use]
pub fn profile_tools(product: Product, profile: ProfileName) -> Vec<McpToolSpec> {
    profile_operations(product, profile)
        .into_iter()
        .map(project_operation)
        .collect()
}
