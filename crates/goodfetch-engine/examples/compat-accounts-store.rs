//! The accounts store's writer for the Compatibility corpus (ADR 0066):
//! through the engine's own accounts store, adds two sign-ins of the OS
//! user at ADR 0059's test `Clock` start, one used an hour later, each with
//! fixed values where the Daemon draws its own from the OS generator, so
//! the set is the same at every write; then writes the store, as SQL text,
//! into the folder given as the only argument, as `accounts-store.sql`.
//! `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::people::AccountsStore;
use goodfetch_protocol::people::SignInRow;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir()?;
    let store = AccountsStore::at(scratch.path().join("accounts-store.sqlite"));
    store.insert(&SignInRow {
        id: "8a6f0c1e5d2b4a7f9e3c1b0d2a4f6e8c".to_owned(),
        verifier: "3c1f6b2e8d0a4c7e9f1b3d5a7c9e1f2b4d6a8c0e2f4b6d8a0c2e4f6b8d0a2c4e".to_owned(),
        person: "os_user".to_owned(),
        created: START.to_owned(),
        last_used: START.to_owned(),
        wrapping_key: "5e7a9c1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c5e7b9d1f3a5c7e9b1d3f5a7c".to_owned(),
    })?;
    store.insert(&SignInRow {
        id: "2d4f6a8c0e1b3d5f7a9c1e3b5d7f9a1c".to_owned(),
        verifier: "9e7c5a3f1d9b7e5c3a1f9d7b5e3c1a9f7d5b3e1c9a7f5d3b1e9c7a5f3d1b9e7c".to_owned(),
        person: "os_user".to_owned(),
        created: START.to_owned(),
        last_used: "2026-01-05T10:00:00Z".to_owned(),
        wrapping_key: "1b3d5f7a9c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d".to_owned(),
    })?;
    let connection = rusqlite::Connection::open(store.path())?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("accounts-store.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    Ok(())
}
