//! The Politeness store layout's writer for the Compatibility corpus
//! (ADRs 0053 and 0066; issue 240): writes the store's frozen-major key
//! and script layout description as `layout.json` into the folder given as
//! its only argument. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::politeness::store::PolitenessStore;

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let mut text = serde_json::to_string_pretty(&PolitenessStore::layout_description())
        .expect("invariant: the Politeness store layout serializes");
    text.push('\n');
    std::fs::write(output.join("layout.json"), text)
        .expect("invariant: the output folder is writable");
}
