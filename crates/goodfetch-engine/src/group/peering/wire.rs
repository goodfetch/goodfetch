//! Outbound peer-link dialing and newline-framed JSON-RPC exchange (ADR 0055):
//! joining a group with a pinned authority key, dialing a peer over mutual
//! TLS 1.3, sending a single request and reading its reply, and framing lines
//! on an accepted or dialled peer link.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::group::{
    GROUP_LEAD_UNREACHABLE, GROUP_MEMBER_UNKNOWN, JOIN, LeadEpoch, PEER_PROTOCOL_VERSION,
    PeerJoinParams, PeerJoinResult,
};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, ServerName};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::TlsConnector;

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::forward::answered_failure;
use crate::framing::{Frame, Framer, Incoming, MAX_MESSAGE_BYTES, decode};
use crate::group::errors::internal;
use crate::group::identity::KeptIdentity;
use crate::group::peer_tls::key_hash_of;
use crate::group::{GroupMemberName, GroupMemberPublicKey, connecting, joining};
use crate::known_certificates::KeyHash;
use crate::product::Identity;
use crate::transport::{PeerStream, connect_peer};
use crate::wire::request_line;

/// How long a peer link has to finish its handshake and send its opening
/// message, and a Daemon that joins or asks a peer to have its answer.
pub(super) const OPENING_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest message of the peer link (aligned with the protocol's
/// [`MAX_MESSAGE_BYTES`] so Run Checkpoints cross the peer link on scale-down
/// and `runs adopt`, ADR 0055); a longer one is refused.
const LINE_LIMIT: usize = MAX_MESSAGE_BYTES;

/// How many bytes one read of the peer link takes at most.
const READ_BYTES: usize = 8 * 1024;

/// The server name a Daemon that joins dials: its verifier checks the
/// authority it pinned, not a name, since it knows no Group member's yet.
const LEAD: &str = "lead-daemon";

/// Who asks the Lead Daemon to join its group.
pub(in crate::group) struct Asking<'a> {
    /// The `host:port` of the Lead Daemon's peer listener.
    pub(in crate::group) lead: &'a str,
    /// The SHA-256 of the group's certificate authority's public key.
    pub(in crate::group) pin: KeyHash,
    pub(in crate::group) token: &'a DaemonToken,
    pub(in crate::group) name: &'a GroupMemberName,
    pub(in crate::group) public_key: &'a GroupMemberPublicKey,
    /// The `host:port` its own peer listener took.
    pub(in crate::group) peer_address: &'a str,
}

/// What a Daemon that joins receives: the Lead Daemon's answer, its
/// certificates and list read.
#[derive(Debug)]
pub(in crate::group) struct Admitted {
    pub(in crate::group) lead_epoch: LeadEpoch,
    /// The Lead Daemon's Group member name.
    pub(in crate::group) lead_name: String,
    pub(in crate::group) certificate: CertificateDer<'static>,
    pub(in crate::group) certificate_expires_at: std::time::SystemTime,
    pub(in crate::group) authority: CertificateDer<'static>,
    pub(in crate::group) revocations: CertificateRevocationListDer<'static>,
}

/// Asks the Lead Daemon to join its group, as the Daemon `identity`: opens
/// a peer link to it, trusting only the authority `asking` pins, and sends
/// its one `group.join`.
///
/// # Errors
///
/// `group_lead_unreachable` when no Lead Daemon answers in time,
/// `authority_key_mismatch` when the one that answers shows another
/// authority, and the Lead Daemon's own refusal.
pub(in crate::group) async fn ask_to_join(
    asking: &Asking<'_>,
    identity: &Identity,
    clock: Clock,
) -> Result<Admitted, CallError> {
    let answered = tokio::select! {
        answered = exchange(asking, identity, clock) => answered?,
        () = clock.sleep(OPENING_TIMEOUT) => {
            return Err(unreachable(asking.lead, "it gave no answer in time"));
        }
    };
    let unreadable = || {
        CallError::protocol(
            ProtocolFailure::InternalError,
            "the Lead Daemon's answer to `group.join` holds no readable certificate",
        )
    };
    let decoded = |text: &str| BASE64.decode(text.as_bytes()).map_err(|_| unreadable());
    let authority = CertificateDer::from(decoded(&answered.authority)?);
    if key_hash_of(&authority).as_ref() != Some(&asking.pin) {
        return Err(mismatch(asking.lead));
    }
    Ok(Admitted {
        lead_epoch: answered.lead_epoch,
        lead_name: answered.lead_name,
        certificate: CertificateDer::from(decoded(&answered.certificate)?),
        certificate_expires_at: answered.certificate_expires_at,
        authority,
        revocations: CertificateRevocationListDer::from(decoded(&answered.revocations)?),
    })
}

