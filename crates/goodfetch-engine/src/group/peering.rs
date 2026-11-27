//! The peer link's network side (ADR 0055): the peer listener a Group
//! member serves, and the one exchange of a Daemon that joins.
//!
//! The listener speaks mutual TLS 1.3 (ADR 0060). A Group member that
//! presents a certificate the group's authority signed opens a
//! [`PeerLink`], or sends one `group.renew` to have that certificate
//! renewed ([`renewal`]) or one `group.leave` to leave the group
//! ([`leaving`]); a Daemon that presents none may send one `group.join`.
//! The Lead Daemon answers each of these before it closes the link. Each
//! opening message is checked in ADR 0095's order, the Product first.
//!
//! The listener presents the identity the group store keeps and trusts
//! the revocation list it holds, both read again at each link, so a
//! renewal or a revocation counts from the next link on.

use std::collections::VecDeque;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::group::{
    GROUP_LEAD_UNREACHABLE, GROUP_MEMBER_EXISTS, GROUP_MEMBER_UNKNOWN, JOIN, JOIN_TOKEN_REFUSED,
    LEAVE, LeadEpoch, PEER_PROTOCOL_VERSION, PeerJoinParams, PeerJoinResult, PeerLeaveParams,
    PeerRenewParams, RENEW,
};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, ServerName};
use rustls::server::ParsedCertificate;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::identity::KeptIdentity;
use super::operations::{internal, not_lead, store_failed};
use super::peer_tls::key_hash_of;
use super::store::{self, Admission, Certified, Joining};
use super::{
    Answer, GroupAuthority, GroupMemberName, GroupMemberPublicKey, LeadEpochs, PeerLink,
    PeerTlsError, accepting, connecting, failure_line, joining, opening_refusal, result_line,
    stale,
};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::engine_context::EngineContext;
use crate::forward::answered_failure;
use crate::framing::{Call, Frame, Framer, Incoming, decode};
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
    /// Its Group member name.
    name: String,
    /// What the listener presented at its last link.
    presenting: Mutex<Presenting>,
    /// The group store.
    store: UserStore,
    clock: Clock,
    /// The lead epochs every peer link records.
    epochs: Mutex<LeadEpochs>,
}

/// The TLS side of the listener, and the certificate and revocation list
/// it was made of.
#[derive(Clone)]
struct Presenting {
    certificate: CertificateDer<'static>,
    /// The list it enforces, which the Lead Daemon hands a Group member
    /// that joins or renews.
    revocations: CertificateRevocationListDer<'static>,
    acceptor: TlsAcceptor,
}

impl Presenting {
    /// The TLS side presenting `kept` at `clock`.
    ///
    /// # Errors
    ///
    /// [`PeerTlsError`] when the kept certificate, key or authority is
    /// unusable.
    fn of(kept: &KeptIdentity, clock: Clock) -> Result<Self, PeerTlsError> {
        Ok(Self {
            certificate: kept.certificate.clone(),
            revocations: kept.revocations.clone(),
            acceptor: TlsAcceptor::from(Arc::new(accepting(
                kept.presented(),
                &kept.trust(),
                clock,
            )?)),
        })
    }
}

/// Starts the peer listener on `listener`, presenting `kept`'s
/// certificate, for the Daemon `identity` of the group whose store is
/// `store` and whose highest lead epoch it recorded is `lead_epoch`, and
/// keeps that certificate renewed; both run until the engine stops or the
/// Daemon leaves its group ([`stop`]).
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
    kept: &KeptIdentity,
    lead_epoch: LeadEpoch,
) -> Result<(), PeerTlsError> {
    let peering = Arc::new(Peering::new(
        identity,
        store,
        kept,
        lead_epoch,
        engine.clock(),
    )?);
    let group = engine.owner().part("daemon group");
    group.spawn(serve(
        listener,
        Arc::clone(&peering),
        group.part("peer link"),
    ));
    group.spawn(renewal::keep_renewed(
        peering,
        group.part("certificate renewal"),
    ));
    engine.hold_peer_link(group);
    Ok(())
}

