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
}

fn verifier(mut log: AuditLog) -> impl FnMut(ListenerKind, Rule) -> Result<bool, AuditUnreadable> {
    move |_, rule| Ok(log.records(rule))
}

fn ops_listener(address: std::net::SocketAddr) -> Listener {
    Listener::new(ListenerKind::OpsListener, Endpoint::Http { address }, TOKEN)
        .with_other_listener_token(OTHER_TOKEN)
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
            unexpected(Rule::TokenInQueryString, unauthorized, answered),
            unexpected(Rule::HostForeign, forbidden, answered),
            unexpected(Rule::OriginForeign, forbidden, answered),
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

#[rstest]
#[case::daemon_socket(ListenerKind::DaemonSocket)]
#[case::private_endpoint(ListenerKind::PrivateEndpoint)]
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
            Rule::HelloNotFirst
        ]
        .map(|rule| RowFailure {
            rule,
            failure: unreadable.clone()
        })
    );
}

#[test]
fn a_row_needing_another_listeners_token_fails_without_one() {
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
        vec![RowFailure {
            rule: Rule::TokenFromAnotherListener,
            failure: Failure::NoOtherListenerToken
        }]
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
