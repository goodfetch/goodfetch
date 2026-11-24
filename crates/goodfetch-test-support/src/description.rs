//! The description `goodfetch-test-env` prints (ADR 0062): one JSON line
//! that gives a TypeScript or Python test the setup a Rust test gets from
//! `TestEnv`. Its JSON Schema, with the schemas of the crate's other file
//! types, the Scripted engine's script format among them, is what
//! `goodfetch-test-env --print-schemas` prints for `xtask codegen`, which
//! commits it among `test-support/`'s shared fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::daemon::DaemonDescription;
use crate::endpoints::EndpointTable;
use crate::env::{START_TIME, TestEnv};
use crate::fault_plan::FaultPlan;
use crate::hostile::{
    HeaderValue, HostileTable, PathName, ResourceLimit, SchemaRef, SecretRedirect,
};
use crate::locales::LocaleTable;
use crate::proving_ground::ProvingGroundDescription;
use crate::proving_ground_format::{
    ArticleAnswers, Observations, OracleSubmission, ROUTES, Routes, ServeLine,
};
use crate::recordings::ModelRecording;
use crate::script::Script;
use crate::snapshot::SnapshotFilters;
use crate::ssrf::{SsrfLayers, SsrfTable};
use crate::tools::Tools;
use crate::unicode::UnicodeRecord;
use crate::webhook_signatures::WebhookSignatures;

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
    /// The Daemon `--daemon` started in the test's home, present only when
    /// asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "DaemonDescription")]
    pub daemon: Option<DaemonDescription>,
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
            daemon: None,
        }
    }

    /// The same description with the Daemon the test started.
    #[must_use]
    pub fn with_daemon(mut self, daemon: DaemonDescription) -> Self {
        self.daemon = Some(daemon);
        self
    }

    /// The same description with the Proving ground the test started.
    #[must_use]
    pub fn with_proving_ground(mut self, proving_ground: ProvingGroundDescription) -> Self {
        self.proving_ground = Some(proving_ground);
        self
    }
}

/// The JSON Schemas of the file types this crate defines, by the path
/// `xtask codegen` commits each under, relative to `test-support/fixtures/`:
/// `schemas/` for most, the script format's among the engine scripts it
/// describes (ADR 0069), and `proving-ground/` the Proving ground's own
/// routes, a fixture the TypeScript and Python test support read.
pub fn schemas() -> BTreeMap<&'static str, serde_json::Value> {
    BTreeMap::from([
        (
            "engine-scripts/script.schema.json",
            schemars::schema_for!(Script).to_value(),
        ),
        (
            "schemas/endpoint-table.schema.json",
            schemars::schema_for!(EndpointTable).to_value(),
        ),
        (
            "schemas/fault-plan.schema.json",
            schemars::schema_for!(FaultPlan).to_value(),
        ),
        (
            "schemas/hostile-header-values.schema.json",
            schemars::schema_for!(HostileTable<HeaderValue>).to_value(),
        ),
        (
            "schemas/hostile-limits.schema.json",
            schemars::schema_for!(HostileTable<ResourceLimit>).to_value(),
        ),
        (
            "schemas/hostile-paths.schema.json",
            schemars::schema_for!(HostileTable<PathName>).to_value(),
        ),
        (
            "schemas/hostile-schema-refs.schema.json",
            schemars::schema_for!(HostileTable<SchemaRef>).to_value(),
        ),
        (
            "schemas/hostile-secret-redirects.schema.json",
            schemars::schema_for!(HostileTable<SecretRedirect>).to_value(),
        ),
        (
            "schemas/locales.schema.json",
            schemars::schema_for!(LocaleTable).to_value(),
        ),
        (
            "proving-ground/routes.json",
            serde_json::to_value(ROUTES).expect("invariant: the routes serialize"),
        ),
        (
            "schemas/model-recording.schema.json",
            schemars::schema_for!(ModelRecording).to_value(),
        ),
        (
            "schemas/proving-ground-article-answers.schema.json",
            schemars::schema_for!(ArticleAnswers).to_value(),
        ),
        (
            "schemas/proving-ground-observations.schema.json",
            schemars::schema_for!(Observations).to_value(),
        ),
        (
            "schemas/proving-ground-oracle-submission.schema.json",
            schemars::schema_for!(OracleSubmission).to_value(),
        ),
        (
            "schemas/proving-ground-routes.schema.json",
            schemars::schema_for!(Routes).to_value(),
        ),
        (
            "schemas/proving-ground-serve-line.schema.json",
            schemars::schema_for!(ServeLine).to_value(),
        ),
        (
            "schemas/snapshot-filters.schema.json",
            schemars::schema_for!(SnapshotFilters).to_value(),
        ),
        (
            "schemas/ssrf-cases.schema.json",
            schemars::schema_for!(SsrfTable).to_value(),
        ),
        (
            "schemas/ssrf-layers.schema.json",
            schemars::schema_for!(SsrfLayers).to_value(),
        ),
        (
            "schemas/test-env-description.schema.json",
            schemars::schema_for!(Description).to_value(),
        ),
        (
            "schemas/test-tool-manifest.schema.json",
            schemars::schema_for!(Tools).to_value(),
        ),
        (
            "schemas/unicode-record.schema.json",
            schemars::schema_for!(UnicodeRecord).to_value(),
        ),
        (
            "schemas/webhook-signatures.schema.json",
            schemars::schema_for!(WebhookSignatures).to_value(),
        ),
    ])
}
