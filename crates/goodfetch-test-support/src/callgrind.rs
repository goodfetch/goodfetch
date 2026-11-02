//! How many times a program calls a C symbol, read from Callgrind (ADR 0140):
//! what no wrapper sees, such as SQLite's own `sqlite3_prepare_v3`, whose C
//! symbol does not inline or change with Goodfetch's code.

use std::path::Path;
use std::process::{Command, Stdio};

/// Why a program's calls could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallgrindError {
    /// Valgrind could not run: on most hosts, it is not installed.
    #[error("Valgrind could not run: {0}")]
    Valgrind(#[source] std::io::Error),
    /// The program failed under Callgrind.
    #[error("{program} failed under Callgrind: {status}")]
    Failed {
        /// The program.
        program: String,
        /// How it ended.
        status: std::process::ExitStatus,
    },
    /// Callgrind's output could not be kept or read.
    #[error("Callgrind's output could not be read: {0}")]
    Output(#[source] std::io::Error),
}

/// How many times `program`, run with `arguments` under Callgrind, called
/// `symbol`.
///
/// # Errors
///
/// [`CallgrindError`]: when Valgrind cannot run, the program fails or the
/// output cannot be read.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark harness's synchronous child and its one output file, on no async worker"
)]
pub fn calls(symbol: &str, program: &Path, arguments: &[&str]) -> Result<u64, CallgrindError> {
    let output = tempfile::NamedTempFile::new().map_err(CallgrindError::Output)?;
    let status = Command::new("valgrind")
        .args(["--tool=callgrind", "--quiet", "--compress-strings=no"])
        .arg(format!("--callgrind-out-file={}", output.path().display()))
        .arg(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(CallgrindError::Valgrind)?;
    if !status.success() {
        return Err(CallgrindError::Failed {
            program: program.display().to_string(),
            status,
        });
    }
    let text = std::fs::read_to_string(output.path()).map_err(CallgrindError::Output)?;
    Ok(calls_in(&text, symbol))
}

/// What `text`, a Callgrind output with its strings uncompressed, counts as
/// calls of `symbol`: the `calls=` line after each `cfn=` naming it, one per
/// calling function.
fn calls_in(text: &str, symbol: &str) -> u64 {
    let mut lines = text.lines();
    let mut calls = 0;
    while let Some(line) = lines.next() {
        if line.strip_prefix("cfn=") != Some(symbol) {
            continue;
        }
        let count = lines
            .by_ref()
            .find_map(|line| line.strip_prefix("calls="))
            .and_then(|call| call.split_whitespace().next())
            .and_then(|count| count.parse::<u64>().ok());
        calls += count.unwrap_or(0);
    }
    calls
}

#[cfg(test)]
mod tests;
