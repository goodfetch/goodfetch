//! The `group` commands through Collie's executable (spec 213, ADR 0055):
//! the group store is the Daemon's, so an Attached engine forwards each
//! `group.*` call to the major's Daemon, starting it when none runs. A
//! Daemon joins over loopback: each test home runs a Daemon of its own.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write Operator policy files and read the Run index SQLite file from their own threads"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use data_encoding::BASE64;
use goodfetch_engine::home::Home;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::PEER_PROTOCOL_VERSION;
use goodfetch_protocol::home::GROUP_STORE;
use goodfetch_protocol::refusals::{RefusalCase, Reply};
use goodfetch_test_client::{DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    Endpoint, EndpointRunner, Listener, ListenerKind, ListenerTls, RuntimeFile, TestEnv, Toxiproxy,
    cli,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, runtime_file, short_idle, wait_until_stopped};
use crate::endpoint::audit_verifier;
use crate::engine_choice::daemon_dispatched;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");
const VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// Writes an Operator policy setting `politeness.sharing = "per_daemon"` so
/// a multi-member group test without a Valkey Politeness store is admitted.
fn allow_per_daemon_politeness(env: &TestEnv) {
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[settings.politeness]\nsharing = \"per_daemon\"\n",
    )
    .unwrap();
}

/// A test home whose Daemon idles for a minute, with no Operator policy.
fn bare_home() -> TestEnv {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    env
}

