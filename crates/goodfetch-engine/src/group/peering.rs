//! The peer link's network side (ADR 0055): the peer listener a Group
//! member serves, and the one exchange of a Daemon that joins.
//!
//! The listener speaks mutual TLS 1.3 (ADR 0060). A Group member that
//! presents a certificate the group's authority signed opens a
//! [`PeerLink`]; a Daemon that presents none may send one `group.join`,
//! which the Lead Daemon answers before it closes the link. Each opening
//! message is checked in ADR 0095's order, the Product first.

use std::collections::VecDeque;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::group::{
    GROUP_LEAD_UNREACHABLE, GROUP_MEMBER_EXISTS, JOIN, JOIN_TOKEN_REFUSED, LeadEpoch,
    PEER_PROTOCOL_VERSION, PeerJoinParams, PeerJoinResult,
};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, ServerName};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::operations::{internal, not_lead, store_failed};
use super::peer_tls::key_hash_of;
use super::store::{self, Admission, Joining};
use super::{
    Answer, GroupAuthority, GroupMemberName, GroupMemberPublicKey, LeadEpochs, PeerIdentity,
    PeerLink, PeerTlsError, PeerTrust, accepting, failure_line, joining, opening_refusal,
    result_line,
};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::engine_context::EngineContext;
use crate::forward::answered_failure;
use crate::framing::{Frame, Framer, Incoming, decode};
use crate::known_certificates::KeyHash;
use crate::ops::connections_own;
use crate::product::Identity;
use crate::sync::Mutex;
use crate::tasks::TaskOwner;
use crate::transport::{PeerListener, PeerStream, connect_peer, listen_peer};
use crate::user_store::UserStore;
use crate::wire::request_line;

/// The most peer links open at once; the listener closes one past them.
const OPEN_AT_MOST: usize = 64;

/// How long a peer link has to finish its handshake and send its opening
/// message, and a Daemon that joins to have its answer.
const OPENING_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest message of the peer link; a longer one is refused.
const LINE_LIMIT: usize = 64 * 1024;

/// How many bytes one read of the peer link takes at most.
const READ_BYTES: usize = 8 * 1024;

/// How long the listener waits after a failure to accept that is not the
/// connection's own, such as too many open files.
const ACCEPT_PAUSE: Duration = Duration::from_secs(1);

/// The server name a Daemon that joins dials: its verifier checks the
/// authority it pinned, not a name, since it knows no Group member's yet.
const LEAD: &str = "lead-daemon";

/// What the peer listener answers with.
struct Peering {
    /// The Daemon's identity, whose Product the group's is.
    identity: Identity,
    /// The group store.
    store: UserStore,
    clock: Clock,
    /// The revocation list the Lead Daemon hands a Daemon that joins.
    revocations: CertificateRevocationListDer<'static>,
    /// The lead epochs every peer link records.
    epochs: Mutex<LeadEpochs>,
}

/// Starts the peer listener on `listener`, presenting `presented` and
/// trusting `trust`, for the Daemon `identity` of the group whose store is
/// `store` and whose highest lead epoch it recorded is `lead_epoch`; it
/// serves until the engine stops.
///
/// # Errors
///
/// [`PeerTlsError`] when the kept certificate, key or authority is
/// unusable.
pub(super) fn start(
    engine: &EngineContext,
    identity: Identity,
    store: UserStore,
    listener: PeerListener,
    presented: PeerIdentity,
    trust: PeerTrust,
    lead_epoch: LeadEpoch,
) -> Result<(), PeerTlsError> {
    let clock = engine.clock();
    let acceptor = TlsAcceptor::from(Arc::new(accepting(presented, &trust, clock)?));
    let peering = Arc::new(Peering {
        identity,
        store,
        clock,
        revocations: trust.revocations,
        epochs: Mutex::new(LeadEpochs::new(lead_epoch)),
    });
    let owner = engine.owner();
    owner.spawn(serve(listener, acceptor, peering, owner.part("peer link")));
    Ok(())
}

