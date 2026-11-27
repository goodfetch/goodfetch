//! Provenance attestation verification, signer identity in the lockfile, and
//! look-alike name protection for `plugin add` (ADRs 0033 and 0059;
//! issue 916).
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests write bundles and read lockfiles outside async workers"
)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;

use goodfetch_engine::browser_cache::{BrowserCache, CacheLocation};
use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::plugin_add::{self, ProvenancePolicy};
use goodfetch_engine::plugin_lockfile;
use goodfetch_engine::plugin_oci::{
    INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, Platform, SIGSTORE_BUNDLE_MEDIA_TYPE,
    TAR_GZIP_MEDIA_TYPE,
};
use goodfetch_engine::plugin_provenance::{AttestationSigner, AttestationTrustRoot};
use goodfetch_engine::plugin_runtime::RuntimeCatalog;
use goodfetch_engine::plugin_store::PluginStore;
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::plugin::approvals::Origin;
use goodfetch_protocol::plugin::index::{
    IndexEntry, PluginIndex, ProvenanceAttestation, ProvenanceStatement, SignerIdentity,
};
use goodfetch_protocol::plugin::lockfile::LOCKFILE_NAME;
use goodfetch_protocol::plugin::settings::PLUGINS;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use hyper::{HeaderMap, StatusCode};
use jiff::Timestamp;
use serde_json::{Map, json};

use crate::downloads::StandIn;
use crate::plugin_install::store;

const ATT_KEY_ID: &str = "sigstore-test-key-1";
const ATT_SEED: [u8; 32] = [0x51; 32];
const OTHER_DIGEST: &str =
    "sha256:9999999999999999999999999999999999999999999999999999999999999999";

fn expected_signer() -> SignerIdentity {
    SignerIdentity {
        issuer: "https://token.actions.githubusercontent.com".to_owned(),
        workflow: "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0"
            .to_owned(),
        repository: "github.com/vendor/kettle".to_owned(),
    }
}

fn wrong_signer() -> SignerIdentity {
    SignerIdentity {
        issuer: "https://token.actions.githubusercontent.com".to_owned(),
        workflow:
            "https://github.com/attacker/kettle/.github/workflows/release.yml@refs/tags/v1.0.0"
                .to_owned(),
        repository: "github.com/attacker/kettle".to_owned(),
    }
}

/// Builds a minimal gzipped tar bundle for a toy Plugin named `name`.
fn toy_bundle(name: &str) -> Vec<u8> {
    let manifest = json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
    })
    .to_string();
    let script = b"#!/bin/sh\nexit 0\n";
    let mut tar_builder = tar::Builder::new(Vec::new());
    for (path, bytes, mode) in [
        ("goodfetch-plugin.json", manifest.as_bytes(), 0o644u32),
        ("plugin", script.as_slice(), 0o755u32),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(bytes.len()).unwrap());
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        tar_builder.append_data(&mut header, path, bytes).unwrap();
    }
    let tar_bytes = tar_builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

/// An OCI multi-platform artifact for Plugin `name`, optionally carrying a
/// provenance attestation served via the OCI 1.1 Referrers API.
#[derive(Clone)]
struct AttestedArtifact {
    name: String,
    index: Vec<u8>,
    manifest: Vec<u8>,
    layer: Vec<u8>,
    referrers: Option<(Vec<u8>, Vec<u8>, Vec<u8>)>,
}

