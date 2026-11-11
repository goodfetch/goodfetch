//! The endpoint rule (ADRs 0017, 0049, 0050 and 0067; spec #564's
//! `endpoint`) through Collie's executable: the reader of the Audit log that
//! every listener's endpoint-table test checks its refusals with, each
//! listener's loopback default, the regression tests of the advisories whose
//! bypasses came from trusting loopback, and the Secret canary that finds an
//! endpoint token in no output.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's files and wait for its Audit log from their own threads"
)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Instant;

use goodfetch_engine::home::Home;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    AuditUnreadable, Canaries, Canary, DaemonEndpoint, Endpoint, EndpointRunner, Listener,
    ListenerKind, MESSAGE_DEADLINE, Output, Rule, RuntimeFile, SecretKind, TestEnv,
};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of, runtime_file, short_idle, socket_listener};
use crate::daemon_run::{ANY_PORT, Foreground, token};
use crate::ops::{Browsers, with_ops};
use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// MCP Inspector's advisory, whose rows the endpoint table cites.
const CVE_49596: &str = "CVE-2025-49596";

/// The reason the Daemon's socket, an Attached engine's private endpoint
/// and the ops listener record `rule`'s refusal with: a listener keeps no
/// token it no longer takes, so an expired, revoked, out-of-scope or other
/// listener's token is a wrong one to it, and a token it cannot read where
/// it takes one is a missing one.
pub(crate) fn reason(rule: Rule) -> Option<&'static str> {
    Some(match rule {
        Rule::TokenMissing | Rule::TokenInQueryString | Rule::OriginMissingWithoutHeaderToken => {
            "token_missing"
        }
        Rule::TokenWrong
        | Rule::TokenFromAnotherListener
        | Rule::TokenExpired
        | Rule::TokenRevoked
        | Rule::TokenScopedElsewhere => "token_wrong",
        Rule::HostForeign => "host_foreign",
        Rule::OriginForeign => "origin_foreign",
        Rule::HelloNotFirst => "hello_not_first",
        Rule::OriginMissing
        | Rule::ProbesAnswerStatusOnly
        | Rule::CsrfTokenMissing
        | Rule::SignInRevoked
        | Rule::TokenForAnotherFile
        | Rule::SignInRateExceeded => return None,
    })
}

/// Every Audit log entry of `env`'s home, as JSON.
fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let home = Home::new(env.home().to_path_buf());
    let Ok(connection) = rusqlite::Connection::open(home.audit_log()) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare("SELECT entry FROM entries ORDER BY sequence")
    else {
        return Vec::new();
    };
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| serde_json::from_str(&entry.unwrap()).unwrap())
        .collect()
}

/// How many of the Audit log's entries of `env`'s home record `listener`'s
/// refusal for `reason`.
fn refusals(env: &TestEnv, listener: &str, reason: &str) -> usize {
    audit_entries(env)
        .iter()
        .filter(|entry: &&Value| {
            entry["kind"] == json!("endpoint_authentication_failed")
                && entry["details"] == json!({ "listener": listener, "reason": reason })
        })
        .count()
}

