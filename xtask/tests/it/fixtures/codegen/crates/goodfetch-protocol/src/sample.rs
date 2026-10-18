//! Collie's sample entries: one of each kind, which carry `xtask codegen`'s
//! pipeline until the areas that own each contract define the real ones
//! (spec #130). Collie carries them, and so does Foxhound, which contains it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, Instrument, Metric, Operation,
    OperationClass, PluginKind, PluginMethod, ProtocolVersion, Stability, schema_of,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `sample.echo` repeats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleEchoParams {
    /// The text to repeat.
    pub text: String,
}

/// What `sample.echo` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleEchoResult {
    /// The text, repeated.
    pub text: String,
}

/// What `sample.echoed` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleEchoed {
    /// The text that was repeated.
    pub text: String,
}

/// What a sample store keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleStorePutParams {
    /// The key to keep the value under.
    pub key: String,
    /// The value to keep.
    pub value: String,
}

/// What a sample store answers once it kept a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleStorePutResult {
    /// Whether a value was already kept under the key.
    pub replaced: bool,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    operations: &[Operation {
        name: "sample.echo",
        defined_in: module_path!(),
        description: "Repeats a text back to the caller.",
        class: OperationClass::Read,
        params: schema_of::<SampleEchoParams>,
        result: schema_of::<SampleEchoResult>,
        events: &["sample.echoed"],
        cancellable: true,
        stability: Stability::Experimental,
        deprecated: Some(Deprecation {
            since: ARRIVED_IN,
            note: "A sample entry, which the real catalog replaces.",
        }),
        arrived_in: ARRIVED_IN,
    }],
    events: &[Event {
        name: "sample.echoed",
        defined_in: module_path!(),
        description: "A text was repeated.",
        payload: schema_of::<SampleEchoed>,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }],
    error_codes: &[ErrorCodeDefinition {
        name: "sample_refused",
        defined_in: module_path!(),
        description: "The sample operation refused the text.",
        group: ErrorGroup::Refused,
        retryable: false,
        arrived_in: ARRIVED_IN,
    }],
    plugin_kinds: &[PluginKind {
        name: "sample_store",
        defined_in: module_path!(),
        description: "Keeps values by key for the sample operations.",
        methods: &[PluginMethod {
            name: "put",
            description: "Keeps a value under a key.",
            params: schema_of::<SampleStorePutParams>,
            result: schema_of::<SampleStorePutResult>,
        }],
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
    metrics: &[Metric {
        name: "goodfetch.sample.echoes",
        defined_in: module_path!(),
        description: "Texts the sample operation repeated.",
        instrument: Instrument::Counter,
        unit: "{echo}",
        stability: Stability::Stable,
        arrived_in: ARRIVED_IN,
    }],
};
