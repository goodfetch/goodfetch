//! The description `goodfetch-test-env` prints (ADR 0062): one JSON line
//! that gives a TypeScript or Python test the setup a Rust test gets from
//! `TestEnv`. Its JSON Schema, with the schemas of the crate's other file
//! types, is what `goodfetch-test-env --print-schemas` prints for
//! `xtask codegen`, which commits it among `test-support/`'s shared fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::endpoints::EndpointTable;
use crate::env::{START_TIME, TestEnv};
use crate::tools::Tools;

/// One test's setup, as the TypeScript and Python packages read it.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Description {
    /// The test's own folder, whose name holds a space and a non-ASCII
    /// letter; removed when the test ends.
    pub folder: PathBuf,
    /// `<folder>/home`, the `GOODFETCH_HOME` of everything the test starts.
    pub home: PathBuf,
    /// The cleared environment of every child the test starts.
    pub env: BTreeMap<String, String>,
    /// The test's seed, at most 2^53 - 1 so every language reads it exactly.
    #[schemars(range(max = 9_007_199_254_740_991_u64))]
    pub seed: u64,
    /// The test's start time, RFC 3339 in UTC.
    #[schemars(extend("format" = "date-time"))]
    pub start_time: String,
    /// The commit's executables, from the test tool manifest.
    pub tools: Tools,
}

impl Description {
    /// The description of a test's setup.
    pub fn new(env: &TestEnv, tools: Tools) -> Self {
        Self {
            folder: env.folder().to_owned(),
            home: env.home().to_owned(),
            env: env.env().clone(),
            seed: env.seed(),
            start_time: START_TIME.to_owned(),
            tools,
        }
    }
}

/// The JSON Schemas of the file types this crate defines, by the file name
/// `xtask codegen` commits each under.
pub fn schemas() -> BTreeMap<&'static str, serde_json::Value> {
    BTreeMap::from([
        (
            "endpoint-table.schema.json",
            schemars::schema_for!(EndpointTable).to_value(),
        ),
        (
            "test-env-description.schema.json",
            schemars::schema_for!(Description).to_value(),
        ),
        (
            "test-tool-manifest.schema.json",
            schemars::schema_for!(Tools).to_value(),
        ),
    ])
}
