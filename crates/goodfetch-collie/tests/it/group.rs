//! The `group` commands through Collie's executable (spec 213, ADR 0055):
//! the group store is the Daemon's, so an Attached engine forwards each
//! `group.*` call to the major's Daemon, starting it when none runs.

use goodfetch_test_support::{TestEnv, cli};
use serde_json::Value;

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::engine_choice::daemon_dispatched;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The one document `arguments` prints under `--json`, of `definition`.
fn answered(env: &TestEnv, arguments: &[&str], definition: &str) -> Value {
    let checked = cli(env, COLLIE)
        .args(arguments)
        .arg("--json")
        .json(definition)
        .unwrap();
    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    checked.documents[0].clone()
}

/// `group init` forms the group in the Daemon it starts, and `group token`
/// mints a join token there for that group's authority.
///
/// Covers: command:group.init, command:group.token
#[test]
fn group_init_and_token_run_on_the_daemon_for_one_authority() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let formed = answered(&env, &["group", "init"], "GroupFormed");
    let minted = answered(&env, &["group", "token"], "JoinTokenMinted");

    assert_eq!(
        (
            formed["lead_epoch"].as_u64(),
            &minted["authority_key"],
            ["group.init", "group.token"].map(|method| daemon_dispatched(&env, method)),
        ),
        (Some(1), &formed["authority_key"], [true; 2]),
    );
}
