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
    join_at(lead, token, key, name, "127.0.0.1:0")
}

/// The arguments of `group join` of [`join`], listening for peer links at
/// `peer_address`.
fn join_at<'a>(
    lead: &'a str,
    token: &'a str,
    key: &'a str,
    name: &'a str,
    peer_address: &'a str,
) -> [&'a str; 12] {
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
        peer_address,
    ]
}

/// What `group join` answers on `member`, joined as `worker` with a token
/// the Lead Daemon `lead`, which `formed` answered, mints, listening for
/// peer links at `peer_address`.
fn joined(lead: &TestEnv, formed: &Value, member: &TestEnv, peer_address: &str) -> Value {
    let minted = answered(lead, &["group", "token"], "JoinTokenMinted");
    answered(
        member,
        &join_at(
            formed["peer_address"].as_str().unwrap(),
            minted["token"].as_str().unwrap(),
            minted["authority_key"].as_str().unwrap(),
            "worker",
            peer_address,
        ),
        "GroupJoined",
    )
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
/// Covers: command:group.join, operation:group.join, story:3-95, story:3-106,
/// error:authority_key_mismatch, error:group_lead_unreachable,
/// error:group_member_exists, error:join_token_refused
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

/// A Group member leaves its group, which frees its name and its peer
/// listener's address for a join again; the Lead Daemon cannot leave, and
/// a Daemon in no group has none to leave.
///
/// Covers: command:group.leave, operation:group.leave, error:lead_cannot_leave,
/// error:not_in_a_group
#[test]
fn a_group_member_leaves_and_may_join_again() {
    let (lead, member) = (home(), home());
    let _stops = [&lead, &member].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let first = joined(&lead, &formed, &member, "127.0.0.1:0");
    let address = first["peer_address"].as_str().unwrap();

    let lead_leaves = refused(&lead, &["group", "leave"]);
    let left = answered(&member, &["group", "leave"], "GroupLeft");
    let again = refused(&member, &["group", "leave"]);
    let rejoined = joined(&lead, &formed, &member, address);

    assert_eq!(
        (
            lead_leaves.as_str(),
            left["name"].as_str(),
            again.as_str(),
            rejoined["peer_address"].as_str(),
        ),
        (
            Some("lead_cannot_leave"),
            Some("worker"),
            Some("not_in_a_group"),
            Some(address),
        )
    );
    assert!(daemon_dispatched(&member, "group.leave"));
}

/// The Lead Daemon revokes a Group member once, after which the member
/// leaves on its side alone, the Lead Daemon refusing its certificate; a
/// Group member revokes no one, and the Lead Daemon is not revoked.
///
/// Covers: command:group.revoke, operation:group.revoke, error:group_member_unknown,
/// error:not_lead_daemon, error:lead_cannot_leave
#[test]
fn the_lead_daemon_revokes_a_group_member_once() {
    let (lead, member) = (home(), home());
    let _stops = [&lead, &member].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    joined(&lead, &formed, &member, "127.0.0.1:0");
    let revoke = |name| ["group", "revoke", "--name", name];

    let refusals = [
        refused(&member, &revoke("worker")),
        refused(&lead, &revoke("lead")),
    ];
    let revoked = answered(&lead, &revoke("worker"), "GroupMemberRevoked");
    let twice = refused(&lead, &revoke("worker"));
    let left = answered(&member, &["group", "leave"], "GroupLeft");

    assert_eq!(
        refusals.map(|code| code.as_str().unwrap().to_owned()),
        ["not_lead_daemon", "lead_cannot_leave"]
    );
    assert_eq!(
        (
            revoked["name"].as_str(),
            twice.as_str(),
            left["name"].as_str()
        ),
        (Some("worker"), Some("group_member_unknown"), Some("worker"))
    );
}
