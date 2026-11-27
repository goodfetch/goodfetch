//! The Plugin index and shared signed-set verification (ADRs 0033, 0047,
//! 0052 and 0059; issue 911): bad signature, untrusted signer, stale
//! version, expiry blocking third-party loading while first-party still
//! loads, immediate revocation stop + Audit log entry, Clean-up rules
//! sharing the signed-set verifier, and delisting without revocation.
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests write Plugin bundles and read Audit entries on blocking setup"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::Redaction;
use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::plugin_add;
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_host::instance::{
    InMemoryPeer, InMemoryPlugin, PluginInstance, Scope,
};
use goodfetch_engine::plugin_index::{IndexError, PluginIndexStore, check_load};
use goodfetch_engine::plugin_kinds::run_plugins::{RunPlugins, StartError};
use goodfetch_engine::plugin_oci::TAR_GZIP_MEDIA_TYPE;
use goodfetch_engine::plugin_store::PluginStore;
use goodfetch_engine::sandbox_probe::SandboxProbe;
use goodfetch_engine::secrets::{Carried, Keychain, Secrets};
use goodfetch_engine::settings::{Layers, Policy, Values, resolve};
use goodfetch_engine::signed_set::{
    self, SignedSetPayload, SignedSetSigner, SignedSetVerdict, TrustRoot,
};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::clean_up_rules::{CleanUpRulesManifest, RulesLists, RulesSource};
use goodfetch_protocol::plugin::approvals::{Origin, admit};
use goodfetch_protocol::plugin::index::{
    ConformanceResult, IndexEntry, PLUGIN_REVOKED_AUDIT, PluginIndex, Revocation, SignerIdentity,
};
use goodfetch_protocol::plugin::lockfile::LOCKFILE_NAME;
use goodfetch_protocol::plugin::{PluginKindName, PluginManifest};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use hyper::StatusCode;
use jiff::Timestamp;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::downloads::StandIn;
use crate::plugin_install::{Artifact, bundle, gzip, store};

const TEST_ISSUER: &str = "https://token.actions.githubusercontent.com";
const TEST_WORKFLOW: &str =
    "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main";
const TEST_KEY_ID: &str = "test-index-key-1";
const SEED_A: [u8; 32] = [0x11; 32];
const SEED_B: [u8; 32] = [0x22; 32];
const DIGEST_FIRST_PARTY: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const DIGEST_THIRD_PARTY: &str =
    "sha256:3333333333333333333333333333333333333333333333333333333333333333";

fn test_signer() -> (SignedSetSigner, TrustRoot) {
    SignedSetSigner::for_root(TEST_KEY_ID, TEST_ISSUER, TEST_WORKFLOW, SEED_A)
}

fn test_signer_identity() -> SignerIdentity {
    SignerIdentity {
        issuer: TEST_ISSUER.to_owned(),
        workflow: "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0"
            .to_owned(),
        repository: "github.com/vendor/kettle".to_owned(),
    }
}

fn ts(unix_seconds: i64) -> Timestamp {
    Timestamp::from_second(unix_seconds).unwrap()
}

fn sample_index(
    version: u64,
    signed_at: Timestamp,
    expires_at: Timestamp,
    plugins: Vec<IndexEntry>,
    revocations: Vec<Revocation>,
) -> PluginIndex {
    PluginIndex {
        version,
        signed_at: signed_at.to_string(),
        expires_at: expires_at.to_string(),
        plugins,
        revocations,
    }
}

fn index_entry(name: &str, digest: &str, origin: Origin) -> IndexEntry {
    IndexEntry {
        name: name.to_owned(),
        reference: format!("ghcr.io/goodfetch/plugins/{name}"),
        digest: digest.to_owned(),
        kinds: vec![PluginKindName::EXTRACT],
        signer: test_signer_identity(),
        origin,
        conformance: vec![ConformanceResult {
            suite: "extract".to_owned(),
            passed: true,
            engine_version: "0.1.0".to_owned(),
        }],
    }
}

fn install_extractor(home: &Home, name: &str, digest: &str) {
    let bundle = home.plugins().join(name).join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
    });
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
    let mut digest_path = bundle.as_os_str().to_owned();
    digest_path.push(".digest");
    std::fs::write(digest_path, format!("{digest}\n")).unwrap();
}

