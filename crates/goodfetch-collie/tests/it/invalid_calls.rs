//! Protocol fuzzing of Collie's Attached engine over stdio (ADR 0067):
//! calls generated from the committed JSON Schema bundle, on a new engine
//! with its own Goodfetch home for each sequence. Every invalid call gets a
//! typed error and leaves the home's files as they were, its logs aside and
//! each SQLite database compared by what it holds; every valid call gets a typed error or a result that meets its schema.
//! It runs `proptest_config()`'s cases on the test's seed, and declares no
//! Feature: the protocol fuzzing story stays outside the Feature inventory.
#![expect(
    clippy::disallowed_methods,
    reason = "the state read walks the Goodfetch home inside a test, outside any async worker (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

use goodfetch_test_support::{
    ProtocolFuzz, ProtocolSchema, ProtocolTransport, TestEnv, TransportFailure, proptest_config,
    sql_dump,
};
use serde_json::Value;

use crate::attached::{Attached, hello};

/// The operations whose valid calls answer at once: `events.subscribe`
/// and `page.body` stay in flight until cancelled or credited.
const ANSWERING_AT_ONCE: [&str; 6] = [
    "config.explain",
    "engine.info",
    "run.abort",
    "run.finish",
    "run.start",
    "sample.echo",
];

/// An Attached engine that answered `hello` and finished its startup
/// writes, with its Goodfetch home.
struct Fuzzed<'env> {
    engine: Option<Attached<'env>>,
    home: PathBuf,
}

impl<'env> Fuzzed<'env> {
    fn start(env: &'env TestEnv) -> Self {
        let home = env.folder().join(env.unique_name("home"));
        let mut engine = Attached::start_with(env, |command| {
            command.env("GOODFETCH_HOME", &home);
        });
        engine.send(&hello("collie", "0.1"));
        let accepted = engine.next().unwrap();
        assert!(accepted.get("result").is_some(), "{accepted}");
        let mut fuzzed = Self {
            engine: Some(engine),
            home,
        };
        fuzzed.wait_for_the_startup_reclaim();
        fuzzed
    }

    /// Waits until the engine's Runs have reclaimed the Slices of dead runs
    /// from the Usage ledger, which they do once at start, after `hello`
    /// may have been answered, and before they answer any Run call (ADR
    /// 0041): until then the ledger's SQLite journal comes and goes in the
    /// home. A `run.finish` of a Run no engine started is the Run call that
    /// writes nothing.
    fn wait_for_the_startup_reclaim(&mut self) {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "run.finish",
            "params": { "run_id": "never-started" },
        });
        let answer = self.call(&request).unwrap();
        assert_eq!(answer["error"]["data"]["code"], "run_not_found", "{answer}");
    }
}

impl ProtocolTransport for Fuzzed<'_> {
    fn call(&mut self, request: &Value) -> Result<Value, TransportFailure> {
        let engine = self.engine.as_mut().unwrap();
        engine.send(request);
        loop {
            let message = engine
                .next()
                .ok_or_else(|| TransportFailure::new("the engine closed its stdout"))?;
            if message.get("id") == request.get("id") {
                return Ok(message);
            }
        }
    }
}

impl Drop for Fuzzed<'_> {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            let _ = engine.finish();
        }
    }
}

/// The Goodfetch home's log folder (ADR 0122), which records every call.
const LOG_FOLDER: &str = "logs";

/// What a SQLite database file starts with.
const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";

/// The suffixes of the files SQLite keeps beside a database while
/// connections use it: its write-ahead log, the log's shared-memory index
/// and its rollback journal.
const SQLITE_SIDE_FILES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// A hash of what the SQLite database at `path` holds: its schema and rows,
/// as a connection reads them through its write-ahead log.
fn database_hash(path: &Path) -> u64 {
    let connection = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap_or_else(|error| panic!("{} does not open: {error}", path.display()));
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let dump = sql_dump(&connection)
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", path.display()));
    let mut hasher = DefaultHasher::new();
    dump.hash(&mut hasher);
    hasher.finish()
}

/// Every file under `home` but its logs, by its path there, with a hash of
/// its bytes, or of its schema and rows for a SQLite database, whose side
/// files are left out: the engine's own reads of its Run index at each
/// storage sample, whatever the calls, create and remove them, and a
/// checkpoint moves the log into the database without changing what it
/// holds.
fn files(home: &Path) -> BTreeMap<PathBuf, u64> {
    let mut files = BTreeMap::new();
    let mut folders = vec![home.to_owned()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path == home.join(LOG_FOLDER) {
                continue;
            }
            let side_file = path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                SQLITE_SIDE_FILES
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            });
            if path.is_dir() {
                folders.push(path);
            } else if !side_file {
                let bytes = std::fs::read(&path).unwrap();
                let hash = if bytes.starts_with(SQLITE_HEADER) {
                    database_hash(&path)
                } else {
                    let mut hasher = DefaultHasher::new();
                    bytes.hash(&mut hasher);
                    hasher.finish()
                };
                let relative = path.strip_prefix(home).unwrap().to_owned();
                files.insert(relative, hash);
            }
        }
    }
    files
}

#[test]
fn every_invalid_call_over_stdio_gets_a_typed_error_and_changes_no_file_of_the_home() {
    let env = TestEnv::new();
    let fuzz = ProtocolFuzz::new(ProtocolSchema::committed().unwrap())
        .with_valid_calls(&ANSWERING_AT_ONCE);
    let mut config = proptest_config();
    config.source_file = Some(file!());

    let fuzzed = fuzz.run(config, || Fuzzed::start(&env), |engine| files(&engine.home));

    if let Err(failure) = fuzzed {
        panic!("{failure:#?}");
    }
}
