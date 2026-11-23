//! Goodfetch's shared Rust test helpers (ADR 0062): `TestEnv` (the per-test
//! folder, Goodfetch home, cleared environment, printed seed and start
//! time), the processes a test starts and the addresses they report, the
//! Proving ground started from a seed with its `resolve` mapping,
//! certificate authority and network fixtures, the
//! test tool manifest, `JourneyRun`, which runs a Rust Journey variant and
//! writes its result document through the constructor its test passes, the
//! paused runtime and the hold of its blocking pool, proptest's configuration, the
//! test `tracing` subscriber and `logged`, which captures what `tracing`
//! wrote, the size-capped tmpfs a test fills on Linux, the Observation log with its fencing and
//! Limits checkers, Fault plans with the generator that shrinks a failing
//! one and writes it with its replay command, seeded kills of real processes and the crash VFS under
//! SQLite (ADR 0060), the byte-stable SQL dump of a SQLite store (ADR 0066),
//! the lines of a Run trace's raw call log (ADR 0099), the strict reader of a child's JSON-RPC stdout
//! (ADR 0070), `fuzz_replay!`, the replay of a fuzz target's seeds and past
//! crashers, `ProtocolFuzz`, the protocol fuzzing generated from the
//! committed JSON Schema bundle over the caller's transport, the endpoint table with `EndpointRunner`, which applies it to a
//! listener, and the SSRF case table with IANA's special-purpose registries,
//! its runner and the Scripted resolver, the hostile-input case tables,
//! `run_host_test`, which plays a host program as an ignored test of the
//! running test binary and reads what it wrote past the harness,
//! `NoExec`, which starts a child that may start no other program on Linux,
//! the hosts that refuse Chromium's sandbox its user namespace
//! (`under_default_seccomp`, `under_apparmor_restriction`),
//! `DnsNamespace`, which starts a child on Linux whose resolver is the
//! Proving ground's DNS fixture,
//! and the Secret canaries, planted and searched for in every encoding
//! with their `gitleaks` rule (ADR 0067); `refused_matches`, which finds
//! what would let a new state or event through a transition table's
//! source unseen (ADR 0128); the shared snapshot
//! filters, the locale and time zone table and the Unicode record, each
//! committed once among `test-support/fixtures/` for three languages
//! (ADRs 0062 and 0073); the Scripted engine's script format and its
//! player, and the Daemon's runtime file (ADRs 0024 and 0069);
//! `goodfetch-test-env`, which gives the same setup to TypeScript and
//! Python as one JSON description; the CLI helper, `cli`, which runs a
//! Product's executable and records each command it ran, checks `--json`
//! output against the committed bundle, runs a command in its `attached`
//! and `daemon` modes, with the shared table's locale cases, and `pty`, which runs it on a pseudo-terminal, with
//! the colour property cases; and
//! `goodfetch-scripted-engine`, which plays a script over its stdio, as a
//! Daemon found through the runtime file, or on loopback TCP. It
//! works on values only: it depends on no Goodfetch crate and is only ever
//! a dev-dependency.

mod browsers;
pub mod callgrind;
mod canaries;
mod cargo_consumer;
mod claude_code;
mod cli;
mod colour;
mod command_tree;
mod counting_vfs;
mod crash_dumps;
mod crash_vfs;
mod daemon;
mod description;
#[cfg(target_os = "linux")]
mod dns_namespace;
mod endpoints;
mod env;
mod exercise;
mod facts;
mod fault_plan;
mod fuzz;
mod host_test;
mod hostile;
mod invariants;
mod journey_run;
mod kill;
#[cfg(target_os = "linux")]
mod lazyfs;
mod locales;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod no_console;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod no_exec;
mod observation;
mod player;
mod process;
mod protocol_fuzz;
mod proving_ground;
mod proving_ground_format;
mod proxy_in_front;
mod pty;
mod raw_calls;
mod received_requests;
mod resident;
mod resolver;
mod runtime;
mod runtime_file;
mod script;
mod sdk_items;
mod seeded;
mod snapshot;
mod special_purpose;
mod sql_dump;
mod sqlite_files;
mod ssrf;
mod strict_stdout;
mod tmpfs;
mod tools;
mod trace;
mod transition_tables;
mod unicode;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod user_namespaces;
mod variables;

