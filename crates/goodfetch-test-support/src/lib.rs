//! Goodfetch's shared Rust test helpers (ADR 0062): `TestEnv` (the per-test
//! folder, Goodfetch home, cleared environment, printed seed and start
//! time), the processes a test starts and the addresses they report, the
//! test tool manifest, the paused runtime, proptest's configuration, the
//! test `tracing` subscriber, the Observation log with its fencing and
//! Limits checkers, and seeded kills of real processes (ADR 0060); and
//! `goodfetch-test-env`, which gives the same
//! setup to TypeScript and Python as one JSON description. It works on
//! values only: it depends on no Goodfetch crate and is only ever a
//! dev-dependency.

mod description;
mod env;
mod fuzz;
mod invariants;
mod kill;
mod observation;
mod process;
mod runtime;
mod tools;
mod trace;
mod variables;

pub use description::{Description, schemas};
pub use env::{START_TIME, TestEnv, TestEnvError, command};
pub use fuzz::{CRASHERS_FOLDER, FuzzReplayError, Replayed, SEEDS_FOLDER, replay_fuzz_target};
pub use invariants::{GuaranteeClass, Violation, ViolationKind, fencing, limits};
pub use kill::{KILL_DEADLINE, KillAtError, Killed, Moment, kill_at, kill_at_within};
pub use observation::{
    Epoch, Fault, LOG_FILE, ObservationLog, Operation, REDACTED, Record, Redacted, ValueDigest,
};
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
