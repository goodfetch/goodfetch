//! Goodfetch's shared Rust test helpers (ADR 0062): `TestEnv` (the per-test
//! folder, Goodfetch home, cleared environment, printed seed and start
//! time), the processes a test starts and the addresses they report, the
//! test tool manifest, the paused runtime, proptest's configuration and the
//! test `tracing` subscriber; and `goodfetch-test-env`, which gives the same
//! setup to TypeScript and Python as one JSON description. It works on
//! values only: it depends on no Goodfetch crate and is only ever a
//! dev-dependency.

mod description;
mod env;
mod process;
mod runtime;
mod tools;
mod trace;
mod variables;

pub use description::{Description, schemas};
pub use env::{START_TIME, TestEnv, TestEnvError, command};
pub use process::{
    Process, ProcessTracker, ProcessesAlive, REPORT_DEADLINE, Reported, ReportedAddressError,
    reported_address, reported_address_within,
};
pub use runtime::{proptest_config, proptest_config_for, runtime};
pub use tools::{Tool, Tools, ToolsError, tools};
pub use trace::{subscriber, subscriber_writing_to};
pub use variables::{
    MAX_SEED, SEED_VARIABLE, SUITE_VARIABLE, SeedError, Suite, SuiteError, TOOLS_VARIABLE,
    Variables, current_test,
};