/// A test home whose Daemon idles for a minute, with Operator policy
/// permitting per-Daemon Politeness in a multi-member group.
fn home() -> TestEnv {
    let env = bare_home();
    allow_per_daemon_politeness(&env);
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

/// What `group join` answers on `member`, joined as `name` with a token
/// the Lead Daemon `lead`, which `formed` answered, mints, listening for
/// peer links at `peer_address`.
fn joined_as(
    lead: &TestEnv,
    formed: &Value,
    member: &TestEnv,
    name: &str,
    peer_address: &str,
) -> Value {
    let minted = answered(lead, &["group", "token"], "JoinTokenMinted");
    answered(
        member,
        &join_at(
            formed["peer_address"].as_str().unwrap(),
            minted["token"].as_str().unwrap(),
            minted["authority_key"].as_str().unwrap(),
            name,
            peer_address,
        ),
        "GroupJoined",
    )
}

/// What `group join` answers on `member`, joined as `worker` with a token
/// the Lead Daemon `lead`, which `formed` answered, mints, listening for
/// peer links at `peer_address`.
fn joined(lead: &TestEnv, formed: &Value, member: &TestEnv, peer_address: &str) -> Value {
    joined_as(lead, formed, member, "worker", peer_address)
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

/// Stops `env`'s Daemon and waits until its serving lock is free so a
/// restart binds the same peer address cleanly.
fn stop_daemon(env: &TestEnv) {
    let output = cli(env, COLLIE).args(["daemon", "stop"]).run().unwrap();
    assert_eq!(output.exit, Some(0), "{output}");
    wait_until_stopped(env);
}

/// Three Daemons on loopback form a group with the per-test CA from
/// `group init`; after all three stop, the Lead Daemon refuses to act until
/// it exchanges lead epochs with a majority (2 of 3) or an operator confirms
/// it with `group lead --confirm`.
///
/// Covers: command:group.lead, operation:group.lead, error:lead_unconfirmed,
/// story:3-106, story:3-108
#[test]
fn three_daemons_form_a_group_and_confirm_lead_by_majority_or_operator() {
    let (lead, one, two) = (home(), home(), home());
    let _stops = [&lead, &one, &two].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    joined_as(&lead, &formed, &one, "worker-one", "127.0.0.1:0");
    joined_as(&lead, &formed, &two, "worker-two", "127.0.0.1:0");

    let initial = answered(&lead, &["group", "lead"], "GroupLeadStatus");
    let worker_lead = refused(&one, &["group", "lead"]);
    assert_eq!(
        (
            initial["name"].as_str(),
            initial["lead_epoch"].as_u64(),
            initial["confirmed"].as_bool(),
            initial["confirmed_by_operator"].as_bool(),
            worker_lead.as_str(),
        ),
        (
            Some("lead"),
            Some(1),
            Some(true),
            Some(false),
            Some("not_lead_daemon")
        )
    );

    // Stop all three and restart only the Lead Daemon: with 1 of 3 reachable,
    // it stays unconfirmed and refuses `group token`.
    for env in [&two, &one, &lead] {
        stop_daemon(env);
    }
    let unconfirmed = answered(&lead, &["group", "lead"], "GroupLeadStatus");
    let blocked = refused(&lead, &["group", "token"]);
    assert_eq!(
        (unconfirmed["confirmed"].as_bool(), blocked.as_str()),
        (Some(false), Some("lead_unconfirmed"))
    );

    // Restarting one Group member gives a 2-of-3 majority; the Lead Daemon
    // confirms automatically and mints tokens again.
    let started = cli(&one, COLLIE).args(["daemon", "start"]).run().unwrap();
    assert_eq!(started.exit, Some(0), "{started}");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut majority = answered(&lead, &["group", "lead"], "GroupLeadStatus");
    while majority["confirmed"].as_bool() != Some(true) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        majority = answered(&lead, &["group", "lead"], "GroupLeadStatus");
    }
    assert_eq!(
        (
            majority["confirmed"].as_bool(),
            majority["confirmed_by_operator"].as_bool()
        ),
        (Some(true), Some(false))
    );
    let _minted = answered(&lead, &["group", "token"], "JoinTokenMinted");

    // Isolated again after stopping both: `group lead --confirm` confirms by
    // hand with the required warning and unblocks `group token`.
    stop_daemon(&one);
    stop_daemon(&lead);
    assert_eq!(
        refused(&lead, &["group", "token"]).as_str(),
        Some("lead_unconfirmed")
    );
    let confirmed = answered(&lead, &["group", "lead", "--confirm"], "GroupLeadStatus");
    assert_eq!(
        (
            confirmed["confirmed"].as_bool(),
            confirmed["confirmed_by_operator"].as_bool(),
            confirmed["warning"].as_str().is_some_and(|w| !w.is_empty()),
        ),
        (Some(true), Some(true), true)
    );
    let _minted = answered(&lead, &["group", "token"], "JoinTokenMinted");
}

/// A group of more than one Group member is refused without a Politeness store
/// or `politeness.sharing = "per_daemon"` in the Operator policy; user
/// configuration alone is not enough, and the refusal leaves the join token
/// valid once the Operator policy permits it.
///
/// Covers: error:group_politeness_unshared, story:3-106
#[test]
fn a_second_member_is_refused_without_shared_politeness_or_operator_policy() {
    let (lead, member) = (bare_home(), bare_home());
    let _stops = [&lead, &member].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let minted = answered(&lead, &["group", "token"], "JoinTokenMinted");
    let (address, token, key) = (
        formed["peer_address"].as_str().unwrap(),
        minted["token"].as_str().unwrap(),
        minted["authority_key"].as_str().unwrap(),
    );

    // Default settings refuse the second member without spending the token.
    let without_policy = refused(&member, &join(address, token, key, "worker"));
    assert_eq!(without_policy.as_str(), Some("group_politeness_unshared"));

    // User configuration `politeness.sharing = "per_daemon"` is not enough:
    // only the Operator policy (or `politeness.store`) allows a multi-member
    // group without shared Politeness.
    lead.user_configuration("daemon.idle_timeout = 60000\npoliteness.sharing = \"per_daemon\"\n");
    stop_daemon(&lead);
    let _lead_up = answered(&lead, &["group", "lead"], "GroupLeadStatus");
    let user_config_only = refused(&member, &join(address, token, key, "worker"));
    assert_eq!(user_config_only.as_str(), Some("group_politeness_unshared"));

    // Adding `politeness.sharing = "per_daemon"` in Operator policy lets the
    // same join token succeed.
    allow_per_daemon_politeness(&lead);
    stop_daemon(&lead);
    let _lead_up = answered(&lead, &["group", "lead"], "GroupLeadStatus");
    let admitted = answered(&member, &join(address, token, key, "worker"), "GroupJoined");
    assert_eq!(admitted["lead_epoch"].as_u64(), Some(1));
}

/// Reads `(owner_json, presence)` for `run_id` from `env`'s Run index SQLite file.
fn run_index_owner(env: &TestEnv, run_id: &str) -> Option<(Option<Value>, String)> {
    let version = ProductVersion::parse(VERSION).unwrap();
    let path = Home::new(env.home().to_path_buf()).run_index(&version);
    let connection = rusqlite::Connection::open(path).ok()?;
    connection
        .query_row(
            "SELECT owner, presence FROM runs WHERE run_id = ?1",
            [run_id],
            |row| {
                let owner: Option<String> = row.get(0)?;
                let presence: String = row.get(1)?;
                Ok((owner.and_then(|s| serde_json::from_str(&s).ok()), presence))
            },
        )
        .ok()
}

/// A Group member feeds the Lead Daemon's group Run index with rows naming
/// that member as `owner`, and `runs reindex` on the Group member re-reports
/// its runs to the Lead Daemon.
///
/// Covers: story:3-106
#[test]
fn a_group_member_feeds_the_lead_daemons_run_index_and_runs_reindex_re_reports() {
    let (lead, member) = (home(), home());
    let _stops = [&lead, &member].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let _joined = joined(&lead, &formed, &member, "127.0.0.1:0");

    // Start and finish a Run on the member's Daemon so its local Run index
    // writes a row and feeds it to the Lead Daemon.
    let runtime = RuntimeFile::read(&runtime_file(&member)).unwrap();
    let mut client = DaemonClient::connect(&runtime.endpoint);
    let hello = client.open(
        &Hello::needing(Product::Collie)
            .id("hello")
            .token(&runtime.token),
    );
    assert!(hello.get("result").is_some(), "{hello}");
    let started = client.call(1, "run.start", &json!({ "run_id": "group-run-1" }));
    assert!(started.get("result").is_some(), "{started}");
    let finished = client.call(2, "run.finish", &json!({ "run_id": "group-run-1" }));
    assert!(finished.get("result").is_some(), "{finished}");
    drop(client);

    // Wait for the live feed to deliver the row to the Lead Daemon's Run index.
    let expected_owner = Some(json!({ "group_member": "worker" }));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = run_index_owner(&lead, "group-run-1");
    while seen.as_ref().and_then(|(o, _)| o.as_ref()) != expected_owner.as_ref()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
        seen = run_index_owner(&lead, "group-run-1");
    }
    assert_eq!(seen, Some((expected_owner.clone(), "present".to_owned())));

    let listed = answered(&lead, &["run", "list"], "RunList");
    assert!(
        listed["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["run_id"] == json!("group-run-1")),
        "Lead Daemon run.list should include the member's Run: {listed}"
    );

    // Rebuilding the Lead Daemon's own index marks remote rows missing (their
    // folders live on the member). Running `runs reindex` on the member
    // re-reports them back to the Lead Daemon as present with `owner = worker`.
    let lead_reindexed = answered(&lead, &["runs", "reindex"], "RunsReindexOutput");
    assert_eq!(lead_reindexed["missing"].as_u64(), Some(1));
    assert_eq!(
        run_index_owner(&lead, "group-run-1"),
        Some((None, "missing".to_owned()))
    );

    let member_reindexed = answered(&member, &["runs", "reindex"], "RunsReindexOutput");
    assert_eq!(member_reindexed["present"].as_u64(), Some(1));
    assert_eq!(
        run_index_owner(&lead, "group-run-1"),
        Some((expected_owner, "present".to_owned()))
    );
}

/// Reads `(key_der, cert_der, ca_der, lead_server_name)` from `env`'s group store.
fn stored_peer_identity(env: &TestEnv) -> (Vec<u8>, Vec<u8>, Vec<u8>, String) {
    let version = ProductVersion::parse(VERSION).unwrap();
    let path = Home::new(env.home().to_path_buf())
        .per_major(&version)
        .join(GROUP_STORE);
    let connection = rusqlite::Connection::open(path).unwrap();
    let (key, cert, authority, lead_name): (String, String, String, String) = connection
        .query_row(
            "SELECT key, certificate, authority, lead_name FROM identity WHERE only = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    (
        BASE64.decode(key.as_bytes()).unwrap(),
        BASE64.decode(cert.as_bytes()).unwrap(),
        BASE64.decode(authority.as_bytes()).unwrap(),
        format!("{lead_name}.group.goodfetch.invalid"),
    )
}

/// Reads `(authority_name, ca_key_der)` from a Lead Daemon's group store.
fn stored_authority(env: &TestEnv) -> (String, Vec<u8>) {
    let version = ProductVersion::parse(VERSION).unwrap();
    let path = Home::new(env.home().to_path_buf())
        .per_major(&version)
        .join(GROUP_STORE);
    let connection = rusqlite::Connection::open(path).unwrap();
    let (name, key): (String, String) = connection
        .query_row(
            "SELECT name, key FROM authority WHERE only = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    (name, BASE64.decode(key.as_bytes()).unwrap())
}

/// Signs a client certificate `(cert_der, key_der)` with `issuer` over the
/// given validity window.
fn sign_client_cert(
    issuer: &rcgen::Issuer<'_, rcgen::KeyPair>,
    name: &str,
    not_before: SystemTime,
    not_after: SystemTime,
) -> (Vec<u8>, Vec<u8>) {
    let mut params =
        rcgen::CertificateParams::new(vec![format!("{name}.group.goodfetch.invalid")]).unwrap();
    params.not_before = not_before.into();
    params.not_after = not_after.into();
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = params.signed_by(&key, issuer).unwrap();
    (cert.der().to_vec(), key.serialize_der())
}

/// The peer link is a column of the endpoint table (ADR 0067): it accepts a
/// valid Group member certificate and refuses another group's CA, a revoked
/// Group member, an expired certificate, a plain connection and a non-`hello`
/// first message, recording every refusal in the Audit log.
///
/// Covers: story:3-106, story:3-108, story:4-21
#[test]
fn the_peer_link_passes_its_column_of_the_endpoint_table() {
    let (lead, valid, to_revoke) = (home(), home(), home());
    let _stops = [&lead, &valid, &to_revoke].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let address: SocketAddr = formed["peer_address"].as_str().unwrap().parse().unwrap();
    let _ = joined_as(&lead, &formed, &valid, "valid-member", "127.0.0.1:0");
    let _ = joined_as(&lead, &formed, &to_revoke, "revoked-member", "127.0.0.1:0");

    let (valid_key, valid_cert, ca_cert, lead_name) = stored_peer_identity(&valid);
    let (revoked_key, revoked_cert, _, _) = stored_peer_identity(&to_revoke);
    let _ = answered(
        &lead,
        &["group", "revoke", "--name", "revoked-member"],
        "GroupMemberRevoked",
    );

    // Expired certificate signed by the group's own authority.
    let (ca_name, ca_key_der) = stored_authority(&lead);
    let ca_key = rcgen::KeyPair::try_from(&PrivatePkcs8KeyDer::from(ca_key_der)).unwrap();
    let mut group_ca_params = rcgen::CertificateParams::default();
    group_ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, ca_name);
    group_ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    group_ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let group_issuer = rcgen::Issuer::new(group_ca_params, ca_key);
    let now = SystemTime::now();
    let (expired_cert, expired_key) = sign_client_cert(
        &group_issuer,
        "expired-member",
        now - Duration::from_hours(72),
        now - Duration::from_hours(24),
    );

    // Certificate signed by another group's authority.
    let mut other_params = rcgen::CertificateParams::default();
    other_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let other_ca_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let other_issuer = rcgen::Issuer::new(other_params, other_ca_key);
    let (unknown_cert, unknown_key) = sign_client_cert(
        &other_issuer,
        "stranger",
        now - Duration::from_hours(1),
        now + Duration::from_hours(24),
    );

    let hello = json!({
        "protocol_version": PEER_PROTOCOL_VERSION,
        "product": Product::Collie,
        "product_version": VERSION,
        "lead_epoch": 1,
    })
    .as_object()
    .unwrap()
    .clone();
    let tls = ListenerTls::new(lead_name, ca_cert).with_client_certificate(valid_cert, valid_key);
    let listener = Listener::new(
        ListenerKind::PeerLink,
        Endpoint::PeerLink { address, hello },
        "",
    )
    .over_tls(tls)
    .with_unknown_authority_certificate(unknown_cert, unknown_key)
    .with_revoked_certificate(revoked_cert, revoked_key)
    .with_expired_certificate(expired_cert, expired_key);

    let ran = EndpointRunner::new(audit_verifier(&lead, "peer_link")).run(&listener);
    assert_eq!(ran, Ok(()));
}

/// Every generated peer-protocol refusal case for Collie (ADR 0066) gets its
/// expected reply over mutual TLS against Collie's live Lead Daemon executable,
/// and a refused `hello` closes the peer link.
///
/// Covers: story:3-95, story:3-108, format:peer-protocol
#[test]
fn refused_peer_versions_are_refused_in_reply_to_hello_by_collie() {
    let (lead, member) = (home(), home());
    let _stops = [&lead, &member].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let address: SocketAddr = formed["peer_address"].as_str().unwrap().parse().unwrap();
    let _ = joined(&lead, &formed, &member, "127.0.0.1:0");

    let (key_der, cert_der, ca_der, lead_name) = stored_peer_identity(&member);
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der)).unwrap();
    let config = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![CertificateDer::from(cert_der)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
            )
            .unwrap(),
    );

    let cases_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../goodfetch-protocol/generated/refusals/peer-protocol");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&cases_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let case: RefusalCase = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let fabricated = case.hello.clone().unwrap();
        if fabricated.engine != Product::Collie {
            continue;
        }
        #[expect(
            clippy::disallowed_types,
            reason = "test opens a raw TCP link directly to the live peer listener to send fabricated refusal cases"
        )]
        let tcp = std::net::TcpStream::connect(address).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let server_name = ServerName::try_from(lead_name.clone()).unwrap();
        let mut tls = StreamOwned::new(
            ClientConnection::new(Arc::clone(&config), server_name).unwrap(),
            tcp,
        );
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hello",
            "params": {
                "protocol_version": case.found,
                "product": fabricated.needs.unwrap(),
                "product_version": VERSION,
                "lead_epoch": 1,
            },
        });
        writeln!(tls, "{request}").unwrap();
        tls.flush().unwrap();

        let mut reader = BufReader::new(tls);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        match fabricated.reply {
            Reply::Accepted { protocol_version } => {
                assert_eq!(
                    reply["result"]["protocol_version"],
                    json!(protocol_version),
                    "{path:?}: {reply:#}"
                );
            }
            Reply::Refused { error_code } => {
                assert_eq!(
                    reply["error"]["data"]["code"],
                    json!(error_code),
                    "{path:?}: {reply:#}"
                );
                let mut trailing = String::new();
                let eof = reader.read_line(&mut trailing).unwrap_or(0);
                assert_eq!(eof, 0, "refused hello must close the peer link: {trailing}");
            }
        }
        checked += 1;
    }
    assert!(checked > 0, "expected Collie peer-protocol refusal cases");
}

