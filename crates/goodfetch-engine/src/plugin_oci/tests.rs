use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    ATTESTATION_LIMIT, Compression, DOCUMENT_LIMIT, DocumentError, INDEX_MEDIA_TYPE, LAYER_LIMIT,
    MANIFEST_MEDIA_TYPE, Platform, Reference, SIGSTORE_BUNDLE_MEDIA_TYPE, TAR_GZIP_MEDIA_TYPE,
    TAR_MEDIA_TYPE, attestation_layer, attestation_manifests, bundle_layer, manifest_for,
};
use crate::pinned_download::Digest;

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

const LINUX_AMD64: Platform = Platform {
    os: "linux",
    architecture: "amd64",
};
const DARWIN_ARM64: Platform = Platform {
    os: "darwin",
    architecture: "arm64",
};

/// A manifest descriptor of `digest` for `os`, `architecture` and, when
/// some, `variant`.
fn entry(digest: &str, os: &str, architecture: &str, variant: Option<&str>) -> Value {
    let mut platform = json!({ "os": os, "architecture": architecture });
    if let Some(variant) = variant {
        platform["variant"] = json!(variant);
    }
    json!({ "mediaType": MANIFEST_MEDIA_TYPE, "digest": digest, "size": 512, "platform": platform })
}

/// An image index naming `manifests`.
fn index(manifests: &[Value]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": INDEX_MEDIA_TYPE,
        "manifests": manifests,
        "annotations": { "org.opencontainers.image.title": "goodfetch-plugin-kettle" },
    }))
    .unwrap()
}

/// An image manifest whose layers are `layers`.
fn manifest(layers: &[Value]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_MEDIA_TYPE,
        "config": { "mediaType": "application/vnd.oci.empty.v1+json", "digest": C, "size": 2 },
        "layers": layers,
    }))
    .unwrap()
}

/// A layer of `media_type` holding `size` bytes.
fn layer(media_type: &str, size: u64) -> Value {
    json!({ "mediaType": media_type, "digest": B, "size": size })
}

/// Covers: story:7-5
#[rstest]
#[case::at_a_port("localhost:5000/goodfetch-plugin-kettle", "kettle")]
#[case::nested("registry.example.com/team/tools/goodfetch-plugin-kettle", "kettle")]
#[case::separators("ghcr.io/a.b_c__d--e/goodfetch-plugin-kettle.v2", "kettle.v2")]
fn a_reference_names_a_registry_a_repository_and_an_index_digest(
    #[case] name: &str,
    #[case] plugin: &str,
) {
    let text = format!("{name}@{A}");

    let reference: Reference = text.parse().unwrap();

    let (registry, repository) = name.split_once('/').unwrap();
    assert_eq!(reference.registry(), registry);
    assert_eq!(reference.repository(), repository);
    assert_eq!(reference.plugin(), plugin);
    assert_eq!(reference.index(), A.parse::<Digest>().unwrap());
    assert_eq!(reference.to_string(), text);
    assert_eq!(
        reference.manifest_path(reference.index()),
        format!("/v2/{repository}/manifests/{A}")
    );
    assert_eq!(
        reference.blob_path(B.parse().unwrap()),
        format!("/v2/{repository}/blobs/{B}")
    );
}

