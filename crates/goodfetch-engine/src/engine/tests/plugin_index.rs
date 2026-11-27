//! `plugin.search` and `plugin.index_import` over JSON-RPC (ADR 0033,
//! issue 919): offline import verified against the engine's injected trust
//! root, audited as `plugin_index_imported`, refusing stale/badly signed
//! indexes with `plugin_index_stale`, refusing non-person surfaces with
//! `person_required`, and searchable on every surface with kinds, signer
//! identity and conformance results.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::plugin::approvals::Origin;
use goodfetch_protocol::plugin::index::{
    ConformanceResult, IndexEntry, PluginIndex, Revocation, SignerIdentity,
};
use goodfetch_test_support::TestEnv;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::consent_walls::audit_entries;
use super::run::call;
use super::{attached_on, engine_with};
use crate::signed_set::{SignedSetSigner, TrustRoot};

const ISSUER: &str = "https://token.actions.githubusercontent.com";
const WORKFLOW: &str =
    "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main";
const KEY_ID: &str = "airgap-test-key-1";
const SEED_TRUSTED: [u8; 32] = [0x31; 32];
const SEED_ROGUE: [u8; 32] = [0x32; 32];
const DIGEST_KETTLE: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const DIGEST_VAULT: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn signer_and_trust() -> (SignedSetSigner, TrustRoot) {
    SignedSetSigner::for_root(KEY_ID, ISSUER, WORKFLOW, SEED_TRUSTED)
}

fn fixture_index(version: u64, signed_at: Timestamp, expires_at: Timestamp) -> PluginIndex {
    PluginIndex {
        version,
        signed_at: signed_at.to_string(),
        expires_at: expires_at.to_string(),
        plugins: vec![
            IndexEntry {
                name: "kettle".to_owned(),
                reference: "ghcr.io/vendor/goodfetch-plugin-kettle".to_owned(),
                digest: DIGEST_KETTLE.to_owned(),
                kinds: vec![PluginKindName::EXTRACT],
                signer: SignerIdentity {
                    issuer: ISSUER.to_owned(),
                    workflow: "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0"
                        .to_owned(),
                    repository: "github.com/vendor/kettle".to_owned(),
                },
                origin: Origin::ThirdParty,
                conformance: vec![ConformanceResult {
                    suite: "extract".to_owned(),
                    passed: true,
                    engine_version: "0.1.0".to_owned(),
                }],
            },
            IndexEntry {
                name: "vault-bridge".to_owned(),
                reference: "ghcr.io/corp/goodfetch-plugin-vault-bridge".to_owned(),
                digest: DIGEST_VAULT.to_owned(),
                kinds: vec![PluginKindName::SECRET],
                signer: SignerIdentity {
                    issuer: ISSUER.to_owned(),
                    workflow: "https://github.com/corp/vault-bridge/.github/workflows/release.yml@refs/tags/v2.0.0"
                        .to_owned(),
                    repository: "github.com/corp/vault-bridge".to_owned(),
                },
                origin: Origin::Private,
                conformance: vec![ConformanceResult {
                    suite: "secret".to_owned(),
                    passed: true,
                    engine_version: "0.1.0".to_owned(),
                }],
            },
        ],
        revocations: vec![Revocation {
            digest: Some(
                "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                    .to_owned(),
            ),
            signer: None,
            reason: "compromised release".to_owned(),
            revoked_at: signed_at.to_string(),
        }],
    }
}

fn import_audit_entries(env: &TestEnv) -> Vec<Value> {
    audit_entries(env)
        .into_iter()
        .filter(|entry| entry["kind"] == "plugin_index_imported")
        .collect()
}

fn hello_msg(surface: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": { "protocol_version": "0.1", "surface": surface, "needs": "collie", "product_version": "1.4.0" },
    })
}

