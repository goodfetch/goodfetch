//! The peer link's accepting side, judged on the lines it answers: the
//! `hello` of a Group member of the group's Product settles on the lower
//! minor, one of the other Product is refused with `not_in_product` naming
//! both, and every message carrying a lower lead epoch than the highest
//! recorded is refused with `lead_epoch_stale`. The generated refusal cases
//! of the peer protocol (ADR 0066) run here in process.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the committed refusal cases on the test thread, not on an async worker"
)]

use std::path::PathBuf;

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::{LeadEpoch, PEER_PROTOCOL_VERSION};
use goodfetch_protocol::refusals::{RefusalCase, Reply};
use rstest::rstest;
use serde_json::{Value, json};

use super::{Answer, LeadEpochs, PeerLink};
use crate::product::{Channel, Identity};

fn identity(product: Product) -> Identity {
    Identity {
        product,
        product_version: ProductVersion::parse("0.4.0").unwrap(),
        channel: Channel::Homebrew,
    }
}

fn hello(product: Product, version: ProtocolVersion, epoch: u64) -> Vec<u8> {
    line(&json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": version,
            "product": product,
            "product_version": "0.4.0",
            "lead_epoch": epoch,
        },
    }))
}

fn lead_epoch(id: u64, epoch: u64) -> Vec<u8> {
    line(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "group.lead_epoch",
        "params": { "lead_epoch": epoch },
    }))
}

fn line(message: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).unwrap();
    line.push(b'\n');
    line
}

/// The reply an answer sends, and whether the link then closes.
fn reply(answer: Answer) -> (Value, bool) {
    let sent = answer.reply.expect("an answer with a reply");
    (serde_json::from_slice(&sent).unwrap(), answer.close)
}

/// Covers: story:3-108, format:peer-protocol
#[test]
fn a_group_member_of_the_groups_product_settles_on_the_lower_minor() {
    let mut link = PeerLink::accepting(identity(Product::Foxhound));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(2));

    let answer = link.receive(
        &mut epochs,
        hello(Product::Foxhound, PEER_PROTOCOL_VERSION, 2),
    );

    assert_eq!(
        reply(answer),
        (
            json!({
                "jsonrpc": "2.0",
                "id": 0,
                "result": { "protocol_version": PEER_PROTOCOL_VERSION, "lead_epoch": 2 },
            }),
            false
        )
    );
}

/// Covers: story:3-145, error:not_in_product
#[test]
fn a_daemon_of_the_other_product_is_refused_naming_both() {
    let mut link = PeerLink::accepting(identity(Product::Foxhound));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(0));

    let (sent, close) = reply(link.receive(
        &mut epochs,
        hello(Product::Collie, PEER_PROTOCOL_VERSION, 0),
    ));

    assert_eq!(
        (&sent["error"]["message"], &sent["error"]["data"], close),
        (
            &json!("This Group member runs Collie, and this Daemon group runs Foxhound 0.4.0."),
            &json!({
                "code": "not_in_product",
                "retryable": false,
                "product": "foxhound",
                "product_version": "0.4.0",
                "needs": "collie",
                "fix": "Start that Daemon with Foxhound, the group's Product (`brew install goodfetch/tap/goodfetch-foxhound`): Group members run one Product.",
                "install": "brew install goodfetch/tap/goodfetch-foxhound",
            }),
            true
        )
    );
}

/// A former Lead Daemon, back with the epoch it had, is fenced off at its
/// `hello` and on every later message, and learns the higher epoch from
/// the refusal; a higher epoch raises the one recorded.
///
/// Covers: error:lead_epoch_stale
#[test]
fn a_message_carrying_a_lower_lead_epoch_is_refused() {
    let mut link = PeerLink::accepting(identity(Product::Collie));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(4));
    let stale_hello = reply(link.receive(
        &mut epochs,
        hello(Product::Collie, PEER_PROTOCOL_VERSION, 3),
    ));
    let mut link = PeerLink::accepting(identity(Product::Collie));
    link.receive(
        &mut epochs,
        hello(Product::Collie, PEER_PROTOCOL_VERSION, 4),
    );

    let raised = reply(link.receive(&mut epochs, lead_epoch(1, 5)));
    let stale = reply(link.receive(&mut epochs, lead_epoch(2, 4)));

    let stale_details = |answer: &(Value, bool)| (answer.0["error"]["data"].clone(), answer.1);
    assert_eq!(
        (
            stale_details(&stale_hello),
            raised,
            stale_details(&stale),
            epochs.highest()
        ),
        (
            (
                json!({
                    "code": "lead_epoch_stale",
                    "retryable": false,
                    "lead_epoch": 3,
                    "highest_lead_epoch": 4,
                }),
                true
            ),
            (
                json!({ "jsonrpc": "2.0", "id": 1, "result": { "lead_epoch": 5 } }),
                false
            ),
            (
                json!({
                    "code": "lead_epoch_stale",
                    "retryable": false,
                    "lead_epoch": 4,
                    "highest_lead_epoch": 5,
                }),
                false
            ),
            LeadEpoch::new(5)
        )
    );
}

/// A peer link opens with `hello`: anything else first is refused, and the
/// link closes.
#[test]
fn a_peer_link_opens_with_hello() {
    let mut link = PeerLink::accepting(identity(Product::Collie));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(0));

    let (sent, close) = reply(link.receive(&mut epochs, lead_epoch(1, 0)));

    assert_eq!(
        (&sent["id"], &sent["error"]["data"]["code"], close),
        (&json!(1), &json!("invalid_request"), true)
    );
}

/// The peer protocol's generated refusal cases (ADR 0066): each
/// fabricated `hello` of a Group member, at a version of the rule's table
/// or of a pair of Products, gets its generated reply.
///
/// Covers: story:3-95, format:peer-protocol
#[rstest]
fn each_fabricated_hello_gets_its_generated_reply(
    #[files("../goodfetch-protocol/generated/refusals/peer-protocol/*.json")] path: PathBuf,
) {
    let case: RefusalCase = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let fabricated = case.hello.clone().unwrap();
    let mut link = PeerLink::accepting(identity(fabricated.engine));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(0));

    let (sent, close) =
        reply(link.receive(&mut epochs, hello(fabricated.needs.unwrap(), case.found, 0)));

    match fabricated.reply {
        Reply::Accepted { protocol_version } => assert_eq!(
            (&sent["result"]["protocol_version"], close),
            (&json!(protocol_version), false),
            "{sent:#}"
        ),
        Reply::Refused { error_code } => {
            assert_eq!(
                (&sent["error"]["data"]["code"], close),
                (&json!(error_code), true),
                "{sent:#}"
            );
            let name = path.file_stem().unwrap().to_string_lossy();
            insta::assert_snapshot!(
                format!("peer_refusal_{name}"),
                serde_json::to_string_pretty(&sent).unwrap()
            );
        }
    }
}

mod peer_framing {
    goodfetch_test_support::fuzz_replay!(super::super::fuzz_peer_framing, "fuzz/peer_framing");
}
