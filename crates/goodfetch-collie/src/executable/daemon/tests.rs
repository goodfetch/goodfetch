//! The start of the major's pick on the machine (ADR 0098), from
//! fabricated versions: the Products' own are `0.y` previews, which never
//! raise a store's lowest minor.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::SettingsFiles;
use goodfetch_engine::{Channel, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::RUN_INDEX;

use super::Machine;

/// Raises the lowest minor the Run index of the major of `version` in
/// `home` records to `lowest_minor`, as a newer engine that a program links
/// would.
fn raise_the_run_index(home: &Home, version: &ProductVersion, lowest_minor: u16) {
    std::fs::create_dir_all(home.per_major(version)).unwrap();
    let connection = rusqlite::Connection::open(home.run_index(version)).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE format (
                name TEXT NOT NULL PRIMARY KEY,
                version TEXT NOT NULL,
                lowest_minor INTEGER NOT NULL DEFAULT 0
            ) STRICT",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO format (name, version, lowest_minor) VALUES (?1, ?2, ?3)",
            (RUN_INDEX.name, "1.1", i64::from(lowest_minor)),
        )
        .unwrap();
}

/// With no installed executable able to be the pick, since a program that
/// links a newer engine raised the Run index's lowest minor past every
/// one, the start fails with the version refusal naming the version to
/// install, which exits its Error code's row, not the plain failure of a
/// Daemon that cannot be reached.
///
/// Covers: story:3-144, command:daemon.start, error:format_version_unsupported
#[test]
fn a_start_with_no_executable_able_to_be_the_pick_fails_with_the_version_refusal() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path().to_owned());
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("2.6.0").unwrap(),
        channel: Channel::CratesIo,
    };
    raise_the_run_index(&home, &identity.product_version, 7);
    let settings =
        SettingsFiles::of(&home, &identity.product_version).machine_settings(Product::Collie);
    let environment = BTreeMap::new();
    let machine = Machine {
        identity: &identity,
        home: &home,
        environment: &environment,
        executable: Some(std::env::current_exe().unwrap()),
        clock: Clock::system(),
        settings: &settings,
    };
    machine.record();

    let failure = machine
        .start_pick(false)
        .expect_err("a Daemon started though no executable can be the pick");
    let exit = failure.report();
    let error = failure.into_error();

    assert_eq!(
        (
            error.data.code.as_str(),
            error.data.details["fix"].as_str(),
            exit,
        ),
        (
            "format_version_unsupported",
            Some("Install Goodfetch 2.7 or newer, then try again."),
            std::process::ExitCode::from(8),
        ),
        "{}",
        error.message
    );
}
