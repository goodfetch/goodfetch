//! A Plugin's spend outside the engine (ADR 0033): `usage.report`, the call
//! back through which a Plugin reports raw units spent while serving a call,
//! and `plugin_price_unknown`, the Error code refusing a call under a currency
//! Limit when a declared spend unit has no price.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};
use crate::jsonrpc::RequestId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Plugin's call back reporting raw spend units spent during a call.
pub const USAGE_REPORT: &str = "usage.report";

/// What `usage.report` reports to the engine: the call being served, the
/// declared unit name, and the raw quantity spent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageReportParams {
    /// The id of the engine's call the Plugin is serving.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call_id: RequestId,
    /// The spend unit's name, as declared in the manifest's `spend` table.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unit: String,
    /// The raw quantity of `unit` spent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub quantity: u64,
}

/// What the engine answers to `usage.report`: acknowledgement that the
/// reported quantity was recorded for the call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageReportResult {}

/// Details carried by `plugin_price_unknown`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginPriceUnknownDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugin: String,
    /// The declared spend unit that has no price.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unit: String,
}

/// A Plugin call refused under a currency Limit because a declared spend unit
/// has no price (ADR 0033).
pub const PLUGIN_PRICE_UNKNOWN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_price_unknown",
    description: "A Plugin call cannot run under a currency Limit because the Plugin declares a spend unit with no price, which a currency Limit treats as unbounded. Set a price for the unit in Operator policy (`plugins.<name>.spend.<unit>.price`), or remove the currency Limit.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Error codes of Plugin spend.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[PLUGIN_PRICE_UNKNOWN],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
