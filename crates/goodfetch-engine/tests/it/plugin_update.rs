//! `plugin update`: permission diff (added, widened, removed), approval for new
//! or wider permissions, `plugin_signer_changed` refusal and re-approval, and
//! no automatic update on engine start, Run or Plugin index refresh (ADRs 0033,
//! 0052 and 0059; issue 924).
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
use goodfetch_engine::plugin_index::PluginIndexStore;
use goodfetch_engine::plugin_lockfile;
use goodfetch_engine::plugin_oci::{
    INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, Platform, SIGSTORE_BUNDLE_MEDIA_TYPE,
    TAR_GZIP_MEDIA_TYPE,
};
use goodfetch_engine::plugin_provenance::AttestationSigner;
use goodfetch_engine::plugin_runtime::RuntimeCatalog;
use goodfetch_engine::plugin_store::PluginStore;
use goodfetch_engine::plugin_update::{self, UpdateOptions};
use goodfetch_engine::signed_set::SignedSetSigner;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::plugin::approvals::{Origin, Permission};
use goodfetch_protocol::plugin::index::{
    IndexEntry, PluginIndex, ProvenanceAttestation, ProvenanceStatement, SignerIdentity,
};
use goodfetch_protocol::plugin::lockfile::LOCKFILE_NAME;
use goodfetch_test_support::TestEnv;
use hyper::{HeaderMap, StatusCode};
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::downloads::StandIn;
use crate::plugin_install::store;

const ATT_KEY_ID: &str = "sigstore-update-key-1";
const ATT_SEED: [u8; 32] = [0x61; 32];
const IDX_KEY_ID: &str = "index-update-key-1";
const IDX_SEED: [u8; 32] = [0x62; 32];
const IDX_ISSUER: &str = "https://token.actions.githubusercontent.com";
const IDX_WORKFLOW: &str =
    "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main";

fn original_signer() -> SignerIdentity {
    SignerIdentity {
        issuer: IDX_ISSUER.to_owned(),
        workflow: "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0"
            .to_owned(),
        repository: "github.com/vendor/kettle".to_owned(),
    }
}

fn new_owner_signer() -> SignerIdentity {
    SignerIdentity {
        issuer: IDX_ISSUER.to_owned(),
        workflow:
            "https://github.com/hijacker/kettle/.github/workflows/release.yml@refs/tags/v2.0.0"
                .to_owned(),
        repository: "github.com/hijacker/kettle".to_owned(),
    }
}

