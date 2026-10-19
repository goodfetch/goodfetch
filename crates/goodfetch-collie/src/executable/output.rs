//! The CLI's output writer (STYLE.md): one of the three writers of stdout,
//! for a command's human output and its `--json` output.

use std::io::Write as _;

use serde::Serialize;

fn write(bytes: &[u8]) {
    #[expect(
        clippy::disallowed_methods,
        reason = "the CLI's output writer is one of the three writers of stdout (STYLE.md)"
    )]
    let mut stdout = std::io::stdout().lock();
    // A closed stdout, such as a pipe whose reader left, ends the output.
    let _ = stdout.write_all(bytes).and_then(|()| stdout.flush());
}

/// Writes `text` and a newline.
pub(super) fn line(text: &str) {
    write(format!("{text}\n").as_bytes());
}

/// Writes `value` as one line of JSON.
pub(super) fn json(value: &impl Serialize) {
    let mut bytes = serde_json::to_vec(value).expect("invariant: the output types serialize");
    bytes.push(b'\n');
    write(&bytes);
}