/// Serves the peer link again when the Daemon starts in a group: listens
/// at the address its group store keeps. A Daemon in no group, or whose
/// engine keeps no group store, listens for nothing.
pub(crate) async fn resume(engine: EngineContext, identity: Identity) {
    let Some(store) = engine.user_store(&store::STORE).cloned() else {
        return;
    };
    let (kept, lead_epoch) = match store::identity(&store).await {
        Ok(Some(kept)) => kept,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "the group store cannot be read; the peer listener stays closed");
            return;
        }
    };
    let listener = match listen_peer(&kept.peer_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!(%error, address = %kept.peer_address, "the peer listener cannot listen again");
            return;
        }
    };
    if let Err(error) = start(
        &engine,
        identity,
        store,
        listener,
        kept.presented(),
        kept.trust(),
        lead_epoch,
    ) {
        tracing::warn!(%error, "the peer listener cannot present the kept identity");
    }
}

/// Accepts peer links on `listener` until `owner` stops, each answered by
/// a task of its own: at most [`OPEN_AT_MOST`] at once.
///
/// # Cancel safety
///
/// Dropping the future stops accepting; the links are `owner`'s, which its
/// stop ends.
async fn serve(
    listener: PeerListener,
    acceptor: TlsAcceptor,
    peering: Arc<Peering>,
    owner: TaskOwner,
) {
    let open = Arc::new(Semaphore::new(OPEN_AT_MOST));
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok(stream) => {
                let Ok(permit) = Arc::clone(&open).try_acquire_owned() else {
                    tracing::debug!("a peer link past the ones open at once is closed");
                    continue;
                };
                let stopping = owner.token().clone();
                let acceptor = acceptor.clone();
                let peering = Arc::clone(&peering);
                owner.spawn(async move {
                    let _permit = permit;
                    tokio::select! {
                        () = peering.answer(&acceptor, stream) => {}
                        () = stopping.cancelled() => {}
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "the peer listener cannot accept a link");
                if !connections_own(&error) {
                    tokio::select! {
                        () = owner.token().cancelled() => return,
                        () = peering.clock.sleep(ACCEPT_PAUSE) => {}
                    }
                }
            }
        }
    }
}

impl Peering {
    /// Answers the peer link `stream` until it closes: a Group member's
    /// messages, or a Daemon's one `group.join`.
    async fn answer(&self, acceptor: &TlsAcceptor, stream: PeerStream) {
        let mut lines = Lines::new();
        let opened = tokio::select! {
            opened = async {
                let mut stream = acceptor.accept(stream).await?;
                let first = lines.next(&mut stream).await;
                Ok::<_, std::io::Error>((stream, first))
            } => opened,
            () = self.clock.sleep(OPENING_TIMEOUT) => {
                tracing::debug!("a peer link sent no opening message in time");
                return;
            }
        };
        let (mut stream, first) = match opened {
            Ok((stream, Some(first))) => (stream, first),
            Ok((_, None)) => return,
            Err(error) => {
                tracing::debug!(%error, "a peer link's handshake failed");
                return;
            }
        };
        if stream.get_ref().1.peer_certificates().is_some() {
            self.member(&mut stream, &mut lines, first).await;
        } else if let Some(reply) = self.join(first).await {
            write(&mut stream, &reply).await;
        }
        if let Err(error) = stream.shutdown().await {
            tracing::debug!(%error, "a peer link did not close cleanly");
        }
    }

    /// Answers a Group member's messages, `first` the first, until the
    /// link closes.
    async fn member(
        &self,
        stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
        lines: &mut Lines,
        first: Frame,
    ) {
        let mut link = PeerLink::accepting(self.identity.clone());
        let mut frame = Some(first);
        while let Some(received) = frame {
            let answer = match received {
                Frame::Line(sent) => {
                    let mut epochs = self.epochs.lock().unwrap_or_else(PoisonError::into_inner);
                    link.receive(&mut epochs, sent)
                }
                Frame::TooLarge => Answer {
                    reply: Some(failure_line(None, &too_large())),
                    close: false,
                },
            };
            if let Some(reply) = &answer.reply
                && !write(stream, reply).await
            {
                return;
            }
            if answer.close {
                return;
            }
            frame = lines.next(stream).await;
        }
    }

