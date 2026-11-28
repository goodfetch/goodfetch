//! Group-wide erasure broadcasts (ADRs 0046 and 0055, issue 258):
//!
//! When a Run is discarded or a Saved identity is deleted on any Group member,
//! the Lead Daemon erases its own local stores (Response cache included) and
//! broadcasts `group.erasure` over mutual TLS to every active Group member,
//! collecting acknowledgements and retrying any temporarily unreachable member
//! until acknowledged.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::group::{
    ErasureTarget, GROUP_ERASURE, PEER_PROTOCOL_VERSION, PeerErasureParams, PeerErasureResult,
};
use rustls::pki_types::CertificateDer;
use tokio::sync::Mutex;

use super::{Asked, Peering, ask_lead, ask_peer, presented_name};
use crate::call_error::CallError;
use crate::engine_context::ErasureBroadcaster;
use crate::group::operations::{internal, store_failed};
use crate::group::{failure_line, result_line, store};
use crate::tasks::TaskOwner;

/// How often the Lead Daemon retries erasures for Group members that were
/// temporarily unreachable during the initial broadcast.
const ERASURE_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// Pending erasure awaiting acknowledgement from one unreachable Group member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingErasure {
    pub(super) member: String,
    pub(super) target: ErasureTarget,
}

/// Queue of erasures awaiting delivery to temporarily offline Group members.
#[derive(Debug, Default)]
pub(super) struct PendingErasures {
    items: Mutex<Vec<PendingErasure>>,
}

impl PendingErasures {
    async fn push(&self, member: String, target: ErasureTarget) {
        let mut items = self.items.lock().await;
        let entry = PendingErasure { member, target };
        if !items.contains(&entry) {
            items.push(entry);
        }
    }

    async fn snapshot(&self) -> Vec<PendingErasure> {
        self.items.lock().await.clone()
    }

    async fn mark_acknowledged(&self, member: &str, target: &ErasureTarget) {
        self.items
            .lock()
            .await
            .retain(|item| !(item.member == member && &item.target == target));
    }
}

/// Engine hook bridging `EngineContext::broadcast_erasure` into the running
/// Daemon group [`Peering`].
pub(super) struct GroupErasureBroadcaster {
    peering: Arc<Peering>,
}

impl GroupErasureBroadcaster {
    pub(super) fn new(peering: Arc<Peering>) -> Self {
        Self { peering }
    }
}

impl ErasureBroadcaster for GroupErasureBroadcaster {
    fn broadcast(
        &self,
        target: ErasureTarget,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send>> {
        let peering = Arc::clone(&self.peering);
        Box::pin(async move { peering.broadcast_erasure(target).await.map(drop) })
    }

    fn check_named_token(
        &self,
        token_sha256: String,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, CallError>> + Send>> {
        let peering = Arc::clone(&self.peering);
        Box::pin(async move { peering.check_named_token(&token_sha256).await })
    }
}

impl Peering {
    /// Applies `target` to this Group member's local stores (Response cache
    /// included).
    pub(super) async fn apply_local_erasure(
        &self,
        target: &ErasureTarget,
    ) -> Result<(), CallError> {
        if let Some(cache) = &self.services.response_cache {
            match target {
                ErasureTarget::Run { run_id } => {
                    cache
                        .forget_run(run_id)
                        .await
                        .map_err(|error| internal(&error))?;
                }
                ErasureTarget::SavedIdentity { name } => {
                    cache
                        .forget_saved_identity(name)
                        .await
                        .map_err(|error| internal(&error))?;
                }
            }
        }
        Ok(())
    }

    /// Broadcasts `target` across the entire Daemon group: on a non-lead Group
    /// member, applies locally and forwards to the Lead Daemon; on the Lead
    /// Daemon, applies locally and fans out `group.erasure` to every active
    /// Group member, collecting acknowledgements and queueing any unreachable
    /// member for retry.
    pub(super) async fn broadcast_erasure(
        &self,
        target: ErasureTarget,
    ) -> Result<PeerErasureResult, CallError> {
        let (kept, _) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(|| internal(&"this Daemon is not in a group"))?;
        let lead_epoch = self.highest_epoch().await;
        if !kept.is_lead() {
            self.apply_local_erasure(&target).await?;
            let params = PeerErasureParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                member: self.name.clone(),
                broadcast: true,
                target,
            };
            let answered =
                ask_lead::<PeerErasureResult>(&kept, self.clock, GROUP_ERASURE, &params).await?;
            if answered.lead_epoch > lead_epoch {
                let _ = self.admit_epoch(answered.lead_epoch).await;
            }
            return Ok(answered);
        }

        let _ = self.require_confirmed_lead()?;
        self.apply_local_erasure(&target).await?;
        let members = store::members(&self.store)
            .await
            .map_err(|error| store_failed(&error))?;
        let mut acknowledged = vec![self.name.clone()];
        let mut pending = Vec::new();

        for member in members {
            if member.name == self.name {
                continue;
            }
            let params = PeerErasureParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                member: self.name.clone(),
                broadcast: false,
                target: target.clone(),
            };
            match ask_peer::<PeerErasureResult>(
                &kept,
                &member.name,
                &member.peer_address,
                self.clock,
                GROUP_ERASURE,
                &params,
            )
            .await
            {
                Ok(answered) => {
                    if answered.lead_epoch > lead_epoch {
                        let _ = self.admit_epoch(answered.lead_epoch).await;
                    }
                    acknowledged.push(member.name);
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        member = %member.name,
                        "group erasure delivery failed; queued for retry until acknowledged"
                    );
                    self.pending_erasures
                        .push(member.name.clone(), target.clone())
                        .await;
                    pending.push(member.name);
                }
            }
        }

