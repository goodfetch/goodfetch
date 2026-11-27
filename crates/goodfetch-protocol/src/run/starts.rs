//! The starts of Runs with no handler in the program (ADRs 0036 and 0102):
//! `recipe.start`, a Recipe's run, and `catalog_run.start`, a code-free
//! catalog run whose verbs come from the catalog. These and a code-free
//! crawl's start take `detached`, since nothing in the program serves them;
//! `run.start` never does, and a detached start that would die with its
//! Connection fails with `detach_unsupported`. Until the engine runs
//! Recipes and code-free targets, it refuses both starts with
//! `run_target_unbuilt`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, ProtocolVersion,
    Reach, Stability, schema_of,
};
use crate::settings::ProjectSettings;

use super::{RUN_ID_PATTERN, RunLimit, RunStarted, Tags};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a Run does with the Response recording its start names (ADR 0105).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordingMode {
    /// Every request the Run would send to a site is answered from the
    /// file, and one it lacks is refused, never sent.
    Replay,
    /// The Run fetches as usual and, when it ends, replaces the file with
    /// every response it received.
    Record,
}

/// A Response recording a start names (ADR 0105): an input of the Run,
/// never a setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    /// Whether the Run replays the file or records it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub mode: RecordingMode,
    /// The WARC 1.1 file, on the program's machine, relative to its working
    /// directory.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
}

/// What `recipe.start` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecipeStartParams {
    /// The Recipe: an installed Recipe's name, or the path to its
    /// manifest.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recipe: String,
    /// The Run's ID, as `run.start`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1", "pattern" = RUN_ID_PATTERN))]
    pub run_id: Option<String>,
    /// The Run's opaque tags.
    #[serde(default, skip_serializing_if = "Tags::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tags: Tags,
    /// The Run's settings, nested by key: the run layer (ADR 0083).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
    /// The Run's own Limits, as `run.start`'s.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Vec<RunLimit>,
    /// The Response recording the Run replays or records (ADR 0105).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recording: Option<Recording>,
    /// How long the Run may last, in milliseconds, at least 1, as
    /// `timeout.run` for this Run: absent for the setting's value, `null`
    /// for none.
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.run"))]
    pub timeout: Option<Option<u64>>,
    /// Whether the Run outlives the program (ADR 0102): closing its value
    /// then frees only the value, `run.finish` waits for its end and
    /// `run.stop` stops it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub detached: bool,
    /// Optional Group member pin in a Daemon group (ADR 0055).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group_member: Option<String>,
}

/// What a code-free catalog run does (ADR 0036): its verbs come from the
/// catalog, so nothing in the program serves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CatalogTarget {
    /// An Extraction of one page by a schema.
    Extract {
        /// The page.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
        /// The JSON Schema of what it extracts.
        #[schemars(extend("arrived_in" = "0.1"))]
        schema: Value,
        /// What to extract, in words, beside the schema.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        instruction: Option<String>,
    },
    /// An Agent task from a page (ADR 0091).
    Agent {
        /// The page it starts from.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
        /// The goal, in words.
        #[schemars(extend("arrived_in" = "0.1"))]
        task: String,
        /// Its Success check, which decides `verified`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        expect: Option<Value>,
        /// The JSON Schema of what it answers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        schema: Option<Value>,
        /// The most steps it takes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        max_steps: Option<u32>,
    },
    /// A Plan replayed with no model call (ADR 0091).
    PlanReplay {
        /// The Plan file, on the program's machine, relative to its working
        /// directory.
        #[schemars(extend("arrived_in" = "0.1"))]
        plan: String,
    },
}

impl CatalogTarget {
    /// Its kind's name on the wire.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Extract { .. } => "extract",
            Self::Agent { .. } => "agent",
            Self::PlanReplay { .. } => "plan_replay",
        }
    }
}

// ADR 0038 requires each wire request struct to carry `#[serde(deny_unknown_fields)]`
// with flat JSON Schema properties; serde's `#[serde(flatten)]` cannot be combined
// with `deny_unknown_fields`, so `CatalogRunStartParams` repeats the run-start
// fields beside `RecipeStartParams`.
// jscpd:ignore-start
/// What `catalog_run.start` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogRunStartParams {
    /// What the Run does.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: CatalogTarget,
    /// The Run's ID, as `run.start`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1", "pattern" = RUN_ID_PATTERN))]
    pub run_id: Option<String>,
    /// The Run's opaque tags.
    #[serde(default, skip_serializing_if = "Tags::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tags: Tags,
    /// The Run's settings, nested by key: the run layer (ADR 0083).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
    /// The Run's own Limits, as `run.start`'s.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Vec<RunLimit>,
    /// The Response recording the Run replays or records (ADR 0105).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recording: Option<Recording>,
    /// How long the Run may last, in milliseconds, at least 1, as
    /// `timeout.run` for this Run: absent for the setting's value, `null`
    /// for none.
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.run"))]
    pub timeout: Option<Option<u64>>,
    /// Whether the Run outlives the program (ADR 0102): closing its value
    /// then frees only the value, `run.finish` waits for its end and
    /// `run.stop` stops it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub detached: bool,
    /// Optional Group member pin in a Daemon group (ADR 0055).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group_member: Option<String>,
}
// jscpd:ignore-end

