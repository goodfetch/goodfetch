//! The Audit log's writer for the Compatibility corpus (ADR 0066): through
//! the engine's own Audit log, at ADR 0059's test `Clock` start, records an
//! Operator policy that switched a security control off, a Secret
//! reference a CLI run used, and a person's acceptance of a changed
//! certificate, then a Run whose `security.audit_chain` was off: its
//! switch, chained, and its key reference's use, unchained; then writes the
//! store, as SQL text, and its head file into
//! the folder given as the only argument, as `audit-log.sql` and
//! `audit-log-head.json`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::audit::{AuditLog, NewEntry, Origin};
use goodfetch_engine::home::Home;
use goodfetch_protocol::audit::{
    Actor, CERTIFICATE_CHANGE_ACCEPTED, SECRET_REFERENCE_USED, SECURITY_SWITCH_TURNED_OFF,
};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::json;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir()?;
    let log = AuditLog::of(&Home::new(scratch.path().to_owned()), None);
    let start: jiff::Timestamp = START.parse()?;
    log.record(
        &Origin {
            actor: Actor::Engine,
            surface: None,
            layer: Some(SettingsLayer::OperatorPolicy),
            run_id: Some(RunId::parse("weakened")?),
        },
        &SECURITY_SWITCH_TURNED_OFF,
        json!({ "setting": "security.browser_sandbox" }),
        start,
    )?;
    log.record(
        &Origin {
            actor: Actor::ClientSurface {
                surface: Surface::Cli,
            },
            surface: None,
            layer: Some(SettingsLayer::Run),
            run_id: Some(RunId::parse("logged-in")?),
        },
        &SECRET_REFERENCE_USED,
        json!({
            "placeholder": "%password%",
            "reference": "env:SITE_PASSWORD",
            "domain": "example.com",
        }),
        start,
    )?;
    log.record(
        &Origin {
            actor: Actor::Person {
                name: "os_user".to_owned(),
            },
            surface: Some(Surface::Console),
            layer: None,
            run_id: None,
        },
        &CERTIFICATE_CHANGE_ACCEPTED,
        json!({
            "host": "intranet.example.com",
            "previous": "sha256/ad0c5cd9a82e8a1ecbd6e9a7a8f7a3b4bd8f4a3c1c8c20c3b0d0bbc6bc6a4f17",
            "accepted": "sha256/3f2a6c8a3e1b7f9d5c4e2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a3b2c1d",
        }),
        start,
    )?;
    let unchained = RunId::parse("unchained")?;
    let switched = Origin {
        actor: Actor::Engine,
        surface: None,
        layer: Some(SettingsLayer::Run),
        run_id: Some(unchained.clone()),
    };
    log.unchain(unchained);
    log.record_together(
        vec![NewEntry {
            origin: switched.clone(),
            kind: &SECURITY_SWITCH_TURNED_OFF,
            details: json!({ "setting": "security.audit_chain", "value": false }),
            always_chained: true,
        }],
        start,
    )?;
    log.record(
        &switched,
        &SECRET_REFERENCE_USED,
        json!({ "placeholder": "run_state.key_reference", "reference": "env:RUN_KEY", "domain": null }),
        start,
    )?;
    let connection = rusqlite::Connection::open(log.path())?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("audit-log.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    std::fs::copy(log.head_path(), output.join("audit-log-head.json"))?;
    Ok(())
}