        Ok(PeerErasureResult {
            protocol_version: PEER_PROTOCOL_VERSION,
            lead_epoch: self.highest_epoch().await,
            acknowledged,
            pending,
        })
    }

    /// Answers an incoming `group.erasure` peer RPC.
    pub(super) async fn erasure_request(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerErasureParams>,
    ) -> Vec<u8> {
        match self
            .handle_erasure(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_erasure(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerErasureParams,
    ) -> Result<PeerErasureResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = presented_name(presented, &params.member)?;
        if params.broadcast {
            let mut result = self.broadcast_erasure(params.target).await?;
            result.protocol_version = settled;
            return Ok(result);
        }
        let (kept, _) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(|| internal(&"this Daemon is not in a group"))?;
        if !kept.is_lead() {
            let _ = presented_name(presented, &kept.lead_name)?;
        }
        self.apply_local_erasure(&params.target).await?;
        Ok(PeerErasureResult {
            protocol_version: settled,
            lead_epoch,
            acknowledged: vec![self.name.clone()],
            pending: Vec::new(),
        })
    }
}

/// Retries pending erasures on the Lead Daemon until every target Group member
/// acknowledges (or is removed from the group).
pub(super) async fn retry_pending_erasures(peering: Arc<Peering>, owner: TaskOwner) {
    peering.run_retry_pending_erasures(owner).await;
}

impl Peering {
    async fn run_retry_pending_erasures(self: Arc<Self>, owner: TaskOwner) {
        loop {
            tokio::select! {
                () = owner.token().cancelled() => return,
                () = self.clock.sleep(ERASURE_RETRY_INTERVAL) => {}
            }
            let pending = self.pending_erasures.snapshot().await;
            if pending.is_empty() {
                continue;
            }
            let Ok(Some((kept, _))) = store::identity(&self.store).await else {
                continue;
            };
            if !kept.is_lead() || self.require_confirmed_lead().is_err() {
                continue;
            }
            let Ok(members) = store::members(&self.store).await else {
                continue;
            };
            let lead_epoch = self.highest_epoch().await;
            for item in pending {
                let Some(member) = members.iter().find(|m| m.name == item.member) else {
                    self.pending_erasures
                        .mark_acknowledged(&item.member, &item.target)
                        .await;
                    continue;
                };
                let params = PeerErasureParams {
                    protocol_version: PEER_PROTOCOL_VERSION,
                    product: self.identity.product,
                    product_version: self.identity.product_version.clone(),
                    lead_epoch,
                    member: self.name.clone(),
                    broadcast: false,
                    target: item.target.clone(),
                };
                if let Ok(answered) = ask_peer::<PeerErasureResult>(
                    &kept,
                    &member.name,
                    &member.peer_address,
                    self.clock,
                    GROUP_ERASURE,
                    &params,
                )
                .await
                {
                    if answered.lead_epoch > lead_epoch {
                        let _ = self.admit_epoch(answered.lead_epoch).await;
                    }
                    self.pending_erasures
                        .mark_acknowledged(&item.member, &item.target)
                        .await;
                }
            }
        }
    }
}
