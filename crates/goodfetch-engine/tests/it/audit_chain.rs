//! Test support's `audit_chain` checker with the product's own chain
//! verifier (ADR 0060): entries the engine records and exports, logged as
//! the Observation log's Audit records, judged by `verify_exported`.

use std::sync::Arc;

use goodfetch_engine::audit::{AuditLog, Origin, verify_exported};
use goodfetch_engine::home::Home;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::audit::{Actor, ExportedAuditEntry, SECRET_REFERENCE_USED};
use goodfetch_protocol::connection::Surface;
use goodfetch_test_support::{
    ObservationLog, Operation, Record, TestEnv, Violation, ViolationKind, audit_chain,
};
use serde_json::json;

/// The hash the first entry chains to: 32 zero bytes.
const FIRST_PREVIOUS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Three entries the engine records, as `audit export` writes them.
fn exported() -> Vec<ExportedAuditEntry> {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let origin = Origin {
        actor: Actor::ClientSurface {
            surface: Surface::Cli,
        },
        surface: None,
        layer: None,
        run_id: None,
    };
    let (runtime, owner) = (
        goodfetch_test_support::runtime(),
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    );
    let log = AuditLog::of(&home, None, Arc::clone(&owner));
    for index in 1..=3 {
        runtime
            .block_on(log.record(
                &origin,
                &SECRET_REFERENCE_USED,
                json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
                "2026-01-05T09:00:00Z".parse().unwrap(),
            ))
            .unwrap();
    }
    let exported = log.export().unwrap();
    drop(log);
    runtime.block_on(owner.wait());
    exported
}

/// The Lead Daemon's Audit records of `entries`, each chained to the one
/// before it.
fn records(entries: &[ExportedAuditEntry]) -> Vec<Record<()>> {
    let mut previous = FIRST_PREVIOUS.to_owned();
    entries
        .iter()
        .map(|exported| {
            let record = Record::new(
                "lead",
                Operation::Audit {
                    hash: exported.hash.clone(),
                    previous: previous.clone(),
                    entry: serde_json::to_string(exported).unwrap(),
                    merges: None,
                },
            );
            previous.clone_from(&exported.hash);
            record
        })
        .collect()
}

/// The product's verifier over one chain of exported lines.
fn verify(entries: &[&str]) -> bool {
    verify_exported(entries.join("\n").as_bytes()).intact
}

fn log(records: &[Record<()>]) -> ObservationLog<()> {
    let mut log = ObservationLog::new();
    for record in records {
        log.push(record.clone());
    }
    log
}

#[test]
fn audit_chain_passes_the_chain_the_engine_exports() {
    let records = records(&exported());

    let verdict = audit_chain(&log(&records), verify);

    assert_eq!(verdict, Ok(()));
}

#[test]
fn audit_chain_fails_on_an_exported_entry_whose_details_were_edited() {
    let mut entries = exported();
    entries[1].entry.details = json!({ "reference": "env:OTHER", "domain": "example.com" });
    let records = records(&entries);

    let verdict = audit_chain(&log(&records), verify);

    assert_eq!(
        verdict,
        Err(Violation {
            kind: ViolationKind::AuditChainBroken,
            records: records[..2].to_vec(),
        })
    );
}
