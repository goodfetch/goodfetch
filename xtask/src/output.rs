//! `xtask`'s output writer: the one place that writes to stdout and stderr
//! (STYLE.md, Logging and secrets; AGENTS.md's stdout trap), and to a file a
//! job's later steps read, such as its summary page (`GITHUB_STEP_SUMMARY`)
//! or its outputs (`GITHUB_OUTPUT`).

use std::io::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};

/// Appends `text` to the file at `path`, creating it when it does not exist.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn append(path: &Path, text: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

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
