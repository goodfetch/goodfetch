//! The one message of a Daemon that joins, judged on what the Lead Daemon
//! makes of it: a `group.join` of the group's Product, at a version it
//! speaks, goes on to be admitted; one of another Product or major is
//! refused as a `hello` would be; anything else is refused or closes the
//! link unanswered. The network exchange runs in Collie's tests, two
//! Daemons over loopback.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the committed refusal cases on the test thread, not on an async worker"
)]

use std::path::PathBuf;

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::PEER_PROTOCOL_VERSION;
use goodfetch_protocol::refusals::{RefusalCase, Reply};
use rstest::rstest;
use serde_json::{Value, json};

use super::join_request;
use crate::product::{Channel, Identity};

fn identity(product: Product) -> Identity {
    Identity {
        product,
        product_version: ProductVersion::parse("0.4.0").unwrap(),
        channel: Channel::Homebrew,
    }
}

fn join(method: &str, product: Product, version: ProtocolVersion) -> Vec<u8> {
    line(&json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": method,
        "params": {
            "protocol_version": version,
            "product": product,
            "product_version": "0.4.0",
            "token": "a-join-token",
            "name": "worker-1",
            "public_key": "BAAA",
            "peer_address": "127.0.0.1:7000",
        },
    }))
}

fn line(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// The refusal line `join_request` answers.
fn refusal(line: Vec<u8>, product: Product) -> Value {
    let refused = join_request(&identity(product), line).expect_err("a refusal");
    serde_json::from_slice(&refused.expect("a refusal line")).unwrap()
}

/// A `group.join` of the group's Product is read, at the lower of the two
/// peer protocol versions.
///
/// Covers: story:3-95
#[test]
fn a_join_of_the_groups_product_is_read_at_the_settled_version() {
    let request = join_request(
        &identity(Product::Foxhound),
        join("group.join", Product::Foxhound, PEER_PROTOCOL_VERSION),
    )
    .unwrap();

    assert_eq!(
        (request.settled, request.params.name.as_str()),
        (PEER_PROTOCOL_VERSION, "worker-1")
    );
}

/// The fabricated Group members of the peer protocol's refusal cases
/// (ADR 0066) are refused, or let through, when they join just as when
/// they say `hello`: one check of the opening message serves both.
///
/// Covers: story:3-95, format:peer-protocol
#[rstest]
fn each_fabricated_join_is_judged_as_its_hello(
    #[files("../goodfetch-protocol/generated/refusals/peer-protocol/*.json")] path: PathBuf,
) {
    let case: RefusalCase = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let fabricated = case.hello.unwrap();

    let judged = join_request(
        &identity(fabricated.engine),
        join("group.join", fabricated.needs.unwrap(), case.found),
    );

    match (fabricated.reply, judged) {
        (Reply::Accepted { protocol_version }, Ok(request)) => {
            assert_eq!(request.settled, protocol_version);
        }
        (Reply::Refused { error_code }, Err(Some(sent))) => {
            let sent: Value = serde_json::from_slice(&sent).unwrap();
            assert_eq!(
                (&sent["id"], &sent["error"]["data"]["code"]),
                (&json!(7), &json!(error_code)),
                "{sent:#}"
            );
        }
        (expected, judged) => panic!("expected {expected:?}, judged {judged:?}"),
    }
}

/// A Daemon without a certificate may send `group.join` alone: a `hello`
/// is refused as an invalid request.
#[test]
fn a_daemon_without_a_certificate_cannot_say_hello() {
    let sent = refusal(
        join("hello", Product::Foxhound, PEER_PROTOCOL_VERSION),
        Product::Foxhound,
    );

    assert_eq!(
        (&sent["id"], &sent["error"]["data"]["code"]),
        (&json!(7), &json!("invalid_request"))
    );
}

/// A `group.join` whose params are not the peer protocol's is refused
/// with `invalid_params`.
#[test]
fn a_join_without_its_params_is_refused() {
    let sent = refusal(
        line(&json!({"jsonrpc": "2.0", "id": 7, "method": "group.join", "params": {}})),
        Product::Foxhound,
    );

    assert_eq!(&sent["error"]["data"]["code"], &json!("invalid_params"));
}

/// A notification asks nothing: the link closes unanswered.
#[test]
fn a_notification_is_left_unanswered() {
    let judged = join_request(
        &identity(Product::Foxhound),
        line(&json!({"jsonrpc": "2.0", "method": "group.join", "params": {}})),
    );

    assert!(matches!(judged, Err(None)), "{judged:?}");
}