    /// The answer to a Daemon that presented no certificate and sent
    /// `first`: none when it sent no request.
    async fn join(&self, first: Frame) -> Option<Vec<u8>> {
        let line = match first {
            Frame::Line(line) => line,
            Frame::TooLarge => return Some(failure_line(None, &too_large())),
        };
        let request = match join_request(&self.identity, line) {
            Ok(request) => request,
            Err(refusal) => return refusal,
        };
        Some(match self.admit(request.settled, request.params).await {
            Ok(admitted) => result_line(&request.id, &admitted),
            Err(error) => failure_line(Some(request.id), &error),
        })
    }

    /// Admits the Daemon that asks with `params` into the group, the two
    /// speaking peer protocol `settled`: signs a Group member certificate
    /// for its public key once its join token and name are accepted.
    async fn admit(
        &self,
        settled: ProtocolVersion,
        params: PeerJoinParams,
    ) -> Result<PeerJoinResult, CallError> {
        let name = GroupMemberName::new(&params.name).map_err(|error| {
            CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
        })?;
        let public_key = BASE64
            .decode(params.public_key.as_bytes())
            .ok()
            .and_then(|raw| GroupMemberPublicKey::from_raw(raw).ok())
            .ok_or_else(|| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "`public_key` is an uncompressed P-256 point in standard base64",
                )
            })?;
        let kept = store::authority(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(not_lead)?;
        let authority = GroupAuthority::restore(kept).map_err(|error| internal(&error))?;
        let now = self.clock.wall();
        let signed = authority
            .sign(&name, &public_key, now)
            .map_err(|error| internal(&error))?;
        let joining = Joining {
            name: params.name,
            peer_address: params.peer_address,
        };
        match store::admit(&self.store, params.token.expose(), joining, now)
            .await
            .map_err(|error| store_failed(&error))?
        {
            Admission::Admitted => Ok(PeerJoinResult {
                protocol_version: settled,
                lead_epoch: self
                    .epochs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .highest(),
                certificate: BASE64.encode(&signed.certificate),
                certificate_expires_at: signed.not_after,
                authority: BASE64.encode(authority.certificate()),
                revocations: BASE64.encode(&self.revocations),
            }),
            Admission::NotLead => Err(not_lead()),
            Admission::Refused(refused) => Err(CallError::new(
                &JOIN_TOKEN_REFUSED,
                format!(
                    "The Lead Daemon refused the join token: {refused}. Mint another with `group token` on the Lead Daemon."
                ),
                &(),
            )),
            Admission::NameTaken => Err(CallError::new(
                &GROUP_MEMBER_EXISTS,
                format!(
                    "The Daemon group has a Group member named `{}` already; join under another name.",
                    name.as_str()
                ),
                &(),
            )),
        }
    }
}

/// A `group.join` a Daemon without a certificate sent, accepted for its
/// Product and version.
#[derive(Debug)]
struct JoinRequest {
    id: RequestId,
    /// The peer protocol version the two settle on.
    settled: ProtocolVersion,
    params: PeerJoinParams,
}

/// Reads `line`, the one message of a Daemon that presented no
/// certificate, as the Daemon `identity`: a `group.join` of its Product and
/// of a version it speaks, or the line refusing it, none when it sent no
/// request.
fn join_request(identity: &Identity, line: Vec<u8>) -> Result<JoinRequest, Option<Vec<u8>>> {
    let (id, call) = match decode(line) {
        Ok(Incoming::Request { id, call }) => (id, call),
        Ok(Incoming::Notification(_) | Incoming::Reply { .. }) => return Err(None),
        Err(error) => {
            return Err(Some(failure_line(
                error.id,
                &CallError::protocol(error.failure, error.detail),
            )));
        }
    };
    let refuse = |error: CallError| Err(Some(failure_line(Some(id.clone()), &error)));
    if call.method() != JOIN {
        return refuse(CallError::protocol(
            ProtocolFailure::InvalidRequest,
            "a Daemon without a Group member certificate sends `group.join` alone",
        ));
    }
    let params: PeerJoinParams = match call.into_params().read() {
        Ok(params) => params,
        Err(error) => {
            return refuse(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("this `group.join` is not the peer protocol's: {error}"),
            ));
        }
    };
    if let Some(error) = opening_refusal(identity, params.product, params.protocol_version) {
        return refuse(error);
    }
    Ok(JoinRequest {
        id,
        settled: PEER_PROTOCOL_VERSION.min(params.protocol_version),
        params,
    })
}

