//! The fixture's stand-in of test support: the types whose schemas
//! `goodfetch-test-env --print-schemas` prints.

/// One test's setup.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct Description {
    /// The test's seed.
    pub seed: u64,
}

/// A script the Scripted engine plays.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct Script {
    /// What the script plays.
    pub description: String,
}
