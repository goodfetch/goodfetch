//! Collie's sample entries: one of each kind, which carry `xtask codegen`'s
//! pipeline until the areas that own each contract define the real ones
//! (spec #130). Collie carries them, and so does Foxhound, which contains it.
//!
//! What the engine sends (a result, an event, a Plugin kind's params) keeps
//! reading when a newer engine adds a field, so it ignores unknown fields;
//! what a program sends (params, a Plugin kind's result) refuses them
//! (ADRs 0077, G3, and 0082).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Deprecation, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Instrument, Metric,
    Operation, OperationClass, OperationKind, PluginKind, PluginMethod, ProtocolVersion, Reach,
    Stability, schema_of,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What `sample.echo` repeats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleEchoParams {
    /// The text to repeat.
    pub text: String,
}

/// What a `hello` says, the Connection's own message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    /// The protocol version the sender speaks.
    pub protocol_version: String,
}

/// What `sample.echo` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SampleEchoResult {
    /// The text, repeated.
    pub text: String,
}

/// What `sample.echoed` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SampleEchoed {
    /// The text that was repeated.
    pub text: String,
}

/// What a sample store keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// What the operation `sample` reports.
const SAMPLE_ECHOED: Event = Event {
    name: "sample.echoed",
    description: "A text was repeated.",
    class: EventClass::Telemetry,
    payload: schema_of::<SampleEchoed>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[Operation {
        name: "sample.echo",
        description: "Repeats a text back to the caller.",
        class: OperationClass::Read,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params: schema_of::<SampleEchoParams>,
        result: schema_of::<SampleEchoResult>,
        events: &[&SAMPLE_ECHOED],
        cancellable: true,
        input: None,
        bound: None,
        stability: Stability::Experimental,
        deprecated: Some(Deprecation {
            since: ARRIVED_IN,
            replacement: None,
            earliest_removal: ProtocolVersion { major: 1, minor: 0 },
            note: "A sample entry, which the real catalog replaces.",
        }),
        arrived_in: ARRIVED_IN,
    }],
    events: &[SAMPLE_ECHOED],
    handler_requests: &[],
    error_codes: &[ErrorCodeDefinition {
        name: "sample_refused",
        description: "The sample operation refused the text.",
        group: ErrorGroup::Refused,
        retryable: false,
        exit_code: None,
        arrived_in: ARRIVED_IN,
    }],
    plugin_kinds: &[PluginKind {
        name: "sample_store",
        description: "Keeps values by key for the sample operations.",
        methods: &[PluginMethod {
            name: "put",
            description: "Keeps a value under a key.",
            params: schema_of::<SampleStorePutParams>,
            result: schema_of::<SampleStorePutResult>,
        }],
        chosen_by: None,
        stability: Stability::Experimental,
        arrived_in: ARRIVED_IN,
    }],
    metrics: &[Metric {
        name: "goodfetch.sample.echoes",
        description: "Texts the sample operation repeated.",
        instrument: Instrument::Counter,
        unit: "{echo}",
        labels: &[],
        stability: Stability::Stable,
        arrived_in: ARRIVED_IN,
    }],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests {
    use super::{SampleEchoParams, SampleEchoResult, SampleEchoed, SampleStorePutParams};

    #[test]
    fn result_from_a_newer_engine_reads_without_its_unknown_field() {
        let result: SampleEchoResult =
            serde_json::from_str(r#"{ "text": "hi", "added_in_a_minor": 1 }"#).unwrap();

        assert_eq!(result, SampleEchoResult { text: "hi".into() });
    }

    #[test]
    fn event_from_a_newer_engine_reads_without_its_unknown_field() {
        let event: SampleEchoed =
            serde_json::from_str(r#"{ "text": "hi", "added_in_a_minor": 1 }"#).unwrap();

        assert_eq!(event, SampleEchoed { text: "hi".into() });
    }

    #[test]
    fn plugin_params_from_a_newer_engine_read_without_their_unknown_field() {
        let params: SampleStorePutParams =
            serde_json::from_str(r#"{ "key": "k", "value": "v", "added_in_a_minor": 1 }"#).unwrap();

        assert_eq!(params.key, "k");
    }

    #[test]
    fn params_refuse_an_unknown_field() {
        let params = serde_json::from_str::<SampleEchoParams>(r#"{ "text": "hi", "txet": 1 }"#);

        assert!(params.is_err());
    }
}
