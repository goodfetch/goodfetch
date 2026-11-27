//! Replicated read-mostly control-state push and sync over the peer link
//! (ADR 0055, issue 243):
//!
//! - The Lead Daemon publishes each change to Operator policy and user
//!   configuration with a strictly increasing version, persists it in its
//!   group store, and pushes unacknowledged snapshots to every Group member
//!   over mutual TLS (`group.control_state.push`).
//! - A Group member verifies the peer certificate names the Lead Daemon,
//!   admits the lead epoch, applies each snapshot strictly monotonically (never
//!   applying an older or duplicate version), persists it in its group store,
//!   and atomically writes the cached TOML file so its own and Attached
//!   engines resolve the Lead Daemon's policy and configuration alongside this
//!   member's `[group_member.<name>]` scope.
//! - On join or restart, a non-lead Group member asks the Lead Daemon for any
//!   snapshots newer than its persisted versions (`group.control_state.sync`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::group::{
    CONTROL_STATE_PUSH, CONTROL_STATE_SYNC, ControlStateSnapshot, PEER_PROTOCOL_VERSION,
    PeerControlStatePushParams, PeerControlStatePushResult, PeerControlStateSyncParams,
    PeerControlStateSyncResult,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::SettingsLayer;
use rustls::pki_types::CertificateDer;
use tokio::sync::broadcast::error::RecvError;

use super::{Asked, Peering, ask_lead, ask_peer, presented_name};
use crate::call_error::CallError;
use crate::control_state::{ApplyOutcome, STORE_OPERATOR_POLICY, STORE_USER_CONFIGURATION};
use crate::group::operations::{internal, store_failed};
use crate::group::{failure_line, result_line, store};
use crate::tasks::TaskOwner;

/// How long the Lead Daemon or a catching-up Group member waits before
/// retrying a failed control-state push or sync.
const RETRY_INTERVAL: Duration = Duration::from_millis(200);

/// The two machine settings files replicated as read-mostly control-state
/// stores (ADR 0055).
const SETTINGS_STORES: [(&str, SettingsLayer); 2] = [
    (STORE_OPERATOR_POLICY, SettingsLayer::OperatorPolicy),
    (STORE_USER_CONFIGURATION, SettingsLayer::UserConfiguration),
];

/// Runs on the Lead Daemon until `owner` stops: watches settings file changes
/// and newly admitted Group members, publishes versioned snapshots for
/// Operator policy and user configuration, and pushes unacknowledged snapshots
/// to every Group member over mutual TLS.
///
/// # Cancel safety
///
/// Dropping the future stops further pushes; each snapshot already persisted
/// and acknowledged stays durable.
pub(super) async fn replicate_control_state(peering: Arc<Peering>, owner: TaskOwner) {
    let mut changes = peering
        .services
        .settings
        .as_ref()
        .map(crate::settings::Settings::subscribe_changes);
    let mut ledger_changes = peering
        .services
        .ledger
        .as_ref()
        .map(crate::ledger::Ledger::subscribe_changes);
    let mut acked: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    loop {
        if owner.token().is_cancelled() {
            return;
        }
        peering.sync_settings_from_disk().await;
        peering.sync_usage_limits().await;
        let pending = peering.push_round(&mut acked).await;
        wait_next(&peering, &owner, &mut changes, &mut ledger_changes, pending).await;
    }
}

