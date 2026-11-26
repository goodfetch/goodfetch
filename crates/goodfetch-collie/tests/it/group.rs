//! The `group` commands through Collie's executable (spec 213, ADR 0055):
//! the group store is the Daemon's, so an Attached engine forwards each
//! `group.*` call to the major's Daemon, starting it when none runs. A
//! Daemon joins over loopback: each test home runs a Daemon of its own.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{TestEnv, cli};
use serde_json::Value;

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::engine_choice::daemon_dispatched;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// An authority key no group has: the SHA-256 form, of 32 zero bytes.
const NO_GROUPS_KEY: &str = "sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

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

/// The error code `arguments` fails with under `--json`.
fn refused(env: &TestEnv, arguments: &[&str]) -> Value {
    let checked = cli(env, COLLIE)
        .args(arguments)
        .arg("--json")
        .json("CommandFailed")
        .unwrap();
    assert_ne!(checked.output.exit, Some(0), "{}", checked.output);
    checked.documents[0]["error"]["code"].clone()
}

/// A test home whose Daemon idles for a minute, and is stopped at the end.
fn home() -> TestEnv {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    env
}

/// The arguments of `group join` to `lead` with `token`, pinning `key`,
/// as the Group member `name`.
fn join<'a>(lead: &'a str, token: &'a str, key: &'a str, name: &'a str) -> [&'a str; 12] {
    [
        "group",
        "join",
        "--lead",
        lead,
        "--token",
        token,
        "--authority-key",
        key,
        "--name",
        name,
        "--peer-address",
        "127.0.0.1:0",
    ]
}

fn formed(env: &TestEnv) -> Value {
    answered(
        env,
        &[
            "group",
            "init",
            "--name",
            "lead",
            "--peer-address",
            "127.0.0.1:0",
        ],
        "GroupFormed",
    )
}

/// `group init` forms the group in the Daemon it starts, and `group token`
/// mints a join token there for that group's authority.
///
/// Covers: command:group.init, command:group.token
#[test]
fn group_init_and_token_run_on_the_daemon_for_one_authority() {
    let env = home();
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let formed = formed(&env);
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

/// A Daemon joins the Lead Daemon's group over the peer link with a join
/// token and the authority key: before it, a wrong key, an address where
/// nothing listens and a taken name are refused, and leave the token
/// valid; after it, the token admits no other Daemon.
///
/// Covers: command:group.join, story:3-95, error:authority_key_mismatch,
/// error:group_lead_unreachable, error:group_member_exists,
/// error:join_token_refused
#[test]
fn a_daemon_joins_the_lead_daemons_group_once_per_token() {
    let (lead, member, late) = (home(), home(), home());
    let _stops = [&lead, &member, &late].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let minted = answered(&lead, &["group", "token"], "JoinTokenMinted");
    let (address, token, key) = (
        formed["peer_address"].as_str().unwrap(),
        minted["token"].as_str().unwrap(),
        minted["authority_key"].as_str().unwrap(),
    );

    let before = [
        refused(&member, &join(address, token, NO_GROUPS_KEY, "worker")),
        refused(&member, &join("127.0.0.1:1", token, key, "worker")),
        refused(&member, &join(address, token, key, "lead")),
    ];
    let joined = answered(&member, &join(address, token, key, "worker"), "GroupJoined");
    let again = refused(&late, &join(address, token, key, "late"));

    assert_eq!(
        before.map(|code| code.as_str().unwrap().to_owned()),
        [
            "authority_key_mismatch",
            "group_lead_unreachable",
            "group_member_exists",
        ]
    );
    assert_eq!(
        (joined["lead_epoch"].as_u64(), again.as_str()),
        (Some(1), Some("join_token_refused"))
    );
    assert!(daemon_dispatched(&member, "group.join"));
}
