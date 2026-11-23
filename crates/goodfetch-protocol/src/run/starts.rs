//! The starts of Runs with no handler in the program (ADRs 0036 and 0102):
//! `recipe.start`, a Recipe's run, and `catalog_run.start`, a code-free
//! catalog run whose verbs come from the catalog. Only these take
//! `detached`, since nothing in the program serves them; `run.start` never
//! does. Until the engine runs Recipes and code-free targets, it refuses
//! both starts with `run_target_unbuilt`.

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
}

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
