//! The CLI's output writer (STYLE.md): one of the three writers of stdout
//! and stderr, for a command's human output and its `--json` output on
//! stdout, and its warnings, hints and failures on stderr, so that stdout
//! under `--json` holds one JSON value and nothing else (ADR 0069).

use std::io::Write as _;

use serde::Serialize;

fn write_stdout(bytes: &[u8]) {
    #[expect(
        clippy::disallowed_methods,
        reason = "the CLI's output writer is one of the three writers of stdout (STYLE.md)"
    )]
    let mut stdout = std::io::stdout().lock();
    // A closed stdout, such as a pipe whose reader left, ends the output.
    let _ = stdout.write_all(bytes).and_then(|()| stdout.flush());
}

fn write_stderr(bytes: &[u8]) {
    #[expect(
        clippy::disallowed_methods,
        reason = "the CLI's output writer is one of the three writers of stderr (STYLE.md)"
    )]
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(bytes).and_then(|()| stderr.flush());
}

/// Writes `text` and a newline on stdout.
pub fn line(text: &str) {
    write_stdout(format!("{text}\n").as_bytes());
}

/// Writes `value` on stdout as one line of JSON.
///
/// # Panics
///
/// If `value` does not serialize, which no output type of
/// `goodfetch-protocol` does.
pub fn json(value: &impl Serialize) {
    let mut bytes = serde_json::to_vec(value).expect("invariant: the output types serialize");
    bytes.push(b'\n');
    write_stdout(&bytes);
}

/// Writes `text` and a newline on stderr: a warning, a hint or a failure,
/// which never reaches stdout.
pub fn note(text: &str) {
    write_stderr(format!("{text}\n").as_bytes());
}
