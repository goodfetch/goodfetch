//! The fixture's stand-in of test support: one type whose schema
//! `goodfetch-test-env --print-schemas` prints.

/// One test's setup.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct Description {
    /// The test's seed.
    pub seed: u64,
}
