//! The peer link's accepting side, judged on the lines it answers: the
//! `hello` of a Group member of the group's Product is accepted, one of the
//! other Product is refused with `not_in_product` naming both, and every
//! message carrying a lower lead epoch than the highest recorded is
//! refused with `lead_epoch_stale`. The generated refusal cases of the peer
//! protocol (ADR 0066) run here in process.
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
use proptest::prelude::*;
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

/// The accepted `hello` answers with the settled version and the lead
/// epoch this Group member records.
#[test]
fn a_hello_of_the_groups_product_is_answered_with_the_version_and_the_lead_epoch() {
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
        (&sent["error"]["data"], close),
        (
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

/// A link whose `hello` at `epoch` was accepted, with `epoch` recorded.
fn opened(epoch: u64) -> (PeerLink, LeadEpochs) {
    let mut link = PeerLink::accepting(identity(Product::Collie));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(epoch));
    link.receive(
        &mut epochs,
        hello(Product::Collie, PEER_PROTOCOL_VERSION, epoch),
    );
    (link, epochs)
}

/// A former Lead Daemon, back with the epoch it had, is fenced off at its
/// `hello`, learns the higher epoch from the refusal, and the link closes.
///
/// Covers: error:lead_epoch_stale
#[test]
fn a_hello_carrying_a_lower_lead_epoch_is_refused_and_closes_the_link() {
    let mut link = PeerLink::accepting(identity(Product::Collie));
    let mut epochs = LeadEpochs::new(LeadEpoch::new(4));

    let (sent, close) = reply(link.receive(
        &mut epochs,
        hello(Product::Collie, PEER_PROTOCOL_VERSION, 3),
    ));

    assert_eq!(
        (&sent["error"]["data"], close),
        (
            &json!({
                "code": "lead_epoch_stale",
                "retryable": false,
                "lead_epoch": 3,
                "highest_lead_epoch": 4,
            }),
            true
        )
    );
}

/// A message after `hello` carrying a lower lead epoch is refused, and the
/// link stays open.
///
/// Covers: error:lead_epoch_stale
#[test]
fn a_later_message_carrying_a_lower_lead_epoch_is_refused_and_the_link_stays_open() {
    let (mut link, mut epochs) = opened(4);

    let (sent, close) = reply(link.receive(&mut epochs, lead_epoch(1, 3)));

    assert_eq!(
        (&sent["error"]["data"], close),
        (
            &json!({
                "code": "lead_epoch_stale",
                "retryable": false,
                "lead_epoch": 3,
                "highest_lead_epoch": 4,
            }),
            false
        )
    );
}

/// A message carrying a higher lead epoch raises the one recorded.
#[test]
fn a_message_carrying_a_higher_lead_epoch_raises_the_one_recorded() {
    let (mut link, mut epochs) = opened(4);

    let answer = reply(link.receive(&mut epochs, lead_epoch(1, 5)));

    assert_eq!(
        (answer, epochs.highest()),
        (
            (
                json!({ "jsonrpc": "2.0", "id": 1, "result": { "lead_epoch": 5 } }),
                false
            ),
            LeadEpoch::new(5)
        )
    );
}

proptest! {
    /// Whatever lead epochs the messages carry, in whatever order, the one
    /// recorded is the highest seen so far, and exactly the messages below
    /// it are refused.
    #[test]
    fn the_recorded_lead_epoch_is_the_running_highest_and_only_lower_ones_are_refused(
        start in 0_u64..8,
        carried in proptest::collection::vec(0_u64..8, 0..16),
    ) {
        let (mut link, mut epochs) = opened(start);
        let mut highest = start;

        for (id, epoch) in (1_u64..).zip(carried) {
            let (sent, _) = reply(link.receive(&mut epochs, lead_epoch(id, epoch)));

            let refused = sent.get("error").is_some();
            prop_assert_eq!(refused, epoch < highest, "{}", sent);
            highest = highest.max(epoch);
            prop_assert_eq!(epochs.highest(), LeadEpoch::new(highest));
        }
    }
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
/// Covers: story:3-95, story:3-108, format:peer-protocol
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