fn manifest_bundle(manifest: &Value) -> Vec<u8> {
    let manifest_text = manifest.to_string();
    let script = b"#!/bin/sh\nexit 0\n";
    let mut tar_builder = tar::Builder::new(Vec::new());
    for (path, bytes, mode) in [
        ("goodfetch-plugin.json", manifest_text.as_bytes(), 0o644u32),
        ("kettle", script.as_slice(), 0o755u32),
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

struct RegistryRoutes {
    routes: BTreeMap<String, Vec<u8>>,
}

impl RegistryRoutes {
    fn new() -> Self {
        Self {
            routes: BTreeMap::new(),
        }
    }

    fn publish(
        &mut self,
        name: &str,
        manifest: &Value,
        att_signer: &AttestationSigner,
        signer_id: SignerIdentity,
    ) -> String {
        let repo = format!("goodfetch-plugin-{name}");
        let layer = manifest_bundle(manifest);
        let oci_manifest = serde_json::to_vec(&json!({
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
                "digest": Digest::of(&oci_manifest).to_string(),
                "size": oci_manifest.len(),
                "platform": { "os": host.os, "architecture": host.architecture },
            }],
        }))
        .unwrap();
        let digest = Digest::of(&index).to_string();
        let attestation: ProvenanceAttestation = att_signer
            .sign(ProvenanceStatement {
                subject_digest: digest.clone(),
                signer: signer_id,
            })
            .unwrap();
        let att_blob = serde_json::to_vec(&attestation).unwrap();
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
                "digest": Digest::of(&att_blob).to_string(),
                "size": att_blob.len(),
            }],
        }))
        .unwrap();
        let referrers = serde_json::to_vec(&json!({
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

        self.routes
            .insert(format!("/v2/{repo}/manifests/{digest}"), index);
        self.routes.insert(
            format!("/v2/{repo}/manifests/{}", Digest::of(&oci_manifest)),
            oci_manifest,
        );
        self.routes
            .insert(format!("/v2/{repo}/blobs/{}", Digest::of(&layer)), layer);
        self.routes
            .insert(format!("/v2/{repo}/referrers/{digest}"), referrers);
        self.routes.insert(
            format!("/v2/{repo}/manifests/{}", Digest::of(&att_manifest)),
            att_manifest,
        );
        self.routes.insert(
            format!("/v2/{repo}/blobs/{}", Digest::of(&att_blob)),
            att_blob,
        );
        digest
    }

    fn handler(self) -> impl Fn(&str, &HeaderMap) -> (StatusCode, Vec<u8>) + use<> {
        let routes = self.routes;
        move |path, _| match routes.get(path) {
            Some(bytes) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }
}

fn index_entry(authority: &str, digest: &str, signer: SignerIdentity, version: u64) -> PluginIndex {
    PluginIndex {
        version,
        signed_at: "2026-11-27T00:00:00Z".to_owned(),
        expires_at: "2026-12-11T00:00:00Z".to_owned(),
        plugins: vec![IndexEntry {
            name: "kettle".to_owned(),
            reference: format!("{authority}/goodfetch-plugin-kettle"),
            digest: digest.to_owned(),
            kinds: vec![PluginKindName::NOTIFICATION],
            signer,
            origin: Origin::ThirdParty,
            conformance: Vec::new(),
        }],
        revocations: Vec::new(),
    }
}

fn lockfile_of(env: &TestEnv) -> std::path::PathBuf {
    env.folder().join(LOCKFILE_NAME)
}

fn read_lockfile_entry(
    path: &Path,
    name: &str,
) -> Option<goodfetch_protocol::plugin::lockfile::LockedPlugin> {
    let text = std::fs::read_to_string(path).ok()?;
    plugin_lockfile::read(&text).ok()?.locked(name).cloned()
}

fn v1_manifest() -> Value {
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["notification"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "none",
        "permissions": {
            "domains": ["api.shop.test", "old.example"],
            "secrets": ["env:TOKEN"],
            "grants": ["effects"],
        },
    })
}

fn v2_wider_manifest() -> Value {
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "2.0.0",
        "kinds": ["notification"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "proxy_tcp",
        "permissions": {
            "domains": ["*.shop.test", "new.example"],
            "secrets": ["env:TOKEN", "env:EXTRA"],
            "grants": ["effects", "secret_value"],
        },
    })
}

fn v3_narrower_manifest() -> Value {
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "3.0.0",
        "kinds": ["notification"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "none",
        "permissions": {
            "domains": ["*.shop.test"],
            "secrets": [],
            "grants": ["effects"],
        },
    })
}

struct UpdateHarness {
    rt: tokio::runtime::Runtime,
    env: TestEnv,
    server: StandIn,
    plugin_store: PluginStore,
    lockfile: std::path::PathBuf,
    blocking: goodfetch_engine::tasks::BlockingThreads,
    cache: BrowserCache,
    catalog: RuntimeCatalog,
    now: Timestamp,
}

impl UpdateHarness {
    fn new(reg: RegistryRoutes) -> Self {
        let rt = goodfetch_test_support::runtime();
        let env = TestEnv::new();
        let server = rt.block_on(StandIn::serve_with(reg.handler()));
        let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
        let plugin_store = PluginStore::new(store(&env));
        let lockfile = lockfile_of(&env);
        let cache = BrowserCache::new(&CacheLocation::new(env.home(), None), blocking.clone());
        let catalog = RuntimeCatalog::built_in();
        let now: Timestamp = "2026-11-27T12:00:00Z".parse().unwrap();
        Self {
            rt,
            env,
            server,
            plugin_store,
            lockfile,
            blocking,
            cache,
            catalog,
            now,
        }
    }

    fn add_kettle(
        &mut self,
        index: &PluginIndex,
        trust: &goodfetch_engine::plugin_provenance::AttestationTrustRoot,
    ) -> goodfetch_protocol::plugin::add::PluginAdded {
        let fetching = self.server.fetching();
        let prov = ProvenancePolicy {
            index: Some(index),
            settings: None,
            trust,
        };
        let out = self
            .rt
            .block_on(plugin_add::add_checked(
                "kettle",
                &prov,
                &fetching,
                &self.blocking,
                &self.plugin_store,
                &self.lockfile,
                &self.cache,
                &self.catalog,
                self.now,
            ))
            .unwrap();
        let _ = self.server.requests();
        out
    }

