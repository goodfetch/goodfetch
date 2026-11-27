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

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::group::{
    GROUP_MEMBER_EXISTS, HELLO, JOIN, JOIN_TOKEN_REFUSED, LEAVE, LeadEpoch, PEER_PROTOCOL_VERSION,
    PeerJoinParams, PeerJoinResult, PeerLeaveParams, PeerRenewParams, PeerRunIndexReportParams,
    RENEW, RUN_INDEX_REPORT,
};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer};
use rustls::server::ParsedCertificate;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

use super::identity::KeptIdentity;
use super::leadership::{ExchangeOutcome, LeadRefusal, LeadSnapshot, Leadership};
use super::operations::{internal, lead_unconfirmed, not_lead, store_failed};
use super::store::{self, Admission, Certified, Joining};
use super::{
    Answer, GroupAuthority, GroupMemberName, GroupMemberPublicKey, LeadEpochs, PeerLink,
    PeerTlsError, accepting, failure_line, opening_refusal, result_line, stale,
};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::endpoint::Refusals;
use crate::engine_context::EngineContext;
use crate::framing::{Call, Frame, Incoming, decode};
use crate::ops::connections_own;
use crate::product::Identity;
use crate::run_index::RunIndex;
use crate::settings::Settings;
use crate::sync::Mutex;
use crate::tasks::TaskOwner;
use crate::transport::{PeerListener, PeerStream, listen_peer};
use crate::user_store::UserStore;

use self::wire::{Lines, OPENING_TIMEOUT, too_large, write};

/// The most peer links open at once; the listener closes one past them.
const OPEN_AT_MOST: usize = 64;

/// How long the listener waits after a failure to accept that is not the
/// connection's own, such as too many open files.
const ACCEPT_PAUSE: Duration = Duration::from_secs(1);

/// Engine services shared with the peer listener (production always provides
/// all three; isolated loopback unit tests may leave unused ones empty).
#[derive(Clone, Default)]
pub(super) struct PeerServices {
    pub(super) settings: Option<Settings>,
    pub(super) run_index: Option<RunIndex>,
    pub(super) refusals: Option<Refusals>,
}

impl PeerServices {
    fn from_engine(engine: &EngineContext) -> Self {
        Self {
            settings: Some(engine.settings().clone()),
            run_index: Some(engine.run_index().clone()),
            refusals: Some(engine.refusals().clone()),
        }
    }
}

/// What the peer listener answers with.
pub(super) struct Peering {
    /// The Daemon's identity, whose Product the group's is.
    identity: Identity,
    /// Its Group member name.
    name: String,
    /// What the listener presented at its last link.
    presenting: Mutex<Presenting>,
    /// The group store.
    store: UserStore,
    clock: Clock,
    /// The lead epochs every peer link durably records; held across the
    /// SQLite write so no concurrent link observes an unpersisted raise.
    epochs: tokio::sync::Mutex<LeadEpochs>,
    /// Lead Daemon confirmation and fencing state (ADR 0055).
    leadership: Mutex<Leadership>,
    /// Engine services (settings, Run index, endpoint refusals).
    services: PeerServices,
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
    members: Vec<String>,
) -> Result<(), PeerTlsError> {
    let peering = Arc::new(Peering::new(
        identity,
        store,
        kept,
        lead_epoch,
        members,
        engine.clock(),
        PeerServices::from_engine(engine),
    )?);
    let group = engine.owner().part("daemon group");
    group.spawn(serve(
        listener,
        Arc::clone(&peering),
        group.part("peer link"),
    ));
    group.spawn(renewal::keep_renewed(
        Arc::clone(&peering),
        group.part("certificate renewal"),
    ));
    if peering.is_unconfirmed() {
        group.spawn(confirmation::confirm_majority(
            Arc::clone(&peering),
            group.part("lead confirmation"),
        ));
    }
    if !kept.is_lead() {
        group.spawn(run_index::feed_run_index(
            Arc::clone(&peering),
            group.part("run index feed"),
        ));
    }
    engine.hold_peer_link(group, peering);
    Ok(())
}