/// Waits for a settings write, a ledger Limit write, a newly admitted member
/// wake, a retry timer (when `pending`), or cancellation.
async fn wait_next(
    peering: &Peering,
    owner: &TaskOwner,
    changes: &mut Option<tokio::sync::broadcast::Receiver<()>>,
    ledger_changes: &mut Option<tokio::sync::broadcast::Receiver<()>>,
    pending: bool,
) {
    let recv_change = async {
        match changes.as_mut() {
            Some(rx) => match rx.recv().await {
                Ok(()) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => std::future::pending::<()>().await,
            },
            None => std::future::pending::<()>().await,
        }
    };
    let recv_ledger = async {
        match ledger_changes.as_mut() {
            Some(rx) => match rx.recv().await {
                Ok(()) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => std::future::pending::<()>().await,
            },
            None => std::future::pending::<()>().await,
        }
    };
    let retry = async {
        if pending {
            peering.clock.sleep(RETRY_INTERVAL).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        () = owner.token().cancelled() => {}
        () = peering.control_wake.notified() => {}
        () = recv_change => {}
        () = recv_ledger => {}
        () = retry => {}
    }
}

/// Runs on a non-lead Group member at join or startup: asks the Lead Daemon
/// for every read-mostly control-state snapshot newer than its persisted
/// versions, retrying while the Lead Daemon is unreachable.
///
/// # Cancel safety
///
/// Dropping the future stops retrying; any snapshot already applied stays
/// durable.
pub(super) async fn sync_control_state(peering: Arc<Peering>, owner: TaskOwner) {
    loop {
        if owner.token().is_cancelled() {
            return;
        }
        let Ok(Some((kept, _))) = store::identity(&peering.store).await else {
            return;
        };
        if kept.is_lead() {
            return;
        }
        let lead_epoch = peering.highest_epoch().await;
        let versions = peering.control_state.lock().await.versions();
        let params = PeerControlStateSyncParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: peering.identity.product,
            product_version: peering.identity.product_version.clone(),
            lead_epoch,
            versions,
        };
        match ask_lead::<PeerControlStateSyncResult>(
            &kept,
            peering.clock,
            CONTROL_STATE_SYNC,
            &params,
        )
        .await
        {
            Ok(answered) => {
                if answered.lead_epoch > lead_epoch {
                    let _ = peering.admit_epoch(answered.lead_epoch).await;
                }
                peering.report_lead_reachable(true, None).await;
                if let Err(error) = peering.apply_snapshots(answered.snapshots).await {
                    tracing::warn!(%error, "failed to apply synced control-state snapshots");
                    tokio::select! {
                        () = owner.token().cancelled() => return,
                        () = peering.clock.sleep(RETRY_INTERVAL) => continue,
                    }
                }
                return;
            }
            Err(error) => {
                peering
                    .report_lead_reachable(false, Some(error.to_string()))
                    .await;
                tracing::debug!(
                    %error,
                    "failed to sync control-state stores from Lead Daemon; retrying"
                );
                tokio::select! {
                    () = owner.token().cancelled() => return,
                    () = peering.clock.sleep(RETRY_INTERVAL) => {}
                }
            }
        }
    }
}

impl Peering {
    /// Loads persisted read-mostly control-state snapshots from the group
    /// store into memory before the peer listener accepts links.
    pub(super) async fn hydrate_control_state(&self) {
        match store::load_replicated_stores(&self.store).await {
            Ok(stored) => {
                self.control_state.lock().await.hydrate(stored);
            }
            Err(error) => {
                tracing::warn!(%error, "could not load persisted replicated control-state stores");
            }
        }
    }

    /// Wakes the Lead Daemon's control-state replicator (for example when a
    /// new Group member joins or a store updates).
    pub(super) fn notify_control_state(&self) {
        self.control_wake.notify_one();
    }

    /// Publishes a new version of a registered read-mostly control-state store
    /// on the Lead Daemon, persists it durably, and wakes the push loop.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is unregistered, declared single-writer,
    /// or cannot be persisted to the group store.
    pub(super) async fn publish_control_store(
        &self,
        store_name: &str,
        content: Option<String>,
    ) -> Result<Option<ControlStateSnapshot>, CallError> {
        let mut state = self.control_state.lock().await;
        let snapshot = state.publish(store_name, content).map_err(|error| {
            CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
        })?;
        if let Some(ref published) = snapshot {
            store::save_replicated_store(&self.store, published.clone())
                .await
                .map_err(|error| store_failed(&error))?;
            self.control_wake.notify_one();
        }
        Ok(snapshot)
    }