impl AttestedArtifact {
    fn new(name: &str) -> Self {
        let layer = toy_bundle(name);
        let manifest = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST_MEDIA_TYPE,
            "config": {
                "mediaType": "application/vnd.oci.empty.v1+json",
                "digest": Digest::of(b"{}").to_string(),
                "size": 2,
            },
            "layers": [{
                "mediaType": TAR_GZIP_MEDIA_TYPE,
                "digest": Digest::of(&layer).to_string(),
                "size": layer.len(),
            }],
        }))
        .unwrap();
        let host = Platform::host();
        let index = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": INDEX_MEDIA_TYPE,
            "manifests": [{
                "mediaType": MANIFEST_MEDIA_TYPE,
                "digest": Digest::of(&manifest).to_string(),
                "size": manifest.len(),
                "platform": { "os": host.os, "architecture": host.architecture },
            }],
        }))
        .unwrap();
        Self {
            name: name.to_owned(),
            index,
            manifest,
            layer,
            referrers: None,
        }
    }

    fn digest(&self) -> String {
        Digest::of(&self.index).to_string()
    }

    fn repository(&self) -> String {
        format!("goodfetch-plugin-{}", self.name)
    }

    fn with_attestation(mut self, attestation: &ProvenanceAttestation) -> Self {
        let blob = serde_json::to_vec(attestation).unwrap();
        let att_manifest = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST_MEDIA_TYPE,
            "artifactType": SIGSTORE_BUNDLE_MEDIA_TYPE,
            "config": {
                "mediaType": "application/vnd.oci.empty.v1+json",
                "digest": Digest::of(b"{}").to_string(),
                "size": 2,
            },
            "layers": [{
                "mediaType": SIGSTORE_BUNDLE_MEDIA_TYPE,
                "digest": Digest::of(&blob).to_string(),
                "size": blob.len(),
            }],
        }))
        .unwrap();
        let referrers_index = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": INDEX_MEDIA_TYPE,
            "manifests": [{
                "mediaType": MANIFEST_MEDIA_TYPE,
                "artifactType": SIGSTORE_BUNDLE_MEDIA_TYPE,
                "digest": Digest::of(&att_manifest).to_string(),
                "size": att_manifest.len(),
            }],
        }))
        .unwrap();
        self.referrers = Some((referrers_index, att_manifest, blob));
        self
    }

    fn handler(&self) -> impl Fn(&str, &HeaderMap) -> (StatusCode, Vec<u8>) + use<> {
        let repo = self.repository();
        let mut routes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        routes.insert(
            format!("/v2/{repo}/manifests/{}", Digest::of(&self.index)),
            self.index.clone(),
        );
        routes.insert(
            format!("/v2/{repo}/manifests/{}", Digest::of(&self.manifest)),
            self.manifest.clone(),
        );
        routes.insert(
            format!("/v2/{repo}/blobs/{}", Digest::of(&self.layer)),
            self.layer.clone(),
        );
        if let Some((referrers_index, att_manifest, blob)) = &self.referrers {
            routes.insert(
                format!("/v2/{repo}/referrers/{}", Digest::of(&self.index)),
                referrers_index.clone(),
            );
            routes.insert(
                format!("/v2/{repo}/manifests/{}", Digest::of(att_manifest)),
                att_manifest.clone(),
            );
            routes.insert(
                format!("/v2/{repo}/blobs/{}", Digest::of(blob)),
                blob.clone(),
            );
        }
        move |path, _| match routes.get(path) {
            Some(bytes) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }
}

fn index_listing(authority: &str, name: &str, digest: &str, signer: SignerIdentity) -> PluginIndex {
    PluginIndex {
        version: 1,
        signed_at: "2026-11-27T00:00:00Z".to_owned(),
        expires_at: "2026-12-11T00:00:00Z".to_owned(),
        plugins: vec![IndexEntry {
            name: name.to_owned(),
            reference: format!("{authority}/goodfetch-plugin-{name}"),
            digest: digest.to_owned(),
            kinds: vec![PluginKindName::EXTRACT],
            signer,
            origin: Origin::ThirdParty,
            conformance: Vec::new(),
        }],
        revocations: Vec::new(),
    }
}

fn policy_allowing(name: &str, digest: &str) -> Resolved {
    let mut map = Map::new();
    map.insert(
        PLUGINS.name.to_owned(),
        json!({
            name: {
                "allow": true,
                "digest": digest,
            }
        }),
    );
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(SettingsLayer::OperatorPolicy, &map, Product::Collie).unwrap(),
            ..Policy::default()
        },
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

