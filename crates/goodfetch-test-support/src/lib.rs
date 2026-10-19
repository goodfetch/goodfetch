//! Goodfetch's shared Rust test helpers (ADR 0062): the per-test folder, the
//! cleared environment, seeds, reported addresses and the shared snapshot
//! settings. It depends on no Goodfetch crate and is only ever a
//! dev-dependency. The Test infrastructure spec fills it.

mod env;
mod process;
mod runtime;
mod trace;
mod variables;

pub use env::{START_TIME, TestEnv, TestEnvError, command};
pub use process::{
    Process, ProcessTracker, ProcessesAlive, REPORT_DEADLINE, Reported, ReportedAddressError,
    reported_address, reported_address_within,
};
pub use runtime::{proptest_config, proptest_config_for, runtime};
pub use trace::{subscriber, subscriber_writing_to};
pub use variables::{
    MAX_SEED, SEED_VARIABLE, SUITE_VARIABLE, SeedError, Suite, SuiteError, TOOLS_VARIABLE,
    Variables, current_test,
};
