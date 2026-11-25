#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use rstest::rstest;
use serde_json::json;

use super::{
    DeclaredKind, HelloRefusal, HookStageName, ManifestField, NetworkLevel, PLUGIN_PROTOCOL,
    PluginHelloParams, PluginHelloResult, PluginKindName, PluginManifest, PluginProtocolRange,
    check_hello,
};
use crate::catalog::{Product, ProtocolVersion};
use crate::error_code::ErrorCode;

const fn version(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion { major, minor }
}

const fn range(min: ProtocolVersion, max: ProtocolVersion) -> PluginProtocolRange {
    PluginProtocolRange { min, max }
}

/// An approved Hook implementing `page_ready`, speaking plugin protocol 0.1.
fn approved() -> PluginManifest {
    PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "shop-hooks",
            "version": "1.2.0",
            "kinds": ["hook"],
            "hook_stages": ["page_ready"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "shop-hooks",
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

/// The `hello` result that matches [`approved`].
fn matching() -> PluginHelloResult {
    PluginHelloResult {
        protocol_version: version(0, 1),
        name: "shop-hooks".to_owned(),
        version: "1.2.0".to_owned(),
        kinds: vec![DeclaredKind {
            kind: PluginKindName::HOOK,
            capabilities: Vec::new(),
        }],
        hook_stages: BTreeSet::from([HookStageName::PAGE_READY]),
        network_level: NetworkLevel::None,
    }
}

/// Covers: story:7-77, story:7-100
#[test]
fn a_hello_that_matches_the_approved_manifest_is_accepted() {
    assert_eq!(
        check_hello(PLUGIN_PROTOCOL, &approved(), &matching()),
        Ok(())
    );
}

/// Covers: story:7-77, story:7-100, error:plugin_manifest_mismatch
#[rstest]
#[case::another_kind(
    |hello: &mut PluginHelloResult| hello.kinds.push(DeclaredKind {
        kind: PluginKindName::EXTRACT,
        capabilities: Vec::new(),
    }),
    ManifestField::Kinds
)]
#[case::no_kind(|hello: &mut PluginHelloResult| hello.kinds.clear(), ManifestField::Kinds)]
#[case::the_same_kind_twice(
    |hello: &mut PluginHelloResult| hello.kinds.push(hello.kinds[0].clone()),
    ManifestField::Kinds
)]
#[case::another_stage(
    |hello: &mut PluginHelloResult| {
        hello.hook_stages.insert(HookStageName::BEFORE_NAVIGATION);
    },
    ManifestField::HookStages
)]
#[case::a_stage_left_out(
    |hello: &mut PluginHelloResult| hello.hook_stages.clear(),
    ManifestField::HookStages
)]
#[case::a_version_outside_the_approved_range(
    |hello: &mut PluginHelloResult| hello.protocol_version = version(0, 2),
    ManifestField::Protocol
)]
#[case::another_network_level(
    |hello: &mut PluginHelloResult| hello.network_level = NetworkLevel::ProxyTcp,
    ManifestField::NetworkLevel
)]
fn a_hello_that_differs_from_the_approved_manifest_is_refused_with_plugin_manifest_mismatch(
    #[case] fabricate: fn(&mut PluginHelloResult),
    #[case] field: ManifestField,
) {
    let mut hello = matching();
    fabricate(&mut hello);
    let engine = range(version(0, 1), version(0, 2));

    let refused = check_hello(engine, &approved(), &hello).unwrap_err();

    assert_eq!(
        (refused.code(), refused),
        (
            ErrorCode::PLUGIN_MANIFEST_MISMATCH,
            HelloRefusal::ManifestMismatch { field }
        )
    );
}

/// Covers: story:7-77, story:7-100, error:plugin_protocol_unsupported
#[rstest]
#[case::a_newer_minor(version(0, 2))]
#[case::a_newer_major(version(1, 0))]
#[case::an_older_version(version(0, 0))]
fn a_hello_in_a_version_the_engine_does_not_speak_is_refused_with_plugin_protocol_unsupported(
    #[case] found: ProtocolVersion,
) {
    let hello = PluginHelloResult {
        protocol_version: found,
        ..matching()
    };

    let refused = check_hello(PLUGIN_PROTOCOL, &approved(), &hello).unwrap_err();

    assert_eq!(
        (refused.code(), refused),
        (
            ErrorCode::PLUGIN_PROTOCOL_UNSUPPORTED,
            HelloRefusal::ProtocolUnsupported {
                found,
                engine: PLUGIN_PROTOCOL
            }
        )
    );
}

/// Covers: story:7-77
#[rstest]
#[case::the_same(range(version(0, 1), version(0, 3)), Some(version(0, 3)))]
#[case::overlapping(range(version(0, 2), version(0, 5)), Some(version(0, 3)))]
#[case::within(range(version(0, 2), version(0, 2)), Some(version(0, 2)))]
#[case::touching(range(version(0, 0), version(0, 1)), Some(version(0, 1)))]
#[case::newer(range(version(0, 4), version(1, 0)), None)]
#[case::older(range(version(0, 0), version(0, 0)), None)]
fn two_ranges_share_their_newest_common_version(
    #[case] other: PluginProtocolRange,
    #[case] shared: Option<ProtocolVersion>,
) {
    let ours = range(version(0, 1), version(0, 3));

    assert_eq!(
        (ours.newest_shared(other), other.newest_shared(ours)),
        (shared, shared)
    );
}

/// Covers: story:7-77
#[test]
fn hello_params_from_a_newer_engine_are_read_without_their_unknown_field() {
    let params: PluginHelloParams = serde_json::from_value(json!({
        "protocol": { "min": "0.1", "max": "0.2" },
        "product": "collie",
        "run_id": "checkout",
        "network_level": "proxy_tcp",
        "_meta": { "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01" },
        "added_in_a_minor": true,
    }))
    .unwrap();

    assert_eq!(
        (
            params.protocol,
            params.product,
            params.run_id.as_str(),
            params.network_level
        ),
        (
            range(version(0, 1), version(0, 2)),
            Product::Collie,
            "checkout",
            NetworkLevel::ProxyTcp
        )
    );
}

/// Covers: story:7-77
#[test]
fn a_kind_or_a_stage_the_program_does_not_know_stays_a_value() {
    let hello: PluginHelloResult = serde_json::from_value(json!({
        "protocol_version": "0.1",
        "name": "crawl-sink",
        "version": "1.0.0",
        "kinds": [{ "kind": "sink", "capabilities": ["write"] }],
        "hook_stages": ["request_ended"],
    }))
    .unwrap();

    assert_eq!(
        (
            hello.kinds[0].kind.as_str(),
            hello
                .hook_stages
                .iter()
                .map(HookStageName::as_str)
                .collect::<Vec<_>>(),
            hello.network_level,
            serde_json::to_value(&hello).unwrap()["kinds"].clone(),
        ),
        (
            "sink",
            vec!["request_ended"],
            NetworkLevel::None,
            json!([{ "kind": "sink", "capabilities": ["write"] }]),
        )
    );
}
