#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use rstest::rstest;
use serde_json::{Value, json};

use super::RefusalCase;
use crate::catalog::{PROTOCOL_VERSION, Product, ProtocolVersion};
use crate::error_code::ErrorCode;
use crate::formats::{Compatibility, PLUGIN_MANIFEST, PUBLISHED_CATALOG};
use crate::plugin::{
    DeclaredKind, HelloRefusal, MANIFEST_VERSION, NetworkLevel, PLUGIN_PROTOCOL, PluginHelloResult,
    PluginKindName, PluginManifest, PluginProtocolRange, check_hello,
};
use crate::published_catalog::{PublishedCatalog, PublishedCatalogError, SCHEMA_DRAFT};

/// Each generated case, every registered format's among them, against the
/// compatibility function of the rule it names: a function that disagrees
/// with its rule's table fails its cases (ADR 0066).
#[rstest]
fn each_generated_case_gets_its_rules_verdict(
    #[files("generated/refusals/*/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();

    let verdict = match (case.engine, case.lowest_minor) {
        (Some(engine), Some(lowest_minor)) => {
            case.rule
                .compatibility_of_store(case.reader, case.found, engine, lowest_minor)
        }
        _ => case.rule.compatibility(case.reader, case.found),
    };

    assert_eq!(verdict, case.compatibility, "{case:#?}");
}

/// The first version of `found`'s line under ADR 0030: `found` itself
/// during `0.y` previews, whose minor counts as a major.
fn line_start(found: ProtocolVersion) -> ProtocolVersion {
    match found.major {
        0 => found,
        major => ProtocolVersion { major, minor: 0 },
    }
}

/// A Hook's manifest at `manifest_version`, speaking `protocol`, as JSON.
fn hook_manifest(manifest_version: ProtocolVersion, protocol: PluginProtocolRange) -> Value {
    json!({
        "manifest_version": manifest_version,
        "name": "shop-hooks",
        "version": "1.2.0",
        "kinds": ["hook"],
        "protocol": protocol,
        "entrypoint": "shop-hooks",
    })
}

