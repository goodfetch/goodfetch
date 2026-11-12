//! The `engine.*` operations: what an engine says about itself, whichever
//! Product runs it (spec #213).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability,
    schema_of,
};
use crate::connection::ProductVersion;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `engine.info` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineInfoParams {}

/// What `engine.info` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EngineInfo {
    /// The Product the engine runs as.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// The engine's Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The protocol version the engine speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The major of its Product version, which names its per-major state.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_major: u64,
}

/// `engine.info`.
pub const ENGINE_INFO: Operation = Operation {
    name: "engine.info",
    description: "Says which Product the engine runs as, its Product version and major, and the protocol version it speaks.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<EngineInfoParams>,
    result: schema_of::<EngineInfo>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[ENGINE_INFO],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