/// The verifier of `listener`'s endpoint-table run: each refused row holds
/// one more `endpoint_authentication_failed` entry of the Audit log of
/// `env`'s home, `{listener, reason}`, than before it, waited for until
/// `MESSAGE_DEADLINE`.
pub(crate) fn audit_verifier<'env>(
    env: &'env TestEnv,
    listener: &'static str,
) -> impl FnMut(ListenerKind, Rule) -> Result<bool, AuditUnreadable> + 'env {
    let mut seen: BTreeMap<&'static str, usize> = BTreeMap::new();
    move |_, rule| {
        let Some(reason) = reason(rule) else {
            return Err(AuditUnreadable::new(format!(
                "{rule:?} is no refusal of {listener}"
            )));
        };
        let earlier = seen.get(reason).copied().unwrap_or(0);
        let deadline = Instant::now() + MESSAGE_DEADLINE;
        loop {
            let now = refusals(env, listener, reason);
            if now > earlier || Instant::now() > deadline {
                seen.insert(reason, now);
                return Ok(now > earlier);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// Writes a new ops token of `canaries` to the ops token's file of `env`'s
/// home, as an operator provisions it before the Daemon starts (ADR 0050),
/// and returns it.
pub(crate) fn provision_ops_token(env: &TestEnv, canaries: &mut Canaries) -> String {
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(layout.daemon_file(DaemonFile::OpsToken), &token).unwrap();
    token
}

/// Whether `endpoint`, where a listener serves, is reached on loopback
/// alone: a Unix socket, or a loopback TCP address.
fn on_loopback(endpoint: &DaemonEndpoint) -> bool {
    match endpoint {
        DaemonEndpoint::Unix(_) => cfg!(unix),
        DaemonEndpoint::Tcp(address) => address.ip().is_loopback(),
    }
}

/// The value `config explain` gives `key` in `env`'s home, `null` when it
/// is unset.
fn explained(env: &TestEnv, key: &str) -> Value {
    let output = json_of(COLLIE.as_ref(), env, &["config", "explain", key, "--json"]);
    output["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["key"] == json!(key))
        .map_or(Value::Null, |setting| setting["value"].clone())
}

/// Whether the `host:port` `value` names a loopback address.
fn names_loopback(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|address| address.parse::<SocketAddr>().ok())
        .is_some_and(|address| address.ip().is_loopback())
}

/// With nothing configured, every listener binds to loopback or not at
/// all: the Daemon's socket is a Unix socket (loopback TCP on Windows), an
/// Attached engine's private endpoint too, `daemon run` and the Console
/// default to `127.0.0.1`, and no ops listener opens until `ops.address`
/// names one.
///
/// Covers: story:4-21, setting:daemon.address, setting:ops.address
#[test]
fn every_listener_binds_to_loopback_unless_configured_otherwise() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let daemon_socket = RuntimeFile::read(&runtime_file(&env)).unwrap().endpoint;
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let reply = engine.next().unwrap();
    let _ = engine.finish();
    let private_endpoint: DaemonEndpoint =
        serde_json::from_value(reply["result"]["private_endpoint"]["address"].clone()).unwrap();
    let defaults = [
        explained(&env, "daemon.address"),
        explained(&env, "console.address"),
        explained(&env, "ops.address"),
    ];
    let foreground = TestEnv::new();
    foreground.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
    let mut daemon = Foreground::start(&foreground, COLLIE, &[]);
    daemon.serving();
    daemon.terminate();
    let ops_listener_opened = daemon
        .logged
        .iter()
        .any(|line| line.contains("the ops listener serves"));

    assert_eq!(
        (
            on_loopback(&daemon_socket),
            on_loopback(&private_endpoint),
            names_loopback(&defaults[0]),
            names_loopback(&defaults[1]),
            defaults[2].clone(),
            ops_listener_opened,
        ),
        (true, true, true, true, Value::Null, false)
    );
}

/// CVE-2025-49596 (MCP Inspector's remote code execution): a browser page's
/// cross-site request reached a local proxy that trusted loopback, with no
/// token and no `Origin` check. On the ops listener every row citing it is
/// refused and audited: no token, the token in the query string, a foreign
/// `Origin`, and no `Origin` with the token only in a cookie.
///
/// Covers: story:4-21, story:4-24
#[test]
fn cve_2025_49596_a_browser_pages_request_without_a_header_token_is_refused_by_the_ops_listener() {
    let env = TestEnv::new();
    let (mut daemon, address) = with_ops(&env, Browsers::Installed);
    let listener = Listener::new(
        ListenerKind::OpsListener,
        Endpoint::Http { address },
        std::fs::read_to_string(HomeLayout::of(env.home()).daemon_file(DaemonFile::OpsToken))
            .unwrap(),
    );

    let ran = EndpointRunner::new(audit_verifier(&env, "ops_listener"))
        .run_advisory(&listener, CVE_49596);

    daemon.terminate();
    assert_eq!(ran, Ok(()));
}

/// CVE-2025-49596 on the Daemon's socket: a first message without the
/// token is closed unanswered and audited.
///
/// Covers: story:4-21, story:4-24
#[test]
fn cve_2025_49596_a_first_message_without_the_token_is_refused_by_the_daemon_socket() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let listener = socket_listener(&env);

    let ran = EndpointRunner::new(audit_verifier(&env, "daemon_socket"))
        .run_advisory(&listener, CVE_49596);

    assert_eq!(ran, Ok(()));
}

/// CVE-2025-66414 (the TypeScript MCP SDK's DNS-rebinding protection off by
/// default): a rebinding page's request names its own `Host`. The ops
/// listener refuses it and audits the refusal.
///
/// Covers: story:4-21, story:4-24
#[test]
fn cve_2025_66414_a_dns_rebinding_pages_host_is_refused_by_the_ops_listener() {
    let env = TestEnv::new();
    let (mut daemon, address) = with_ops(&env, Browsers::Installed);
    let listener = Listener::new(
        ListenerKind::OpsListener,
        Endpoint::Http { address },
        std::fs::read_to_string(HomeLayout::of(env.home()).daemon_file(DaemonFile::OpsToken))
            .unwrap(),
    );

    let ran = EndpointRunner::new(audit_verifier(&env, "ops_listener"))
        .run_advisory(&listener, "CVE-2025-66414");

    daemon.terminate();
    assert_eq!(ran, Ok(()));
}

/// After the ops listener's whole column, the ops token, an endpoint-token
/// Secret canary the operator provisioned, and the Daemon's protocol token
/// reach neither the Daemon's log nor the Audit log, raw or encoded, while
/// the Audit log holds the refusals.
///
/// Covers: story:4-24
#[test]
fn no_endpoint_token_reaches_the_daemons_log_or_the_audit_log() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let ops_token = provision_ops_token(&env, &mut canaries);
    let (mut daemon, address) = with_ops(&env, Browsers::Installed);
    let protocol_token = token(&env);
    canaries.add(Canary::known(
        "protocol-token",
        SecretKind::EndpointToken,
        &protocol_token,
    ));
    let listener = Listener::new(
        ListenerKind::OpsListener,
        Endpoint::Http { address },
        ops_token,
    )
    .with_other_listener_token(protocol_token.clone())
    .with_revoked_token("a-replaced-ops-token")
    .with_scoped_elsewhere_token(protocol_token);
    let ran = EndpointRunner::new(audit_verifier(&env, "ops_listener")).run(&listener);
    daemon.terminate();

    let home = Home::new(env.home().to_path_buf());
    // The entries a closed connection left in the write-ahead log (#2110),
    // when one is left: the last connection to close may copy it into the
    // file and remove it, as SQLite does on a clean close.
    let mut wal = home.audit_log().into_os_string();
    wal.push("-wal");
    let wal = std::path::PathBuf::from(wal);
    let (audit_log, audit_log_head) = (home.audit_log(), home.audit_log_head());
    let stdout = daemon.logged.join("\n");
    let mut outputs = vec![
        Output::Bytes {
            name: "the Daemon's log",
            bytes: stdout.as_bytes(),
        },
        Output::Path(&audit_log),
        Output::Path(&audit_log_head),
    ];
    if wal.exists() {
        outputs.push(Output::Path(&wal));
    }
    let leaks = canaries.leaks(&outputs).unwrap();
    assert_eq!(
        (
            ran,
            leaks,
            refusals(&env, "ops_listener", "token_wrong") > 0
        ),
        (Ok(()), vec![], true)
    );
}
