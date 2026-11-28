//! A Group member leaving its group (ADR 0055): it sends the Lead Daemon
//! one `group.leave` on a peer link it opens with its certificate, and the
//! Lead Daemon revokes every certificate it signed for it, as
//! `group revoke` does, before it answers. The Lead Daemon cannot leave.

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::group::{
    GROUP_MEMBER_UNKNOWN, LEAVE, LeadEpoch, PEER_PROTOCOL_VERSION, PeerLeaveParams, PeerLeaveResult,
};
use rustls::pki_types::CertificateDer;

use super::{Asked, Peering, ask_lead, presented_name};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::group::errors::{revoked, store_failed};
use crate::group::identity::KeptIdentity;
use crate::group::{failure_line, result_line, store};
use crate::product::Identity;

impl Peering {
    /// The answer to the `group.leave` `request` of the Group member that
    /// presented `presented`.
    pub(super) async fn leaving(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerLeaveParams>,
    ) -> Vec<u8> {
        match self
            .remove(presented, request.settled, request.params)
            .await
        {
            Ok(left) => result_line(&request.id, &left),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    /// Removes the Group member that asks with `params` from the group,
    /// the two speaking peer protocol `settled`, once its lead epoch is
    /// current and the certificate it `presented` names it.
    async fn remove(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerLeaveParams,
    ) -> Result<PeerLeaveResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let name = presented_name(presented, &params.name)?;
        let revocation = store::revoke(&self.store, params.name, self.clock.wall())
            .await
            .map_err(|error| store_failed(&error))?;
        revoked(revocation, &name)?;
        self.on_member_removed(name.as_str());
        Ok(PeerLeaveResult {
            protocol_version: settled,
            lead_epoch,
        })
    }
}

/// Asks the Lead Daemon of the Group member `kept`, which recorded
/// `lead_epoch` as the highest, to remove it from the group, as the Daemon
/// `identity`, at `clock`. A Lead Daemon that knows no Group member of its
/// name, or refuses its certificate as revoked, removed it already.
///
/// # Errors
///
/// `group_lead_unreachable` when the Lead Daemon gives no answer in time,
/// and its own refusal.
pub(in crate::group) async fn ask_to_leave(
    kept: &KeptIdentity,
    lead_epoch: LeadEpoch,
    identity: &Identity,
    clock: Clock,
) -> Result<(), CallError> {
    let params = PeerLeaveParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity.product,
        product_version: identity.product_version.clone(),
        lead_epoch,
        name: kept.name.clone(),
    };
    match ask_lead::<PeerLeaveResult>(kept, clock, LEAVE, &params).await {
        Ok(_) => Ok(()),
        Err(error) if error.code() == GROUP_MEMBER_UNKNOWN.name => {
            tracing::info!(name = %kept.name, "the Lead Daemon had removed this Group member already");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