/// Stops the peer listener and the certificate's renewal of a Daemon that
/// left its group: once this answers, the listener's address is free.
pub(super) async fn stop(engine: &EngineContext) {
    engine.stop_peer_link().await;
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
    if let Err(error) = start(&engine, identity, store, listener, &kept, lead_epoch) {
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
async fn serve(listener: PeerListener, peering: Arc<Peering>, owner: TaskOwner) {
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
                let peering = Arc::clone(&peering);
                owner.spawn(async move {
                    let _permit = permit;
                    tokio::select! {
                        () = peering.answer(stream) => {}
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
    /// What the Daemon `identity`, the Group member `kept` of the group
    /// whose store is `store` and whose highest lead epoch it recorded is
    /// `lead_epoch`, answers with, at `clock`.
    ///
    /// # Errors
    ///
    /// [`PeerTlsError`] when the kept certificate, key or authority is
    /// unusable.
    fn new(
        identity: Identity,
        store: UserStore,
        kept: &KeptIdentity,
        lead_epoch: LeadEpoch,
        clock: Clock,
    ) -> Result<Self, PeerTlsError> {
        Ok(Self {
            identity,
            name: kept.name.clone(),
            presenting: Mutex::new(Presenting::of(kept, clock)?),
            store,
            clock,
            epochs: Mutex::new(LeadEpochs::new(lead_epoch)),
        })
    }

    /// What the listener presents now: the identity the group store keeps,
    /// made again when its certificate or revocation list changed since
    /// the last link; none once the Daemon left its group. A store that
    /// cannot be read, or an identity that cannot be presented, leaves the
    /// last one in place.
    async fn presenting(&self) -> Option<Presenting> {
        let kept = match store::identity(&self.store).await {
            Ok(Some((kept, _))) => Some(kept),
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, "the group store cannot be read; the peer listener presents what it read last");
                None
            }
        };
        let mut presenting = self
            .presenting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(kept) = kept
            && (kept.certificate != presenting.certificate
                || kept.revocations != presenting.revocations)
        {
            match Presenting::of(&kept, self.clock) {
                Ok(current) => *presenting = current,
                Err(error) => {
                    tracing::warn!(%error, "the group store's identity cannot be presented; the peer listener presents what it read last");
                }
            }
        }
        Some(presenting.clone())
    }

    /// The highest lead epoch recorded, once `lead_epoch`, which a Group
    /// member's message carries, is admitted.
    ///
    /// # Errors
    ///
    /// `lead_epoch_stale` when it is below the highest recorded.
    fn admit_epoch(&self, lead_epoch: LeadEpoch) -> Result<LeadEpoch, CallError> {
        let mut epochs = self.epochs.lock().unwrap_or_else(PoisonError::into_inner);
        epochs
            .admit(lead_epoch)
            .map_err(|details| stale(&details))?;
        Ok(epochs.highest())
    }

    /// The group's certificate authority, which the Lead Daemon alone
    /// keeps.
    ///
    /// # Errors
    ///
    /// `not_lead_daemon` on another Group member, and the store's.
    async fn authority(&self) -> Result<GroupAuthority, CallError> {
        let kept = store::authority(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(not_lead)?;
        GroupAuthority::restore(kept).map_err(|error| internal(&error))
    }

    /// Answers the peer link `stream` until it closes: a Group member's
    /// messages or its one `group.renew` or `group.leave`, or a Daemon's
    /// one `group.join`.
    async fn answer(&self, stream: PeerStream) {
        let mut lines = Lines::new();
        let opened = tokio::select! {
            opened = async {
                let Some(presenting) = self.presenting().await else {
                    tracing::debug!("a peer link reached a Daemon that left its group");
                    return Ok(None);
                };
                let mut stream = presenting.acceptor.accept(stream).await?;
                let first = lines.next(&mut stream).await;
                Ok::<_, std::io::Error>(first.map(|first| (presenting.revocations, stream, first)))
            } => opened,
            () = self.clock.sleep(OPENING_TIMEOUT) => {
                tracing::debug!("a peer link sent no opening message in time");
                return;
            }
        };
        let (revocations, mut stream, first) = match opened {
            Ok(Some(opened)) => opened,
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(%error, "a peer link's handshake failed");
                return;
            }
        };
        let presented = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|chain| chain.first())
            .cloned();
        let reply = if let Some(presented) = presented {
            match opening(&self.identity, first) {
                Opening::Renew(request) => {
                    Some(self.renewal(&presented, &revocations, request).await)
                }
                Opening::Leave(request) => Some(self.leaving(&presented, request).await),
                Opening::Refused(reply) => Some(reply),
                Opening::Other(first) => {
                    self.member(&mut stream, &mut lines, first).await;
                    None
                }
            }
        } else {
            self.join(first, &revocations).await
        };
        if let Some(reply) = reply {
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
    /// `first`, the listener enforcing `revocations`: none when it sent no
    /// request.
    async fn join(
        &self,
        first: Frame,
        revocations: &CertificateRevocationListDer<'_>,
    ) -> Option<Vec<u8>> {
        let line = match first {
            Frame::Line(line) => line,
            Frame::TooLarge => return Some(failure_line(None, &too_large())),
        };
        let request = match join_request(&self.identity, line) {
            Ok(request) => request,
            Err(refusal) => return refusal,
        };
        Some(
            match self
                .admit(request.settled, request.params, revocations)
                .await
            {
                Ok(admitted) => result_line(&request.id, &admitted),
                Err(error) => failure_line(Some(request.id), &error),
            },
        )
    }

    /// Admits the Daemon that asks with `params` into the group, the two
    /// speaking peer protocol `settled`: signs a Group member certificate
    /// for its public key once its join token and name are accepted, and
    /// hands it `revocations`, the list the listener enforces.
    async fn admit(
        &self,
        settled: ProtocolVersion,
        params: PeerJoinParams,
        revocations: &CertificateRevocationListDer<'_>,
    ) -> Result<PeerJoinResult, CallError> {
        let name = GroupMemberName::new(&params.name).map_err(|error| {
            CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
        })?;
        let public_key = public_key_of(&params.public_key)?;
        let authority = self.authority().await?;
        let now = self.clock.wall();
        let signed = authority
            .sign(&name, &public_key, now)
            .map_err(|error| internal(&error))?;
        let joining = Joining {
            name: params.name,
            peer_address: params.peer_address,
            certificate: Certified::from(&signed),
        };
        match store::admit(&self.store, params.token.expose(), joining, now)
            .await
            .map_err(|error| store_failed(&error))?
        {
            Admission::Admitted => Ok(PeerJoinResult {
                lead_name: self.name.clone(),
                protocol_version: settled,
                lead_epoch: self
                    .epochs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .highest(),
                certificate: BASE64.encode(&signed.certificate),
                certificate_expires_at: signed.not_after,
                authority: BASE64.encode(authority.certificate()),
                revocations: BASE64.encode(revocations),
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

/// The public key a Daemon sent as `text` for the Lead Daemon to sign.
///
/// # Errors
///
/// `invalid_params` when it is no uncompressed P-256 point in standard
/// base64.
fn public_key_of(text: &str) -> Result<GroupMemberPublicKey, CallError> {
    BASE64
        .decode(text.as_bytes())
        .ok()
        .and_then(|raw| GroupMemberPublicKey::from_raw(raw).ok())
        .ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                "`public_key` is an uncompressed P-256 point in standard base64",
            )
        })
}

/// The name `name` a Group member gives, which the certificate it
/// `presented` must name.
///
/// # Errors
///
/// `invalid_params` when `name` is no Group member name, or the
/// certificate names another.
fn presented_name(
    presented: &CertificateDer<'_>,
    name: &str,
) -> Result<GroupMemberName, CallError> {
    let name = GroupMemberName::new(name)
        .map_err(|error| CallError::protocol(ProtocolFailure::InvalidParams, error.to_string()))?;
    let names = ParsedCertificate::try_from(presented).is_ok_and(|parsed| {
        rustls::client::verify_server_name(&parsed, &name.server_name()).is_ok()
    });
    if !names {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "the certificate this peer link presents is not `{}`'s",
                name.as_str()
            ),
        ));
    }
    Ok(name)
}

/// A request a peer link opens with, accepted for its Product and version.
#[derive(Debug)]
struct Asked<P> {
    id: RequestId,
    /// The peer protocol version the two settle on.
    settled: ProtocolVersion,
    params: P,
}

/// What the opening message of a peer link that presents a certificate
/// is.
enum Opening {
    /// A `group.renew` of the group's Product and of a version this
    /// Daemon speaks.
    Renew(Asked<PeerRenewParams>),
    /// A `group.leave`, likewise.
    Leave(Asked<PeerLeaveParams>),
    /// The line refusing either.
    Refused(Vec<u8>),
    /// Another message, which opens a [`PeerLink`].
    Other(Frame),
}

/// What `first`, the opening message of a peer link that presents a
/// certificate, is to the Daemon `identity`.
fn opening(identity: &Identity, first: Frame) -> Opening {
    let Frame::Line(line) = first else {
        return Opening::Other(first);
    };
    let Ok(Incoming::Request { id, call }) = decode(line.clone()) else {
        return Opening::Other(Frame::Line(line));
    };
    let read = match call.method() {
        RENEW => asked(identity, id, call, |params: &PeerRenewParams| {
            (params.product, params.protocol_version)
        })
        .map(Opening::Renew),
        LEAVE => asked(identity, id, call, |params: &PeerLeaveParams| {
            (params.product, params.protocol_version)
        })
        .map(Opening::Leave),
        _ => return Opening::Other(Frame::Line(line)),
    };
    read.unwrap_or_else(Opening::Refused)
}

/// `call`, the request `id`, read as the peer protocol's params `P` by the
/// Daemon `identity` and accepted for the Product and version
/// `opened_by` reads of them, or the line refusing it.
fn asked<P: DeserializeOwned>(
    identity: &Identity,
    id: RequestId,
    call: Call,
    opened_by: impl FnOnce(&P) -> (Product, ProtocolVersion),
) -> Result<Asked<P>, Vec<u8>> {
    let method = call.method().to_owned();
    let refuse = |error: CallError| failure_line(Some(id.clone()), &error);
    let params: P = call.into_params().read().map_err(|error| {
        refuse(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("this `{method}` is not the peer protocol's: {error}"),
        ))
    })?;
    let (product, version) = opened_by(&params);
    if let Some(error) = opening_refusal(identity, product, version) {
        return Err(refuse(error));
    }
    Ok(Asked {
        id,
        settled: PEER_PROTOCOL_VERSION.min(version),
        params,
    })
}

/// Reads `line`, the one message of a Daemon that presented no
/// certificate, as the Daemon `identity`: a `group.join` of its Product and
/// of a version it speaks, or the line refusing it, none when it sent no
/// request.
fn join_request(
    identity: &Identity,
    line: Vec<u8>,
) -> Result<Asked<PeerJoinParams>, Option<Vec<u8>>> {
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
    if call.method() != JOIN {
        return Err(Some(failure_line(
            Some(id),
            &CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "a Daemon without a Group member certificate sends `group.join` alone",
            ),
        )));
    }
    asked(identity, id, call, |params: &PeerJoinParams| {
        (params.product, params.protocol_version)
    })
    .map_err(Some)
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
    /// The Lead Daemon's Group member name.
    pub(super) lead_name: String,
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

/// Sends the Lead Daemon of the Group member `kept` the one request
/// `method` with `params`, on a peer link it opens with its certificate
/// at `clock`, and reads its answer.
///
/// # Errors
///
/// `group_lead_unreachable` when the Lead Daemon gives no answer in time,
/// `group_member_unknown` when it refuses the certificate as revoked, and
/// its own refusal.
async fn ask_lead<T: DeserializeOwned>(
    kept: &KeptIdentity,
    clock: Clock,
    method: &str,
    params: &impl Serialize,
) -> Result<T, CallError> {
    let lead = kept.lead_address.as_str();
    let asked = async {
        let name = GroupMemberName::new(&kept.lead_name).map_err(|error| internal(&error))?;
        let config =
            connecting(kept.presented(), &kept.trust(), clock).map_err(|error| internal(&error))?;
        let stream = connect_peer(lead)
            .await
            .map_err(|error| unreachable(lead, &error))?;
        let mut stream = TlsConnector::from(Arc::new(config))
            .connect(name.server_name(), stream)
            .await
            .map_err(|error| unreachable(lead, &error))?;
        let answered = ask(&mut stream, lead, method, params).await?;
        if let Err(error) = stream.shutdown().await {
            tracing::debug!(%error, "a peer link did not close cleanly");
        }
        Ok(answered)
    };
    tokio::select! {
        asked = asked => asked,
        () = clock.sleep(OPENING_TIMEOUT) => Err(unreachable(lead, "it gave no answer in time")),
    }
}

/// Sends the Lead Daemon at `lead` the one request `method` with `params`
/// on the peer link `stream` and reads its answer.
///
/// # Errors
///
/// `group_lead_unreachable` when the link closes first,
/// `group_member_unknown` when the Lead Daemon closes it on this side's
/// certificate as revoked, and the Lead Daemon's own refusal.
async fn ask<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    lead: &str,
    method: &str,
    params: &impl Serialize,
) -> Result<T, CallError> {
    let request =
        request_line(&RequestId::Number(1), method, params).map_err(|error| internal(&error))?;
    if !write(stream, &request).await {
        return Err(unreachable(
            lead,
            "the link closed before the request was sent",
        ));
    }
    let mut lines = Lines::new();
    let Some(Frame::Line(reply)) = lines.next(stream).await else {
        if lines.refused_as_revoked() {
            return Err(CallError::new(
                &GROUP_MEMBER_UNKNOWN,
                format!(
                    "The Lead Daemon at `{lead}` refused this Daemon's certificate as revoked: it is no Group member any more."
                ),
                &(),
            ));
        }
        return Err(unreachable(lead, "the link closed before its answer"));
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
    /// The read failure that ended it, if one did.
    failure: Option<std::io::Error>,
}

impl Lines {
    fn new() -> Self {
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
                    self.failure = Some(error);
                }
            }
            self.ready.extend(frames);
        }
    }
}

mod leaving;
mod renewal;

pub(super) use leaving::ask_to_leave;

#[cfg(test)]
mod loopback;
#[cfg(test)]
mod tests;
