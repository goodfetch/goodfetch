//! Certificates whose key changed in "Needs you", through the engine's
//! public interface over the in-memory network (spec #592's Seam 3; ADR
//! 0042), on the paused `Clock`: each pinned host that presented another
//! key, as the known-certificates store holds the change waiting for its
//! person (here the test's `test.certificate_changed` call stands for the
//! TLS exceptions' stop), is listed with both keys until that person
//! answers with `certs.accept` or `certs.forget`, each answer recorded in
//! the Audit log.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::consent_walls::{audit_entries, configure, opened, unrecordable};
use super::disk_view::DiskView;
use super::needs_you::{Seen, listed};
use super::{attached_on, engine_with};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Engine;
use crate::engine_context::CALLER;
use crate::known_certificates::KeyHash;

const MB: u64 = 1_000_000;

/// The pinned host.
const HOST: &str = "self-signed.test";

/// Pins a host's first key for the caller and records a second one it
/// presented in a Run, as the TLS exceptions do when they stop a host with
/// `certificate_changed`.
const CERTIFICATE_CHANGED: Operation = Operation {
    name: "test.certificate_changed",
    description: "Pins a host and records the changed key it presented.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// The first key's hash, and the changed one's.
fn keys() -> (KeyHash, KeyHash) {
    (
        KeyHash::of_public_key(b"first key"),
        KeyHash::of_public_key(b"changed key"),
    )
}

fn changes() -> Contribution {
    Contribution::new("test-changed-certificates").with_handler(Handler::new(
        &CERTIFICATE_CHANGED,
        |params: Value, context: CallContext| async move {
            let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
            let engine = context.engine();
            let (first, changed) = keys();
            let now = engine.now();
            engine
                .known_certificates()
                .pin(CALLER, HOST, &first, Some(&run_id), now)
                .unwrap();
            engine
                .known_certificates()
                .record_change(CALLER, HOST, &changed, Some(&run_id), now)
                .unwrap();
            Ok::<Value, CallError>(json!({}))
        },
    ))
}

/// A Collie engine with the test's call, whose governor samples the test
/// home's filesystem, which "Needs you" reads again at each sample.
fn changing_engine(env: &TestEnv) -> Engine {
    let view = DiskView::default();
    view.mount(env.home(), 100 * MB, 50 * MB);
    engine_with(Product::Collie, Some(changes())).with_governor(view.system())
}

/// A changed key is listed with both hashes and the person whose store
/// holds the pin, answered with `certs.accept` or `certs.forget`; the
/// answer, the caller's own, takes the item off, resolved, and goes to the
/// Audit log.
///
/// Covers: story:12-28, operation:needs_you.list, event:needs_you.changed
#[rstest]
/// Covers: operation:certs.accept
#[case::accepted("certs.accept", "certificate_change_accepted")]
/// Covers: operation:certs.forget
#[case::forgotten("certs.forget", "certificate_pin_forgotten")]
fn a_changed_key_is_listed_until_its_person_answers(#[case] method: &str, #[case] audited: &str) {
    let env = TestEnv::new();
    configure(&env, "");
    let (first, changed) = keys();
    let method = method.to_owned();
    let params = if method == "certs.accept" {
        json!({ "host": HOST, "spki_sha256": changed.to_string() })
    } else {
        json!({ "host": HOST })
    };

    let (items, removed) = attached_on(&changing_engine(&env), &env, |mut client| async move {
        opened(&mut client, "console").await;
        let mut seen = Seen::default();
        seen.call(&mut client, 2, "run.start", json!({ "run_id": "pinning" }))
            .await;
        seen.call(
            &mut client,
            3,
            "test.certificate_changed",
            json!({ "run_id": "pinning" }),
        )
        .await;
        seen.until(&mut client, |items| items.len() == 1).await;
        let listed = listed(&seen.call(&mut client, 4, "needs_you.list", json!({})).await);
        seen.call(&mut client, 5, &method, params).await;
        seen.until(&mut client, std::collections::BTreeMap::is_empty)
            .await;
        let removed: Vec<Value> = seen
            .changes
            .iter()
            .filter(|change| change["change"] == "removed")
            .map(|change| json!([change["id"], change["reason"]]))
            .collect();
        (listed, removed)
    });

    let shown: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "id": item["id"],
                "kind": item["kind"],
                "run": item["run"],
                "details": item["details"],
                "actions": item["actions"],
            })
        })
        .collect();
    let entries = audit_entries(&env);
    assert_eq!(
        (
            shown,
            removed,
            entries
                .iter()
                .map(|entry| json!([entry["kind"], entry["actor"], entry["details"]["host"]]))
                .collect::<Vec<_>>(),
        ),
        (
            vec![json!({
                "id": "certificate_changed:self-signed.test",
                "kind": "certificate_changed",
                "run": "pinning",
                "details": {
                    "host": HOST,
                    "person": CALLER,
                    "pinned_spki_sha256": first.to_string(),
                    "presented_spki_sha256": changed.to_string(),
                },
                "actions": ["certs.accept", "certs.forget"],
            })],
            vec![json!(["certificate_changed:self-signed.test", "resolved"])],
            vec![json!([
                audited,
                { "kind": "person", "name": CALLER },
                HOST
            ])],
        )
    );
}

/// An answer to a changed key the Audit log cannot record is made all the
/// same, since the person gave it, and answers `internal_error` saying the
/// entry is missing rather than succeeding unrecorded.
///
/// Covers: story:12-28
#[rstest]
/// Covers: operation:certs.accept
#[case::accepted("certs.accept")]
/// Covers: operation:certs.forget
#[case::forgotten("certs.forget")]
fn an_answer_the_audit_log_cannot_record_is_made_and_says_so(#[case] method: &str) {
    let env = TestEnv::new();
    configure(&env, "");
    let (_, changed) = keys();
    let method = method.to_owned();
    let params = if method == "certs.accept" {
        json!({ "host": HOST, "spki_sha256": changed.to_string() })
    } else {
        json!({ "host": HOST })
    };
    let home = env.home().to_path_buf();

    let (answer, items) = attached_on(&changing_engine(&env), &env, |mut client| async move {
        opened(&mut client, "console").await;
        let mut seen = Seen::default();
        seen.call(&mut client, 2, "run.start", json!({ "run_id": "pinning" }))
            .await;
        seen.call(
            &mut client,
            3,
            "test.certificate_changed",
            json!({ "run_id": "pinning" }),
        )
        .await;
        seen.until(&mut client, |items| items.len() == 1).await;
        unrecordable(&home);
        let answer = seen.call(&mut client, 5, &method, params).await;
        seen.until(&mut client, std::collections::BTreeMap::is_empty)
            .await;
        (answer, seen.items.len())
    });

    assert_eq!(
        (&answer["error"]["code"], items),
        (&json!(-32603), 0),
        "{answer}"
    );
}
