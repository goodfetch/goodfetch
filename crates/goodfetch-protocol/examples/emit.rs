//! Writes the JSON Schema bundle and catalog file into the folder given as
//! the only argument, as `bundle.json` and `catalog.json`. `xtask codegen`
//! runs it and commits what it writes (ADR 0038).
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous build tool; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(std::env::args_os().nth(1).ok_or("usage: emit <folder>")?);
    let emitted = goodfetch_protocol::emit::emit();
    for (name, value) in [
        ("bundle.json", emitted.bundle),
        ("catalog.json", emitted.catalog),
    ] {
        let mut text = serde_json::to_string_pretty(&value)?;
        text.push('\n');
        std::fs::write(folder.join(name), text)?;
    }
    Ok(())
}
