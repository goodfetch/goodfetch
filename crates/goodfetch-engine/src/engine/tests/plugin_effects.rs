//! Effects, `page_script`, and navigation scope for a Plugin's page and
//! network calls (ADRs 0010, 0014 and 0033; issue 845):
//! - `page_script` is refused without domain approval, and allowed, audited
//!   (and journaled when logged in) with it;
//! - changing `page.*` actions, logged-in navigations, and non-`GET`/`HEAD`
//!   (or method-override) `net.fetch` toward the run's sites are refused
//!   without `effects` approval and appear in `effect_journal` and `AuditLog`
//!   with it;
//! - navigations stay within the page's registrable domain and (only when not
//!   logged in) the Plugin's declared domains, never widening the run's domain
//!   policy.

use std::path::PathBuf;
use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::approvals::{
    Approvals, PLUGIN_PERMISSION_DENIED, PLUGIN_PERMISSION_USED,
};
use goodfetch_protocol::plugin::net::{NetFetchHeader, NetFetchParams, PLUGIN_CALL_BACK_REFUSED};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use url::Url;

use super::run::call;
use super::{attached_on, engine_with};
use crate::composition::{CallContext, Contribution, Handler};
use crate::egress::{EgressClass, Policy};
use crate::plugin_gate::{EffectsGate, PluginEffectJournal, PluginFetch};
use crate::runs::RunEffectJournal;
use crate::settings::{Layers, Values, resolve};

fn manifest_with(grants: &[&str], domains: &[&str]) -> PluginManifest {
    PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "checkout-helper",
            "version": "1.0.0",
            "kinds": ["hook"],
            "hook_stages": ["before_navigation", "after_navigation", "page_ready"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
            "permissions": {
                "domains": domains,
                "grants": grants,
            },
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

fn make_gate(
    context: &CallContext,
    run_id: &RunId,
    manifest: PluginManifest,
    approved: Approvals,
    page_policy: Policy,
    journal: Arc<dyn PluginEffectJournal>,
) -> EffectsGate {
    EffectsGate::new(
        "checkout-helper".to_owned(),
        manifest,
        approved,
        page_policy,
        Some(context.engine().audit().clone()),
        run_id.clone(),
        context.engine().clock(),
        journal,
    )
}

async fn send_hello(client: &mut super::Client) {
    client
        .send(&[json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": "0.1",
                "surface": "cli",
                "needs": "collie",
                "product_version": "1.4.0",
            },
        })])
        .await;
    client.next().await.unwrap();
}