fn policy_allowing(plugins: &Value) -> goodfetch_engine::settings::Resolved {
    let Value::Object(map) = json!({ "plugins": plugins }) else {
        panic!("expected object");
    };
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(SettingsLayer::OperatorPolicy, &map, Product::Collie).unwrap(),
            ..Policy::default()
        },
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

fn empty_secrets(
    home: &Home,
    audit: Arc<AuditLog>,
    owner: Arc<TaskOwner>,
    clock: Clock,
    seed: u64,
) -> goodfetch_engine::secrets::RunSecrets {
    Secrets::new(
        Carried {
            environment: BTreeMap::new(),
            keychain: Keychain::absent(),
            home: home.clone(),
            entropy: Entropy::seeded(seed),
        },
        audit,
        owner,
        clock,
    )
    .for_run_with(Duration::from_secs(30), Redaction::On)
}

fn test_manifest(name: &str) -> PluginManifest {
    PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": name,
            "version": "1.0.0",
            "kinds": ["extract"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

/// An index with a bad signature, an untrusted signer workflow, an older
/// `version` than the last seen, or past its expiry is refused with
/// `plugin_index_stale` and leaves the stored index and last-seen version
/// untouched.
///
/// Covers: story:7-8, story:7-115
#[test]
fn index_refuses_bad_signature_untrusted_signer_stale_version_and_expired() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let index_store = PluginIndexStore::new(home.plugin_index());
    let (signer, trust) = test_signer();
    let (rogue_signer, _) =
        SignedSetSigner::for_root(TEST_KEY_ID, TEST_ISSUER, TEST_WORKFLOW, SEED_B);
    let now = ts(1_760_000_000);
    let expires = ts(1_760_000_000 + 14 * 86_400);

    // Accept v5 first.
    let v5 = signer
        .sign(sample_index(5, now, expires, Vec::new(), Vec::new()))
        .unwrap();
    index_store.apply(v5, &trust, now).unwrap();
    assert_eq!(index_store.last_seen_version().unwrap(), Some(5));

    // 1. Bad signature (signed by wrong key).
    let bad_sig = rogue_signer
        .sign(sample_index(6, now, expires, Vec::new(), Vec::new()))
        .unwrap();
    let err = index_store.apply(bad_sig, &trust, now).unwrap_err();
    let call_err = match err {
        IndexError::Refused(e) => *e,
        other => panic!("expected Refused, got {other}"),
    };
    assert_eq!(call_err.code(), "plugin_index_stale");
    assert_eq!(
        call_err.details().unwrap()["reason"],
        json!("bad_signature")
    );
    assert_eq!(index_store.last_seen_version().unwrap(), Some(5));

    // 2. Untrusted signer (valid signature by key, wrong workflow claim).
    let wrong_workflow = signer
        .sign_with_claims(
            sample_index(6, now, expires, Vec::new(), Vec::new()),
            TEST_KEY_ID,
            TEST_ISSUER,
            "https://github.com/attacker/fork/.github/workflows/plugin-index.yml@refs/heads/main",
        )
        .unwrap();
    let err = index_store.apply(wrong_workflow, &trust, now).unwrap_err();
    let call_err = err.into_call_error();
    assert_eq!(call_err.code(), "plugin_index_stale");
    assert_eq!(
        call_err.details().unwrap()["reason"],
        json!("untrusted_signer")
    );
    assert_eq!(index_store.last_seen_version().unwrap(), Some(5));

    // 3. Stale version (v4 < last_seen v5).
    let stale = signer
        .sign(sample_index(4, now, expires, Vec::new(), Vec::new()))
        .unwrap();
    let err = index_store.apply(stale, &trust, now).unwrap_err();
    let call_err = err.into_call_error();
    assert_eq!(call_err.code(), "plugin_index_stale");
    assert_eq!(call_err.details().unwrap()["reason"], json!("stale"));
    assert_eq!(call_err.details().unwrap()["version"], json!(4));
    assert_eq!(call_err.details().unwrap()["last_seen"], json!(5));
    assert_eq!(index_store.last_seen_version().unwrap(), Some(5));

    // 4. Expired index (expires_at < now).
    let past_expiry_bundle = signer
        .sign(sample_index(
            6,
            ts(1_750_000_000),
            ts(1_759_000_000),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
    let err = index_store
        .apply(past_expiry_bundle, &trust, now)
        .unwrap_err();
    let call_err = err.into_call_error();
    assert_eq!(call_err.code(), "plugin_index_stale");
    assert_eq!(call_err.details().unwrap()["reason"], json!("expired"));
    assert_eq!(index_store.last_seen_version().unwrap(), Some(5));
}

/// An expired cached index blocks third-party Plugin loading with
/// `plugin_index_stale` even when Operator policy allows the Plugin, while
/// a first-party Plugin still passes index and policy admission.
///
/// Covers: story:7-8
#[test]
fn expired_index_blocks_third_party_loading_while_first_party_still_loads() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_extractor(&home, "official-ext", DIGEST_FIRST_PARTY);
    install_extractor(&home, "community-ext", DIGEST_THIRD_PARTY);

    let (signer, trust) = test_signer();
    let signed_at = ts(1_760_000_000);
    let expires_at = ts(1_760_000_000 + 14 * 86_400);
    let index = sample_index(
        1,
        signed_at,
        expires_at,
        vec![
            index_entry("official-ext", DIGEST_FIRST_PARTY, Origin::FirstParty),
            index_entry("community-ext", DIGEST_THIRD_PARTY, Origin::ThirdParty),
        ],
        Vec::new(),
    );
    let index_store = PluginIndexStore::new(home.plugin_index());
    index_store
        .apply(signer.sign(index).unwrap(), &trust, signed_at)
        .unwrap();

    // Advance past expiry (15 days after signing).
    let after_expiry = ts(1_760_000_000 + 15 * 86_400);
    let settings = policy_allowing(&json!({
        "community-ext": { "allow": true, "digest": DIGEST_THIRD_PARTY }
    }));
    let probe = SandboxProbe::for_this_host(BTreeMap::new());

    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::from(after_expiry));
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let audit = Arc::new(AuditLog::of(&home, None, Arc::clone(&owner)));
        let plugins = RunPlugins::new(
            &home,
            None,
            Product::Collie,
            RunId::parse("expired-index-test").unwrap(),
            (Arc::clone(&owner), clock),
        );
        let secrets = empty_secrets(&home, audit, Arc::clone(&owner), clock, 1);
        // Third-party Plugin is blocked by expired index.
        let err = plugins
            .extractor("community-ext", Some(&settings), &secrets, &probe)
            .await
            .unwrap_err();
        let StartError::Refused(call_err) = err else {
            panic!("expected Refused for expired index, got {err:?}");
        };
        assert_eq!(call_err.code(), "plugin_index_stale");
        assert_eq!(call_err.details().unwrap()["reason"], json!("expired"));

        // First-party Plugin is NOT blocked by expired index or missing
        // Operator policy `allow`: it passes index + policy admission and
        // only reaches `places: None` setup (not `plugin_index_stale` or
        // `plugin_not_allowed`).
        let status = index_store.status(after_expiry).unwrap();
        assert!(status.is_expired());
        let origin = check_load(&status, "official-ext", Some(DIGEST_FIRST_PARTY), None).unwrap();
        assert_eq!(origin, Origin::FirstParty);
        assert!(admit(origin, None, None, Some(DIGEST_FIRST_PARTY)).is_ok());

        let first_party_result = plugins
            .extractor("official-ext", Some(&settings), &secrets, &probe)
            .await;
        if let Err(StartError::Refused(call_err)) = first_party_result {
            assert_ne!(
                call_err.code(),
                "plugin_index_stale",
                "first-party Plugin must not be blocked by expired index"
            );
            assert_ne!(
                call_err.code(),
                "plugin_not_allowed",
                "first-party Plugin must load without explicit Operator policy allow"
            );
        }
    });
}

