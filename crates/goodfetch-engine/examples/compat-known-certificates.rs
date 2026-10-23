//! The known-certificates store's writer for the Compatibility corpus (ADR
//! 0066): through the engine's own store, at ADR 0059's test `Clock`
//! start, pins two hosts for the OS user, one by a Run, and the same host
//! as one of them for another person of a shared Daemon, and records a
//! changed key waiting for the Run-pinned host; then writes the
//! store, as SQL text, into the folder given as the only argument, as
//! `known-certificates.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::{KeyHash, KnownCertificates};
use goodfetch_protocol::run::RunId;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir()?;
    let store = KnownCertificates::of(&Home::new(scratch.path().to_owned()), None);
    let start: jiff::Timestamp = START.parse()?;
    store.pin(
        "os_user",
        "expired.certificates.test",
        &KeyHash::of_public_key(b"the expired leaf's public key"),
        Some(&RunId::parse("pinned-on-first-use")?),
        start,
    )?;
    store.pin(
        "os_user",
        "intranet.example",
        &KeyHash::of_public_key(b"the intranet's public key"),
        None,
        start,
    )?;
    store.pin(
        "grace",
        "intranet.example",
        &KeyHash::of_public_key(b"the intranet's key as grace met it"),
        None,
        start,
    )?;
    store.record_change(
        "os_user",
        "expired.certificates.test",
        &KeyHash::of_public_key(b"the expired leaf's changed key"),
        Some(&RunId::parse("stopped-by-a-change")?),
        start,
    )?;
    let connection = rusqlite::Connection::open(store.path())?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("known-certificates.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    Ok(())
}
