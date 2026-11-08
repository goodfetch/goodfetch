//! The executables the root workspace built for the Kennel's tests, from
//! the test tool manifest `just kennel-check` writes and names in
//! `GOODFETCH_TEST_TOOLS` (ADR 0062). A test that needs one fails without
//! the manifest, naming `just kennel-check`, as the root workspace's tests
//! fail naming `just test`: a test that skipped would pass without having
//! run. The token test, a binary of its own, compiles this file too.
#![expect(
    clippy::disallowed_methods,
    reason = "a test helper reads the test tool manifest's variable and file (ADR 0060)"
)]

use std::path::PathBuf;

use serde::Deserialize;

/// The variable naming the test tool manifest.
const TOOLS_VARIABLE: &str = "GOODFETCH_TEST_TOOLS";

/// The test tool manifest's executables.
#[derive(Deserialize)]
struct Tools {
    scripted_engine: Option<PathBuf>,
    collie: Option<PathBuf>,
}

/// The Scripted engine the root workspace built.
pub(crate) fn scripted_engine() -> PathBuf {
    tools()
        .scripted_engine
        .expect("the test tool manifest names goodfetch-scripted-engine")
}

/// Collie's executable the root workspace built, for the tests against a
/// real Daemon.
pub(crate) fn collie() -> PathBuf {
    tools()
        .collie
        .expect("the test tool manifest names goodfetch-collie")
}

/// The test tool manifest.
fn tools() -> Tools {
    let manifest = std::env::var_os(TOOLS_VARIABLE).unwrap_or_else(|| {
        panic!(
            "{TOOLS_VARIABLE} is unset, so the root workspace built no test tool: run the Kennel's tests with `just kennel-check`"
        )
    });
    let text = std::fs::read_to_string(&manifest).unwrap();
    serde_json::from_str(&text).unwrap()
}