fn test_op(name: &'static str) -> &'static Operation {
    Box::leak(Box::new(Operation {
        name,
        description: "Exercises Plugin Effects, page_script and navigation scope inside a live Run.",
        class: OperationClass::Read,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params: schema_of::<Value>,
        result: schema_of::<Value>,
        events: &[],
        cancellable: false,
        input: None,
        bound: None,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    }))
}

fn journal_rows(run_folder: &std::path::Path) -> Vec<(String, String, Option<String>)> {
    let connection = rusqlite::Connection::open(run_folder.join("run.sqlite")).unwrap();
    let mut stmt = connection
        .prepare("SELECT key, state, first_command FROM effect_journal ORDER BY entry")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn run_with_op(
    op_name: &'static str,
    run_id: &'static str,
    contribution: Contribution,
) -> (TestEnv, Value, Value) {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(contribution));
    let (answer, audit) = attached_on(&engine, &env, |mut client| async move {
        send_hello(&mut client).await;
        let _ = call(&mut client, 1, "run.start", json!({ "run_id": run_id })).await;
        let answer = call(&mut client, 2, op_name, json!({ "run_id": run_id })).await;
        let _ = call(&mut client, 3, "run.finish", json!({ "run_id": run_id })).await;
        let audit = call(&mut client, 4, "audit.export", json!({})).await;
        (answer, audit)
    });
    (env, answer, audit)
}

/// `page_script` is refused without Operator policy approval for the domain;
/// with approval it is allowed, recorded in the Audit log as
/// `plugin_permission_used`, and journaled as a possible Effect when the
/// Session is logged in (ADRs 0010 and 0033).
///
/// Covers: story:7-50, story:7-49
#[test]
fn page_script_is_refused_without_approval_and_audited_and_journaled_when_logged_in() {
    let op = test_op("test.plugin_page_script");
    let contribution = Contribution::new("test-plugin-page-script").with_handler(Handler::new(
        op,
        move |params: Value, context| async move {
            let run_id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(run_id.clone()).await.unwrap();
            let resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
            let page_policy = Policy::of(&resolved).unwrap().for_plugin();
            let journal: Arc<dyn PluginEffectJournal> =
                RunEffectJournal::new(parts.state.clone(), context.engine().clock());
            let page_url = Url::parse("https://www.shop.test/checkout").unwrap();
            let manifest = manifest_with(&["page_script"], &["shop.test"]);

            let unapproved = make_gate(
                &context,
                &run_id,
                manifest.clone(),
                Approvals::default(),
                page_policy.clone(),
                Arc::clone(&journal),
            );
            let denied = unapproved
                .authorize_page_call("page.evaluate", None, false, &page_url, None)
                .await
                .err()
                .map(|error| error.code().to_owned());

            let mut approved = Approvals::default();
            approved.page_script.insert("shop.test".to_owned());
            let gate = make_gate(&context, &run_id, manifest, approved, page_policy, journal);
            let anon_entry = gate
                .authorize_page_call("page.evaluate", None, false, &page_url, None)
                .await?;
            let logged_in_entry = gate
                .authorize_page_call("page.evaluate", None, true, &page_url, None)
                .await?;
            let logged_in_journaled = logged_in_entry.is_some();
            if let Some(entry) = logged_in_entry {
                entry.done().await?;
            }
            Ok::<_, crate::CallError>(json!({
                "denied": denied,
                "anon_journaled": anon_entry.is_some(),
                "logged_in_journaled": logged_in_journaled,
                "folder": parts.identity.folder().display().to_string(),
            }))
        },
    ));

    let (_env, answer, audit) = run_with_op("test.plugin_page_script", "script", contribution);
    assert_eq!(
        (
            answer["result"]["denied"].as_str(),
            answer["result"]["anon_journaled"].as_bool(),
            answer["result"]["logged_in_journaled"].as_bool(),
        ),
        (Some(PLUGIN_PERMISSION_DENIED.name), Some(false), Some(true)),
        "{answer}"
    );
    let rows = journal_rows(&PathBuf::from(answer["result"]["folder"].as_str().unwrap()));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, "done");
    assert!(
        rows[0]
            .2
            .as_deref()
            .is_some_and(|cmd| cmd.contains("page.evaluate")),
        "{rows:?}"
    );
    let used: Vec<&Value> = audit["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == PLUGIN_PERMISSION_USED.name)
        .collect();
    assert_eq!(used.len(), 2, "{audit}");
    assert_eq!(used[0]["details"]["permission"], json!("page_script"));
    assert_eq!(used[0]["details"]["domain"], json!("shop.test"));
}

async fn check_unapproved_effects(
    gate: EffectsGate,
    resolved: &crate::settings::Resolved,
    context: &CallContext,
    page_url: &Url,
    confirm_url: &Url,
    order_url: &Url,
) -> Value {
    let click = gate
        .authorize_page_call("locator.click", None, false, page_url, None)
        .await
        .err()
        .map(|error| error.code().to_owned());
    let nav = gate
        .authorize_page_call("page.goto", None, true, page_url, Some(confirm_url))
        .await
        .err()
        .map(|error| error.code().to_owned());
    let fetch = PluginFetch::of(
        resolved,
        "checkout-helper",
        context.engine().clock(),
        context.engine().owner().blocking_threads(),
    )
    .unwrap()
    .with_effects(gate);
    let post = fetch
        .fetch(NetFetchParams {
            call_id: RequestId::Number(1),
            method: "POST".to_owned(),
            url: order_url.to_string(),
            headers: Vec::new(),
            body: None,
            secrets: Vec::new(),
        })
        .await
        .err()
        .map(|error| error.code().to_owned());
    let override_hdr = fetch
        .fetch(NetFetchParams {
            call_id: RequestId::Number(1),
            method: "GET".to_owned(),
            url: order_url.to_string(),
            headers: vec![NetFetchHeader {
                name: "X-HTTP-Method-Override".to_owned(),
                value: "DELETE".to_owned(),
            }],
            body: None,
            secrets: Vec::new(),
        })
        .await
        .err()
        .map(|error| error.code().to_owned());
    json!({
        "click": click,
        "nav": nav,
        "post": post,
        "override": override_hdr,
    })
}

/// A changing `page.*` action (`locator.click`), a logged-in navigation
/// (`page.goto`) and a non-`GET`/`HEAD` (or method-override) `net.fetch`
/// toward the run's sites are refused without `effects` for that domain and,
/// with it, appear in the Effect journal and Audit log (ADRs 0010 and 0033).
///
/// Covers: story:7-49
#[test]
fn changing_page_action_logged_in_navigation_and_non_get_fetch_need_effects_and_journal() {
    let op = test_op("test.plugin_effects");
    let contribution = Contribution::new("test-plugin-effects").with_handler(Handler::new(
        op,
        move |params: Value, context| async move {
            let run_id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(run_id.clone()).await.unwrap();
            let resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
            let page_policy = Policy::of(&resolved).unwrap().for_plugin();
            let journal: Arc<dyn PluginEffectJournal> =
                RunEffectJournal::new(parts.state.clone(), context.engine().clock());
            let manifest = manifest_with(&["effects"], &["shop.test"]);
            let page_url = Url::parse("https://www.shop.test/cart").unwrap();
            let confirm_url = Url::parse("https://checkout.shop.test/confirm").unwrap();
            let order_url = Url::parse("https://shop.test/api/order").unwrap();

            let unapproved = make_gate(
                &context,
                &run_id,
                manifest.clone(),
                Approvals::default(),
                page_policy.clone(),
                Arc::clone(&journal),
            );
            let denied = check_unapproved_effects(
                unapproved,
                &resolved,
                &context,
                &page_url,
                &confirm_url,
                &order_url,
            )
            .await;

            let mut approved = Approvals::default();
            approved.effects.insert("shop.test".to_owned());
            let gate = make_gate(&context, &run_id, manifest, approved, page_policy, journal);
            gate.authorize_page_call("locator.click", None, false, &page_url, None)
                .await?
                .unwrap()
                .done()
                .await?;
            gate.authorize_page_call("page.goto", None, true, &page_url, Some(&confirm_url))
                .await?
                .unwrap()
                .done()
                .await?;
            gate.authorize_net_fetch("POST", &order_url, true, EgressClass::PageReachable, None)
                .await?
                .unwrap()
                .done()
                .await?;

            Ok::<_, crate::CallError>(json!({
                "denied": denied,
                "folder": parts.identity.folder().display().to_string(),
            }))
        },
    ));

    let (_env, answer, audit) = run_with_op("test.plugin_effects", "effects", contribution);
    let expected = Some(PLUGIN_PERMISSION_DENIED.name);
    let denied = &answer["result"]["denied"];
    assert_eq!(
        (
            denied["click"].as_str(),
            denied["nav"].as_str(),
            denied["post"].as_str(),
            denied["override"].as_str(),
        ),
        (expected, expected, expected, expected),
        "{answer}"
    );
    let rows = journal_rows(&PathBuf::from(answer["result"]["folder"].as_str().unwrap()));
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows.iter().all(|(_, state, _)| state == "done"), "{rows:?}");
    let commands: Vec<&str> = rows
        .iter()
        .filter_map(|(_, _, cmd)| cmd.as_deref())
        .collect();
    assert!(commands[0].contains("locator.click"), "{commands:?}");
    assert!(commands[1].contains("page.goto"), "{commands:?}");
    assert!(commands[2].contains("net.fetch POST"), "{commands:?}");
    let audited: usize = audit["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry["kind"] == PLUGIN_PERMISSION_USED.name
                && entry["details"]["permission"] == "effects"
        })
        .count();
    assert_eq!(audited, 3, "{audit}");
}

