//! `EndpointRunner` (Seam B): applied to fixture listeners, it reports every
//! row a permissive listener should refuse, passes a strict one, and fails a
//! row whose refusal its Audit log lacks.
#![expect(
    clippy::disallowed_types,
    reason = "the fixture listeners stand in for a product's, outside any Simulation (ADR 0060)"
)]

mod listeners;

use goodfetch_test_support::{
    AuditUnreadable, Endpoint, EndpointRunError, EndpointRunner, Expected, Failure, Hello,
    Listener, ListenerAddress, ListenerKind, Observed, RowFailure, Rule,
};
use rstest::rstest;

use listeners::{AuditLog, Behaviour, TOKEN};

/// The token another listener issued.
const OTHER_TOKEN: &str = "other-listener-token-5Kp9";

/// The token of the listener's earlier launch.
const EXPIRED_TOKEN: &str = "expired-token-8Rw1";

/// The token the listener took until it was replaced.
const REVOKED_TOKEN: &str = "revoked-token-3Lc6";

/// A token of the same Daemon scoped to another use.
const SCOPED_ELSEWHERE_TOKEN: &str = "scoped-elsewhere-token-9Tz4";

/// The fixture listeners' `hello`, whose `token` field carries the token.
fn hello() -> Hello {
    let params = serde_json::json!({ "protocol": "1.0", "surface": "sdk" });
    Hello::new(params.as_object().unwrap().clone(), "token")
}

fn json_rpc(kind: ListenerKind, address: ListenerAddress) -> Listener {
    Listener::new(
        kind,
        Endpoint::JsonRpc {
            address,
            hello: hello(),
        },
        TOKEN,
    )
    .with_other_listener_token(OTHER_TOKEN)
    .with_expired_token(EXPIRED_TOKEN)
    .with_revoked_token(REVOKED_TOKEN)
    .with_scoped_elsewhere_token(SCOPED_ELSEWHERE_TOKEN)
}

fn verifier(mut log: AuditLog) -> impl FnMut(ListenerKind, Rule) -> Result<bool, AuditUnreadable> {
    move |_, rule| Ok(log.records(rule))
}

fn ops_listener(address: std::net::SocketAddr) -> Listener {
    Listener::new(ListenerKind::OpsListener, Endpoint::Http { address }, TOKEN)
        .with_other_listener_token(OTHER_TOKEN)
        .with_revoked_token(REVOKED_TOKEN)
        .with_scoped_elsewhere_token(SCOPED_ELSEWHERE_TOKEN)
}

fn failures(result: Result<(), EndpointRunError>) -> Vec<RowFailure> {
    match result {
        Err(EndpointRunError::Rows { failures }) => failures,
        other => panic!("expected failing rows, got {other:?}"),
    }
}

fn unexpected(rule: Rule, expected: Expected, observed: Observed) -> RowFailure {
    RowFailure {
        rule,
        failure: Failure::Unexpected { expected, observed },
    }
}

#[test]
fn a_permissive_json_rpc_listener_fails_every_row_of_its_column() {
    let (address, log) = listeners::json_rpc_on_tcp(Behaviour::Permissive);
    let listener = json_rpc(ListenerKind::DaemonSocket, ListenerAddress::Tcp(address));

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(
        failures(result),
        vec![
            unexpected(Rule::TokenMissing, Expected::Closed, Observed::Replied),
            unexpected(Rule::TokenWrong, Expected::Closed, Observed::Replied),
            unexpected(
                Rule::TokenFromAnotherListener,
                Expected::Closed,
                Observed::Replied
            ),
            unexpected(Rule::TokenExpired, Expected::Closed, Observed::Replied),
            unexpected(Rule::TokenRevoked, Expected::Closed, Observed::Replied),
            unexpected(
                Rule::TokenScopedElsewhere,
                Expected::Closed,
                Observed::Replied
            ),
            unexpected(Rule::HelloNotFirst, Expected::Closed, Observed::Replied),
        ]
    );
}