    fn update_kettle(
        &mut self,
        options: UpdateOptions,
        index: &PluginIndex,
        trust: &goodfetch_engine::plugin_provenance::AttestationTrustRoot,
    ) -> Result<goodfetch_protocol::plugin::update::PluginUpdated, goodfetch_engine::CallError>
    {
        let fetching = self.server.fetching();
        let prov = ProvenancePolicy {
            index: Some(index),
            settings: None,
            trust,
        };
        let out = self
            .rt
            .block_on(plugin_update::update_checked(
                "kettle",
                options,
                &prov,
                &fetching,
                &self.blocking,
                &self.plugin_store,
                &self.lockfile,
                &self.cache,
                &self.catalog,
                self.now,
            ))
            .map(|(updated, _)| updated);
        let _ = self.server.requests();
        out
    }
}

/// `plugin update` computes added, widened and removed permissions, requires
/// approval for any new or wider permission (leaving the store and lockfile
/// untouched on preview or unapproved apply), applies once approved, and
/// requires no new permission approval when an update only removes permissions.
///
/// Covers: story:7-11
#[test]
fn plugin_update_shows_permission_diff_and_requires_approval_for_new_or_wider() {
    let (att_signer, trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let mut reg = RegistryRoutes::new();
    let d1 = reg.publish("kettle", &v1_manifest(), &att_signer, original_signer());
    let d2 = reg.publish(
        "kettle",
        &v2_wider_manifest(),
        &att_signer,
        original_signer(),
    );
    let d3 = reg.publish(
        "kettle",
        &v3_narrower_manifest(),
        &att_signer,
        original_signer(),
    );
    let mut h = UpdateHarness::new(reg);

    // Install v1.0.0.
    let idx_v1 = index_entry(&h.server.authority(), &d1, original_signer(), 1);
    assert_eq!(h.add_kettle(&idx_v1, &trust).digest, d1);

    // Preview update to v2.0.0: lists added, widened and removed, leaves v1 untouched.
    let idx_v2 = index_entry(&h.server.authority(), &d2, original_signer(), 2);
    let preview_opts = UpdateOptions {
        preview: true,
        ..UpdateOptions::default()
    };
    let preview = h.update_kettle(preview_opts, &idx_v2, &trust).unwrap();
    assert!(!preview.applied && !preview.signer_changed && preview.diff.requires_approval);
    assert_eq!(preview.digest, d2);
    assert_eq!(preview.signer, Some(original_signer()));
    assert_v2_diff(&preview.diff);
    assert_eq!(
        read_lockfile_entry(&h.lockfile, "kettle").unwrap().digest,
        d1
    );

    // Applying without `approve: true` is refused with `plugin_permission_denied`.
    let refused = h
        .update_kettle(UpdateOptions::default(), &idx_v2, &trust)
        .unwrap_err();
    assert_eq!(refused.code(), "plugin_permission_denied");
    assert_eq!(
        read_lockfile_entry(&h.lockfile, "kettle").unwrap().digest,
        d1
    );

    // Applying with `approve: true` updates store and lockfile.
    let approve_opts = UpdateOptions {
        approve: true,
        ..UpdateOptions::default()
    };
    let applied = h.update_kettle(approve_opts, &idx_v2, &trust).unwrap();
    assert!(applied.applied);
    assert_eq!(applied.version, "2.0.0");
    assert_eq!(
        read_lockfile_entry(&h.lockfile, "kettle").unwrap().digest,
        d2
    );
    assert!(store(&h.env).join("kettle").join("2.0.0").exists());
    assert!(!store(&h.env).join("kettle").join("1.0.0").exists());

    // Shrinking-only update to v3.0.0 applies without requiring new permission approval.
    let idx_v3 = index_entry(&h.server.authority(), &d3, original_signer(), 3);
    let shrunk = h
        .update_kettle(UpdateOptions::default(), &idx_v3, &trust)
        .unwrap();
    assert!(shrunk.applied && !shrunk.diff.requires_approval);
    assert_ne!(shrunk.diff.removed, []);
    assert_eq!(
        read_lockfile_entry(&h.lockfile, "kettle").unwrap().digest,
        d3
    );
}

fn assert_v2_diff(diff: &goodfetch_protocol::plugin::update::PermissionDiff) {
    let added_effect = Permission::Effects {
        domain: "new.example".to_owned(),
    };
    let removed_effect = Permission::Effects {
        domain: "old.example".to_owned(),
    };
    assert!(diff.added.iter().any(|p| p.permission == added_effect));
    assert!(
        diff.widened
            .iter()
            .any(|w| w.kind == "network_level" && w.from == "none" && w.to == "proxy_tcp")
    );
    assert!(
        diff.widened
            .iter()
            .any(|w| w.kind == "effects" && w.from == "api.shop.test" && w.to == "*.shop.test")
    );
    assert!(diff.removed.iter().any(|p| p.permission == removed_effect));
}

/// A release whose verified signer changed from the previously installed
/// release is refused with `plugin_signer_changed` (leaving store and lockfile
/// untouched) and accepted once the operator approves the signer change.
///
/// Covers: story:7-12
#[test]
fn plugin_update_refuses_changed_signer_and_accepts_after_approval() {
    let (att_signer, trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let mut reg = RegistryRoutes::new();
    let d1 = reg.publish("kettle", &v1_manifest(), &att_signer, original_signer());
    let mut v2 = v1_manifest();
    v2["version"] = json!("2.0.0");
    let d2 = reg.publish("kettle", &v2, &att_signer, new_owner_signer());
    let mut h = UpdateHarness::new(reg);

    // Install v1.0.0 signed by `original_signer`.
    let idx_v1 = index_entry(&h.server.authority(), &d1, original_signer(), 1);
    h.add_kettle(&idx_v1, &trust);

    // Index now points at v2.0.0 signed by `new_owner_signer`.
    let idx_v2 = index_entry(&h.server.authority(), &d2, new_owner_signer(), 2);

    // Preview flags `signer_changed: true` with both signers named.
    let preview = h
        .update_kettle(
            UpdateOptions {
                preview: true,
                ..UpdateOptions::default()
            },
            &idx_v2,
            &trust,
        )
        .unwrap();
    assert!(preview.signer_changed);
    assert_eq!(preview.previous_signer, Some(original_signer().to_string()));
    assert_eq!(preview.signer, Some(new_owner_signer()));

    // Applying without `approve_signer_change: true` fails with `plugin_signer_changed`.
    let refused = h
        .update_kettle(
            UpdateOptions {
                approve: true,
                ..UpdateOptions::default()
            },
            &idx_v2,
            &trust,
        )
        .unwrap_err();
    assert_eq!(refused.code(), "plugin_signer_changed");
    assert_eq!(
        read_lockfile_entry(&h.lockfile, "kettle").unwrap().digest,
        d1
    );

    // Accepting after approval replaces the Plugin and records the new signer.
    let accepted = h
        .update_kettle(
            UpdateOptions {
                approve: true,
                approve_signer_change: true,
                ..UpdateOptions::default()
            },
            &idx_v2,
            &trust,
        )
        .unwrap();
    assert!(accepted.applied);
    assert!(accepted.signer_changed);
    let locked = read_lockfile_entry(&h.lockfile, "kettle").unwrap();
    assert_eq!(locked.digest, d2);
    assert_eq!(locked.signer, Some(new_owner_signer().to_string()));
}

/// Importing a newer Plugin index that advertises a newer release and digest
/// never automatically updates an already-installed Plugin's bundle or
/// lockfile entry.
///
/// Covers: story:7-11, story:7-12
#[test]
fn index_refresh_never_auto_updates_an_installed_plugin() {
    let (att_signer, att_trust) = AttestationSigner::for_root(ATT_KEY_ID, ATT_SEED);
    let (idx_signer, idx_trust) =
        SignedSetSigner::for_root(IDX_KEY_ID, IDX_ISSUER, IDX_WORKFLOW, IDX_SEED);
    let mut reg = RegistryRoutes::new();
    let d1 = reg.publish("kettle", &v1_manifest(), &att_signer, original_signer());
    let d2 = reg.publish(
        "kettle",
        &v2_wider_manifest(),
        &att_signer,
        original_signer(),
    );
    let mut h = UpdateHarness::new(reg);

    // Install v1.0.0.
    let idx_v1 = index_entry(&h.server.authority(), &d1, original_signer(), 1);
    h.add_kettle(&idx_v1, &att_trust);

    // Refresh the Plugin index store with v2 advertising `d2`.
    let idx_v2 = index_entry(&h.server.authority(), &d2, original_signer(), 2);
    let bundle_v2 = idx_signer.sign(idx_v2).unwrap();
    let index_store = PluginIndexStore::new(h.env.home().join("plugin-index"));
    index_store.apply(bundle_v2, &idx_trust, h.now).unwrap();

    // Installed bundle and lockfile remain pinned to v1.0.0 (`d1`).
    assert_eq!(
        read_lockfile_entry(&h.lockfile, "kettle").unwrap().digest,
        d1
    );
    assert!(store(&h.env).join("kettle").join("1.0.0").exists());
    assert!(!store(&h.env).join("kettle").join("2.0.0").exists());
}
