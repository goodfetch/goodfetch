//! `xtask`'s output writer: the one place that writes to stdout and stderr
//! (STYLE.md, Logging and secrets; AGENTS.md's stdout trap).

use std::io::Write as _;

#[expect(
    clippy::disallowed_methods,
    reason = "xtask's own output writer is one of the three writers allowed to touch stdout (STYLE.md)"
)]
pub(crate) fn line(text: &str) {
    let mut stdout = std::io::stdout().lock();
    // A closed stdout (`xtask … | head`) loses output, never the verdict.
    let _ = writeln!(stdout, "{text}");
}

#[expect(
    clippy::disallowed_methods,
    reason = "xtask's own output writer is one of the three writers allowed to touch stderr (STYLE.md)"
)]
pub(crate) fn error(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{text}");
}
