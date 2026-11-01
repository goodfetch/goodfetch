//! J-009, "See where each setting's value comes from", on the `cli`
//! surface (ADRs 0063 and 0075): a person sets one setting in user
//! configuration, the same one and a second in project configuration, and
//! asks `config explain` where each effective value comes from, through
//! Collie's shipped executable in a Goodfetch home and a project folder of
//! the test's own. The variant's result document joins with the Journey
//! runner's Rust helper (#350).
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write their configuration files"
)]

use std::fs;

use goodfetch_test_support::{TestEnv, command, journey_engine};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Covers: story:3-89, row:RT-22
#[test]
fn cli() {
    let env = TestEnv::new();
    // Set one setting in the user configuration, and set it and a second
    // setting in the project configuration.
    let major = env.home().join("0.0");
    fs::create_dir_all(&major).unwrap();
    fs::write(
        major.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 45000\n",
    )
    .unwrap();
    fs::write(
        env.folder().join("goodfetch.toml"),
        "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 60000\nfirst_byte = 10000\n",
    )
    .unwrap();

    // Ask where each effective value comes from.
    let output = command(&env, journey_engine(COLLIE))
        .args(["config", "explain", "timeout", "--json"])
        .output()
        .unwrap();

    let explained: Value = serde_json::from_slice(&output.stdout).unwrap();
    let by_key = |key: &str| {
        explained["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["key"] == key)
            .cloned()
            .unwrap()
    };
    let shown = |setting: Value| {
        json!({
            "value": setting["value"],
            "layer": setting["layer"],
            "locked_by": setting.get("locked_by"),
        })
    };
    // Each effective value is shown with the layer that set it and the
    // layer that locked it, or none: the project configuration for the
    // setting both configurations set, and the built-in default for a
    // setting no layer set. The setting both set shows the project
    // configuration's value, since no lower layer locked it.
    assert_eq!(
        (
            output.status.success(),
            shown(by_key("timeout.navigation")),
            shown(by_key("timeout.first_byte")),
            shown(by_key("timeout.run")),
        ),
        (
            true,
            json!({ "value": 60000, "layer": "project_configuration", "locked_by": null }),
            json!({ "value": 10000, "layer": "project_configuration", "locked_by": null }),
            json!({ "value": null, "layer": "default", "locked_by": null }),
        )
    );
}
