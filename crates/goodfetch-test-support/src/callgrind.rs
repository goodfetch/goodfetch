//! How many times a program calls a symbol, read from Callgrind (ADR 0140):
//! what no wrapper sees, such as SQLite's own `sqlite3_prepare_v3`, whose C
//! symbol does not inline or change with Goodfetch's code, or what has no
//! counter to count on, such as a `CountedWork`'s own construction, whose
//! reader checks that the symbol is still called at all. And the
//! instructions a program runs beneath one function
//! ([`instructions_beneath`]), such as a page's clean-up.

mod at_once;

use std::num::NonZeroUsize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::Variables;
use at_once::Bound;

/// The Callgrind children this process runs, through [`bounded`].
static BOUND: Bound = Bound::new();

/// How many of them [`bounded`] lets run at once, read once.
static MOST: OnceLock<usize> = OnceLock::new();

/// Runs `run`, which runs one Callgrind child, once fewer than half the
/// machine's cores, and at most eight, run in this process: a benchmark that
/// starts one per cell and outcome side by side would otherwise start them
/// all at once. `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE`, which `xtask counts
/// --callgrind-at-once` sets, lowers that bound, never raises it, so a counts
/// run can share a loaded machine. A count Callgrind reads does not depend
/// on how many run beside it.
///
/// # Panics
///
/// When `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE` is no whole number of 1 or more.
pub fn bounded<T>(run: impl FnOnce() -> T) -> T {
    let most = *MOST.get_or_init(|| {
        let cap = Variables::from_process()
            .callgrind_at_once()
            .unwrap_or_else(|error| panic!("{error}"));
        let cores = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        at_once::most_at_once(cores, cap)
    });
    BOUND.run(most, run)
}

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

/// How many instructions `program`, run with `arguments` under Callgrind,
/// ran in each of `functions` and in whatever it called. Each is named as
/// Callgrind names a function, where `*` stands for any text, and none may
/// call another: Callgrind turns its collection on as one is entered and
/// off as it returns, so a second one entered beneath the first turns it
/// off for the time it runs. None when the program never entered one of
/// them: a function renamed, or inlined into its callers, would otherwise
/// read as free. What ran before the program entered the
/// function `from`, when one is named, is left out: Callgrind zeroes its
/// costs there, so a program can do first what its figure must not hold,
/// such as work done once however many units follow. A `from` the program
/// never enters leaves that work in, and the figure above its limit.
///
/// # Errors
///
/// [`CallgrindError`]: when Valgrind cannot run, the program fails or the
/// output cannot be read.
pub fn instructions_beneath(
    functions: &[&str],
    from: Option<&str>,
    program: &Path,
    arguments: &[&str],
) -> Result<Option<u64>, CallgrindError> {
    let mut named: Vec<String> = functions
        .iter()
        .map(|function| format!("--toggle-collect={function}"))
        .collect();
    named.extend(from.map(|function| format!("--zero-before={function}")));
    let mut options = vec!["--collect-atstart=no"];
    options.extend(named.iter().map(String::as_str));
    let text = output(&options, program, arguments)?;
    if !functions.iter().all(|function| entered_in(&text, function)) {
        return Ok(None);
    }
    Ok(total_in(&text).filter(|total| *total > 0))
}

/// Whether `text`, a Callgrind output with its strings uncompressed, holds
/// costs of a function `pattern` names, `*` standing for any text: a
/// function the program entered while Callgrind collected.
fn entered_in(text: &str, pattern: &str) -> bool {
    text.lines()
        .filter_map(|line| line.strip_prefix("fn="))
        .any(|name| named_by(pattern, name))
}

/// Whether `pattern`, where `*` stands for any text, names all of `name`.
fn named_by(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let mut parts = parts.peekable();
    if parts.peek().is_none() {
        return rest.is_empty();
    }
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    true
}

/// The events `text`, a Callgrind output, collected in all: its `totals:`
/// line's first figure, the instructions.
fn total_in(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("totals:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
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
/// [`CallgrindError`]: when Valgrind cannot run, the program fails or the
/// output cannot be read.
pub fn calls_beneath(
    symbol: &str,
    depth: u32,
    beneath: &str,
    except: &[&str],
    program: &Path,
    arguments: &[&str],
) -> Result<Calls, CallgrindError> {
    let mut run = calls_beneath_each(symbol, depth, &[(beneath, except)], &[], program, arguments)?;
    Ok(run.beneath.remove(0))
}

/// What one run of a program under Callgrind called, as
/// [`calls_beneath_each`] reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunCalls {
    /// The calls of the symbol whose callers are told apart, for each owner
    /// asked for, in their order.
    pub beneath: Vec<Calls>,
    /// Every call of each other symbol asked for, in their order.
    pub others: Vec<u64>,
}

/// As [`calls_beneath`], for each of `owners` (the text a caller's name
/// holds, and the texts left out) from one run of `program`: what several
/// owners' code prepares, each apart, for the cost of one run. The same run
/// also counts every call of each of `others`, a symbol by its whole name
/// as Callgrind writes it.
///
/// # Errors
///
/// [`CallgrindError`]: when Valgrind cannot run, the program fails or the
/// output cannot be read.
pub fn calls_beneath_each(
    symbol: &str,
    depth: u32,
    owners: &[(&str, &[&str])],
    others: &[&str],
    program: &Path,
    arguments: &[&str],
) -> Result<RunCalls, CallgrindError> {
    let separate = format!("--separate-callers{depth}={symbol}");
    let text = output(&[&separate], program, arguments)?;
    Ok(RunCalls {
        beneath: owners
            .iter()
            .map(|(beneath, except)| calls_beneath_in(&text, symbol, beneath, except))
            .collect(),
        others: others.iter().map(|other| calls_in(&text, other)).collect(),
    })
}

/// What `program`, run with `arguments` under Callgrind and its `options`,
/// wrote, its strings uncompressed.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark harness's synchronous child and its one output file, on no async worker"
)]
fn output(options: &[&str], program: &Path, arguments: &[&str]) -> Result<String, CallgrindError> {
    let output = tempfile::NamedTempFile::new().map_err(CallgrindError::Output)?;
    let status = bounded(|| {
        Command::new("valgrind")
            .args(["--tool=callgrind", "--quiet", "--compress-strings=no"])
            .args(options)
            .arg(format!("--callgrind-out-file={}", output.path().display()))
            .arg(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
    })
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