/// A reference names its index by digest, its registry by host and port,
/// and a `goodfetch-plugin-*` repository in the OCI distribution
/// specification's lowercase grammar; a tag, a path or anything else is
/// refused.
///
/// Covers: story:7-5
#[rstest]
#[case::a_tag("localhost:5000/goodfetch-plugin-kettle:1.0")]
#[case::a_tag_and_a_digest(&format!("localhost:5000/goodfetch-plugin-kettle:1.0@{A}"))]
#[case::another_algorithm("localhost:5000/goodfetch-plugin-kettle@sha512:aa")]
#[case::uppercase_digest(&format!("localhost:5000/goodfetch-plugin-kettle@{}", A.to_uppercase()))]
#[case::no_registry(&format!("goodfetch-plugin-kettle@{A}"))]
#[case::a_url(&format!("https://localhost/goodfetch-plugin-kettle@{A}"))]
#[case::credentials(&format!("user:pass@localhost/goodfetch-plugin-kettle@{A}"))]
#[case::port_zero(&format!("localhost:0/goodfetch-plugin-kettle@{A}"))]
#[case::uppercase_repository(&format!("localhost/Goodfetch-plugin-kettle@{A}"))]
#[case::traversal(&format!("localhost/../goodfetch-plugin-kettle@{A}"))]
#[case::empty_segment(&format!("localhost//goodfetch-plugin-kettle@{A}"))]
#[case::query(&format!("localhost/goodfetch-plugin-kettle?x=1@{A}"))]
#[case::another_image(&format!("localhost/kettle@{A}"))]
#[case::prefix_not_last(&format!("localhost/goodfetch-plugin-kettle/other@{A}"))]
fn a_reference_that_is_no_pinned_plugin_image_is_refused(#[case] text: &str) {
    let refused = text.parse::<Reference>().unwrap_err();

    assert_eq!(refused.reference, text);
}

proptest! {
    /// A reference that reads is one whose text reads back the same, and
    /// whose paths stay below the repository's.
    ///
    /// Covers: story:7-5
    #[test]
    fn a_reference_reads_back_from_its_text(text in "[a-z0-9.:/@_-]{0,40}(sha256:[0-9a-f]{64})?") {
        if let Ok(reference) = text.parse::<Reference>() {
            prop_assert_eq!(reference.to_string().parse::<Reference>(), Ok(reference.clone()));
            let path = reference.manifest_path(reference.index());
            prop_assert!(!path.contains("/../") && !path.contains('?') && !path.contains('#'));
        }
    }
}

/// The index names the manifest of the first entry that runs on the
/// platform: its OS and architecture, and no variant but `v8` on `arm64`.
///
/// Covers: story:7-5
#[rstest]
#[case::the_first_that_runs(LINUX_AMD64, A)]
#[case::arm64_v8(DARWIN_ARM64, C)]
fn the_index_names_the_platforms_manifest(#[case] platform: Platform, #[case] expected: &str) {
    let json = index(&[
        entry(B, "linux", "amd64", Some("v3")),
        entry(A, "linux", "amd64", None),
        entry(B, "linux", "amd64", None),
        entry(B, "darwin", "arm64", Some("v9")),
        entry(C, "darwin", "arm64", Some("v8")),
    ]);

    let descriptor = manifest_for(&json, platform).unwrap();

    assert_eq!(descriptor.digest, expected.parse().unwrap());
    assert_eq!(descriptor.size, 512);
}

/// An index the engine does not follow, or that names no manifest for the
/// platform, is refused with why.
///
/// Covers: story:7-5
#[rstest]
#[case::no_json(b"not json".to_vec(), "Malformed")]
#[case::no_platform(index(&[entry(A, "windows", "amd64", None), entry(A, "linux", "arm64", None)]), "NoPlatform")]
#[case::no_platform_named(index(&[json!({ "mediaType": MANIFEST_MEDIA_TYPE, "digest": A, "size": 1 })]), "NoPlatform")]
#[case::schema_1(serde_json::to_vec(&json!({ "schemaVersion": 1, "manifests": [] })).unwrap(), "SchemaVersion")]
#[case::a_manifest_not_an_index(serde_json::to_vec(&json!({ "schemaVersion": 2, "mediaType": MANIFEST_MEDIA_TYPE, "manifests": [] })).unwrap(), "MediaType")]
#[case::a_nested_index(index(&[json!({ "mediaType": INDEX_MEDIA_TYPE, "digest": A, "size": 1, "platform": { "os": "linux", "architecture": "amd64" } })]), "MediaType")]
#[case::another_digest(index(&[json!({ "mediaType": MANIFEST_MEDIA_TYPE, "digest": "sha512:00", "size": 1, "platform": { "os": "linux", "architecture": "amd64" } })]), "Digest")]
#[case::a_huge_manifest(index(&[json!({ "mediaType": MANIFEST_MEDIA_TYPE, "digest": A, "size": DOCUMENT_LIMIT + 1, "platform": { "os": "linux", "architecture": "amd64" } })]), "TooLarge")]
#[case::too_large(vec![b' '; usize::try_from(DOCUMENT_LIMIT).unwrap() + 1], "TooLarge")]
fn an_index_the_engine_does_not_follow_is_refused(#[case] json: Vec<u8>, #[case] why: &str) {
    let refused = manifest_for(&json, LINUX_AMD64).unwrap_err();

    assert!(format!("{refused:?}").starts_with(why), "{refused:?}");
}

