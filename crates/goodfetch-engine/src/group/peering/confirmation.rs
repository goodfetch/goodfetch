//! Majority lead-epoch confirmation on a restarting or promoted Lead Daemon
//! (ADR 0055): the Lead Daemon exchanges `hello` (carrying its lead epoch)
//! over mutual TLS with its last-known Group members until a strict majority
//! (itself included) has durably recorded its epoch, or until a peer reports
//! a higher epoch and fences it off, or until an operator confirms it with
//! `group lead --confirm`.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::group::{
    HELLO, LEAD_EPOCH_STALE, LeadEpoch, PEER_PROTOCOL_VERSION, PeerHelloParams, PeerHelloResult,
};

use super::{Peering, ask_peer};
use crate::group::{ExchangeOutcome, store};
use crate::tasks::TaskOwner;

/// How long an unconfirmed Lead Daemon waits between confirmation rounds.
const CONFIRM_RETRY: Duration = Duration::from_millis(200);

/// Exchanges lead epochs with last-known Group members until `peering` is
/// confirmed by majority or operator, fenced by a higher epoch, or `owner`
/// stops.
///
/// # Cancel safety
///
/// Dropping the future stops further rounds; each exchange that finished has
/// already updated durable state.
pub(super) async fn confirm_majority(peering: Arc<Peering>, owner: TaskOwner) {
    peering.run_confirm_majority(owner).await;
}

impl Peering {
    async fn run_confirm_majority(self: Arc<Self>, owner: TaskOwner) {
        while self.is_unconfirmed() {
            tokio::select! {
                () = owner.token().cancelled() => return,
                () = self.confirm_round() => {}
            }
            if !self.is_unconfirmed() {
                return;
            }
            tokio::select! {
                () = owner.token().cancelled() => return,
                () = self.clock.sleep(CONFIRM_RETRY) => {}
            }
        }
    }
    /// One round of exchanging our lead epoch with every unconfirmed
    /// last-known Group member.
    pub(super) async fn confirm_round(&self) {
        let unconfirmed = self.unconfirmed_peers();
        if unconfirmed.is_empty() {
            return;
        }
        let Ok(Some((kept, _))) = store::identity(&self.store).await else {
            return;
        };
        let Ok(members) = store::members(&self.store).await else {
            return;
        };
        let lead_epoch = self.highest_epoch().await;
        let params = PeerHelloParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
        };
        for member in members {
            if !unconfirmed.contains(&member.name) || !self.is_unconfirmed() {
                continue;
            }
            match ask_peer::<PeerHelloResult>(
                &kept,
                &member.name,
                &member.peer_address,
                self.clock,
                HELLO,
                &params,
            )
            .await
            {
                Ok(answered) => {
                    if answered.lead_epoch > lead_epoch {
                        let _ = self.admit_epoch(answered.lead_epoch).await;
                    }
                    let outcome = self.record_peer_exchange(&member.name, answered.lead_epoch);
                    if matches!(
                        outcome,
                        ExchangeOutcome::Confirmed | ExchangeOutcome::Fenced(_)
                    ) {
                        return;
                    }
                }
                Err(error) if error.code() == LEAD_EPOCH_STALE.name => {
                    let higher = error
                        .details()
                        .and_then(|details| details.get("highest_lead_epoch"))
                        .and_then(serde_json::Value::as_u64)
                        .map_or_else(|| LeadEpoch::new(lead_epoch.get() + 1), LeadEpoch::new);
                    let _ = self.admit_epoch(higher).await;
                    let _ = self.record_peer_exchange(&member.name, higher);
                    return;
                }
                Err(error) => {
                    tracing::debug!(
                        peer = %member.name,
                        address = %member.peer_address,
                        %error,
                        "a Group member could not be reached for lead confirmation"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