/// `plugin.index_import` accepts a fresh fixture index verified offline under
/// the test trust root, audits it as `plugin_index_imported`, refuses stale,
/// badly signed or expired indexes with `plugin_index_stale`, and refuses a
/// non-person surface with `person_required`.
///
/// Covers: story:7-9
#[test]
fn plugin_index_import_accepts_fresh_refuses_stale_or_bad_signature_and_is_audited() {
    let env = TestEnv::new();
    let (signer, trust) = signer_and_trust();
    let (rogue, _) = SignedSetSigner::for_root(KEY_ID, ISSUER, WORKFLOW, SEED_ROGUE);
    let now: Timestamp = "2026-01-05T09:00:00Z".parse().unwrap();
    let fresh_exp = Timestamp::from_second(now.as_second() + 14 * 86_400).unwrap();
    let past_exp = Timestamp::from_second(now.as_second() - 3600).unwrap();

    let v2 = serde_json::to_value(signer.sign(fixture_index(2, now, fresh_exp)).unwrap()).unwrap();
    let v1_stale =
        serde_json::to_value(signer.sign(fixture_index(1, now, fresh_exp)).unwrap()).unwrap();
    let v3_bad_sig =
        serde_json::to_value(rogue.sign(fixture_index(3, now, fresh_exp)).unwrap()).unwrap();
    let v3_expired =
        serde_json::to_value(signer.sign(fixture_index(3, now, past_exp)).unwrap()).unwrap();

    let engine = engine_with(Product::Collie, None).with_plugin_index_trust_root(trust);

    // Non-person surface (SDK) is refused with `person_required`.
    let sdk_refused = attached_on(&engine, &env, |mut client| {
        let v2_val = v2.clone();
        async move {
            client.send(&[hello_msg("sdk_typescript")]).await;
            client.next().await.unwrap();
            call(
                &mut client,
                1,
                "plugin.index_import",
                json!({ "bundle": v2_val }),
            )
            .await
        }
    });
    assert_eq!(
        sdk_refused["error"]["data"]["code"],
        json!("person_required")
    );
    assert_eq!(import_audit_entries(&env), Vec::<Value>::new());

    // Person surface (CLI): fresh v2 succeeds; stale, bad-sig and expired are refused.
    let (imported, stale_err, bad_sig_err, expired_err) =
        attached_on(&engine, &env, |mut client| async move {
            client.send(&[hello_msg("cli")]).await;
            client.next().await.unwrap();
            let ok = call(
                &mut client,
                1,
                "plugin.index_import",
                json!({ "bundle": v2 }),
            )
            .await;
            let stale = call(
                &mut client,
                2,
                "plugin.index_import",
                json!({ "bundle": v1_stale }),
            )
            .await;
            let bad = call(
                &mut client,
                3,
                "plugin.index_import",
                json!({ "bundle": v3_bad_sig }),
            )
            .await;
            let expired = call(
                &mut client,
                4,
                "plugin.index_import",
                json!({ "bundle": v3_expired }),
            )
            .await;
            (ok, stale, bad, expired)
        });

    assert_eq!(imported["result"]["version"], json!(2));
    assert_eq!(imported["result"]["plugins"], json!(2));
    assert_eq!(imported["result"]["revocations"], json!(1));
    assert_eq!(
        stale_err["error"]["data"]["code"],
        json!("plugin_index_stale")
    );
    assert_eq!(stale_err["error"]["data"]["reason"], json!("stale"));
    assert_eq!(
        bad_sig_err["error"]["data"]["code"],
        json!("plugin_index_stale")
    );
    assert_eq!(
        bad_sig_err["error"]["data"]["reason"],
        json!("bad_signature")
    );
    assert_eq!(
        expired_err["error"]["data"]["code"],
        json!("plugin_index_stale")
    );
    assert_eq!(expired_err["error"]["data"]["reason"], json!("expired"));

    // Exactly one Audit log entry recorded (the accepted v2 import).
    let audits = import_audit_entries(&env);
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["actor"]["kind"], json!("person"));
    assert_eq!(audits[0]["details"]["version"], json!(2));
    assert_eq!(audits[0]["details"]["plugins"], json!(2));
    assert_eq!(audits[0]["details"]["revocations"], json!(1));
}

/// `plugin.search` works on every surface and returns each matching Plugin's
/// kinds, signer identity and conformance results from the imported index,
/// supporting optional `query` and `kind` filters.
///
/// Covers: story:7-1
#[test]
fn plugin_search_lists_kinds_signer_and_conformance_results_on_every_surface() {
    let env = TestEnv::new();
    let (signer, trust) = signer_and_trust();
    let now: Timestamp = "2026-01-05T09:00:00Z".parse().unwrap();
    let fresh_exp = Timestamp::from_second(now.as_second() + 14 * 86_400).unwrap();
    let bundle = signer.sign(fixture_index(5, now, fresh_exp)).unwrap();
    let engine = engine_with(Product::Collie, None).with_plugin_index_trust_root(trust);

    // Import as person first.
    let imported = attached_on(&engine, &env, |mut client| {
        let bundle_val = serde_json::to_value(&bundle).unwrap();
        async move {
            client
                .send(&[json!({
                    "jsonrpc": "2.0", "id": 0, "method": "hello",
                    "params": { "protocol_version": "0.1", "surface": "cli", "needs": "collie", "product_version": "1.4.0" },
                })])
                .await;
            client.next().await.unwrap();
            call(
                &mut client,
                1,
                "plugin.index_import",
                json!({ "bundle": bundle_val }),
            )
            .await
        }
    });
    assert_eq!(imported["result"]["version"], json!(5));

    // Search from a non-person SDK surface (every surface).
    let (all, by_query, by_kind) = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[json!({
                "jsonrpc": "2.0", "id": 0, "method": "hello",
                "params": { "protocol_version": "0.1", "surface": "sdk_typescript", "needs": "collie", "product_version": "1.4.0" },
            })])
            .await;
        client.next().await.unwrap();
        let all = call(&mut client, 1, "plugin.search", json!({})).await;
        let by_query = call(
            &mut client,
            2,
            "plugin.search",
            json!({ "query": "kettle" }),
        )
        .await;
        let by_kind = call(&mut client, 3, "plugin.search", json!({ "kind": "secret" })).await;
        (all, by_query, by_kind)
    });

    assert_eq!(all["result"]["index_version"], json!(5));
    let plugins = all["result"]["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0]["name"], json!("kettle"));
    assert_eq!(plugins[0]["kinds"], json!(["extract"]));
    assert_eq!(
        plugins[0]["signer"]["repository"],
        json!("github.com/vendor/kettle")
    );
    assert_eq!(plugins[0]["conformance"][0]["suite"], json!("extract"));
    assert_eq!(plugins[0]["conformance"][0]["passed"], json!(true));
    assert_eq!(plugins[1]["name"], json!("vault-bridge"));
    assert_eq!(plugins[1]["kinds"], json!(["secret"]));

    assert_eq!(by_query["result"]["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(by_query["result"]["plugins"][0]["name"], json!("kettle"));

    assert_eq!(by_kind["result"]["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(
        by_kind["result"]["plugins"][0]["name"],
        json!("vault-bridge")
    );
}