fn empty_policy() -> Resolved {
    resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap()
}

fn run_add_checked(
    env: &TestEnv,
    stand_in: &StandIn,
    raw_reference: &str,
    index: Option<&PluginIndex>,
    settings: Option<&Resolved>,
    trust: &AttestationTrustRoot,
    lockfile: &Path,
) -> Result<goodfetch_protocol::plugin::add::PluginAdded, goodfetch_engine::CallError> {
    let plugin_store = PluginStore::new(store(env));
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
    let catalog = RuntimeCatalog::built_in();
    let fetching = stand_in.fetching();
    let provenance = ProvenancePolicy {
        index,
        settings,
        trust,
    };
    let now = Timestamp::from_second(1_764_200_000).unwrap();
    goodfetch_test_support::runtime().block_on(plugin_add::add_checked(
        raw_reference,
        &provenance,
        &fetching,
        &blocking,
        &plugin_store,
        lockfile,
        &cache,
        &catalog,
        now,
    ))
}

/// A toy Plugin with a matching provenance attestation installs without an
/// Operator policy digest override, returns its verified signer identity,
/// and records the signer identity in `goodfetch-plugins.lock`.
///
/// Covers: story:7-7
#[test]
fn toy_plugin_with_matching_attestation_installs_and_records_signer_in_lockfile() {
    let env = TestEnv::new();
    let (att_signer, trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let signer = expected_signer();
    let base = AttestedArtifact::new("kettle");
    let digest = base.digest();
    let attestation = att_signer
        .sign(ProvenanceStatement {
            subject_digest: digest.clone(),
            signer: signer.clone(),
        })
        .unwrap();
    let artifact = base.with_attestation(&attestation);

    let stand_in =
        goodfetch_test_support::runtime().block_on(StandIn::serve_with(artifact.handler()));
    let index = index_listing(&stand_in.authority(), "kettle", &digest, signer.clone());
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let reference = format!(
        "{}/{}@{digest}",
        stand_in.authority(),
        artifact.repository()
    );
    let policy = empty_policy();

    let added = run_add_checked(
        &env,
        &stand_in,
        &reference,
        Some(&index),
        Some(&policy),
        &trust,
        &lockfile,
    )
    .unwrap();

    assert_eq!(added.name, "kettle");
    assert_eq!(added.digest, digest);
    assert_eq!(added.signer, Some(signer.clone()));

    let pins = plugin_lockfile::read(&std::fs::read_to_string(&lockfile).unwrap()).unwrap();
    let locked = pins.locked("kettle").unwrap();
    assert_eq!(locked.digest, digest);
    assert_eq!(locked.signer.as_deref(), Some(signer.to_string().as_str()));
}

/// A toy Plugin with a wrong signer or mismatched subject digest is refused
/// with `plugin_attestation_invalid` (leaving the store and lockfile
/// untouched), unless Operator policy allows its exact digest.
///
/// Covers: story:7-7
#[test]
fn wrong_signer_and_digest_mismatch_refused_unless_operator_policy_allows_exact_digest() {
    let (att_signer, trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let expected = expected_signer();
    let policy_none = empty_policy();

    // 1. Wrong signer -> refused with plugin_attestation_invalid (signer_mismatch).
    {
        let env = TestEnv::new();
        let base = AttestedArtifact::new("kettle");
        let digest = base.digest();
        let bad_att = att_signer
            .sign(ProvenanceStatement {
                subject_digest: digest.clone(),
                signer: wrong_signer(),
            })
            .unwrap();
        let artifact = base.with_attestation(&bad_att);
        let stand_in =
            goodfetch_test_support::runtime().block_on(StandIn::serve_with(artifact.handler()));
        let index = index_listing(&stand_in.authority(), "kettle", &digest, expected.clone());
        let lockfile = env.home().join("project").join(LOCKFILE_NAME);
        let reference = format!(
            "{}/{}@{digest}",
            stand_in.authority(),
            artifact.repository()
        );

        let err = run_add_checked(
            &env,
            &stand_in,
            &reference,
            Some(&index),
            Some(&policy_none),
            &trust,
            &lockfile,
        )
        .unwrap_err();
        assert_eq!(err.code(), "plugin_attestation_invalid");
        assert_eq!(err.details().unwrap()["reason"], json!("signer_mismatch"));
        assert!(!lockfile.exists());
        assert!(!store(&env).join("kettle").exists());

        // Allowing its exact digest in Operator policy lets it install.
        let policy_allow = policy_allowing("kettle", &digest);
        let added = run_add_checked(
            &env,
            &stand_in,
            &reference,
            Some(&index),
            Some(&policy_allow),
            &trust,
            &lockfile,
        )
        .unwrap();
        assert_eq!(added.name, "kettle");
        assert_eq!(added.digest, digest);
        assert_eq!(added.signer, None);
    }

    // 2. Attestation covering a different digest -> refused (digest_mismatch).
    {
        let env = TestEnv::new();
        let base = AttestedArtifact::new("kettle");
        let digest = base.digest();
        let wrong_digest_att = att_signer
            .sign(ProvenanceStatement {
                subject_digest: OTHER_DIGEST.to_owned(),
                signer: expected.clone(),
            })
            .unwrap();
        let artifact = base.with_attestation(&wrong_digest_att);
        let stand_in =
            goodfetch_test_support::runtime().block_on(StandIn::serve_with(artifact.handler()));
        let index = index_listing(&stand_in.authority(), "kettle", &digest, expected);
        let lockfile = env.home().join("project").join(LOCKFILE_NAME);
        let reference = format!(
            "{}/{}@{digest}",
            stand_in.authority(),
            artifact.repository()
        );

        let err = run_add_checked(
            &env,
            &stand_in,
            &reference,
            Some(&index),
            Some(&policy_none),
            &trust,
            &lockfile,
        )
        .unwrap_err();
        assert_eq!(err.code(), "plugin_attestation_invalid");
        assert_eq!(err.details().unwrap()["reason"], json!("digest_mismatch"));
    }
}

/// An unattested toy Plugin is refused with `plugin_attestation_invalid`
/// (`missing`) even when Operator policy allows a different digest, and
/// installs once Operator policy allows its exact digest.
///
/// Covers: story:7-7
#[test]
fn unattested_plugin_refused_unless_operator_policy_allows_exact_digest() {
    let (_, trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let expected = expected_signer();
    let policy_none = empty_policy();

    let env = TestEnv::new();
    let artifact = AttestedArtifact::new("kettle");
    let digest = artifact.digest();
    let stand_in =
        goodfetch_test_support::runtime().block_on(StandIn::serve_with(artifact.handler()));
    let index = index_listing(&stand_in.authority(), "kettle", &digest, expected);
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let reference = format!(
        "{}/{}@{digest}",
        stand_in.authority(),
        artifact.repository()
    );

    let err = run_add_checked(
        &env,
        &stand_in,
        &reference,
        Some(&index),
        Some(&policy_none),
        &trust,
        &lockfile,
    )
    .unwrap_err();
    assert_eq!(err.code(), "plugin_attestation_invalid");
    assert_eq!(err.details().unwrap()["reason"], json!("missing"));
    assert!(!lockfile.exists());

    // Policy allowing a different digest still refuses.
    let policy_other = policy_allowing("kettle", OTHER_DIGEST);
    let err = run_add_checked(
        &env,
        &stand_in,
        &reference,
        Some(&index),
        Some(&policy_other),
        &trust,
        &lockfile,
    )
    .unwrap_err();
    assert_eq!(err.code(), "plugin_attestation_invalid");

    // Policy allowing the exact digest installs.
    let policy_exact = policy_allowing("kettle", &digest);
    let added = run_add_checked(
        &env,
        &stand_in,
        &reference,
        Some(&index),
        Some(&policy_exact),
        &trust,
        &lockfile,
    )
    .unwrap();
    assert_eq!(added.digest, digest);
    assert_eq!(added.signer, None);
}

/// A one-letter look-alike of an index entry (`ketle` for `kettle`) is
/// refused with `plugin_name_ambiguous` (`ProgramError`) when given without
/// an exact digest (making zero registry requests), and installs cleanly when
/// given with an exact digest; an exact bare index name resolves from the
/// index and installs with its signer recorded.
///
/// Covers: story:7-6
#[test]
fn one_letter_look_alike_refused_without_exact_digest_and_installs_with_one() {
    let env = TestEnv::new();
    let (att_signer, trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let signer = expected_signer();

    // Artifact for look-alike `ketle` AND artifact for listed `kettle`.
    let typo_artifact = AttestedArtifact::new("ketle");
    let typo_digest = typo_artifact.digest();

    let listed_base = AttestedArtifact::new("kettle");
    let listed_digest = listed_base.digest();
    let listed_att = att_signer
        .sign(ProvenanceStatement {
            subject_digest: listed_digest.clone(),
            signer: signer.clone(),
        })
        .unwrap();
    let listed_artifact = listed_base.with_attestation(&listed_att);

    let typo_handler = typo_artifact.handler();
    let listed_handler = listed_artifact.handler();
    let mut stand_in =
        goodfetch_test_support::runtime().block_on(StandIn::serve_with(move |path, headers| {
            let (status, body) = listed_handler(path, headers);
            if status != StatusCode::NOT_FOUND {
                return (status, body);
            }
            typo_handler(path, headers)
        }));

    let index = index_listing(
        &stand_in.authority(),
        "kettle",
        &listed_digest,
        signer.clone(),
    );
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let policy_none = empty_policy();

    // Bare one-letter look-alike `ketle` is refused with `plugin_name_ambiguous`.
    let err = run_add_checked(
        &env,
        &stand_in,
        "ketle",
        Some(&index),
        Some(&policy_none),
        &trust,
        &lockfile,
    )
    .unwrap_err();
    assert_eq!(err.code(), "plugin_name_ambiguous");
    assert_eq!(err.details().unwrap()["similar"], json!(["kettle"]));
    assert_eq!(stand_in.requests(), []);

    // Unpinned registry path for look-alike `ketle` is also refused.
    let unpinned = format!("{}/goodfetch-plugin-ketle", stand_in.authority());
    let err = run_add_checked(
        &env,
        &stand_in,
        &unpinned,
        Some(&index),
        Some(&policy_none),
        &trust,
        &lockfile,
    )
    .unwrap_err();
    assert_eq!(err.code(), "plugin_name_ambiguous");
    assert_eq!(stand_in.requests(), []);

    // Giving the exact digest installs `ketle` (when Operator policy allows that exact digest).
    let exact_typo = format!(
        "{}/goodfetch-plugin-ketle@{typo_digest}",
        stand_in.authority()
    );
    let policy_typo = policy_allowing("ketle", &typo_digest);
    let added_typo = run_add_checked(
        &env,
        &stand_in,
        &exact_typo,
        Some(&index),
        Some(&policy_typo),
        &trust,
        &lockfile,
    )
    .unwrap();
    assert_eq!(added_typo.name, "ketle");
    assert_eq!(added_typo.digest, typo_digest);

    // Exact bare index name `kettle` resolves from the index, verifies attestation,
    // and records its signer in the lockfile.
    let added_listed = run_add_checked(
        &env,
        &stand_in,
        "kettle",
        Some(&index),
        Some(&policy_none),
        &trust,
        &lockfile,
    )
    .unwrap();
    assert_eq!(added_listed.name, "kettle");
    assert_eq!(added_listed.digest, listed_digest);
    assert_eq!(added_listed.signer, Some(signer.clone()));
    let pins = plugin_lockfile::read(&std::fs::read_to_string(&lockfile).unwrap()).unwrap();
    assert_eq!(
        pins.locked("kettle").unwrap().signer.as_deref(),
        Some(signer.to_string().as_str())
    );
}
