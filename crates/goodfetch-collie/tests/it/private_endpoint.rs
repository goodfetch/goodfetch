//! An Attached engine's private endpoint on Collie's executable (spec
//! #213's Seam 1; ADRs 0024, 0031 and 0067): the engine tells the Client
//! of its stdio where it listens and where its token is, in the reply to
//! `hello`, and accepts a second Connection there whose `hello` carries the
//! token; the endpoint passes its column of the endpoint table, each
//! refusal found in the Audit log; the Run index names it as the owner of
//! the engine's live Runs, and
//! the Daemon forwards an abort of such a Run there.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the engine's files, logs and Run index themselves"
)]

use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    DaemonEndpoint, Endpoint, EndpointRunner, Hello, Listener, ListenerAddress, ListenerKind,
    RuntimeFile, TestEnv,
};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::daemon::{RawClient, StopsTheDaemon, json_of, short_idle};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The private endpoint an engine reported: where it listens, its token
/// file, and the report as the engine wrote it.
struct PrivateEndpoint {
    address: DaemonEndpoint,
    token_file: PathBuf,
    reported: Value,
}

/// The private endpoint the engine reports to the Client of its stdio.
fn reported(engine: &mut Attached) -> PrivateEndpoint {
    engine.send(&hello("collie", "0.1"));
    let reply = engine.next().unwrap();
    let reported = reply["result"]["private_endpoint"].clone();
    PrivateEndpoint {
        address: serde_json::from_value(reported["address"].clone())
            .unwrap_or_else(|error| panic!("no private endpoint in {reply}: {error}")),
        token_file: PathBuf::from(reported["token_file"].as_str().unwrap()),
        reported,
    }
}

fn token(private: &PrivateEndpoint) -> String {
    std::fs::read_to_string(&private.token_file).unwrap()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode(_path: &Path) -> u32 {
    0o600
}

/// The mode of the endpoint's socket; a TCP endpoint has no file.
fn socket_mode(private: &PrivateEndpoint) -> u32 {
    match &private.address {
        DaemonEndpoint::Unix(path) => mode(path),
        DaemonEndpoint::Tcp(_) => 0o600,
    }
}

/// Covers: story:3-17
#[test]
fn an_attached_engine_reports_its_private_endpoint_where_a_second_connection_with_its_token_is_accepted()
 {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    let private = reported(&mut engine);
    let mut second = RawClient::opened_at(&private.address, &token(&private), "collie");

    let info = second.call(1, "engine.info", &json!({}));

    let modes = (mode(&private.token_file), socket_mode(&private));
    drop(second);
    let _ = engine.finish();
    assert_eq!(
        (
            info["result"]["product"].clone(),
            modes,
            private.token_file.exists()
        ),
        (json!("collie"), (0o600, 0o600), false)
    );
}

/// The endpoint table's private endpoint column (ADR 0067), each refusal
/// found in the Audit log: the token of an earlier Attached engine, which
/// ended with it, is expired.
///
/// Covers: story:3-17, story:4-21, story:4-24, story:4-90
#[test]
fn the_private_endpoint_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    let mut earlier = Attached::start(&env);
    let expired = token(&reported(&mut earlier));
    let _ = earlier.finish();
    let mut engine = Attached::start(&env);
    let private = reported(&mut engine);
    let address = match &private.address {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => ListenerAddress::Unix(path.clone()),
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => unreachable!("a private endpoint listens on TCP here"),
        DaemonEndpoint::Tcp(address) => ListenerAddress::Tcp(*address),
    };
    let hello = json!({
        "protocol_version": PROTOCOL_VERSION.to_string(),
        "surface": "sdk_python",
        "needs": "collie",
        "product_version": env!("CARGO_PKG_VERSION"),
    });
    let listener = Listener::new(
        ListenerKind::PrivateEndpoint,
        Endpoint::JsonRpc {
            address,
            hello: Hello::new(hello.as_object().unwrap().clone(), "token"),
        },
        token(&private),
    )
    .with_other_listener_token("a-daemons-token")
    .with_expired_token(expired);

    let ran = EndpointRunner::new(crate::endpoint::audit_verifier(&env, "private_endpoint"))
        .run(&listener);

    let _ = engine.finish();
    assert_eq!(ran, Ok(()));
}

/// The owner the Run index names for `run_id`.
fn owner(env: &TestEnv, run_id: &str) -> Value {
    let owner: Option<String> = Connection::open(HomeLayout::of(env.home()).run_index())
        .unwrap()
        .query_row(
            "SELECT owner FROM runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap();
    owner.map_or(Value::Null, |owner| serde_json::from_str(&owner).unwrap())
}

/// Covers: story:3-17, operation:run.abort
#[test]
fn the_daemon_forwards_an_abort_of_an_attached_engines_run_to_its_private_endpoint() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let mut engine = Attached::start(&env);
    let private = reported(&mut engine);
    call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": "attached" }),
    );
    let owned = owner(&env, "attached");
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found =
        RuntimeFile::read(&RuntimeFile::path(env.home(), env!("CARGO_PKG_VERSION"))).unwrap();
    let mut client = RawClient::opened(&found, "collie");

    let aborted = client.call(2, "run.abort", &json!({ "run_id": "attached" }));

    let after = owner(&env, "attached");
    drop(client);
    let _ = engine.finish();
    assert_eq!(
        (owned, aborted["result"]["lifecycle"].clone(), after),
        (
            json!({ "attached": private.reported }),
            json!("stopped"),
            Value::Null
        ),
        "{aborted}"
    );
}