/// Who asks the Lead Daemon to join its group.
pub(super) struct Asking<'a> {
    /// The `host:port` of the Lead Daemon's peer listener.
    pub(super) lead: &'a str,
    /// The SHA-256 of the group's certificate authority's public key.
    pub(super) pin: KeyHash,
    pub(super) token: &'a DaemonToken,
    pub(super) name: &'a GroupMemberName,
    pub(super) public_key: &'a GroupMemberPublicKey,
    /// The `host:port` its own peer listener took.
    pub(super) peer_address: &'a str,
}

/// What a Daemon that joins receives: the Lead Daemon's answer, its
/// certificates and list read.
#[derive(Debug)]
pub(super) struct Admitted {
    pub(super) lead_epoch: LeadEpoch,
    pub(super) certificate: CertificateDer<'static>,
    pub(super) certificate_expires_at: std::time::SystemTime,
    pub(super) authority: CertificateDer<'static>,
    pub(super) revocations: CertificateRevocationListDer<'static>,
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
pub(super) async fn ask_to_join(
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
    let request = request_line(
        &RequestId::Number(1),
        JOIN,
        &PeerJoinParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: identity.product,
            product_version: identity.product_version.clone(),
            token: asking.token.clone(),
            name: asking.name.as_str().to_owned(),
            public_key: BASE64.encode(asking.public_key.raw()),
            peer_address: asking.peer_address.to_owned(),
        },
    )
    .map_err(|error| internal(&error))?;
    if !write(&mut stream, &request).await {
        return Err(unreachable(
            asking.lead,
            "the link closed before the request was sent",
        ));
    }
    let Some(Frame::Line(reply)) = Lines::new().next(&mut stream).await else {
        return Err(unreachable(
            asking.lead,
            "the link closed before its answer",
        ));
    };
    let unexpected = || {
        CallError::protocol(
            ProtocolFailure::InternalError,
            "the Lead Daemon's answer to `group.join` is not the peer protocol's",
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

/// `group_lead_unreachable` for the Lead Daemon at `lead`.
fn unreachable(lead: &str, why: impl std::fmt::Display) -> CallError {
    CallError::new(
        &GROUP_LEAD_UNREACHABLE,
        format!("No Lead Daemon answered at `{lead}`: {why}."),
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
fn too_large() -> CallError {
    CallError::protocol(
        ProtocolFailure::MessageTooLarge,
        format!("a peer link's message is longer than {LINE_LIMIT} bytes"),
    )
}

/// Writes `line` to `stream`: whether it went.
async fn write(stream: &mut (impl AsyncWrite + Unpin), line: &[u8]) -> bool {
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
struct Lines {
    framer: Framer,
    buffer: Vec<u8>,
    ready: VecDeque<Frame>,
    ended: bool,
}

impl Lines {
    fn new() -> Self {
        Self {
            framer: Framer::new(LINE_LIMIT),
            buffer: vec![0; READ_BYTES],
            ready: VecDeque::new(),
            ended: false,
        }
    }

    /// The next message `stream` sends, none once it ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no byte read: the framer keeps them.
    async fn next(&mut self, stream: &mut (impl AsyncRead + Unpin)) -> Option<Frame> {
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
                }
            }
            self.ready.extend(frames);
        }
    }
}

#[cfg(test)]
mod tests;