/// The manifest's one layer is the bundle, a tar archive, compressed with
/// gzip or not.
///
/// Covers: story:7-5
#[rstest]
#[case::plain(TAR_MEDIA_TYPE, Compression::None)]
#[case::gzip(TAR_GZIP_MEDIA_TYPE, Compression::Gzip)]
fn the_manifests_one_layer_is_the_bundle(
    #[case] media_type: &str,
    #[case] compression: Compression,
) {
    let layer = bundle_layer(&manifest(&[layer(media_type, 2048)])).unwrap();

    assert_eq!(layer.compression, compression);
    assert_eq!(layer.descriptor.digest, B.parse().unwrap());
    assert_eq!(layer.descriptor.size, 2048);
}

/// Covers: story:7-5
#[rstest]
#[case::no_layer(manifest(&[]), "Layers")]
#[case::two_layers(manifest(&[layer(TAR_MEDIA_TYPE, 1), layer(TAR_MEDIA_TYPE, 1)]), "Layers")]
#[case::another_media_type(manifest(&[layer("application/vnd.oci.image.layer.v1.tar+zstd", 1)]), "MediaType")]
#[case::too_large(manifest(&[layer(TAR_MEDIA_TYPE, LAYER_LIMIT + 1)]), "LayerTooLarge")]
#[case::an_index(index(&[]), "Malformed")]
fn a_manifest_that_holds_no_one_bundle_layer_is_refused(#[case] json: Vec<u8>, #[case] why: &str) {
    let refused: DocumentError = bundle_layer(&json).unwrap_err();

    assert!(format!("{refused:?}").starts_with(why), "{refused:?}");
}

#[test]
fn the_host_platform_is_named_as_oci_names_it() {
    let host = Platform::host();

    assert!(
        ["linux", "darwin", "windows"].contains(&host.os),
        "{host:?}"
    );
    assert!(["amd64", "arm64"].contains(&host.architecture), "{host:?}");
}

#[test]
fn attestation_manifests_filters_sigstore_referrers_and_attestation_layer_decodes_bundle() {
    let referrers = index(&[
        json!({
            "mediaType": MANIFEST_MEDIA_TYPE,
            "artifactType": "application/spdx+json",
            "digest": A,
            "size": 100,
        }),
        json!({
            "mediaType": MANIFEST_MEDIA_TYPE,
            "artifactType": SIGSTORE_BUNDLE_MEDIA_TYPE,
            "digest": B,
            "size": 256,
        }),
    ]);
    let found = attestation_manifests(&referrers).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].digest, B.parse::<Digest>().unwrap());
    assert_eq!(found[0].size, 256);

    let att_manifest = manifest(&[layer(SIGSTORE_BUNDLE_MEDIA_TYPE, 512)]);
    let desc = attestation_layer(&att_manifest).unwrap();
    assert_eq!(desc.digest, B.parse::<Digest>().unwrap());
    assert_eq!(desc.size, 512);

    let oversize = manifest(&[layer(SIGSTORE_BUNDLE_MEDIA_TYPE, ATTESTATION_LIMIT + 1)]);
    assert!(matches!(
        attestation_layer(&oversize),
        Err(DocumentError::TooLarge { .. })
    ));
}

goodfetch_test_support::fuzz_replay!(
    crate::plugin_oci::fuzz_oci_image_index,
    "fuzz/oci_image_index"
);