fn spawn_hello_peer(owner: &TaskOwner, mut peer: InMemoryPeer) {
    owner.spawn(async move {
        let mut reader = BufReader::new(peer.from_engine);
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            let req: Value = serde_json::from_str(&line).unwrap();
            let reply = json!({
                "jsonrpc": "2.0",
                "id": req["id"],
                "result": {
                    "protocol": "0.1",
                    "kinds": [{ "kind": "extract", "capabilities": [] }],
                    "hook_stages": [],
                }
            });
            let mut encoded = serde_json::to_vec(&reply).unwrap();
            encoded.push(b'\n');
            let _ = peer.to_engine.write_all(&encoded).await;
            let _ = peer.to_engine.flush().await;
        }
        line.clear();
        let _ = reader.read_line(&mut line).await;
    });
}

/// A revoked digest stops a running Plugin instance at once, refuses new
/// starts with `plugin_revoked`, and records `plugin_revoked` in the Audit
/// log.
///
/// Covers: story:7-8
#[test]
fn revoked_digest_stops_running_instance_refuses_start_and_audits() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    install_extractor(&home, "revoked-ext", DIGEST_THIRD_PARTY);

    let (signer, trust) = test_signer();
    let now = ts(1_760_000_000);
    let expires = ts(1_760_000_000 + 14 * 86_400);

    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::from(now));
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let audit = Arc::new(AuditLog::of(&home, None, Arc::clone(&owner)));
        let run_id = RunId::parse("revoked-index-test").unwrap();
        let plugins = RunPlugins::new(
            &home,
            None,
            Product::Collie,
            run_id.clone(),
            (Arc::clone(&owner), clock),
        )
        .with_audit((*audit).clone());
        let resolved = policy_allowing(&json!({
            "revoked-ext": { "allow": true, "digest": DIGEST_THIRD_PARTY }
        }));
        let fetch = Arc::new(
            PluginFetch::of(&resolved, "revoked-ext", clock, owner.blocking_threads()).unwrap(),
        );
        let (instance, peer) = PluginInstance::start_in_memory(
            &owner,
            InMemoryPlugin {
                manifest: test_manifest("revoked-ext"),
                product: Product::Collie,
                fetch,
                clock,
                spend: None,
            },
            Scope::Run(run_id.clone()),
        );
        spawn_hello_peer(&owner, peer);

        plugins
            .track_instance(
                "revoked-ext".to_owned(),
                Some(DIGEST_THIRD_PARTY.to_owned()),
                None,
                instance.clone(),
            )
            .await;

        // Publish v2 revoking DIGEST_THIRD_PARTY.
        let v2 = sample_index(
            2,
            now,
            expires,
            Vec::new(),
            vec![Revocation {
                digest: Some(DIGEST_THIRD_PARTY.to_owned()),
                signer: None,
                reason: "compromised release workflow".to_owned(),
                revoked_at: now.to_string(),
            }],
        );
        let index_store = PluginIndexStore::new(home.plugin_index());
        let applied = index_store
            .apply(signer.sign(v2).unwrap(), &trust, now)
            .unwrap();

        // Enforce revocations: stops the running instance immediately.
        let stopped = plugins.enforce_revocations(&applied).await;
        assert_eq!(stopped, vec!["revoked-ext".to_owned()]);
        let ended = instance.ended().expect("revoked instance must have ended");
        assert_eq!(ended.code(), "plugin_revoked");
        assert_eq!(
            ended.details().unwrap()["digest"],
            json!(DIGEST_THIRD_PARTY)
        );

        // Subsequent start attempt is refused with `plugin_revoked`.
        let secrets = empty_secrets(&home, Arc::clone(&audit), Arc::clone(&owner), clock, 2);
        let probe = SandboxProbe::for_this_host(BTreeMap::new());
        let start_err = plugins
            .extractor("revoked-ext", Some(&resolved), &secrets, &probe)
            .await
            .unwrap_err();
        let StartError::Refused(call_err) = start_err else {
            panic!("expected Refused on revoked start, got {start_err:?}");
        };
        assert_eq!(call_err.code(), "plugin_revoked");

        // Audit log records both the running stop and the refused start.
        let entries = audit.export().unwrap();
        let revoked_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.entry.kind == PLUGIN_REVOKED_AUDIT.name)
            .collect();
        assert_eq!(revoked_entries.len(), 2);
        let first = &revoked_entries[0].entry.details;
        assert_eq!(first["plugin"], json!("revoked-ext"));
        assert_eq!(first["digest"], json!(DIGEST_THIRD_PARTY));
        assert_eq!(first["stopped_running"], json!(true));
        assert_eq!(
            revoked_entries[1].entry.details["stopped_running"],
            json!(false)
        );
    });
}

