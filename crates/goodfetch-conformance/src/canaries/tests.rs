#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the outputs the scan reads"
)]

use std::collections::BTreeSet;

use goodfetch_test_support::TestEnv;
use serde::Deserialize;

use super::held_under;

/// Test support's shared canary search fixture, which its Rust, TypeScript
/// and Python searches pass too.
const SHARED: &str = include_str!("../../../../test-support/fixtures/canaries/search.json");

#[derive(Deserialize)]
struct Fixture {
    canaries: Vec<Canary>,
    outputs: Vec<Output>,
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
struct Canary {
    id: String,
    value: String,
}

#[derive(Deserialize)]
struct Output {
    name: String,
    text: String,
}

#[derive(Deserialize)]
struct Hit {
    canary: String,
    encoding: String,
    output: String,
}

/// The harness's scan finds every canary of the shared fixture in the
/// outputs and encodings test support's search finds it in, and nowhere
/// else.
#[test]
fn the_scan_finds_the_shared_fixtures_canaries_where_test_supports_search_does() {
    let env = TestEnv::new();
    let fixture: Fixture = serde_json::from_str(SHARED).unwrap();
    let folder = env.folder().join("outputs");
    std::fs::create_dir_all(&folder).unwrap();
    for output in &fixture.outputs {
        std::fs::write(folder.join(&output.name), &output.text).unwrap();
    }
    let canaries: Vec<(&str, &str)> = fixture
        .canaries
        .iter()
        .map(|canary| (canary.id.as_str(), canary.value.as_str()))
        .collect();

    let found: BTreeSet<(String, String, String)> = held_under(&folder, &canaries)
        .unwrap()
        .into_iter()
        .map(|held| {
            (
                held.canary,
                held.file
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                held.encoding.to_owned(),
            )
        })
        .collect();

    assert_eq!(
        found,
        fixture
            .hits
            .into_iter()
            .map(|hit| (hit.canary, hit.output, hit.encoding))
            .collect::<BTreeSet<_>>()
    );
}

/// A symbolic link to a folder outside the one searched is not followed,
/// so a Profile directory a Run uses in place, outside the Goodfetch home,
/// is never read through one (ADR 0056).
#[cfg(unix)]
#[test]
fn a_link_out_of_the_folder_is_not_followed() {
    let env = TestEnv::new();
    let outside = env.folder().join("profile");
    let folder = env.folder().join("home");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(outside.join("Cookies"), "session=value-outside").unwrap();
    std::os::unix::fs::symlink(&outside, folder.join("profile")).unwrap();

    assert_eq!(
        held_under(&folder, &[("cookie", "value-outside")]).unwrap(),
        Vec::new()
    );
}

/// A folder that does not exist holds no canary.
#[test]
fn a_missing_folder_holds_none() {
    let env = TestEnv::new();

    assert_eq!(
        held_under(&env.folder().join("gone"), &[("token", "value")]).unwrap(),
        Vec::new()
    );
}
