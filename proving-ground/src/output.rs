//! The Proving ground's output writer, the only code that writes its stdout
//! (STYLE.md): `serve`'s one JSON line, which test support reads, and the task
//! list `tasks` prints (ADR 0063).

use std::io::Write as _;

use anyhow::{Context as _, Result};
use serde::Serialize;

/// Writes one value as a JSON line and flushes it.
pub(crate) fn line(value: &impl Serialize) -> Result<()> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the output writer is the Proving ground's one writer to stdout"
    )]
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).context("writing the output line")?;
    stdout
        .write_all(b"\n")
        .and_then(|()| stdout.flush())
        .context("writing the output line")
}
