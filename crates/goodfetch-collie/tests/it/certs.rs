//! `certs list` through Collie's executable (spec #564's seam; ADRs 0042
//! and 0066): the known-certificates store of the Goodfetch home
//! `GOODFETCH_HOME` names, written through the engine's own store for two
//! people of one shared store, is read by the CLI command and by
//! `certs.list` over the protocol, which answer the same pins: the
//! caller's, and nothing of the other person's. A store of a newer major is
//! refused with the registry's `format_version_unsupported`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::{KeyHash, KnownCertificates};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The person an engine without accounts acts for: the OS user.
const CALLER: &str = "os_user";

/// A store holding a pin of the OS user's, made by a Run, and two of
/// another person's on the same shared store, one for the same host.
fn shared_store(env: &TestEnv) -> KnownCertificates {
    let store = KnownCertificates::of(&Home::new(env.home().to_path_buf()), None);
    let at = "2026-01-05T09:00:00Z".parse().unwrap();
    store
        .pin(
            CALLER,
            "expired.certificates.test",
            &KeyHash::of_public_key(b"the caller's key"),
            Some(&RunId::parse("pinned").unwrap()),
            at,
        )
        .unwrap();
    store
        .pin(
            "grace",
            "expired.certificates.test",
            &KeyHash::of_public_key(b"grace's key"),
            None,
            at,
        )
        .unwrap();
    store
        .pin(
            "grace",
            "intranet.example",
            &KeyHash::of_public_key(b"grace's intranet key"),
            None,
            at,
        )
        .unwrap();
    store
}

/// What `certs list --json` printed, and what `certs.list` answered an
/// Attached engine's Client.
fn listed_both_ways(env: &TestEnv) -> (Value, Value) {
    let printed = cli(env, COLLIE)
        .args(["certs", "list", "--json"])
        .run()
        .unwrap();
    let mut engine = Attached::start(env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let answered = call(&mut engine, 1, "certs.list", &json!({}));
    let _ = engine.finish();
    (serde_json::from_str(&printed.stdout).unwrap(), answered)
}

/// Stories 4-35 and 4-43: the caller's pins, with the SHA-256 of the key
/// each one holds, and none of the other person's on the same store.
/// Covers: story:4-35, story:4-43, command:certs.list, operation:certs.list
#[test]
fn certs_list_shows_the_callers_pins_and_nothing_of_another_persons() {
    let env = TestEnv::new();
    shared_store(&env);

    let (printed, answered) = listed_both_ways(&env);

    // SHA-256 of "the caller's key", in base64, computed apart.
    let expected = json!({
        "pins": [{
            "host": "expired.certificates.test",
            "spki_sha256": "sha256/ACwLaIK/Nlbggk81BpIt0Z2eZrh9gHln/WS7pnwGge0=",
            "pinned_at": "2026-01-05T09:00:00Z",
            "run_id": "pinned",
        }],
    });
    assert_eq!(
        (printed, answered["result"].clone()),
        (expected.clone(), expected)
    );
}

/// Covers: format:known-certificates, error:format_version_unsupported
#[test]
fn certs_list_refuses_a_store_of_a_newer_major() {
    let env = TestEnv::new();
    let store = shared_store(&env);
    Connection::open(store.path())
        .unwrap()
        .execute("UPDATE format SET version = '1.0'", [])
        .unwrap();

    let refused = cli(&env, COLLIE)
        .args(["certs", "list", "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap();
    assert_eq!(
        (
            &printed["error"]["code"],
            &printed["error"]["format"],
            &printed["error"]["found"],
        ),
        (
            &json!("format_version_unsupported"),
            &json!("known-certificates"),
            &json!("1.0")
        ),
        "{printed}"
    );
}
