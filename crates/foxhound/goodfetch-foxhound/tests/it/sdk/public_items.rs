//! The check of the SDK's public items (ADRs 0081, 0095 and 0117): the
//! crate re-exports only items of Foxhound's catalog from
//! `goodfetch-protocol`, and its `foxhound::error_code` module of
//! Foxhound's Error codes (ADR 0099), every value a connection hands out from
//! `goodfetch-collie-client`, never a glob; its connections carry Collie's
//! members under the same names; and no public type of its own has a
//! lifetime parameter.

#![expect(
    clippy::disallowed_methods,
    reason = "the check reads Foxhound's catalog on the test's own thread, never an async worker"
)]

use std::path::Path;

use goodfetch_test_support::{
    public_methods, public_types_with_lifetimes, re_exports, sdk_sources,
};

/// The module of Foxhound's Error codes, the one module of
/// `goodfetch-protocol` the crate re-exports (ADR 0099).
const ERROR_CODES: &str = "goodfetch_protocol::foxhound::error_code";

/// The crate's own folder.
fn crate_folder() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The names Foxhound's catalog defines.
fn foxhound_definitions() -> serde_json::Map<String, serde_json::Value> {
    let catalog = crate_folder().join("../../goodfetch-protocol/generated/foxhound.json");
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(catalog).unwrap()).unwrap();
    catalog["$defs"].as_object().unwrap().clone()
}

/// Covers: story:11-1004
#[test]
fn the_sdk_re_exports_the_client_crates_values_and_foxhounds_protocol_items() {
    let definitions = foxhound_definitions();
    let paths = re_exports(&crate_folder().join("src/lib.rs"));

    let protocol: Vec<&String> = paths
        .iter()
        .filter(|path| path.starts_with("goodfetch_protocol::") && *path != ERROR_CODES)
        .collect();
    assert!(!protocol.is_empty(), "{paths:?}");
    assert!(
        paths.iter().any(|path| path == ERROR_CODES),
        "{ERROR_CODES} is not re-exported: {paths:?}"
    );
    for path in protocol {
        let name = path.rsplit("::").next().unwrap();
        assert!(
            definitions.contains_key(name),
            "{path} is not in Foxhound's catalog"
        );
    }
    for path in &paths {
        assert!(!path.ends_with('*'), "{path} is a glob re-export");
    }
    for value in ["Connection", "Error", "Locator", "Page", "Run", "Session"] {
        let path = format!("goodfetch_collie_client::{value}");
        assert!(
            paths.contains(&path),
            "{path} is not re-exported: {paths:?}"
        );
    }
}

/// `Foxhound` and `blocking::Foxhound` have the public methods of Collie's
/// `Collie` and `blocking::Collie`, under the same names.
/// Covers: story:11-1003
#[test]
fn each_foxhound_connection_has_the_members_of_its_collie_twin() {
    let collie = crate_folder().join("../../goodfetch-collie/src");
    let foxhound = crate_folder().join("src");

    let members = [
        (
            public_methods(&foxhound.join("foxhound.rs"), "Foxhound"),
            public_methods(&collie.join("collie.rs"), "Collie"),
        ),
        (
            public_methods(&foxhound.join("blocking.rs"), "Foxhound"),
            public_methods(&collie.join("blocking.rs"), "Collie"),
        ),
    ];

    for (foxhounds, collies) in members {
        assert!(collies.contains("start"), "{collies:?}");
        assert_eq!(foxhounds, collies);
    }
}

/// Collie's crate checks its own types and the client crate's.
#[test]
fn no_public_type_of_the_sdk_has_a_lifetime_parameter() {
    let files = sdk_sources(&crate_folder().join("src"));
    assert!(
        files.iter().any(|file| file.ends_with("foxhound.rs")),
        "{files:?}"
    );

    // A crawl's lifetime is the one its handlers borrow for, so a handler
    // may borrow from the scope that serves it (ADR 0112's exception).
    assert_eq!(public_types_with_lifetimes(&files), ["Crawl".to_owned()]);
}
