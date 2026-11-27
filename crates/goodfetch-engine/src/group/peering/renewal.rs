//! The renewal of each Group member's certificate (ADR 0055). A
//! certificate lives [`CERTIFICATE_LIFETIME`]; two thirds of the way
//! ([`renew_at`]) the Group member generates a new key and asks the Lead
//! Daemon to sign a certificate for it, with one `group.renew` on a peer
//! link it opens with the certificate it holds. The Lead Daemon signs it
//! when that certificate names the Group member and the Group member is
//! still in its group, and its own certificate it signs itself. The Group
//! member keeps the new certificate and presents it from its next peer
//! link on. A renewal that fails is tried again after a tenth of the time
//! the certificate has left, a second at least, until it expires.
//!
//! [`CERTIFICATE_LIFETIME`]: crate::group::CERTIFICATE_LIFETIME
//! [`renew_at`]: crate::group::identity::renew_at

use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime};

use data_encoding::BASE64;
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::group::{PEER_PROTOCOL_VERSION, PeerRenewParams, PeerRenewResult, RENEW};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer};
use rustls::server::ParsedCertificate;
use tokio::io::AsyncWriteExt;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::{OPENING_TIMEOUT, Peering, ask, public_key_of, unreachable};
use crate::call_error::CallError;
use crate::clock::rfc3339;
use crate::framing::{Frame, Incoming, decode};
use crate::group::identity::{KeptIdentity, Renewed};
use crate::group::operations::{internal, store_failed};
use crate::group::{
    GroupMemberKey, GroupMemberName, accepting, connecting, failure_line, opening_refusal,
    result_line, stale, store,
};
use crate::product::Identity;
use crate::tasks::TaskOwner;
use crate::transport::connect_peer;

/// The least time between two tries at renewing a certificate.
const RETRY_AT_LEAST: Duration = Duration::from_secs(1);

/// What the opening message of a peer link that presents a certificate
/// is.
pub(super) enum Opening {
    /// A `group.renew` of the group's Product and of a version this
    /// Daemon speaks.
    Renew(RenewRequest),
    /// The line refusing a `group.renew`.
    Refused(Vec<u8>),
    /// Another message, which opens a [`PeerLink`](crate::group::PeerLink).
    Other(Frame),
}

/// A `group.renew`, accepted for its Product and version.
pub(super) struct RenewRequest {
    id: RequestId,
    /// The peer protocol version the two settle on.
    settled: ProtocolVersion,
    params: PeerRenewParams,
}

/// What `first`, the opening message of a peer link that presents a
/// certificate, is to the Daemon `identity`.
pub(super) fn opening(identity: &Identity, first: Frame) -> Opening {
    let Frame::Line(line) = first else {
        return Opening::Other(first);
    };
    let Ok(Incoming::Request { id, call }) = decode(line.clone()) else {
        return Opening::Other(Frame::Line(line));
    };
    if call.method() != RENEW {
        return Opening::Other(Frame::Line(line));
    }
    let refuse = |error: CallError| Opening::Refused(failure_line(Some(id.clone()), &error));
    let params: PeerRenewParams = match call.into_params().read() {
        Ok(params) => params,
        Err(error) => {
            return refuse(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("this `group.renew` is not the peer protocol's: {error}"),
            ));
        }
    };
    if let Some(error) = opening_refusal(identity, params.product, params.protocol_version) {
        return refuse(error);
    }
    Opening::Renew(RenewRequest {
        settled: PEER_PROTOCOL_VERSION.min(params.protocol_version),
        id,
        params,
    })
}

/// Keeps the certificate of the Group member `peering` serves renewed,
/// until `owner` stops or the Daemon is in no group.
///
/// # Cancel safety
///
/// Dropping the future stops renewing; a renewal under way records all of
/// itself or nothing.
pub(super) async fn keep_renewed(peering: Arc<Peering>, owner: TaskOwner) {
    loop {
        let wait = tokio::select! {
            () = owner.token().cancelled() => return,
            wait = peering.renew_when_due() => wait,
        };
        let Some(wait) = wait else {
            return;
        };
        tokio::select! {
            () = owner.token().cancelled() => return,
            () = peering.clock.sleep(wait) => {}
        }
    }
}