/// The `group.join` exchange of [`ask_to_join`], with no time limit.
async fn exchange(
    asking: &Asking<'_>,
    identity: &Identity,
    clock: Clock,
) -> Result<PeerJoinResult, CallError> {
    let stream = connect_peer(asking.lead)
        .await
        .map_err(|error| unreachable(asking.lead, &error))?;
    let server =
        ServerName::try_from(LEAD).expect("invariant: the Lead Daemon's name is a DNS name");
    let mut stream = TlsConnector::from(Arc::new(joining(asking.pin, clock)))
        .connect(server, stream)
        .await
        .map_err(|error| {
            let certificate = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
                .is_some_and(|inner| matches!(inner, rustls::Error::InvalidCertificate(_)));
            if certificate {
                mismatch(asking.lead)
            } else {
                unreachable(asking.lead, &error)
            }
        })?;
    let params = PeerJoinParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity.product,
        product_version: identity.product_version.clone(),
        token: asking.token.clone(),
        name: asking.name.as_str().to_owned(),
        public_key: BASE64.encode(asking.public_key.raw()),
        peer_address: asking.peer_address.to_owned(),
    };
    ask(&mut stream, asking.lead, JOIN, &params).await
}

/// Opens a mutual-TLS 1.3 peer link from `kept` to the Group member
/// `peer_name` at `peer_address`, verified at `clock`.
pub(super) async fn connect_peer_tls(
    kept: &KeptIdentity,
    peer_name: &str,
    peer_address: &str,
    clock: Clock,
) -> Result<tokio_rustls::client::TlsStream<PeerStream>, CallError> {
    let name = GroupMemberName::new(peer_name).map_err(|error| internal(&error))?;
    let config =
        connecting(kept.presented(), &kept.trust(), clock).map_err(|error| internal(&error))?;
    let stream = connect_peer(peer_address)
        .await
        .map_err(|error| unreachable(peer_address, &error))?;
    TlsConnector::from(Arc::new(config))
        .connect(name.server_name(), stream)
        .await
        .map_err(|error| unreachable(peer_address, &error))
}

/// Sends the Group member `peer_name` at `peer_address` the one request
/// `method` with `params`, on a peer link `kept` opens with its certificate
/// at `clock`, and reads its answer.
pub(super) async fn ask_peer<T: DeserializeOwned>(
    kept: &KeptIdentity,
    peer_name: &str,
    peer_address: &str,
    clock: Clock,
    method: &str,
    params: &impl Serialize,
) -> Result<T, CallError> {
    let asked = async {
        let mut stream = connect_peer_tls(kept, peer_name, peer_address, clock).await?;
        let answered = ask(&mut stream, peer_address, method, params).await?;
        if let Err(error) = stream.shutdown().await {
            tracing::debug!(%error, "a peer link did not close cleanly");
        }
        Ok(answered)
    };
    tokio::select! {
        asked = asked => asked,
        () = clock.sleep(OPENING_TIMEOUT) => Err(unreachable(peer_address, "it gave no answer in time")),
    }
}

/// Sends the Lead Daemon of the Group member `kept` the one request
/// `method` with `params`, on a peer link it opens with its certificate
/// at `clock`, and reads its answer.
///
/// # Errors
///
/// `group_lead_unreachable` when the Lead Daemon gives no answer in time,
/// `group_member_unknown` when it refuses the certificate as revoked, and
/// its own refusal.
pub(super) async fn ask_lead<T: DeserializeOwned>(
    kept: &KeptIdentity,
    clock: Clock,
    method: &str,
    params: &impl Serialize,
) -> Result<T, CallError> {
    ask_peer(
        kept,
        &kept.lead_name,
        &kept.lead_address,
        clock,
        method,
        params,
    )
    .await
}

