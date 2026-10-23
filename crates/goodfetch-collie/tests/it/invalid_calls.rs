//! Protocol fuzzing of Collie's Attached engine over stdio (ADR 0067):
//! calls generated from the committed JSON Schema bundle, on a new engine
//! with its own Goodfetch home for each sequence. Every invalid call gets a
//! typed error and leaves the home's files as they were, its logs aside;
//! every valid call gets a typed error or a result that meets its schema.
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

/// Every file under `home` but its logs, by its path there, with a hash of
/// its bytes.
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
            if path.is_dir() {
                folders.push(path);
            } else {
                let mut hasher = DefaultHasher::new();
                std::fs::read(&path).unwrap().hash(&mut hasher);
                let relative = path.strip_prefix(home).unwrap().to_owned();
                files.insert(relative, hasher.finish());
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
