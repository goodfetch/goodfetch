//! The Accounts store's writer for the Compatibility corpus (ADR 0066):
//! through the engine's own store, at ADR 0059's test `Clock` start, makes
//! the person's Account, adds a passkey, two recovery codes, one of them
//! used an hour later, and a Named token; then writes the store, as SQL
//! text, into the folder given as the only argument, as
//! `accounts-store.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::accounts::AccountsStore;
use goodfetch_engine::home::Home;
use goodfetch_protocol::connection::ProductVersion;
use jiff::{SignedDuration, Timestamp};
use secrecy::SecretString;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir()?;
    let home = Home::new(scratch.path().to_owned());
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION"))?;
    let store = AccountsStore::of(&home, &version)?;
    let start: Timestamp = START.parse()?;
    store.make_account("Grace", start)?;
    store.add_passkey(
        b"the security key's credential",
        b"the security key's COSE public key",
        start,
    )?;
    let used = SecretString::from("used-recovery-code");
    store.replace_recovery_codes(&[used.clone(), SecretString::from("unused-recovery-code")])?;
    store.use_recovery_code(&used, start + SignedDuration::from_hours(1))?;
    store.add_named_token("ci", &SecretString::from("the ci token's value"), start)?;
    drop(store);
    let connection = rusqlite::Connection::open(home.accounts_store(&version))?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("accounts-store.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    Ok(())
}