/// Sends the peer at `address` the one request `method` with `params` on
/// `stream` and reads its answer.
///
/// # Errors
///
/// `group_lead_unreachable` when the link closes first,
/// `group_member_unknown` when the peer closes it on this side's
/// certificate as revoked, and the peer's own refusal.
pub(super) async fn ask<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    address: &str,
    method: &str,
    params: &impl Serialize,
) -> Result<T, CallError> {
    let request =
        request_line(&RequestId::Number(1), method, params).map_err(|error| internal(&error))?;
    if !write(stream, &request).await {
        return Err(unreachable(
            address,
            "the link closed before the request was sent",
        ));
    }
    let mut lines = Lines::new();
    let Some(Frame::Line(reply)) = lines.next(stream).await else {
        if lines.refused_as_revoked() {
            return Err(CallError::new(
                &GROUP_MEMBER_UNKNOWN,
                format!(
                    "The Lead Daemon at `{address}` refused this Daemon's certificate as revoked: it is no Group member any more."
                ),
                &(),
            ));
        }
        return Err(unreachable(address, "the link closed before its answer"));
    };
    let unexpected = || {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the Lead Daemon's answer to `{method}` is not the peer protocol's"),
        )
    };
    match decode(reply) {
        Ok(Incoming::Reply {
            answer: Ok(result), ..
        }) => result.read().map_err(|_| unexpected()),
        Ok(Incoming::Reply {
            answer: Err(failure),
            ..
        }) => Err(answered_failure(&failure)),
        Ok(Incoming::Request { .. } | Incoming::Notification(_)) | Err(_) => Err(unexpected()),
    }
}

/// `group_lead_unreachable` for the peer at `address`.
fn unreachable(address: &str, why: impl std::fmt::Display) -> CallError {
    CallError::new(
        &GROUP_LEAD_UNREACHABLE,
        format!("No Lead Daemon answered at `{address}`: {why}."),
        &(),
    )
}

/// `authority_key_mismatch` for the Daemon at `lead`.
fn mismatch(lead: &str) -> CallError {
    CallError::new(
        &goodfetch_protocol::group::AUTHORITY_KEY_MISMATCH,
        format!(
            "The Daemon at `{lead}` showed no certificate authority whose public key `group join` pins, so this Daemon sent it nothing."
        ),
        &(),
    )
}

/// The refusal of a peer message longer than [`LINE_LIMIT`].
pub(super) fn too_large() -> CallError {
    CallError::protocol(
        ProtocolFailure::MessageTooLarge,
        format!("a peer link's message is longer than {LINE_LIMIT} bytes"),
    )
}

/// Writes `line` to `stream`: whether it went.
pub(super) async fn write(stream: &mut (impl AsyncWrite + Unpin), line: &[u8]) -> bool {
    let written = async {
        stream.write_all(line).await?;
        stream.flush().await
    };
    match written.await {
        Ok(()) => true,
        Err(error) => {
            tracing::debug!(%error, "a peer link closed while it was written to");
            false
        }
    }
}

/// The messages a peer link reads, framed by newlines.
pub(super) struct Lines {
    framer: Framer,
    buffer: Vec<u8>,
    ready: VecDeque<Frame>,
    ended: bool,
    /// The read failure that ended it, if one did.
    failure: Option<std::io::Error>,
}

impl Lines {
    pub(super) fn new() -> Self {
        Self {
            framer: Framer::new(LINE_LIMIT),
            buffer: vec![0; READ_BYTES],
            ready: VecDeque::new(),
            ended: false,
            failure: None,
        }
    }

    /// Whether the link ended on the other side's TLS alert that this
    /// side's certificate is revoked.
    fn refused_as_revoked(&self) -> bool {
        self.failure
            .as_ref()
            .and_then(std::io::Error::get_ref)
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .is_some_and(|inner| {
                matches!(
                    inner,
                    rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked)
                )
            })
    }

    /// The next message `stream` sends, none once it ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no byte read: the framer keeps them.
    pub(super) async fn next(&mut self, stream: &mut (impl AsyncRead + Unpin)) -> Option<Frame> {
        loop {
            if let Some(frame) = self.ready.pop_front() {
                return Some(frame);
            }
            if self.ended {
                return None;
            }
            let mut frames = Vec::new();
            match stream.read(&mut self.buffer).await {
                Ok(0) => {
                    self.ended = true;
                    frames.extend(self.framer.finish());
                }
                Ok(read) => self.framer.feed(&self.buffer[..read], &mut frames),
                Err(error) => {
                    tracing::debug!(%error, "a peer link failed while it was read");
                    self.ended = true;
                    self.failure = Some(error);
                }
            }
            self.ready.extend(frames);
        }
    }
}
