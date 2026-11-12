//! Collie's executable carries no Console (ADR 0137): its command line has
//! no `console` command, no `console.*` setting is one it knows, and the
//! Daemon it starts publishes its endpoint and no Console's address.

use std::collections::BTreeSet;

use goodfetch_collie::executable::{OwnCommands, command};
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{console_address_answer, runtime_file_members};
use serde_json::json;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

#[test]
fn the_command_line_has_no_console_command() {
    let parsed = command(Product::Collie, OwnCommands::COLLIE)
        .try_get_matches_from(["goodfetch-collie", "console"]);

    assert_eq!(
        parsed.map(|_| ()).map_err(|error| error.kind()),
        Err(clap::error::ErrorKind::InvalidSubcommand)
    );
}

#[test]
fn a_console_address_is_no_setting() {
    let answer = console_address_answer(COLLIE.as_ref()).unwrap();

    assert_eq!(answer, (Some(3), json!("setting_unknown")));
}

#[test]
fn the_daemons_runtime_file_names_its_endpoint_and_no_console() {
    let members = runtime_file_members(COLLIE.as_ref()).unwrap();

    assert_eq!(
        members,
        BTreeSet::from(["endpoint", "pid", "token", "version"].map(String::from))
    );
}
