//! The description `goodfetch-test-env` prints (ADR 0062): one JSON line
//! that gives a TypeScript or Python test the setup a Rust test gets from
//! `TestEnv`. Its JSON Schema, with the schemas of the crate's other file
//! types, is what `goodfetch-test-env --print-schemas` prints for
//! `xtask codegen`, which commits it among `test-support/`'s shared fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::endpoints::EndpointTable;
use crate::env::{START_TIME, TestEnv};
use crate::hostile::{
    HeaderValue, HostileTable, PathName, ResourceLimit, SchemaRef, SecretRedirect,
};
use crate::locales::LocaleTable;
use crate::proving_ground::ProvingGroundDescription;
use crate::snapshot::SnapshotFilters;
use crate::ssrf::SsrfTable;
use crate::tools::Tools;
use crate::unicode::UnicodeRecord;

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
    /// The Proving ground `--proving-ground <seed>` started, present only
    /// when asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "ProvingGroundDescription")]
    pub proving_ground: Option<ProvingGroundDescription>,
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
            proving_ground: None,
        }
    }

    /// The same description with the Proving ground the test started.
    #[must_use]
    pub fn with_proving_ground(mut self, proving_ground: ProvingGroundDescription) -> Self {
        self.proving_ground = Some(proving_ground);
        self
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
            "hostile-header-values.schema.json",
            schemars::schema_for!(HostileTable<HeaderValue>).to_value(),
        ),
        (
            "hostile-limits.schema.json",
            schemars::schema_for!(HostileTable<ResourceLimit>).to_value(),
        ),
        (
            "hostile-paths.schema.json",
            schemars::schema_for!(HostileTable<PathName>).to_value(),
        ),
        (
            "hostile-schema-refs.schema.json",
            schemars::schema_for!(HostileTable<SchemaRef>).to_value(),
        ),
        (
            "hostile-secret-redirects.schema.json",
            schemars::schema_for!(HostileTable<SecretRedirect>).to_value(),
        ),
        (
            "locales.schema.json",
            schemars::schema_for!(LocaleTable).to_value(),
        ),
        (
            "snapshot-filters.schema.json",
            schemars::schema_for!(SnapshotFilters).to_value(),
        ),
        (
            "ssrf-cases.schema.json",
            schemars::schema_for!(SsrfTable).to_value(),
        ),
        (
            "test-env-description.schema.json",
            schemars::schema_for!(Description).to_value(),
        ),
        (
            "test-tool-manifest.schema.json",
            schemars::schema_for!(Tools).to_value(),
        ),
        (
            "unicode-record.schema.json",
            schemars::schema_for!(UnicodeRecord).to_value(),
        ),
    ])
}