#[test]
fn a_permissive_ops_listener_fails_every_row_of_its_column() {
    let (address, log) = listeners::ops_listener(Behaviour::Permissive);
    let listener = ops_listener(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    let answered = Observed::Answered {
        status: 200,
        body_length: 2,
    };
    let unauthorized = Expected::Status { status: 401 };
    let forbidden = Expected::Status { status: 403 };
    assert_eq!(
        failures(result),
        vec![
            unexpected(Rule::TokenMissing, unauthorized, answered),
            unexpected(Rule::TokenWrong, unauthorized, answered),
            unexpected(Rule::TokenFromAnotherListener, unauthorized, answered),
            unexpected(Rule::TokenRevoked, unauthorized, answered),
            unexpected(Rule::TokenScopedElsewhere, unauthorized, answered),
            unexpected(Rule::TokenInQueryString, unauthorized, answered),
            unexpected(Rule::HostForeign, forbidden, answered),
            unexpected(Rule::OriginForeign, forbidden, answered),
            unexpected(
                Rule::OriginMissingWithoutHeaderToken,
                unauthorized,
                answered
            ),
            unexpected(Rule::ProbesAnswerStatusOnly, Expected::StatusOnly, answered),
        ]
    );
}

#[test]
fn a_strict_ops_listener_passes() {
    let (address, log) = listeners::ops_listener(Behaviour::Strict);
    let listener = ops_listener(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(result, Ok(()));
}

#[test]
fn an_ops_listener_refusing_the_valid_request_fails_the_control() {
    let (address, log) = listeners::ops_listener(Behaviour::RefuseAll);
    let listener = ops_listener(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(
        result,
        Err(EndpointRunError::ControlRefused {
            observed: Observed::Answered {
                status: 403,
                body_length: 0
            }
        })
    );
}

fn websocket(address: std::net::SocketAddr) -> Listener {
    Listener::new(
        ListenerKind::DaemonWebsocket,
        Endpoint::WebSocket {
            address,
            hello: hello(),
        },
        TOKEN,
    )
    .with_other_listener_token(OTHER_TOKEN)
}

#[test]
fn a_permissive_websocket_listener_fails_every_row_of_its_column() {
    let (address, log) = listeners::websocket(Behaviour::Permissive);
    let listener = websocket(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    let forbidden = Expected::Status { status: 403 };
    assert_eq!(
        failures(result),
        vec![
            unexpected(Rule::TokenMissing, Expected::Closed, Observed::Replied),
            unexpected(Rule::TokenWrong, Expected::Closed, Observed::Replied),
            unexpected(
                Rule::TokenFromAnotherListener,
                Expected::Closed,
                Observed::Replied
            ),
            unexpected(
                Rule::TokenInQueryString,
                Expected::Closed,
                Observed::Replied
            ),
            unexpected(Rule::HostForeign, forbidden, Observed::Replied),
            unexpected(Rule::OriginForeign, forbidden, Observed::Replied),
            unexpected(Rule::OriginMissing, forbidden, Observed::Replied),
            unexpected(Rule::HelloNotFirst, Expected::Closed, Observed::Replied),
        ]
    );
}

#[test]
fn a_strict_websocket_listener_passes() {
    let (address, log) = listeners::websocket(Behaviour::Strict);
    let listener = websocket(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(result, Ok(()));
}

#[test]
fn a_websocket_listener_refusing_the_valid_handshake_fails_the_control() {
    let (address, log) = listeners::websocket(Behaviour::RefuseAll);
    let listener = websocket(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(
        result,
        Err(EndpointRunError::ControlRefused {
            observed: Observed::Answered {
                status: 403,
                body_length: 0
            }
        })
    );
}

#[rstest]
#[case::daemon_socket(ListenerKind::DaemonSocket)]
#[case::private_endpoint(ListenerKind::PrivateEndpoint)]
#[case::exposed_daemon(ListenerKind::ExposedDaemon)]
fn a_strict_json_rpc_listener_on_tcp_passes(#[case] kind: ListenerKind) {
    let (address, log) = listeners::json_rpc_on_tcp(Behaviour::Strict);
    let listener = json_rpc(kind, ListenerAddress::Tcp(address));

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(result, Ok(()));
}

#[cfg(unix)]
#[test]
fn a_strict_json_rpc_listener_on_a_unix_socket_passes() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("daemon.sock");
    let log = listeners::json_rpc_on_unix(&path, Behaviour::Strict);
    let listener = json_rpc(ListenerKind::DaemonSocket, ListenerAddress::Unix(path));

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(result, Ok(()));
}

#[test]
fn a_refusal_missing_from_the_audit_log_fails_its_row() {
    let (address, _log) = listeners::json_rpc_on_tcp(Behaviour::Strict);
    let listener = json_rpc(ListenerKind::DaemonSocket, ListenerAddress::Tcp(address));
    let stub = |_: ListenerKind, rule: Rule| Ok(rule != Rule::TokenWrong);

    let result = EndpointRunner::new(stub).run(&listener);

    assert_eq!(
        failures(result),
        vec![RowFailure {
            rule: Rule::TokenWrong,
            failure: Failure::NotAudited
        }]
    );
}

#[test]
fn an_audit_log_that_cannot_be_read_fails_each_refused_row() {
    let (address, _log) = listeners::json_rpc_on_tcp(Behaviour::Strict);
    let listener = json_rpc(ListenerKind::PrivateEndpoint, ListenerAddress::Tcp(address));
    let stub = |_: ListenerKind, _: Rule| Err(AuditUnreadable::new("the log is locked"));

    let result = EndpointRunner::new(stub).run(&listener);

    let unreadable = Failure::AuditUnreadable {
        message: "the log is locked".to_owned(),
    };
    assert_eq!(
        failures(result),
        [
            Rule::TokenMissing,
            Rule::TokenWrong,
            Rule::TokenFromAnotherListener,
            Rule::TokenExpired,
            Rule::HelloNotFirst
        ]
        .map(|rule| RowFailure {
            rule,
            failure: unreadable.clone()
        })
    );
}

#[test]
fn a_row_presenting_a_token_of_its_own_fails_without_one() {
    let (address, log) = listeners::json_rpc_on_tcp(Behaviour::Strict);
    let listener = Listener::new(
        ListenerKind::DaemonSocket,
        Endpoint::JsonRpc {
            address: ListenerAddress::Tcp(address),
            hello: hello(),
        },
        TOKEN,
    );

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(
        failures(result),
        [
            Rule::TokenFromAnotherListener,
            Rule::TokenExpired,
            Rule::TokenRevoked,
            Rule::TokenScopedElsewhere
        ]
        .map(|rule| RowFailure {
            rule,
            failure: Failure::TokenNotGiven
        })
    );
}

#[test]
fn a_listener_refusing_the_valid_hello_fails_the_control() {
    let (address, log) = listeners::json_rpc_on_tcp(Behaviour::RefuseAll);
    let listener = json_rpc(ListenerKind::DaemonSocket, ListenerAddress::Tcp(address));

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(
        result,
        Err(EndpointRunError::ControlRefused {
            observed: Observed::Closed
        })
    );
}

#[test]
fn a_listener_described_with_another_kinds_transport_is_refused() {
    let (address, log) = listeners::json_rpc_on_tcp(Behaviour::Strict);
    let listener = json_rpc(ListenerKind::OpsListener, ListenerAddress::Tcp(address));

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(
        result,
        Err(EndpointRunError::TransportMismatch {
            listener: ListenerKind::OpsListener
        })
    );
}

/// The Console's sign-in cookie, which a signed-address listener never
/// reads.
const SIGN_IN_COOKIE: &str = "goodfetch_console_0.0=id.secret";

fn signed_address(address: std::net::SocketAddr) -> Listener {
    Listener::new(
        ListenerKind::ContentOrigin,
        Endpoint::SignedAddress {
            address,
            host: address.to_string(),
            other_file: listeners::OTHER_FILE.to_owned(),
        },
        listeners::SIGNED_ADDRESS,
    )
    .with_other_listener_token(SIGN_IN_COOKIE)
}

#[test]
fn a_permissive_signed_address_listener_fails_every_row_of_its_column() {
    let (address, log) = listeners::signed_address(Behaviour::Permissive);
    let listener = signed_address(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    let answered = Observed::Answered {
        status: 200,
        body_length: 4,
    };
    let unauthorized = Expected::Status { status: 401 };
    let forbidden = Expected::Status { status: 403 };
    assert_eq!(
        failures(result),
        vec![
            unexpected(Rule::TokenMissing, unauthorized, answered),
            unexpected(Rule::TokenWrong, unauthorized, answered),
            unexpected(Rule::TokenFromAnotherListener, unauthorized, answered),
            unexpected(Rule::HostForeign, forbidden, answered),
            unexpected(Rule::OriginForeign, forbidden, answered),
            unexpected(Rule::TokenForAnotherFile, unauthorized, answered),
        ]
    );
}

#[test]
fn a_strict_signed_address_listener_passes() {
    let (address, log) = listeners::signed_address(Behaviour::Strict);
    let listener = signed_address(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(result, Ok(()));
}

#[test]
fn a_run_for_an_advisory_tries_only_the_rows_citing_it() {
    let (address, log) = listeners::ops_listener(Behaviour::Permissive);
    let listener = ops_listener(address);

    let result = EndpointRunner::new(verifier(log)).run_advisory(&listener, "CVE-2025-66414");

    assert_eq!(
        failures(result),
        vec![unexpected(
            Rule::HostForeign,
            Expected::Status { status: 403 },
            Observed::Answered {
                status: 200,
                body_length: 2
            }
        )]
    );
}

#[test]
fn an_advisory_no_row_of_the_column_cites_fails() {
    let (address, log) = listeners::json_rpc_on_tcp(Behaviour::Strict);
    let listener = json_rpc(ListenerKind::DaemonSocket, ListenerAddress::Tcp(address));

    let result = EndpointRunner::new(verifier(log)).run_advisory(&listener, "CVE-2025-66414");

    assert_eq!(
        result,
        Err(EndpointRunError::NoRowCites {
            listener: ListenerKind::DaemonSocket,
            advisory: "CVE-2025-66414".to_owned()
        })
    );
}

#[test]
fn a_strict_ops_listener_passes_the_rows_citing_an_advisory() {
    let (address, log) = listeners::ops_listener(Behaviour::Strict);
    let listener = ops_listener(address);

    let result = EndpointRunner::new(verifier(log)).run_advisory(&listener, "CVE-2025-49596");

    assert_eq!(result, Ok(()));
}

/// Another listener's token, as a Session's proxy relay gets it: another
/// Session's relay's password.
const OTHER_RELAY_TOKEN: &str = "other-relay-token-2Vn8";

fn http_proxy(address: std::net::SocketAddr) -> Listener {
    Listener::new(
        ListenerKind::ProxyRelay,
        Endpoint::HttpProxy {
            address,
            username: listeners::PROXY_USERNAME.to_owned(),
            target: "site.test:443".to_owned(),
        },
        TOKEN,
    )
    .with_other_listener_token(OTHER_RELAY_TOKEN)
}

#[test]
fn a_permissive_http_proxy_fails_every_row_of_its_column() {
    let (address, log) = listeners::http_proxy(Behaviour::Permissive);
    let listener = http_proxy(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    let tunnelled = Observed::Answered {
        status: 200,
        body_length: 0,
    };
    let refused = Expected::Status { status: 407 };
    assert_eq!(
        failures(result),
        vec![
            unexpected(Rule::TokenMissing, Expected::Challenge, tunnelled),
            unexpected(Rule::TokenWrong, refused, tunnelled),
            unexpected(Rule::TokenFromAnotherListener, refused, tunnelled),
        ]
    );
}

#[test]
fn a_strict_http_proxy_passes() {
    let (address, log) = listeners::http_proxy(Behaviour::Strict);
    let listener = http_proxy(address);

    let result = EndpointRunner::new(verifier(log)).run(&listener);

    assert_eq!(result, Ok(()));
}
