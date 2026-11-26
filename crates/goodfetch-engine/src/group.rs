//! Daemon groups (ADR 0055): the accepting side of the peer link, which
//! answers the lines another Group member sends, and the lead epochs a
//! Group member records.
//!
//! A peer link opens with the connecting Group member's `hello`, checked
//! in ADR 0095's order: its Product first, which must be the group's, since
//! runs move between Group members, refused with `not_in_product` naming
//! both; then its peer protocol version, the two settling on the lower
//! minor of one major and another major refused with
//! `protocol_version_unsupported` (ADR 0066); then its lead epoch. Every
//! message carries the group's lead epoch, and one carrying a lower epoch
//! than the highest recorded is refused with `lead_epoch_stale`, so a
//! former Lead Daemon that comes back is fenced off; a refused `hello`
//! closes the link.
//!
//! The lines come from the peer link's mutual TLS (ADR 0060), whose
//! configurations [`accepting`] and [`connecting`] make. The group's
//! certificate authority, [`GroupAuthority`], signs each Group member's
//! certificate for it once that Group member presents a [`JoinTokens`] token, and
//! revokes one through its revocation list.

mod authority;
mod join_tokens;
mod peer_tls;

pub use authority::{
    AUTHORITY_LIFETIME, AuthorityError, CERTIFICATE_LIFETIME, GroupAuthority,
    GroupMemberCertificate, GroupMemberKey, GroupMemberName, GroupMemberNameInvalid,
    GroupMemberPublicKey, GroupMemberPublicKeyInvalid, RevocationList,
};
pub use join_tokens::{JOIN_TOKEN_LIFETIME, JoinToken, JoinTokenRefused, JoinTokens};
pub use peer_tls::{PeerIdentity, PeerTlsError, PeerTrust, accepting, connecting};

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{
    NOT_IN_PRODUCT, PROTOCOL_VERSION_UNSUPPORTED, ProductVersion, ProtocolVersionUnsupportedDetails,
};
use goodfetch_protocol::formats::{Compatibility, PEER_PROTOCOL};
use goodfetch_protocol::group::{
    HELLO, LEAD_EPOCH, LEAD_EPOCH_STALE, LeadEpoch, LeadEpochParams, LeadEpochResult,
    LeadEpochStaleDetails, PEER_PROTOCOL_VERSION, PeerHelloParams, PeerHelloResult,
};
use goodfetch_protocol::jsonrpc::{ErrorResponse, ProtocolFailure, RequestId, Version};
use serde::Serialize;

use crate::call_error::CallError;
use crate::framing::{Call, Frame, Framer, Incoming, decode};
use crate::product::{Channel, Identity};
use crate::wire::{line, success_line};

/// The target of the `tracing` events that name the lead-epoch model's
/// transitions, each with its `transition` and the epochs as fields.
pub const TARGET: &str = "goodfetch::group";

/// The lead epochs a Group member has seen: the highest, which no peer
/// message may go below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeadEpochs {
    highest: LeadEpoch,
}

impl LeadEpochs {
    /// The record of a Group member whose highest lead epoch is `highest`.
    pub fn new(highest: LeadEpoch) -> Self {
        Self { highest }
    }

    /// The highest lead epoch recorded.
    pub fn highest(&self) -> LeadEpoch {
        self.highest
    }

    /// Admits a message carrying `epoch`, raising the highest to it, or
    /// refuses it when it is lower.
    pub fn admit(&mut self, epoch: LeadEpoch) -> Result<(), LeadEpochStaleDetails> {
        if epoch < self.highest {
            tracing::debug!(
                target: TARGET,
                transition = "refuse_stale",
                lead_epoch = epoch.get(),
                highest_lead_epoch = self.highest.get(),
                "a peer message's lead epoch is lower than the highest recorded"
            );
            return Err(LeadEpochStaleDetails {
                lead_epoch: epoch,
                highest_lead_epoch: self.highest,
            });
        }
        if epoch > self.highest {
            tracing::debug!(
                target: TARGET,
                transition = "raise",
                lead_epoch = epoch.get(),
                highest_lead_epoch = self.highest.get(),
                "a peer message raises the highest lead epoch"
            );
            self.highest = epoch;
        }
        Ok(())
    }
}