/// Each generated refusal case of the plugin protocol (ADR 0066), a Plugin
/// that speaks the case's version's line up to it: when the rule reads the
/// version, the Plugin answers `hello` in the newest version it shares
/// with the engine and the engine accepts it; otherwise they share none,
/// and a `hello` result in the case's version is refused with
/// `plugin_protocol_unsupported` before the manifest is looked at.
///
/// Covers: story:7-100, format:plugin-protocol, error:plugin_protocol_unsupported
#[rstest]
fn each_generated_refusal_of_the_plugin_protocol_is_settled_at_hello(
    #[files("generated/refusals/plugin-protocol/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();
    let speaks = PluginProtocolRange {
        min: line_start(case.found),
        max: case.found,
    };
    let approved = PluginManifest::read(
        hook_manifest(MANIFEST_VERSION, speaks)
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let hello = |protocol_version| PluginHelloResult {
        protocol_version,
        name: "shop-hooks".to_owned(),
        version: "1.2.0".to_owned(),
        kinds: vec![DeclaredKind {
            kind: PluginKindName::HOOK,
            capabilities: Vec::new(),
        }],
        hook_stages: BTreeSet::new(),
        network_level: NetworkLevel::None,
    };
    assert_eq!(case.reader, PLUGIN_PROTOCOL.max, "{case:#?}");

    let shared = speaks.newest_shared(PLUGIN_PROTOCOL);

    match (case.compatibility, shared) {
        (Compatibility::Compatible, Some(spoken)) => {
            assert_eq!(
                check_hello(PLUGIN_PROTOCOL, &approved, &hello(spoken)),
                Ok(()),
                "{case:#?}"
            );
        }
        (compatibility, None) if compatibility != Compatibility::Compatible => {
            let refused = check_hello(PLUGIN_PROTOCOL, &approved, &hello(case.found)).unwrap_err();
            assert_eq!(
                (refused.code(), refused),
                (
                    ErrorCode::PLUGIN_PROTOCOL_UNSUPPORTED,
                    HelloRefusal::ProtocolUnsupported {
                        found: case.found,
                        engine: PLUGIN_PROTOCOL,
                    }
                ),
                "{case:#?}"
            );
        }
        (compatibility, shared) => {
            panic!("the rule says {compatibility:?}, and the ranges share {shared:?}: {case:#?}")
        }
    }
}

/// Each generated refusal case of the Plugin manifest (ADR 0066), a
/// manifest of the case's version holding a field a newer version adds:
/// its version reads before the rest, and the format's rule gives the
/// case's verdict, so a newer manifest is refused for its version before
/// any field of it is read.
///
/// Covers: story:7-100, format:plugin-manifest
#[rstest]
fn each_generated_refusal_of_the_plugin_manifest_is_settled_by_its_version(
    #[files("generated/refusals/plugin-manifest/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();
    let mut manifest = hook_manifest(case.found, PLUGIN_PROTOCOL);
    manifest["added_in_a_newer_version"] = json!(true);
    let manifest = manifest.to_string();
    assert_eq!(case.reader, PLUGIN_MANIFEST.version, "{case:#?}");

    let found = PluginManifest::version_of(manifest.as_bytes()).unwrap();

    assert_eq!(found, case.found, "{case:#?}");
    assert_eq!(
        PLUGIN_MANIFEST
            .rule
            .compatibility(PLUGIN_MANIFEST.version, found),
        case.compatibility,
        "{case:#?}"
    );
}

/// Each generated refusal case of the published Operation catalog (ADR 0066;
/// issue 1662): its `format_version` is read before any field added by a newer
/// version, and `PublishedCatalog::read` refuses every incompatible version.
///
/// Covers: format:published-catalog
#[rstest]
fn each_generated_refusal_of_the_published_catalog_is_settled_by_its_version(
    #[files("generated/refusals/published-catalog/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();
    let payload = json!({
        "format_version": case.found.to_string(),
        "protocol_version": PROTOCOL_VERSION.to_string(),
        "$schema": SCHEMA_DRAFT,
        "catalog": {},
        "connection": {},
        "$defs": {},
        "added_in_a_newer_version": true,
    })
    .to_string();
    assert_eq!(case.reader, PUBLISHED_CATALOG.version, "{case:#?}");

    let found = PublishedCatalog::version_of(payload.as_bytes()).unwrap();
    assert_eq!(found, case.found, "{case:#?}");
    assert_eq!(
        PUBLISHED_CATALOG
            .rule
            .compatibility(PUBLISHED_CATALOG.version, found),
        case.compatibility,
        "{case:#?}"
    );
    if case.compatibility != Compatibility::Compatible {
        let error = PublishedCatalog::read(payload.as_bytes()).unwrap_err();
        assert!(
            matches!(error, PublishedCatalogError::UnsupportedVersion { found: actual, .. } if actual == case.found),
            "expected UnsupportedVersion for {case:#?}, got {error}"
        );
    }
}

/// The committed `compat/published-catalog/head/catalog.json` and the full
/// generated `goodfetch-catalog.json` both parse through
/// `PublishedCatalog::read`, carry both Products' marks, and declare the
/// current protocol version.
///
/// Covers: format:published-catalog
#[test]
fn published_catalog_head_and_generated_file_parse_with_both_product_marks() {
    const HEAD: &str = include_str!("../../../../compat/published-catalog/head/catalog.json");
    const GENERATED: &str = include_str!("../../generated/goodfetch-catalog.json");

    let head = PublishedCatalog::read(HEAD.as_bytes()).unwrap();
    assert_eq!(head.format_version, PUBLISHED_CATALOG.version);
    assert_eq!(head.protocol_version, PROTOCOL_VERSION);

    let generated = PublishedCatalog::read(GENERATED.as_bytes()).unwrap();
    assert_eq!(generated.format_version, PUBLISHED_CATALOG.version);
    assert_eq!(generated.protocol_version, PROTOCOL_VERSION);
    let operations = generated.catalog["operations"].as_array().unwrap();
    assert!(
        operations
            .iter()
            .any(|op| op["product"] == Product::Collie.as_str()),
        "generated catalog must carry Collie operations"
    );
    assert!(
        operations
            .iter()
            .any(|op| op["product"] == Product::Foxhound.as_str()),
        "generated catalog must carry Foxhound operations"
    );
}