/// The running peer link state on `engine`, if this Daemon is currently in a
/// group.
pub(super) fn running(engine: &EngineContext) -> Option<Arc<Peering>> {
    engine.peer_link_state::<Peering>()
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
    let members = if kept.is_lead() {
        match store::members(&store).await {
            Ok(members) => members.into_iter().map(|member| member.name).collect(),
            Err(error) => {
                tracing::warn!(%error, "the group store's members cannot be read; the peer listener stays closed");
                return;
            }
        }
    } else {
        Vec::new()
    };
    let listener = match listen_peer(&kept.peer_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!(%error, address = %kept.peer_address, "the peer listener cannot listen again");
            return;
        }
    };
    if let Err(error) = start(
        &engine, identity, store, listener, &kept, lead_epoch, members,
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
        members: Vec<String>,
        clock: Clock,
        services: PeerServices,
    ) -> Result<Self, PeerTlsError> {
        let leadership = if kept.is_lead() {
            Leadership::starting(&kept.name, lead_epoch, members)
        } else {
            Leadership::not_lead(&kept.name)
        };
        Ok(Self {
            identity,
            name: kept.name.clone(),
            presenting: Mutex::new(Presenting::of(kept, clock)?),
            store,
            clock,
            epochs: tokio::sync::Mutex::new(LeadEpochs::new(lead_epoch)),
            leadership: Mutex::new(leadership),
            services,
        })
    }

    /// This Group member's name.
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// Whether this Lead Daemon is still waiting for majority confirmation.
    pub(super) fn is_unconfirmed(&self) -> bool {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_unconfirmed()
    }

    /// Last-known Group members that have not yet confirmed this Lead Daemon.
    pub(super) fn unconfirmed_peers(&self) -> Vec<String> {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unconfirmed_peers()
    }

    /// Records the lead epoch `peer_epoch` that `peer` answered with.
    pub(super) fn record_peer_exchange(
        &self,
        peer: &str,
        peer_epoch: LeadEpoch,
    ) -> ExchangeOutcome {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record_peer_exchange(peer, peer_epoch)
    }

    /// Fences this Lead Daemon when a higher lead epoch is observed.
    fn fence_if_higher(&self, higher: LeadEpoch) {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fence_if_higher(higher);
    }

    /// Requires that this Daemon is a confirmed Lead Daemon before acting.
    pub(super) fn require_confirmed_lead(&self) -> Result<LeadEpoch, CallError> {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .can_act()
            .map_err(lead_refusal)
    }

    /// Confirms this Lead Daemon by hand (`group lead --confirm`).
    pub(super) fn confirm_by_operator(&self) -> Result<LeadSnapshot, CallError> {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .confirm_by_operator()
            .map_err(lead_refusal)
    }

    /// Current Lead Daemon confirmation status (`group lead`).
    pub(super) fn lead_status(&self) -> Result<LeadSnapshot, CallError> {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .status()
            .map_err(lead_refusal)
    }

    /// Records an admitted Group member once confirmed.
    pub(super) fn on_member_added(&self, member: String) {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .on_member_added(member);
    }

    /// Records a removed Group member once confirmed.
    pub(super) fn on_member_removed(&self, member: &str) {
        self.leadership
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .on_member_removed(member);
    }

    /// The highest lead epoch durably recorded.
    pub(super) async fn highest_epoch(&self) -> LeadEpoch {
        self.epochs.lock().await.highest()
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

    /// The highest lead epoch durably recorded, once `lead_epoch`, which a
    /// Group member's message carries, is admitted and persisted.
    ///
    /// # Errors
    ///
    /// `lead_epoch_stale` when it is below the highest recorded, or the
    /// store's error if a raised epoch cannot be persisted.
    pub(super) async fn admit_epoch(&self, lead_epoch: LeadEpoch) -> Result<LeadEpoch, CallError> {
        let mut epochs = self.epochs.lock().await;
        let before = epochs.highest();
        if lead_epoch < before {
            return match epochs.admit(lead_epoch) {
                Ok(()) => Ok(epochs.highest()),
                Err(details) => Err(stale(&details)),
            };
        }
        if lead_epoch > before {
            match store::admit_epoch(&self.store, lead_epoch)
                .await
                .map_err(|error| store_failed(&error))?
            {
                Ok(persisted) => {
                    let _ = epochs.admit(persisted);
                    self.fence_if_higher(persisted);
                }
                Err(details) => {
                    *epochs = LeadEpochs::new(details.highest_lead_epoch);
                    self.fence_if_higher(details.highest_lead_epoch);
                    return Err(stale(&details));
                }
            }
        }
        Ok(epochs.highest())
    }

    /// Records a peer-link refusal in the Audit log when refusals are wired.
    async fn record_refusal(&self, reason: RefusalReason) {
        if let Some(refusals) = &self.services.refusals {
            refusals.refused(Listener::PeerLink, reason).await;
        }
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
                if let Some(reason) = handshake_refusal(&error) {
                    self.record_refusal(reason).await;
                }
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
                Opening::RunIndexReport(request) => {
                    Some(self.run_index_report(&presented, request).await)
                }
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
        let mut first_message = true;
        while let Some(received) = frame {
            let answer = match received {
                Frame::Line(sent) => {
                    if first_message && is_not_hello(&sent) {
                        self.record_refusal(RefusalReason::HelloNotFirst).await;
                    }
                    first_message = false;
                    let mut epochs = self.epochs.lock().await;
                    let before = epochs.highest();
                    let mut staged = epochs.clone();
                    let answer = link.receive(&mut staged, sent);
                    let after = staged.highest();
                    if after > before {
                        match store::admit_epoch(&self.store, after).await {
                            Ok(Ok(persisted)) => {
                                *epochs = LeadEpochs::new(persisted);
                                self.fence_if_higher(persisted);
                                answer
                            }
                            Ok(Err(details)) => {
                                *epochs = LeadEpochs::new(details.highest_lead_epoch);
                                self.fence_if_higher(details.highest_lead_epoch);
                                Answer {
                                    reply: Some(failure_line(None, &stale(&details))),
                                    close: answer.close,
                                }
                            }
                            Err(error) => Answer {
                                reply: Some(failure_line(None, &store_failed(&error))),
                                close: true,
                            },
                        }
                    } else {
                        answer
                    }
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
        let lead_epoch = self.require_confirmed_lead()?;
        if let Some(settings) = &self.services.settings {
            politeness::check_politeness_for_group(settings).await?;
        }
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
            Admission::Admitted => {
                self.on_member_added(name.as_str().to_owned());
                Ok(PeerJoinResult {
                    lead_name: self.name.clone(),
                    protocol_version: settled,
                    lead_epoch,
                    certificate: BASE64.encode(&signed.certificate),
                    certificate_expires_at: signed.not_after,
                    authority: BASE64.encode(authority.certificate()),
                    revocations: BASE64.encode(revocations),
                })
            }
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
    /// A `group.run_index.report`, likewise.
    RunIndexReport(Asked<PeerRunIndexReportParams>),
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
        RUN_INDEX_REPORT => asked(identity, id, call, |params: &PeerRunIndexReportParams| {
            (params.product, params.protocol_version)
        })
        .map(Opening::RunIndexReport),
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

/// Classifies a peer-link TLS handshake error into its endpoint-table Audit
/// refusal reason (ADR 0067).
fn handshake_refusal(error: &std::io::Error) -> Option<RefusalReason> {
    let inner = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())?;
    if let Some(CertificateFailure::Known(known)) = crate::tls::certificate_failure(inner) {
        return match known {
            KnownCertificateFailure::UnknownIssuer | KnownCertificateFailure::SelfSigned => {
                Some(RefusalReason::CertificateUnknownAuthority)
            }
            KnownCertificateFailure::Revoked => Some(RefusalReason::CertificateRevoked),
            KnownCertificateFailure::Expired | KnownCertificateFailure::NotYetValid => {
                Some(RefusalReason::CertificateExpired)
            }
            _ => None,
        };
    }
    matches!(inner, rustls::Error::InvalidMessage(_)).then_some(RefusalReason::TlsRequired)
}

/// Whether `line`, the first message on an authenticated peer link that was
/// neither `group.renew`, `group.leave`, nor `group.run_index.report`, is not
/// a `hello` request.
fn is_not_hello(line: &[u8]) -> bool {
    !matches!(
        decode(line.to_vec()),
        Ok(Incoming::Request { call, .. }) if call.method() == HELLO
    )
}

/// Maps a [`LeadRefusal`] to its wire [`CallError`].
fn lead_refusal(refusal: LeadRefusal) -> CallError {
    match refusal {
        LeadRefusal::NotLead => not_lead(),
        LeadRefusal::Unconfirmed {
            lead_epoch,
            confirmed_by,
            total_members,
        } => lead_unconfirmed(lead_epoch, confirmed_by, total_members),
        LeadRefusal::Fenced(details) => stale(&details),
    }
}

mod confirmation;
mod leaving;
mod politeness;
mod renewal;
mod run_index;
mod wire;

pub(super) use leaving::ask_to_leave;
pub use run_index::reindex_to_lead;
pub(super) use wire::{Asking, ask_to_join};
#[cfg(test)]
use wire::{ask, connect_peer_tls};
use wire::{ask_lead, ask_peer};

#[cfg(test)]
mod loopback;
#[cfg(test)]
mod tests;
