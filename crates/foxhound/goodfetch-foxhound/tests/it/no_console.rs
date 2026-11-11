//! Foxhound's executable carries no Console (ADR 0137): its command line has
//! no `console` command, and no `console.*` setting is one it knows.

use goodfetch_collie::executable::command;
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

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
fn a_console_address_is_no_setting() {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\n");

    let output = cli(&env, FOXHOUND)
        .args(["config", "explain", "--json"])
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (output.exit, printed["error"]["code"].clone()),
        (Some(3), json!("setting_unknown")),
        "{output}"
    );
}