    /// Reads Operator policy and user configuration from disk on the Lead
    /// Daemon and publishes a new version for whichever file changed.
    pub(super) async fn sync_settings_from_disk(&self) {
        let Some(settings) = self.services.settings.clone() else {
            return;
        };
        for (store_name, layer) in SETTINGS_STORES {
            let text = match settings.read_file_text(layer).await {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(%error, store = store_name, "could not read settings file for replication");
                    continue;
                }
            };
            if let Err(error) = self.publish_control_store(store_name, text).await {
                tracing::warn!(%error, store = store_name, "could not publish control-state store");
            }
        }
    }

    /// Pushes unacknowledged snapshots to every non-lead Group member. Returns
    /// `true` if any member remains behind or the Lead Daemon is still
    /// unconfirmed and needs a retry.
    async fn push_round(&self, acked: &mut BTreeMap<String, BTreeMap<String, u64>>) -> bool {
        let Ok(lead_epoch) = self.require_confirmed_lead() else {
            return true;
        };
        let Ok(Some((kept, _))) = store::identity(&self.store).await else {
            return false;
        };
        if !kept.is_lead() {
            return false;
        }
        let Ok(members) = store::members(&self.store).await else {
            return true;
        };
        let mut any_pending = false;
        for member in members {
            if member.name == self.name {
                continue;
            }
            let snapshots = {
                let state = self.control_state.lock().await;
                let member_acked = acked.entry(member.name.clone()).or_default();
                state.snapshots_newer_than(member_acked)
            };
            if snapshots.is_empty() {
                continue;
            }
            let params = PeerControlStatePushParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                snapshots,
            };
            match ask_peer::<PeerControlStatePushResult>(
                &kept,
                &member.name,
                &member.peer_address,
                self.clock,
                CONTROL_STATE_PUSH,
                &params,
            )
            .await
            {
                Ok(answered) => {
                    if answered.lead_epoch > lead_epoch {
                        let _ = self.admit_epoch(answered.lead_epoch).await;
                    }
                    let member_acked = acked.entry(member.name.clone()).or_default();
                    for (store_name, version) in answered.versions {
                        let slot = member_acked.entry(store_name).or_insert(0);
                        if version > *slot {
                            *slot = version;
                        }
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        member = %member.name,
                        "control-state push to Group member failed; will retry"
                    );
                    any_pending = true;
                }
            }
        }
        any_pending
    }

    /// Applies `snapshots` monotonically while holding the control-state lock
    /// across SQLite persistence and cached settings file writes so concurrent
    /// pushes never apply or write out of order.
    pub(super) async fn apply_snapshots(
        &self,
        snapshots: Vec<ControlStateSnapshot>,
    ) -> Result<BTreeMap<String, u64>, CallError> {
        let mut state = self.control_state.lock().await;
        for snapshot in snapshots {
            let store_name = snapshot.store.clone();
            let content = snapshot.content.clone();
            let outcome = state.apply(snapshot.clone()).map_err(|error| {
                CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
            })?;
            if outcome == ApplyOutcome::Applied {
                store::save_replicated_store(&self.store, snapshot)
                    .await
                    .map_err(|error| store_failed(&error))?;
                if let Some(settings) = &self.services.settings
                    && let Some(layer) = settings_layer_for(&store_name)
                {
                    settings
                        .write_cached_file(layer, content)
                        .await
                        .map_err(|error| internal(&error))?;
                }
            }
        }
        Ok(state.versions())
    }

    /// The answer to `group.control_state.push` from `presented`.
    pub(super) async fn control_state_push(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerControlStatePushParams>,
    ) -> Vec<u8> {
        match self
            .receive_control_state_push(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn receive_control_state_push(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerControlStatePushParams,
    ) -> Result<PeerControlStatePushResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let Some((kept, _)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "this Daemon is no longer in a group",
            ));
        };
        // Only the Lead Daemon may push control-state snapshots.
        let _ = presented_name(presented, &kept.lead_name)?;
        let versions = self.apply_snapshots(params.snapshots).await?;
        Ok(PeerControlStatePushResult {
            protocol_version: settled,
            lead_epoch,
            versions,
        })
    }

    /// The answer to `group.control_state.sync` from `presented`.
    pub(super) async fn control_state_sync(
        &self,
        _presented: &CertificateDer<'_>,
        request: Asked<PeerControlStateSyncParams>,
    ) -> Vec<u8> {
        match self
            .answer_control_state_sync(request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn answer_control_state_sync(
        &self,
        settled: ProtocolVersion,
        params: PeerControlStateSyncParams,
    ) -> Result<PeerControlStateSyncResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        self.sync_settings_from_disk().await;
        let snapshots = self
            .control_state
            .lock()
            .await
            .snapshots_newer_than(&params.versions);
        Ok(PeerControlStateSyncResult {
            protocol_version: settled,
            lead_epoch,
            snapshots,
        })
    }
}

fn settings_layer_for(store_name: &str) -> Option<SettingsLayer> {
    SETTINGS_STORES
        .iter()
        .find_map(|&(name, layer)| (name == store_name).then_some(layer))
}

#[cfg(test)]
mod tests;
