//! Collie's executable carries no Console (ADR 0137): the composition its
//! `main` passes holds no Console build, its command line has no `console`
//! command, and the Daemon it starts serves none, whatever its settings.

use goodfetch_collie::executable::{OwnCommands, command};
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{ConsoleState, RuntimeFile, TestEnv};

use crate::daemon::{StopsTheDaemon, json_of, runtime_file};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

#[test]
fn the_executables_composition_carries_no_console() {
    let composition = goodfetch_collie::composition();

    assert!(composition.console().is_none());
}

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
fn a_daemon_given_a_console_address_serves_no_console() {
    let env = TestEnv::new();
    env.user_configuration("daemon.idle_timeout = 60000\nconsole.address = \"127.0.0.1:0\"\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);

    let served = RuntimeFile::read(&runtime_file(&env)).unwrap().console;
    assert_eq!(served, ConsoleState::Absent);
}