/// What the accepting side does with a line it received.
#[derive(Debug, PartialEq)]
pub struct Answer {
    /// The line it sends back, if any.
    pub reply: Option<Vec<u8>>,
    /// Whether the link then closes.
    pub close: bool,
}

impl Answer {
    fn reply(reply: Vec<u8>) -> Self {
        Self {
            reply: Some(reply),
            close: false,
        }
    }

    fn closing(reply: Option<Vec<u8>>) -> Self {
        Self { reply, close: true }
    }

    fn none() -> Self {
        Self {
            reply: None,
            close: false,
        }
    }
}

/// The accepting side of one peer link.
#[derive(Debug)]
pub struct PeerLink {
    identity: Identity,
    /// The peer protocol version settled at `hello`; none before.
    settled: Option<ProtocolVersion>,
}

impl PeerLink {
    /// A link that waits for its `hello`, answered as `identity`.
    pub fn accepting(identity: Identity) -> Self {
        Self {
            identity,
            settled: None,
        }
    }

    /// Answers `line`, one message of the connecting Group member, checking
    /// the lead epoch it carries against `epochs`.
    pub fn receive(&mut self, epochs: &mut LeadEpochs, line: Vec<u8>) -> Answer {
        let message = match decode(line) {
            Ok(message) => message,
            Err(error) => {
                let failure =
                    failure_line(error.id, &CallError::protocol(error.failure, error.detail));
                return if self.settled.is_some() {
                    Answer::reply(failure)
                } else {
                    Answer::closing(Some(failure))
                };
            }
        };
        match (self.settled, message) {
            (None, Incoming::Request { id, call }) if call.method() == HELLO => {
                self.hello(epochs, &id, call)
            }
            (None, Incoming::Request { id, .. }) => Answer::closing(Some(failure_line(
                Some(id),
                &CallError::protocol(
                    ProtocolFailure::InvalidRequest,
                    "a peer link opens with `hello`",
                ),
            ))),
            (None, Incoming::Notification(_) | Incoming::Reply { .. }) => Answer::closing(None),
            (Some(_), Incoming::Request { id, call }) => Self::call(epochs, id, call),
            (Some(_), Incoming::Notification(_) | Incoming::Reply { .. }) => Answer::none(),
        }
    }

    /// Answers the link's `hello`: the Product, then the version, then the
    /// lead epoch.
    fn hello(&mut self, epochs: &mut LeadEpochs, id: &RequestId, call: Call) -> Answer {
        let refuse =
            |error: CallError| Answer::closing(Some(failure_line(Some(id.clone()), &error)));
        let hello: PeerHelloParams = match call.into_params().read() {
            Ok(hello) => hello,
            Err(error) => {
                return refuse(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("a Group member's `hello` is not the peer protocol's: {error}"),
                ));
            }
        };
        if hello.product != self.identity.product {
            let (message, details) = self.identity.not_in_group_product(hello.product);
            return refuse(CallError::new(&NOT_IN_PRODUCT, message, &details));
        }
        if let Some((message, details)) = version_refusal(hello.protocol_version) {
            return refuse(CallError::new(
                &PROTOCOL_VERSION_UNSUPPORTED,
                message,
                &details,
            ));
        }
        if let Err(details) = epochs.admit(hello.lead_epoch) {
            return refuse(stale(&details));
        }
        let settled = PEER_PROTOCOL_VERSION.min(hello.protocol_version);
        self.settled = Some(settled);
        Answer::reply(result_line(
            id,
            &PeerHelloResult {
                protocol_version: settled,
                lead_epoch: epochs.highest(),
            },
        ))
    }

    /// Answers a call on an open link.
    fn call(epochs: &mut LeadEpochs, id: RequestId, call: Call) -> Answer {
        match call.method() {
            LEAD_EPOCH => {
                let params: LeadEpochParams = match call.into_params().read() {
                    Ok(params) => params,
                    Err(error) => {
                        return Answer::reply(failure_line(
                            Some(id),
                            &CallError::protocol(
                                ProtocolFailure::InvalidParams,
                                format!("`group.lead_epoch` names a `lead_epoch`: {error}"),
                            ),
                        ));
                    }
                };
                match epochs.admit(params.lead_epoch) {
                    Ok(()) => Answer::reply(result_line(
                        &id,
                        &LeadEpochResult {
                            lead_epoch: epochs.highest(),
                        },
                    )),
                    Err(details) => Answer::reply(failure_line(Some(id), &stale(&details))),
                }
            }
            HELLO => Answer::reply(failure_line(
                Some(id),
                &CallError::protocol(
                    ProtocolFailure::InvalidRequest,
                    "a peer link opens with `hello` once",
                ),
            )),
            method => Answer::reply(failure_line(
                Some(id),
                &CallError::protocol(
                    ProtocolFailure::MethodNotFound,
                    format!("the peer protocol has no method `{method}`"),
                ),
            )),
        }
    }
}

