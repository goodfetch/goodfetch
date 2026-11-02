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
pub fn calls(symbol: &str, program: &Path, arguments: &[&str]) -> Result<u64, CallgrindError> {
    let text = output(&[], program, arguments)?;
    Ok(calls_in(&text, symbol))
}

/// How many times a program called a symbol: in all, and beneath a caller
/// [`calls_beneath`] names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calls {
    /// Every call.
    pub all: u64,
    /// The calls with a caller whose name holds the text asked for among
    /// their nearest callers, and none whose name holds a text left out.
    pub beneath: u64,
}

/// How many times `program`, run with `arguments` under Callgrind, called
/// `symbol`, in all and beneath a caller whose name holds `beneath` among
/// its `depth` nearest callers, leaving out a call with a caller whose name
/// holds one of `except` there: what one owner's code prepares on SQLite,
/// whichever function of SQLite's wrapper it went through.
///
/// # Errors
///
/// As [`calls`].
pub fn calls_beneath(
    symbol: &str,
    depth: u32,
    beneath: &str,
    except: &[&str],
    program: &Path,
    arguments: &[&str],
) -> Result<Calls, CallgrindError> {
    let separate = format!("--separate-callers{depth}={symbol}");
    let text = output(&[&separate], program, arguments)?;
    Ok(calls_beneath_in(&text, symbol, beneath, except))
}

/// What `program`, run with `arguments` under Callgrind and its `options`,
/// wrote, its strings uncompressed.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark harness's synchronous child and its one output file, on no async worker"
)]
fn output(options: &[&str], program: &Path, arguments: &[&str]) -> Result<String, CallgrindError> {
    let output = tempfile::NamedTempFile::new().map_err(CallgrindError::Output)?;
    let status = Command::new("valgrind")
        .args(["--tool=callgrind", "--quiet", "--compress-strings=no"])
        .args(options)
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
    std::fs::read_to_string(output.path()).map_err(CallgrindError::Output)
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

/// What `text`, a Callgrind output with `symbol`'s callers separated, counts
/// as calls of `symbol` in all and beneath a caller holding `beneath` and
/// none holding one of `except`: each context's name is the symbol's, then
/// its callers', nearest first, joined by `'`.
fn calls_beneath_in(text: &str, symbol: &str, beneath: &str, except: &[&str]) -> Calls {
    let mut lines = text.lines();
    let mut calls = Calls::default();
    while let Some(line) = lines.next() {
        let Some(context) = line.strip_prefix("cfn=") else {
            continue;
        };
        let mut names = context.split('\'');
        if names.next() != Some(symbol) {
            continue;
        }
        let callers: Vec<&str> = names.collect();
        let count = lines
            .by_ref()
            .find_map(|line| line.strip_prefix("calls="))
            .and_then(|call| call.split_whitespace().next())
            .and_then(|count| count.parse::<u64>().ok())
            .unwrap_or(0);
        calls.all += count;
        if callers.iter().any(|caller| caller.contains(beneath))
            && !callers
                .iter()
                .any(|caller| except.iter().any(|left_out| caller.contains(left_out)))
        {
            calls.beneath += count;
        }
    }
    calls
}

#[cfg(test)]
mod tests;
