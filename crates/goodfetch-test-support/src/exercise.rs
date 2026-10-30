//! The exercise record of the running test (ADR 0072): what it reached, for
//! `xtask features audit`. Under `just test`, which names a folder in
//! `GOODFETCH_TEST_RECORDS`, each test nextest runs has one JSON-lines file
//! there: a first line `{"test": "<binary id> <test name>"}`, then the
//! engine's JSON log lines that `TestEnv` copies from its Goodfetch home when
//! it drops, the dispatch and outcome records the test `tracing` subscriber
//! writes for an in-process test, the lines of the test's command record,
//! which the CLI helper writes, and those of its answer record, the calls a
//! Client read answered from an engine it then killed ([`AnsweredCalls`]).
//! A test run by hand, or by any runner but nextest, has none.
#![expect(
    clippy::disallowed_methods,
    reason = "test support writes the record outside any async worker (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::variables::{BINARY_ID_VARIABLE, RECORDS_VARIABLE, TEST_NAME_VARIABLE, Variables};

/// The `tracing` target of `rpc`'s dispatch records, which the test
/// subscriber writes into the record.
pub const DISPATCH_TARGET: &str = "goodfetch::rpc::dispatch";

/// The `tracing` target of the engine's outcome records: each Error code a
/// Run recorded and no reply carries, such as a work item's `handler_lost`,
/// which the subscriber writes into the record beside the dispatch records.
pub const OUTCOME_TARGET: &str = "goodfetch::run::outcome";

/// The target of the CLI helper's command records, which the exercise
/// audit reads beside the dispatch records.
pub const COMMAND_TARGET: &str = "goodfetch::test::command";

/// The test's command record, in its folder: the CLI helper's lines, which
/// teardown copies into the exercise record.
pub const COMMAND_RECORD: &str = "commands.jsonl";

/// The test's command record in `folder`.
pub(crate) fn command_record(folder: &Path) -> PathBuf {
    folder.join(COMMAND_RECORD)
}

/// The test's answer record, in its folder: the dispatch records of the
/// calls a Client read answered from an engine that did not exit by
/// itself, which teardown copies into the exercise record after the
/// command record.
pub const ANSWER_RECORD: &str = "answers.jsonl";

/// The engine's log folder in the Goodfetch home (ADR 0122).
pub(crate) const LOG_FOLDER: &str = "logs";

/// The calls one engine answered, as its Client read them (ADR 0072). A
/// test's helper records them in the test's answer record when its engine
/// ends without exiting by itself, killed or left to teardown: the engine's
/// log is lossy (ADR 0122), so a killed engine loses the dispatch records
/// its writer's thread had not written yet, though the Client read their
/// answers (#1779).
#[derive(Debug, Default)]
pub struct AnsweredCalls {
    /// Each request sent and not answered yet: its `id`, as JSON text, and
    /// its method.
    asked: BTreeMap<String, String>,
    /// Each answered call's dispatch record, in the order its answer was
    /// read.
    answered: Vec<Value>,
}

impl AnsweredCalls {
    /// Notes `message`, sent to the engine, when it is a request: a
    /// notification or a reply to the engine's own request is no call.
    pub fn sent(&mut self, message: &Value) {
        if let (Some(method), Some(id)) = (message["method"].as_str(), message.get("id"))
            && !id.is_null()
        {
            self.asked.insert(id.to_string(), method.to_owned());
        }
    }

    /// Notes `message`, read from the engine, when it answers a request
    /// [`AnsweredCalls::sent`] noted: its method, and its Error code when it
    /// failed, as `rpc`'s dispatch records them.
    pub fn read(&mut self, message: &Value) {
        if message.get("method").is_some() {
            return;
        }
        let Some(method) = message
            .get("id")
            .and_then(|id| self.asked.remove(&id.to_string()))
        else {
            return;
        };
        let fields = match message["error"]["data"]["code"].as_str() {
            Some(error_code) => json!({
                "message": "request failed, as its Client read it",
                "method": method,
                "error_code": error_code,
            }),
            None => {
                json!({ "message": "request answered, as its Client read it", "method": method })
            }
        };
        self.answered
            .push(json!({ "target": DISPATCH_TARGET, "fields": fields }));
    }

    /// Appends the calls noted since the last time to the answer record of
    /// the test whose [`TestEnv::folder`](crate::TestEnv::folder) is
    /// `folder`.
    ///
    /// # Errors
    ///
    /// When the record cannot be opened or written.
    pub fn record_in(&mut self, folder: &Path) -> io::Result<()> {
        if self.answered.is_empty() {
            return Ok(());
        }
        let mut lines = Vec::new();
        for record in self.answered.drain(..) {
            serde_json::to_writer(&mut lines, &record)?;
            lines.push(b'\n');
        }
        // One write, so lines another Client appends meanwhile stay whole.
        File::options()
            .create(true)
            .append(true)
            .open(folder.join(ANSWER_RECORD))?
            .write_all(&lines)
    }
}

/// Every line the exercise record takes from the test whose folder is
/// `folder` and whose home is `home`: its command record's, its answer
/// record's, then the engine's log files', in the files' order.
pub(crate) fn taken_lines(folder: &Path, home: &Path) -> io::Result<Vec<String>> {
    let mut logs = match std::fs::read_dir(home.join(LOG_FOLDER)) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?,
        Err(_) => Vec::new(),
    };
    logs.sort();
    let own = [command_record(folder), folder.join(ANSWER_RECORD)];
    let mut lines = Vec::new();
    for file in own.into_iter().filter(|file| file.is_file()).chain(logs) {
        let text = std::fs::read_to_string(&file)?;
        lines.extend(
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(ToOwned::to_owned),
        );
    }
    Ok(lines)
}

/// One test's exercise record.
#[derive(Clone, Debug)]
pub(crate) struct ExerciseRecord {
    path: PathBuf,
    test: String,
}

impl ExerciseRecord {
    /// The running test's record, when `just test` asked for records and
    /// nextest runs the test.
    pub(crate) fn of(variables: &Variables) -> Option<Self> {
        let folder = variables.get(RECORDS_VARIABLE)?;
        let binary = variables.get(BINARY_ID_VARIABLE)?;
        let name = variables.get(TEST_NAME_VARIABLE)?;
        let test = format!("{binary} {name}");
        Some(Self {
            path: Path::new(folder).join(format!("{}.jsonl", file_name(&test))),
            test,
        })
    }

    /// Opens the record to append to it, first writing the line that names
    /// its test when the file is new.
    pub(crate) fn open(&self) -> io::Result<File> {
        if let Some(folder) = self.path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        match File::options()
            .create_new(true)
            .append(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                let header = serde_json::json!({ "test": self.test });
                file.write_all(format!("{header}\n").as_bytes())?;
                Ok(file)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                File::options().append(true).open(&self.path)
            }
            Err(error) => Err(error),
        }
    }

    /// Appends every line [`taken_lines`] gives of the test whose folder is
    /// `folder` and whose home is `home`.
    pub(crate) fn append_logs(&self, folder: &Path, home: &Path) -> io::Result<()> {
        let mut file = self.open()?;
        for line in taken_lines(folder, home)? {
            // Each line in one write, its newline included, so no line an
            // in-process subscriber appends meanwhile lands inside it.
            file.write_all(format!("{line}\n").as_bytes())?;
        }
        Ok(())
    }
}

/// The test's ID as a file name every OS accepts: each byte outside
/// `[A-Za-z0-9_.-]` as `%XX`, so two tests never share a file.
fn file_name(test: &str) -> String {
    test.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"_.-".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
