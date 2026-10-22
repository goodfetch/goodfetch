//! `audit verify` through Collie's executable (spec #564's seam; ADRs 0016,
//! 0066 and 0067): the user's Audit log, in the Goodfetch home
//! `GOODFETCH_HOME` names, written through the engine's `record`, is
//! verified by the CLI command and by `audit.verify` over the protocol,
//! which answer the same value: intact, or the first sequence that does not
//! verify. A log of a newer major is refused with the registry's
//! `format_version_unsupported`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::audit::{AuditLog, Origin};
use goodfetch_engine::home::Home;
use goodfetch_protocol::audit::{Actor, SECRET_REFERENCE_USED};
use goodfetch_protocol::connection::Surface;
use goodfetch_test_support::{TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Writes `count` entries to the Audit log of `env`'s home through the
/// engine's own `record`.
fn written(env: &TestEnv, count: u64) -> AuditLog {
    let log = AuditLog::of(&Home::new(env.home().to_path_buf()), None);
    for index in 1..=count {
        log.record(
            &Origin {
                actor: Actor::ClientSurface {
                    surface: Surface::Cli,
                },
                surface: None,
                layer: None,
                run_id: None,
            },
            &SECRET_REFERENCE_USED,
            json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
            "2026-01-05T09:00:00Z".parse().unwrap(),
        )
        .unwrap();
    }
    log
}

/// What `audit verify --json` printed, and what `audit.verify` answered
/// an Attached engine's Client.
fn verified_both_ways(env: &TestEnv) -> (Value, Value) {
    let printed = cli(env, COLLIE)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();
    let mut engine = Attached::start(env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let answered = call(&mut engine, 1, "audit.verify", &json!({}));
    let _ = engine.finish();
    (serde_json::from_str(&printed.stdout).unwrap(), answered)
}

/// Covers: story:4-69, story:4-70, command:audit.verify, operation:audit.verify
#[test]
fn audit_verify_finds_a_log_written_through_record_intact() {
    let env = TestEnv::new();
    written(&env, 3);

    let (printed, answered) = verified_both_ways(&env);

    assert_eq!(
        (printed, answered["result"].clone()),
        (
            json!({ "intact": true, "entries": 3 }),
            json!({ "intact": true, "entries": 3 })
        )
    );
}

/// Covers: story:4-70, command:audit.verify, operation:audit.verify
#[test]
fn audit_verify_names_the_first_entry_an_edit_broke() {
    let env = TestEnv::new();
    let log = written(&env, 3);
    let connection = Connection::open(log.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER entries_never_updated;
             UPDATE entries SET entry = replace(entry, 'SECRET_2', 'SECRET_X') WHERE sequence = 2;",
        )
        .unwrap();

    let (printed, answered) = verified_both_ways(&env);

    assert_eq!(
        (printed, answered["result"].clone()),
        (
            json!({ "intact": false, "entries": 3, "first_broken": 2 }),
            json!({ "intact": false, "entries": 3, "first_broken": 2 })
        )
    );
}

/// Covers: format:audit-log, error:format_version_unsupported
#[test]
fn audit_verify_refuses_a_log_of_a_newer_major() {
    let env = TestEnv::new();
    let log = written(&env, 1);
    Connection::open(log.path())
        .unwrap()
        .execute("UPDATE format SET version = '1.0'", [])
        .unwrap();

    let refused = cli(&env, COLLIE)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap();
    assert_eq!(
        (
            &printed["error"]["code"],
            &printed["error"]["format"],
            &printed["error"]["found"]
        ),
        (
            &json!("format_version_unsupported"),
            &json!("audit-log"),
            &json!("1.0")
        )
    );
}