/// A Plugin's navigations stay within the page's registrable domain and (only
/// while not logged in) its declared domains, and never widen the run's domain
/// policy (ADRs 0014 and 0033).
///
/// Covers: story:7-60
#[test]
fn navigation_scope_enforces_registrable_and_declared_domains_and_run_domain_policy() {
    let op = test_op("test.plugin_nav_scope");
    let contribution = Contribution::new("test-plugin-nav-scope").with_handler(Handler::new(
        op,
        move |params: Value, context| async move {
            let run_id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(run_id.clone()).await.unwrap();
            let blocked_layers = Layers {
                run: Values::read(
                    SettingsLayer::Run,
                    json!({ "egress": { "blocked_domains": ["partner.test"] } })
                        .as_object()
                        .unwrap(),
                    Product::Collie,
                )
                .unwrap(),
                ..Layers::default()
            };
            let open_policy =
                Policy::of(&resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap())
                    .unwrap()
                    .for_plugin();
            let blocked_policy =
                Policy::of(&resolve(&blocked_layers, Product::Collie, Product::Collie).unwrap())
                    .unwrap()
                    .for_plugin();
            let journal: Arc<dyn PluginEffectJournal> =
                RunEffectJournal::new(parts.state.clone(), context.engine().clock());
            let manifest = manifest_with(&["effects"], &["partner.test"]);
            let mut approved = Approvals::default();
            approved.effects.insert("shop.test".to_owned());
            approved.effects.insert("partner.test".to_owned());

            let gate = make_gate(
                &context,
                &run_id,
                manifest.clone(),
                approved.clone(),
                open_policy,
                Arc::clone(&journal),
            );
            let page_url = Url::parse("https://www.shop.test/items").unwrap();
            let same_reg = Url::parse("https://checkout.shop.test/pay").unwrap();
            let partner = Url::parse("https://cdn.partner.test/widget").unwrap();
            let outside = Url::parse("https://evil.example/steal").unwrap();

            let blocked_gate =
                make_gate(&context, &run_id, manifest, approved, blocked_policy, journal);

            Ok::<_, crate::CallError>(json!({
                "same_reg_ok": gate.authorize_page_call("page.goto", None, false, &page_url, Some(&same_reg)).await.is_ok(),
                "partner_anon_ok": gate.authorize_page_call("page.goto", None, false, &page_url, Some(&partner)).await.is_ok(),
                "outside_err": gate.authorize_page_call("page.goto", None, false, &page_url, Some(&outside)).await.err().map(|e| e.code().to_owned()),
                "partner_logged_in_err": gate.authorize_page_call("page.goto", None, true, &page_url, Some(&partner)).await.err().map(|e| e.code().to_owned()),
                "partner_policy_blocked": blocked_gate.authorize_page_call("page.goto", None, false, &page_url, Some(&partner)).await.err().map(|e| e.code().to_owned()),
            }))
        },
    ));

    let (_env, answer, _) = run_with_op("test.plugin_nav_scope", "nav", contribution);
    assert_eq!(
        (
            answer["result"]["same_reg_ok"].as_bool(),
            answer["result"]["partner_anon_ok"].as_bool(),
            answer["result"]["outside_err"].as_str(),
            answer["result"]["partner_logged_in_err"].as_str(),
            answer["result"]["partner_policy_blocked"].as_str(),
        ),
        (
            Some(true),
            Some(true),
            Some(PLUGIN_CALL_BACK_REFUSED.name),
            Some(PLUGIN_CALL_BACK_REFUSED.name),
            Some("egress_refused"),
        ),
        "{answer}"
    );
}
