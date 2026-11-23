//! Collie's Claude Code plugin (story:11-239): its executable crate's
//! `generated/` folder, the `.claude-plugin/plugin.json` codegen writes
//! beside the skills it renders, passes the pinned Claude Code's
//! `plugin validate --strict`, which runs with no network.
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the manifest it validates"
)]

use std::path::Path;

use goodfetch_test_support::{TestEnv, validate_plugin};
use serde_json::{Value, json};

/// The plugin's root folder.
const PLUGIN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/generated");

/// Claude Code reads the plugin through its manifest, which names it as
/// the executable, so its skills load as `goodfetch-collie:<skill>`; a
/// folder without one passes as a bare skills tree.
///
/// Covers: story:11-239
#[test]
fn collies_plugin_passes_claude_codes_strict_validation() {
    let env = TestEnv::new();

    let validation = validate_plugin(&env, Path::new(PLUGIN)).unwrap();

    let manifest = std::fs::read(format!("{PLUGIN}/.claude-plugin/plugin.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    assert_eq!(
        (
            validation.passed,
            &validation.report["manifest"]["type"],
            manifest.as_ref().map(|manifest| &manifest["name"]),
        ),
        (true, &json!("plugin"), Some(&json!("goodfetch-collie"))),
        "{}",
        validation.output
    );
}