#[cfg(unix)]
pub use browsers::ReadOnlyBrowserCache;
pub use browsers::{
    BROWSERS_PATH_VARIABLE, BrowserCacheError, CHROME_FOR_TESTING_DOWNLOADS_VARIABLE,
    DebuggedBrowser, LIGHTPANDA_DOWNLOADS_VARIABLE, browser_builds, browser_cache,
    browser_cache_in_home, browser_cache_with_headless_shell, browser_cache_with_lightpanda,
    chrome_for_testing, chrome_for_testing_debugged, chrome_for_testing_dumping_dom,
    chrome_for_testing_files, chrome_headless, chrome_headless_shell, empty_browser_cache,
    lay_out_browser_entry, lightpanda_copy, reported_debugging_port, skipped_without_gui_session,
};
pub use canaries::{
    Canaries, Canary, CanaryError, Encoding, Hit, Output, Placement, PlantAccount, SHORT_VALUE,
    SecretKind, Sighting, gitleaks_rule,
};
pub use cargo_consumer::{CargoRun, Consumer, ConsumerError, cargo_consumer, cargo_install};
pub use claude_code::{
    CLAUDE_CODE_DOWNLOADS_VARIABLE, ClaudeCodeError, PluginValidation, claude_code_copy,
    validate_plugin,
};
pub use cli::{
    Cli, CliError, CliJson, CliOutput, Mode, Stream, cli, record_command, record_command_in,
};
pub use colour::{ColourError, ColourProperty};
pub use command_tree::{
    certificate_verification_switches, command_help, command_paths, long_flags, unknown_options,
};
pub use counting_vfs::CountingVfs;
pub use crash_dumps::{CRASH_DUMP_VARIABLE, CrashDumps};
pub use crash_vfs::{
    AfterCrashError, Crash, CrashKind, CrashVfs, CrashVfsError, CrashedFile, IoFault, Landed,
    after_crash,
};
pub use daemon::{DAEMON_DEADLINE, Daemon, DaemonDescription, DaemonError};
pub use description::{Description, schemas};
#[cfg(target_os = "linux")]
pub use dns_namespace::{
    DNS_NAMESPACE_COMMAND, DnsNamespace, DnsNamespaceError, Inside, InsideUser, Mechanism,
    MechanismRefusal,
};
pub use endpoints::{
    AuditUnreadable, AuditVerifier, Endpoint, EndpointRunError, EndpointRunner, Expected, Failure,
    Hello, Listener, ListenerAddress, ListenerKind, Observed, RowFailure, Rule,
};
pub use env::{START_TIME, TEST_SETTINGS, TestEnv, TestEnvError, command};
pub use exercise::{ANSWER_RECORD, AnsweredCalls, COMMAND_RECORD, COMMAND_TARGET};
pub use facts::{CHROME_FOR_TESTING_BUILD, DISPATCH_TARGET, LIGHTPANDA_BUILD, OUTCOME_TARGET};
pub use fault_plan::{
    FAULT_PLAN_FILE, FAULT_PLAN_SOURCE_FILE, FaultPlan, FaultPlanAction, FaultPlanError,
    FaultPlanFailure, FaultPlanGenerator, FaultPlanStep,
};
pub use fuzz::{CRASHERS_FOLDER, FuzzReplayError, Replayed, SEEDS_FOLDER, replay_fuzz_target};
pub use host_test::{HOST_DEADLINE, HostRun, past_the_harness, run_host_test};
pub use hostile::{
    BodyFraming, ExpectedError, HeaderSource, HeaderValue, HostileCase, HostileTable,
    HostileTableError, HostileTables, JsonChannel, LimitSetting, MessageChannel, Os, PathHazard,
    PathName, PathTarget, RefKeyword, ResourceLimit, Schedule, SchemaRef, SchemaUse,
    SecretPlacement, SecretRedirect, ServedBy, TableGap, TableGaps,
};
/// insta, for [`assert_cli_snapshot!`], whose expansion names it.
#[doc(hidden)]
pub use insta as __insta;
pub use invariants::{GuaranteeClass, Violation, ViolationKind, fencing, limits};
pub use journey_run::{
    BodyResult, Finished, JourneyRun, JourneyRunError, UnfinishedJourneyRun, VariantNotProved,
    journey_engine,
};
pub use kill::{KILL_DEADLINE, KillAtError, Killed, Moment, kill_at, kill_at_within};
#[cfg(target_os = "linux")]
pub use lazyfs::{LAZYFS_DEADLINE, LazyFs, LazyFsError};
pub use locales::{LocaleTableError, TimeZoneCase, locales, time_zones};
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use no_console::{console_address_answer, runtime_file_members};
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use no_exec::NoExec;
pub use observation::{
    Epoch, Fault, LOG_FILE, ObservationLog, Operation, REDACTED, Record, Redacted, ValueDigest,
};
pub use player::{
    ATTACHED_ENVIRONMENT, ATTACHED_RECORD, ATTACHED_SCRIPT, Ending, PlayError, play, play_relinking,
};
pub use process::{
    Process, ProcessTracker, ProcessesAlive, REPORT_DEADLINE, Reported, ReportedAddressError,
    reported_address, reported_address_within,
};
pub use protocol_fuzz::{
    Broken, FuzzedCall, Invalidity, ProtocolFuzz, ProtocolFuzzError, ProtocolSchema,
    ProtocolTransport, SchemaUnreadable, TransportFailure,
};
pub use proving_ground::{
    BROWSER_BUILDS_HOST, ClientCertificate, LOOPBACK_PORT_0, PROVING_GROUND_DEADLINE,
    ProvingGround, ProvingGroundDescription, ProvingGroundError, ProvingGroundFixtures, UNSHAPED,
    path_of, serve_arguments,
};
pub use proving_ground_format::{
    ArticleAnswer, ArticleAnswers, ArticleLink, FixtureAddresses, OWN_TARGETS, Observations,
    ObservedRequest, OracleSubmission, ProxyCredentials, ROUTES, ReceivedRequests, RequestTiming,
    Routes, ServeLine, ToolStatus,
};
pub use proxy_in_front::proxy_in_front;
pub use pty::{Pty, PtyError, PtyOutput, PtyRun, pty};
pub use raw_calls::raw_call_lines;
pub use resident::{peak_resident_bytes, private_resident_bytes_of};
pub use resolver::{Lookup, LookupFailure, ResolverHandle, ScriptedResolver};
pub use runtime::{
    proptest_config, proptest_config_for, real_time_runtime, runtime,
    runtime_with_one_blocking_thread, with_the_blocking_pool_held,
};
pub use runtime_file::{
    DAEMON_SERVING_LOCK, DAEMON_SOCKET, DaemonEndpoint, RUNTIME_FILE, RuntimeFile,
    RuntimeFileError, SCRIPTED_DAEMON_TOKEN, SOCKET_FOLDER_PREFIX,
};
pub use script::{
    Call, ENGINE_SCRIPTS, Exit, Expectation, Raw, Reply, Script, ScriptError, Sent, StderrLines,
    Step, engine_script,
};
pub use sdk_items::{public_methods, public_types_with_lifetimes, re_exports, sdk_sources};
pub use snapshot::{SnapshotValues, redact, snapshot_settings};
pub use special_purpose::{
    Cidr, Reachability, RegistryError, SpecialPurposeBlock, SpecialPurposeRegistries,
};
pub use sql_dump::sql_dump;
pub use sqlite_files::{DamageTableError, damage_table, wal_of};
pub use ssrf::{
    Answer, CaseDomains, CaseLookup, CoverageGap, CoverageGaps, EgressRule, Family, Layer,
    LayerGap, LayerGaps, LayerRunner, LetThrough, Miss, Outcome, Pending, Redirect, Refusal, Route,
    RunFailed, Runner, RunnerError, SsrfCase, SsrfLayers, SsrfTable, SsrfTableError, run_cases,
};
pub use strict_stdout::{
    Line, MESSAGE_DEADLINE, STDERR_GRACE, STDERR_LIMIT, StrictStdout, StrictStdoutError,
    StrictStdoutErrorKind,
};
pub use tmpfs::{
    fill_filesystem, free_filesystem, inside_mount, read_only_mounted, tmpfs_mounted,
    tmpfs_mounted_home,
};
pub use tools::{Product, Tool, Tools, ToolsError, tools};
pub use trace::{
    CapturedLog, capturing, exercise_layer, logged, subscriber, subscriber_unless_own,
    subscriber_writing_to,
};
pub use transition_tables::refused_matches;
pub use unicode::{UnicodeField, UnicodeRecord, unicode_record};
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use user_namespaces::{
    apparmor_restricts_user_namespaces, under_apparmor_restriction, under_default_seccomp,
};
pub use variables::{
    CALLGRIND_AT_ONCE_VARIABLE, CASE_LOGS_VARIABLE, CallgrindAtOnceError, DNS_NAMESPACE_VARIABLE,
    ENGINE_VARIABLE, LAZYFS_FOLDER_VARIABLE, LAZYFS_VARIABLE, MAX_SEED, NO_GUI_SESSION_VARIABLE,
    RECORDS_VARIABLE, RESULT_DOCUMENTS_VARIABLE, SEED_VARIABLE, SUITE_VARIABLE, SeedError, Suite,
    SuiteError, TOOLS_VARIABLE, Variables, case_logs, current_test,
};
