//! The peer protocol's writer for the Compatibility corpus (ADR 0066): one
//! fixed scenario between two Group members of a Daemon group (ADR 0055),
//! the messages each side sends built from the protocol's own types,
//! written into the folder given as the only argument as `connecting.jsonl`
//! and `accepting.jsonl`, one message per line. The Group member that
//! connects says `hello` at lead epoch 4, which the other accepts; it then
//! exchanges lead epochs with a message a former Lead Daemon would send,
//! carrying epoch 3, which the other refuses with `lead_epoch_stale`.
//! `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

#[path = "support/transcript.rs"]
mod transcript;

use std::path::PathBuf;

use goodfetch_protocol::catalog::{ErrorGroup, Product};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::{
    HELLO, LEAD_EPOCH, LEAD_EPOCH_STALE, LeadEpoch, LeadEpochParams, LeadEpochStaleDetails,
    PEER_PROTOCOL_VERSION, PeerHelloParams, PeerHelloResult,
};
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, Message, RequestId, Version,
};
use serde::Serialize;
use serde_json::{Map, Value};
use transcript::{call, result, to_value, write};

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let current = LeadEpoch::new(4);
    let former = LeadEpoch::new(3);
    let connecting = [
        call(
            RequestId::Number(0),
            HELLO,
            &PeerHelloParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: Product::Foxhound,
                product_version: ProductVersion::parse("0.4.0")
                    .expect("invariant: the Product version is well formed"),
                lead_epoch: current,
            },
        ),
        call(
            RequestId::Number(1),
            LEAD_EPOCH,
            &LeadEpochParams { lead_epoch: former },
        ),
    ];
    let accepting = [
        result(
            RequestId::Number(0),
            &PeerHelloResult {
                protocol_version: PEER_PROTOCOL_VERSION,
                lead_epoch: current,
            },
        ),
        Message::Error(ErrorResponse {
            jsonrpc: Version,
            id: Some(RequestId::Number(1)),
            error: ErrorObject {
                code: ErrorGroup::Refused.json_rpc_class(),
                message: "This message carries lead epoch 3, lower than 4, the highest this Group member has recorded.".to_owned(),
                data: ErrorData {
                    code: LEAD_EPOCH_STALE.name.to_owned(),
                    retryable: LEAD_EPOCH_STALE.retryable,
                    details: object(&LeadEpochStaleDetails {
                        lead_epoch: former,
                        highest_lead_epoch: current,
                    }),
                },
            },
        }),
    ];
    write(&output.join("connecting.jsonl"), &connecting);
    write(&output.join("accepting.jsonl"), &accepting);
}

/// `value` as a JSON object.
fn object(value: &impl Serialize) -> Map<String, Value> {
    match to_value(value) {
        Value::Object(map) => map,
        _ => unreachable!("invariant: details serialize to an object"),
    }
}