/// Fixture Clean-up rules signed set reusing `signed_set::verify` (ADR 0047).
#[derive(Clone, Debug, Serialize)]
struct CleanUpRulesSignedSet {
    version: u64,
    manifest: CleanUpRulesManifest,
}

impl SignedSetPayload for CleanUpRulesSignedSet {
    fn version(&self) -> u64 {
        self.version
    }

    fn expires_at(&self) -> Result<Timestamp, String> {
        let expiry = self
            .manifest
            .expires_at
            .ok_or_else(|| "missing expires_at".to_owned())?;
        Timestamp::try_from(expiry).map_err(|error| error.to_string())
    }
}

fn assert_signed_set_verdicts<T: SignedSetPayload + Clone>(
    payload: &T,
    signer: &SignedSetSigner,
    other_signer: &SignedSetSigner,
    trust: &TrustRoot,
    now: Timestamp,
    expires: Timestamp,
) {
    assert_eq!(
        signed_set::verify(&signer.sign(payload.clone()).unwrap(), trust, Some(2), now),
        SignedSetVerdict::Valid {
            version: 3,
            expires_at: expires,
        }
    );
    assert_eq!(
        signed_set::verify(
            &other_signer.sign(payload.clone()).unwrap(),
            trust,
            Some(2),
            now,
        ),
        SignedSetVerdict::BadSignature
    );
    assert_eq!(
        signed_set::verify(&signer.sign(payload.clone()).unwrap(), trust, Some(4), now),
        SignedSetVerdict::Stale {
            version: 3,
            last_seen: 4,
        }
    );
    let past = ts(1_760_000_000 + 15 * 86_400);
    assert_eq!(
        signed_set::verify(&signer.sign(payload.clone()).unwrap(), trust, Some(2), past),
        SignedSetVerdict::Expired {
            version: 3,
            expires_at: expires,
        }
    );
}

