//! Writes the Chromium build gate's WPT launch arguments (`none` and
//! `js_injection` Fingerprint methods at [`goodfetch_page::WPT_IDENTITY_SEED`])
//! into the file path given as the only argument (ADR 0074; ticket #395).
//! `xtask codegen` runs it and commits the result to `chromium/launch-args.json`.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous codegen helper that writes a file (ADR 0074)"
)]

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: launch-arguments <output-file>")?,
    );
    let document = goodfetch_page::wpt_launch_args(goodfetch_page::WPT_IDENTITY_SEED);
    let mut json = serde_json::to_string_pretty(&document)?;
    json.push('\n');
    std::fs::write(&target, json)?;
    Ok(())
}