/// `lead_epoch_stale` for `details`.
fn stale(details: &LeadEpochStaleDetails) -> CallError {
    CallError::new(
        &LEAD_EPOCH_STALE,
        format!(
            "This message carries lead epoch {}, lower than {}, the highest this Group member has recorded.",
            details.lead_epoch, details.highest_lead_epoch
        ),
        details,
    )
}

/// The message and details of `protocol_version_unsupported` for a Group
/// member that speaks peer protocol `found`; `None` when the two settle on
/// the lower minor of one major (ADR 0055). During `0.y` previews each
/// `0.y` minor counts as a major (ADR 0030).
fn version_refusal(found: ProtocolVersion) -> Option<(String, ProtocolVersionUnsupportedDetails)> {
    let own = PEER_PROTOCOL_VERSION;
    let fix = match PEER_PROTOCOL.rule.compatibility(own, found) {
        Compatibility::Compatible => return None,
        Compatibility::NewerMinor | Compatibility::NewerMajor => {
            format!("Upgrade this Daemon to a version that speaks peer protocol {found}.")
        }
        Compatibility::OlderMajor => {
            format!("Upgrade that Daemon to one that speaks peer protocol {own}.")
        }
    };
    Some((
        format!("This Group member speaks peer protocol {found}, and this Daemon speaks {own}."),
        ProtocolVersionUnsupportedDetails {
            engine_protocol_version: own,
            client_protocol_version: found,
            fix,
            handover: false,
        },
    ))
}

/// The line of the success of the request `id`.
fn result_line(id: &RequestId, result: &impl Serialize) -> Vec<u8> {
    success_line(id, result).expect("invariant: the peer protocol's results encode")
}

/// The line of a failure.
fn failure_line(id: Option<RequestId>, error: &CallError) -> Vec<u8> {
    line(&ErrorResponse {
        jsonrpc: Version,
        id,
        error: error.to_error_object(),
    })
}

/// The `peer_framing` fuzz target's body (ADR 0067): frames `data` in two
/// chunks under a small limit and has a fresh link answer each line, as a
/// Group member at lead epoch 1; the recorded lead epoch never goes down,
/// and the link answers no line after it closes. It panics only on a bug.
pub fn fuzz_peer_framing(data: &[u8]) {
    let mut splitter = Framer::new(4096);
    let mut frames = Vec::new();
    let split = data
        .first()
        .map_or(0, |first| usize::from(*first) % (data.len() + 1));
    splitter.feed(&data[..split], &mut frames);
    splitter.feed(&data[split..], &mut frames);
    frames.extend(splitter.finish());
    let mut accepting = PeerLink::accepting(Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("0.4.0")
            .expect("invariant: the Product version is well formed"),
        channel: Channel::Installer,
    });
    let mut epochs = LeadEpochs::new(LeadEpoch::new(1));
    for frame in frames {
        let Frame::Line(line) = frame else {
            continue;
        };
        let before = epochs.highest();
        let answer = accepting.receive(&mut epochs, line);
        assert!(epochs.highest() >= before, "the lead epoch went down");
        if let Some(reply) = &answer.reply {
            assert!(
                serde_json::from_slice::<serde_json::Value>(reply).is_ok(),
                "a reply is one JSON message"
            );
        }
        if answer.close {
            break;
        }
    }
}

#[cfg(test)]
mod tests;
