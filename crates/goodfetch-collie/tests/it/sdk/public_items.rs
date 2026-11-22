//! The check of the SDK's public items (ADRs 0081, 0095 and 0099): the
//! crate re-exports only Collie's items of `goodfetch-protocol`, the ones
//! Collie's catalog defines, never Foxhound's module nor a glob, its Block
//! cause type knows none of Foxhound's values, and no public type of the
//! SDK, its own or its client crate's, has a lifetime parameter.

#![expect(
    clippy::disallowed_methods,
    reason = "the check reads Collie's catalog on the test's own thread, never an async worker"
)]

use std::path::Path;

use goodfetch_protocol::block::BlockCause;
use goodfetch_test_support::{public_types_with_lifetimes, re_exports, sdk_sources};

/// The crate's own folder.
fn crate_folder() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The names Collie's catalog defines.
fn collie_definitions() -> serde_json::Map<String, serde_json::Value> {
    let catalog = crate_folder().join("../goodfetch-protocol/generated/collie.json");
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(catalog).unwrap()).unwrap();
    catalog["$defs"].as_object().unwrap().clone()
}

#[test]
fn the_sdk_re_exports_only_collies_protocol_items() {
    let definitions = collie_definitions();
    let paths = re_exports(&crate_folder().join("src/lib.rs"));

    let protocol: Vec<&String> = paths
        .iter()
        .filter(|path| path.starts_with("goodfetch_protocol::"))
        .collect();
    assert!(!protocol.is_empty(), "{paths:?}");
    for path in protocol {
        let name = path.rsplit("::").next().unwrap();
        assert!(!path.contains("foxhound"), "{path} is Foxhound's");
        assert!(
            definitions.contains_key(name),
            "{path} is not in Collie's catalog"
        );
    }
    for path in &paths {
        assert!(!path.ends_with('*'), "{path} is a glob re-export");
    }
}

/// The Block cause type a Collie program reads keeps every value Foxhound's
/// catalog marks Foxhound's as unknown, as received (ADR 0095's note of
/// 2026-11-21): the public-item check lists types, not enum variants, so a
/// Foxhound value added back to Collie's type fails here.
#[test]
fn collies_block_cause_knows_no_value_of_foxhounds() {
    let catalog = crate_folder().join("../goodfetch-protocol/generated/foxhound.json");
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(catalog).unwrap()).unwrap();
    let foxhounds: Vec<&str> = catalog["shared_type_values"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| value["type"] == "block_cause" && value["product"] == "foxhound")
        .map(|value| value["name"].as_str().unwrap())
        .collect();
    assert!(foxhounds.contains(&"decoy"), "{foxhounds:?}");

    let known: Vec<&str> = foxhounds
        .into_iter()
        .filter(|name| {
            !matches!(
                serde_json::from_value(serde_json::json!(name)).unwrap(),
                BlockCause::Unknown(_)
            )
        })
        .collect();

    assert_eq!(known, Vec::<&str>::new());
}

#[test]
fn no_public_type_of_the_sdk_has_a_lifetime_parameter() {
    let mut files = sdk_sources(&crate_folder().join("src"));
    files.extend(sdk_sources(
        &crate_folder().join("../goodfetch-collie-client/src"),
    ));
    assert!(
        files.iter().any(|file| file.ends_with("collie.rs")),
        "{files:?}"
    );

    assert_eq!(public_types_with_lifetimes(&files), Vec::<String>::new());
}
