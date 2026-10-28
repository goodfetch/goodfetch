//! `browser clean` through Foxhound's executable, which carries Collie's
//! command unchanged (ADRs 0090 and 0098): on the shared Browser cache of
//! one Goodfetch home it deletes only builds no installed executable pins
//! and none used within `browser_cache.max_unused_age`, so a build an
//! installed Collie executable pins survives.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out cache entries and the list of installed executables by hand"
)]

use std::fs;

use goodfetch_protocol::browser_cache::BrowserCleaned;
use goodfetch_test_support::{TestEnv, cli, lay_out_browser_entry};
use serde_json::json;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// Covers: story:11-296, command:browser.clean
#[test]
fn foxhounds_browser_clean_keeps_a_build_an_installed_collie_pins() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out_browser_entry(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");
    let pinned = lay_out_browser_entry(&cache, "pinned-by-collie", "2020-01-01T00:00:00Z");
    let collie = env.folder().join("goodfetch-collie");
    fs::write(&collie, "#!collie\n").unwrap();
    let list = json!({
        "format_version": "0.1",
        "executables": [{
            "kind": "product_executable", "product": "collie",
            "product_version": env!("CARGO_PKG_VERSION"),
            "path": collie.display().to_string(),
            "size": 9, "modified_at": "2020-01-01T00:00:00Z",
            "pins": [{ "kind": "browser", "name": "chrome-for-testing", "build": "pinned-by-collie" }],
            "last_seen": "2100-01-01T00:00:00Z",
        }],
    });
    fs::create_dir_all(env.home()).unwrap();
    fs::write(
        env.home().join("installed-executables.json"),
        list.to_string(),
    )
    .unwrap();

    let checked = cli(&env, FOXHOUND)
        .args(["browser", "clean", "--json"])
        .json("BrowserCleaned")
        .unwrap();

    let cleaned: BrowserCleaned = serde_json::from_value(checked.documents[0].clone()).unwrap();
    assert_eq!(
        (
            checked.output.exit,
            cleaned
                .removed
                .iter()
                .map(|build| build.version.as_str())
                .collect::<Vec<_>>(),
            unused.exists(),
            pinned.join("entry.json").exists(),
        ),
        (Some(0), vec!["unused-since-2020"], false, true)
    );
}