/// Toxiproxy partitions one Group member from its Lead Daemon in multi-process
/// tests (ADR 0060): while partitioned, `group leave` fails with
/// `group_lead_unreachable` and the member stays in the group; once healed,
/// `group leave` succeeds.
///
/// Covers: story:3-106
#[test]
fn toxiproxy_partition_blocks_a_group_member_until_healed() {
    let (lead, member) = (home(), home());
    let _stops = [&lead, &member].map(|env| StopsTheDaemon::of(env, COLLIE));
    let formed = formed(&lead);
    let lead_addr: SocketAddr = formed["peer_address"].as_str().unwrap().parse().unwrap();

    let toxiproxy = Toxiproxy::start(&lead).unwrap();
    let proxied = toxiproxy
        .proxy("peer-to-lead", lead_addr, IpAddr::V4(Ipv4Addr::LOCALHOST))
        .unwrap();
    let minted = answered(&lead, &["group", "token"], "JoinTokenMinted");
    let _joined = answered(
        &member,
        &join(
            &proxied.to_string(),
            minted["token"].as_str().unwrap(),
            minted["authority_key"].as_str().unwrap(),
            "partitioned",
        ),
        "GroupJoined",
    );

    toxiproxy.partition("peer-to-lead").unwrap();
    let while_partitioned = refused(&member, &["group", "leave"]);
    assert_eq!(while_partitioned.as_str(), Some("group_lead_unreachable"));

    toxiproxy.heal("peer-to-lead").unwrap();
    let after_heal = answered(&member, &["group", "leave"], "GroupLeft");
    assert_eq!(after_heal["name"].as_str(), Some("partitioned"));
}