/// The shared signed-set verifier serves both the Plugin index and a
/// fixture Clean-up rules set alike, reporting identical verdicts for
/// valid, bad signature, untrusted signer, stale version and expired sets
/// (ADR 0047).
///
/// Covers: story:7-8
#[test]
fn signed_set_check_serves_plugin_index_and_clean_up_rules_alike() {
    let (signer, trust) = test_signer();
    let (other_signer, _) =
        SignedSetSigner::for_root(TEST_KEY_ID, TEST_ISSUER, TEST_WORKFLOW, SEED_B);
    let now = ts(1_760_000_000);
    let expires = ts(1_760_000_000 + 14 * 86_400);

    let index_v3 = sample_index(3, now, expires, Vec::new(), Vec::new());
    let rules_v3 = CleanUpRulesSignedSet {
        version: 3,
        manifest: CleanUpRulesManifest {
            format_version: goodfetch_protocol::catalog::ProtocolVersion { major: 0, minor: 1 },
            source: RulesSource::Daily,
            published_at: SystemTime::from(now),
            expires_at: Some(SystemTime::from(expires)),
            lists: RulesLists {
                ads: "easylist.txt".to_owned(),
                trackers: "easyprivacy.txt".to_owned(),
                annoyances: "annoyances.txt".to_owned(),
                unbreak: "unbreak.txt".to_owned(),
            },
        },
    };
    assert_signed_set_verdicts(&index_v3, &signer, &other_signer, &trust, now, expires);
    assert_signed_set_verdicts(&rules_v3, &signer, &other_signer, &trust, now, expires);
}