/// `recipe.start`.
pub const RECIPE_START: Operation = Operation {
    name: "recipe.start",
    description: "Starts a Recipe's run, a Run with no handler in the program, and answers once it is started. Its Run ID works as `run.start`'s. With `detached`, the Run outlives the program that started it: closing its value frees only the value, `run.finish` waits for its end and `run.stop` stops it.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Opener,
    reach: Reach::AllSurfaces,
    params: schema_of::<RecipeStartParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `catalog_run.start`.
pub const CATALOG_RUN_START: Operation = Operation {
    name: "catalog_run.start",
    description: "Starts a code-free catalog run, a Run with no handler in the program whose verbs come from the catalog (ADR 0036): an Extraction of a page by a schema, `extract`, an Agent task, `agent`, or a Plan replayed with no model call, `plan_replay`; and answers once it is started. Its Run ID works as `run.start`'s. With `detached`, the Run outlives the program that started it: closing its value frees only the value, `run.finish` waits for its end and `run.stop` stops it.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Opener,
    reach: Reach::AllSurfaces,
    params: schema_of::<CatalogRunStartParams>,
    result: schema_of::<RunStarted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A start of a Run this engine does not run yet.
pub const RUN_TARGET_UNBUILT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_target_unbuilt",
    description: "This engine does not run what the start names yet, a Recipe's run or a code-free catalog run's target, so it starts no Run: the details name the start and the target's kind.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `run_target_unbuilt`'s details hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunTargetUnbuiltDetails {
    /// The start: `recipe.start` or `catalog_run.start`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operation: String,
    /// The target's kind: `recipe`, or the code-free target's `kind`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: String,
}

/// A `detached` start of a Run that cannot outlive its Connection (ADRs
/// 0078 and 0102).
pub const DETACH_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "detach_unsupported",
    description: "A `detached` start of a Run that would die with its Connection, so no Run started: an Attached or linked engine ends with its Connection, and a Run whose handler is in the program waits on it. The details name what keeps the Run from detaching, `engine` or the key that gives it a handler in the program, and the fix.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `detach_unsupported`'s details hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DetachUnsupportedDetails {
    /// What keeps the Run from detaching: `engine`, the Connection's
    /// engine, which is no Daemon, or the key of the start that gives the
    /// Run a handler in the program.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// How to start it detached, or how to start it without `detached`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A request a replaying Run's Response recording has no response for
/// (ADR 0105).
pub const RESPONSE_NOT_RECORDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "response_not_recorded",
    description: "The Run replays a Response recording that holds no response for a request it would send, so the request was refused and nothing reached the site. The details name the request's method and URL, the recording, and the recorded URLs of the same host and path, which show what differs, such as a cache-buster in the query. Record the file again, or answer the request with a route.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `response_not_recorded`'s details hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResponseNotRecordedDetails {
    /// The request's method.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The request's URL, without its fragment.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The Response recording's path, as the start named it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recording: String,
    /// The recorded URLs of the request's host and path, in the order
    /// recorded, at most five.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub nearest: Vec<String>,
    /// What to do: record the file again, or answer the request with a
    /// route.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A Response recording the engine cannot replay (ADRs 0030 and 0105).
pub const RECORDING_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "recording_invalid",
    description: "The Response recording a start names cannot be read, is not WARC, is a WARC version the engine does not read, or holds a record that is not what it says, so no Run started. The details name the recording and what is wrong with it. Record the file again, or name another.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `recording_invalid`'s details hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordingInvalidDetails {
    /// The Response recording's path, as the start named it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recording: String,
    /// What is wrong with it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::CatalogTarget;

    /// Each code-free target's kind is its tag on the wire.
    #[rstest]
    #[case::extract(json!({ "kind": "extract", "url": "https://a.test/", "schema": {} }))]
    #[case::agent(json!({ "kind": "agent", "url": "https://a.test/", "task": "Sign up" }))]
    #[case::plan_replay(json!({ "kind": "plan_replay", "plan": "plan.json" }))]
    fn a_targets_kind_is_its_tag(#[case] wire: Value) {
        let target: CatalogTarget =
            serde_json::from_value(wire.clone()).expect("a target reads from its wire form");

        assert_eq!(json!(target.kind()), wire["kind"]);
    }
}