impl Peering {
    /// The answer to the `group.renew` `request` of the Group member that
    /// presented `presented`.
    pub(super) async fn renewal(
        &self,
        presented: &CertificateDer<'_>,
        request: RenewRequest,
    ) -> Vec<u8> {
        match self.renew(presented, request.settled, request.params).await {
            Ok(renewed) => result_line(&request.id, &renewed),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    /// Signs a new certificate for the Group member that asks with
    /// `params`, the two speaking peer protocol `settled`, once its lead
    /// epoch is current, the certificate it `presented` names it and it is
    /// a Group member still.
    async fn renew(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRenewParams,
    ) -> Result<PeerRenewResult, CallError> {
        let lead_epoch = {
            let mut epochs = self.epochs.lock().unwrap_or_else(PoisonError::into_inner);
            epochs
                .admit(params.lead_epoch)
                .map_err(|details| stale(&details))?;
            epochs.highest()
        };
        let name = GroupMemberName::new(&params.name).map_err(|error| {
            CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
        })?;
        if !names(presented, &name) {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "the certificate this peer link presents is not `{}`'s",
                    name.as_str()
                ),
            ));
        }
        let public_key = public_key_of(&params.public_key)?;
        let authority = self.authority().await?;
        if !store::is_member(&self.store, params.name)
            .await
            .map_err(|error| store_failed(&error))?
        {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                format!("`{}` is no Group member of this group", name.as_str()),
            ));
        }
        let signed = authority
            .sign(&name, &public_key, self.clock.wall())
            .map_err(|error| internal(&error))?;
        Ok(PeerRenewResult {
            protocol_version: settled,
            lead_epoch,
            certificate: BASE64.encode(&signed.certificate),
            certificate_expires_at: signed.not_after,
            revocations: BASE64.encode(&self.revocations),
        })
    }

    /// Renews this Group member's certificate once its time has come: how
    /// long to wait before it looks again, none once it is in no group or
    /// its certificate expired.
    async fn renew_when_due(&self) -> Option<Duration> {
        let kept = match store::identity(&self.store).await {
            Ok(Some((kept, _))) => kept,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, "the group store cannot be read; the certificate's renewal waits");
                return Some(RETRY_AT_LEAST);
            }
        };
        let now = self.clock.wall();
        if let Some(wait) = kept
            .renew_at
            .duration_since(now)
            .ok()
            .filter(|wait| !wait.is_zero())
        {
            return Some(wait);
        }
        let Ok(left) = kept.certificate_expires_at.duration_since(now) else {
            tracing::warn!(
                name = %kept.name,
                "this Group member's certificate expired before it was renewed; it cannot open a peer link until it joins the group again"
            );
            return None;
        };
        let retry = (left / 10).max(RETRY_AT_LEAST);
        match self.renewed(kept, now).await {
            Ok(renewed) => {
                tracing::info!(
                    name = %renewed.name,
                    expires_at = %rfc3339(renewed.certificate_expires_at),
                    "the Group member's certificate is renewed"
                );
                Some(
                    renewed
                        .renew_at
                        .duration_since(self.clock.wall())
                        .unwrap_or_default(),
                )
            }
            Err(error) => {
                tracing::warn!(%error, retry_in = ?retry, "the Group member's certificate could not be renewed; it tries again");
                Some(retry)
            }
        }
    }

    /// `kept` with a certificate renewed at `now`, which the group store
    /// keeps and the peer listener presents from its next link on.
    async fn renewed(
        &self,
        kept: KeptIdentity,
        now: SystemTime,
    ) -> Result<KeptIdentity, CallError> {
        let renewed = if kept.is_lead() {
            self.sign_own(&kept, now).await?
        } else {
            self.ask_lead(&kept).await?
        };
        let renewed = kept.with(renewed);
        let acceptor = accepting(renewed.presented(), &renewed.trust(), self.clock)
            .map_err(|error| internal(&error))?;
        if !store::renew(&self.store, renewed.clone())
            .await
            .map_err(|error| store_failed(&error))?
        {
            return Err(CallError::protocol(
                ProtocolFailure::InternalError,
                "this Daemon left its group while its certificate was renewed",
            ));
        }
        *self.acceptor.lock().unwrap_or_else(PoisonError::into_inner) =
            TlsAcceptor::from(Arc::new(acceptor));
        Ok(renewed)
    }

    /// The Lead Daemon's own certificate, signed at `now` for a new key.
    async fn sign_own(&self, kept: &KeptIdentity, now: SystemTime) -> Result<Renewed, CallError> {
        let authority = self.authority().await?;
        let name = GroupMemberName::new(&kept.name).map_err(|error| internal(&error))?;
        let key = GroupMemberKey::generate().map_err(|error| internal(&error))?;
        let signed = authority
            .sign(&name, &key.public_key(), now)
            .map_err(|error| internal(&error))?;
        Ok(Renewed {
            key: key.kept(),
            certificate: signed.certificate,
            expires_at: signed.not_after,
            revocations: kept.revocations.clone(),
            received_at: now,
        })
    }

    /// The certificate the Lead Daemon signs for a new key of the Group
    /// member `kept`, which asks for it on a peer link it opens with the
    /// certificate it holds.
    ///
    /// # Errors
    ///
    /// `group_lead_unreachable` when the Lead Daemon gives no answer in
    /// time, `lead_epoch_stale` when it answers with a lead epoch below the
    /// highest recorded, and the Lead Daemon's own refusal.
    async fn ask_lead(&self, kept: &KeptIdentity) -> Result<Renewed, CallError> {
        let lead = kept.lead_address.as_str();
        let asked = async {
            let name = GroupMemberName::new(&kept.lead_name).map_err(|error| internal(&error))?;
            let config = connecting(kept.presented(), &kept.trust(), self.clock)
                .map_err(|error| internal(&error))?;
            let stream = connect_peer(lead)
                .await
                .map_err(|error| unreachable(lead, &error))?;
            let mut stream = TlsConnector::from(Arc::new(config))
                .connect(name.server_name(), stream)
                .await
                .map_err(|error| unreachable(lead, &error))?;
            let key = GroupMemberKey::generate().map_err(|error| internal(&error))?;
            let params = PeerRenewParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch: self
                    .epochs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .highest(),
                name: kept.name.clone(),
                public_key: BASE64.encode(key.public_key().raw()),
            };
            let answered: PeerRenewResult = ask(&mut stream, lead, RENEW, &params).await?;
            if let Err(error) = stream.shutdown().await {
                tracing::debug!(%error, "a peer link did not close cleanly");
            }
            Ok::<_, CallError>((key, answered))
        };
        let (key, answered) = tokio::select! {
            asked = asked => asked?,
            () = self.clock.sleep(OPENING_TIMEOUT) => {
                return Err(unreachable(lead, "it gave no answer in time"));
            }
        };
        self.epochs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(answered.lead_epoch)
            .map_err(|details| stale(&details))?;
        let decoded = |text: &str| {
            BASE64.decode(text.as_bytes()).map_err(|_| {
                CallError::protocol(
                    ProtocolFailure::InternalError,
                    "the Lead Daemon's answer to `group.renew` holds no readable certificate",
                )
            })
        };
        Ok(Renewed {
            key: key.kept(),
            certificate: CertificateDer::from(decoded(&answered.certificate)?),
            expires_at: answered.certificate_expires_at,
            revocations: CertificateRevocationListDer::from(decoded(&answered.revocations)?),
            received_at: self.clock.wall(),
        })
    }
}

/// Whether `certificate` names the Group member `name`.
fn names(certificate: &CertificateDer<'_>, name: &GroupMemberName) -> bool {
    ParsedCertificate::try_from(certificate).is_ok_and(|parsed| {
        rustls::client::verify_server_name(&parsed, &name.server_name()).is_ok()
    })
}

#[cfg(test)]
mod tests;