/// A delisted Plugin is removed from the index's listed entries without
/// joining the revocation list, so it remains installable by exact digest
/// and loadable where Operator policy allows it (ADR 0052).
///
/// Covers: story:7-115
#[test]
fn delisted_plugin_stays_installable_by_exact_digest_and_joins_no_revocation() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let artifact = Artifact::of(gzip(&bundle(&env.home().join("ran"))), TAR_GZIP_MEDIA_TYPE);
    let exact_digest = Digest::of(&artifact.index).to_string();

    let (signer, trust) = test_signer();
    let now = ts(1_760_000_000);
    let expires = ts(1_760_000_000 + 14 * 86_400);
    let index_store = PluginIndexStore::new(home.plugin_index());

    // v1 lists `kettle`.
    let v1 = sample_index(
        1,
        now,
        expires,
        vec![index_entry("kettle", &exact_digest, Origin::ThirdParty)],
        Vec::new(),
    );
    index_store
        .apply(signer.sign(v1).unwrap(), &trust, now)
        .unwrap();

    // v2 delists `kettle` (removes entry, adds no revocation).
    let v2 = sample_index(2, now, expires, Vec::new(), Vec::new());
    let applied_v2 = index_store
        .apply(signer.sign(v2).unwrap(), &trust, now)
        .unwrap();
    assert!(applied_v2.entry("kettle").is_none());
    assert!(
        applied_v2
            .revocation_for(Some(&exact_digest), None)
            .is_none()
    );

    // Installing by exact digest still succeeds.
    let plugin_store = PluginStore::new(store(&env));
    let lockfile = env.home().join("project").join(LOCKFILE_NAME);
    let blocking = TaskOwner::root(BlockingPolicy::Inline).blocking_threads();
    let added = goodfetch_test_support::runtime().block_on(async {
        let stand_in = StandIn::serve_with(artifact.answer(artifact.clone())).await;
        let reference = artifact.reference(&stand_in);
        let fetching = stand_in.fetching();
        plugin_add::add(&reference, &fetching, &blocking, &plugin_store, &lockfile)
            .await
            .unwrap()
    });
    assert_eq!(added.name, "kettle");
    assert_eq!(added.digest, exact_digest);

    // Where Operator policy allows it at that exact digest, index + policy admit it.
    let status = index_store.status(now).unwrap();
    let origin = check_load(&status, "kettle", Some(&exact_digest), None).unwrap();
    assert_eq!(origin, Origin::ThirdParty);
    assert!(admit(origin, Some(true), Some(&exact_digest), Some(&exact_digest)).is_ok());
}

/// Fetching the Plugin index over HTTPS via `PluginIndexStore::refresh`
/// verifies the signature and freshness against the injected trust root
/// through the engine's own HTTP client.
///
/// Covers: story:7-8
#[test]
fn plugin_index_refreshes_over_https_and_refuses_stale_replay() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let index_store = PluginIndexStore::new(home.plugin_index());
    let (signer, trust) = test_signer();
    let now = ts(1_760_000_000);
    let expires = ts(1_760_000_000 + 14 * 86_400);

    let v2_body = serde_json::to_vec(
        &signer
            .sign(sample_index(2, now, expires, Vec::new(), Vec::new()))
            .unwrap(),
    )
    .unwrap();
    let v1_body = serde_json::to_vec(
        &signer
            .sign(sample_index(1, now, expires, Vec::new(), Vec::new()))
            .unwrap(),
    )
    .unwrap();

    goodfetch_test_support::runtime().block_on(async {
        let v2_bytes = v2_body.clone();
        let v1_bytes = v1_body.clone();
        let stand_in = StandIn::serve_with(move |path, _| match path {
            "/plugin-index/v2.json" => (StatusCode::OK, v2_bytes.clone()),
            "/plugin-index/v1.json" => (StatusCode::OK, v1_bytes.clone()),
            _ => (StatusCode::NOT_FOUND, Vec::new()),
        })
        .await;
        let fetching = stand_in.fetching();
        let url_v2 = format!("https://{}/plugin-index/v2.json", stand_in.authority());
        let url_v1 = format!("https://{}/plugin-index/v1.json", stand_in.authority());

        let applied = index_store
            .refresh(&url_v2, &fetching, &trust, now)
            .await
            .unwrap();
        assert_eq!(applied.version, 2);

        let err = index_store
            .refresh(&url_v1, &fetching, &trust, now)
            .await
            .unwrap_err()
            .into_call_error();
        assert_eq!(err.code(), "plugin_index_stale");
        assert_eq!(err.details().unwrap()["reason"], json!("stale"));
    });
}
