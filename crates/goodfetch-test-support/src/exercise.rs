//! The exercise record of the running test (ADR 0072): what it reached, for
//! `xtask features audit`. Under `just test`, which names a folder in
//! `GOODFETCH_TEST_RECORDS`, each test nextest runs has one JSON-lines file
//! there: a first line `{"test": "<binary id> <test name>"}`, then the
//! engine's JSON log lines that `TestEnv` copies from its Goodfetch home when
//! it drops, and the dispatch records the test `tracing` subscriber writes
//! for an in-process test. A test run by hand, or by any runner but nextest,
//! has none.
#![expect(
    clippy::disallowed_methods,
    reason = "test support writes the record outside any async worker (ADR 0062)"
)]

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::variables::{BINARY_ID_VARIABLE, TEST_NAME_VARIABLE, Variables};

/// The folder of every test's exercise record, which `just test` sets.
pub const RECORDS_VARIABLE: &str = "GOODFETCH_TEST_RECORDS";

/// The `tracing` target of `rpc`'s dispatch records, which the test
/// subscriber writes into the record.
pub const DISPATCH_TARGET: &str = "goodfetch::rpc::dispatch";

/// The engine's log folder in the Goodfetch home (ADR 0122).
pub(crate) const LOG_FOLDER: &str = "logs";

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
                writeln!(file, "{header}")?;
                Ok(file)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                File::options().append(true).open(&self.path)
            }
            Err(error) => Err(error),
        }
    }

    /// Appends every line of the engine's log files in `home`, in the
    /// files' order.
    pub(crate) fn append_logs(&self, home: &Path) -> io::Result<()> {
        let mut file = self.open()?;
        let folder = home.join(LOG_FOLDER);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return Ok(());
        };
        let mut logs = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        logs.sort();
        for log in logs {
            let text = std::fs::read_to_string(&log)?;
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                writeln!(file, "{line}")?;
            }
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
