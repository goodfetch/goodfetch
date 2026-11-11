//! Foxhound's executable carries no Console (ADR 0137): the composition its
//! `main` passes holds no Console build, its command line has no `console`
//! command, and the Daemon it starts serves none, whatever its settings.

use goodfetch_collie::executable::command;
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{ConsoleState, RuntimeFile, TestEnv, cli};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn the_executables_composition_carries_no_console() {
    let composition = goodfetch_foxhound::composition();

    assert!(composition.console().is_none());
}

#[test]
fn the_command_line_has_no_console_command() {
    let parsed = command(Product::Foxhound, goodfetch_foxhound::executable::COMMANDS)
        .try_get_matches_from(["goodfetch-foxhound", "console"]);

    assert_eq!(
        parsed.map(|_| ()).map_err(|error| error.kind()),
        Err(clap::error::ErrorKind::InvalidSubcommand)
    );
}

#[test]
fn a_daemon_given_a_console_address_serves_no_console() {
    let env = TestEnv::new();
    env.user_configuration("daemon.idle_timeout = 60000\nconsole.address = \"127.0.0.1:0\"\n");

    let started = cli(&env, FOXHOUND)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();

    let served =
        RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).map(|file| file.console);
    let _ = cli(&env, FOXHOUND).args(["daemon", "stop"]).run();
    assert_eq!(
        (started.exit, served.ok()),
        (Some(0), Some(ConsoleState::Absent))
    );
}
