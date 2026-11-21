//! The Scripted engine as this crate's Attached engine, for the cases
//! `sdk-rust` plays through the Rust Client's Attached transport, which
//! it has no mode for: the `hello` the engine expects, its script and its
//! record of the Client's messages.

use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_test_support::{ATTACHED_SCRIPT, TestEnv, Tool, engine_script, tools};
use serde_json::Value;

/// This crate's `hello` to an Attached engine.
pub(super) fn attached_hello() -> HelloParams {
    HelloParams {
        needs: Some(Product::Collie),
        ..HelloParams::new(
            Surface::Rust,
            ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap(),
        )
    }
}

/// Installs the Scripted engine's corpus script `script` in `env`'s home,
/// with this crate's `hello`, and returns the Scripted engine's
/// executable.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
pub(super) fn install_script(env: &TestEnv, script: &str) -> PathBuf {
    let mut script: Value =
        serde_json::from_slice(&std::fs::read(engine_script(script)).unwrap()).unwrap();
    script["steps"][0]["expect"]["params"] = serde_json::to_value(attached_hello()).unwrap();
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, serde_json::to_vec(&script).unwrap()).unwrap();
    tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_path_buf()
}

/// The methods of the Client's messages the Scripted engine recorded in
/// `record`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Scripted engine's record once the Client's Connection to it ended"
)]
pub(super) fn client_methods(record: &Path) -> Vec<String> {
    std::fs::read_to_string(record)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["from"] == "client")
        .filter_map(|entry| entry["message"]["method"].as_str().map(str::to_owned))
        .collect()
}
