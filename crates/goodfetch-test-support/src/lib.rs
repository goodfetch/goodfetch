//! Goodfetch's shared Rust test helpers (ADR 0062): `TestEnv` (the per-test
//! folder, Goodfetch home, cleared environment, printed seed and start
//! time), the processes a test starts and the addresses they report, the
//! Proving ground started from a seed with its `resolve` mapping,
//! certificate authority and network fixtures, the
//! test tool manifest, the paused runtime, proptest's configuration, the
//! test `tracing` subscriber, the Observation log with its fencing and
//! Limits checkers, seeded kills of real processes and the crash VFS under
//! SQLite (ADR 0060), the strict reader of a child's JSON-RPC stdout
//! (ADR 0070), `fuzz_replay!`, the replay of a fuzz target's seeds and past
//! crashers, the endpoint table with `EndpointRunner`, which applies it to a
//! listener, and the SSRF case table with IANA's special-purpose registries,
//! its runner and the Scripted resolver (ADR 0067); the shared snapshot
//! filters, the locale and time zone table and the Unicode record, each
//! committed once among `test-support/fixtures/` for three languages
//! (ADRs 0062 and 0073); and `goodfetch-test-env`,
//! which gives the same
//! setup to TypeScript and Python as one JSON description. It works on
//! values only: it depends on no Goodfetch crate and is only ever a
//! dev-dependency.

mod crash_dumps;
mod crash_vfs;
mod description;
mod endpoints;
mod env;
mod fuzz;
mod invariants;
mod kill;
mod locales;
mod observation;
mod process;
mod proving_ground;
mod resolver;
mod runtime;
mod snapshot;
mod special_purpose;
mod ssrf;
mod strict_stdout;
mod tools;
mod trace;
mod unicode;
mod variables;

pub use crash_dumps::{CRASH_DUMP_VARIABLE, CrashDumps};
pub use crash_vfs::{
    AfterCrashError, Crash, CrashKind, CrashVfs, CrashVfsError, CrashedFile, Landed, after_crash,
};
pub use description::{Description, schemas};
pub use endpoints::{
    AuditUnreadable, AuditVerifier, Endpoint, EndpointRunError, EndpointRunner, Expected, Failure,
    Hello, Listener, ListenerAddress, ListenerKind, Observed, RowFailure, Rule,
};
pub use env::{START_TIME, TestEnv, TestEnvError, command};
pub use fuzz::{CRASHERS_FOLDER, FuzzReplayError, Replayed, SEEDS_FOLDER, replay_fuzz_target};
pub use invariants::{GuaranteeClass, Violation, ViolationKind, fencing, limits};
pub use kill::{KILL_DEADLINE, KillAtError, Killed, Moment, kill_at, kill_at_within};
pub use locales::{TimeZoneCase, locales, time_zones};
pub use observation::{
    Epoch, Fault, LOG_FILE, ObservationLog, Operation, REDACTED, Record, Redacted, ValueDigest,
};
pub use process::{
    Process, ProcessTracker, ProcessesAlive, REPORT_DEADLINE, Reported, ReportedAddressError,
    reported_address, reported_address_within,
};
pub use proving_ground::{
    PROVING_GROUND_DEADLINE, ProvingGround, ProvingGroundDescription, ProvingGroundError,
    ProvingGroundFixtures, ProxyCredentials,
};
pub use resolver::{Lookup, LookupFailure, ResolverHandle, ScriptedResolver};
pub use runtime::{proptest_config, proptest_config_for, runtime};
pub use snapshot::{SnapshotValues, snapshot_settings};
pub use special_purpose::{
    Cidr, Reachability, RegistryError, SpecialPurposeBlock, SpecialPurposeRegistries,
};
pub use ssrf::{
    Answer, CaseLookup, CoverageGap, CoverageGaps, EgressRule, Family, LetThrough, Miss, Outcome,
    Redirect, Refusal, Route, SsrfCase, SsrfTable, SsrfTableError, run_cases,
};
pub use strict_stdout::{
    Line, MESSAGE_DEADLINE, STDERR_GRACE, StrictStdout, StrictStdoutError, StrictStdoutErrorKind,
};
pub use tools::{Tool, Tools, ToolsError, tools};
pub use trace::{subscriber, subscriber_writing_to};
pub use unicode::{UnicodeField, UnicodeRecord, unicode_record};
pub use variables::{
    MAX_SEED, SEED_VARIABLE, SUITE_VARIABLE, SeedError, Suite, SuiteError, TOOLS_VARIABLE,
    Variables, current_test,
};
