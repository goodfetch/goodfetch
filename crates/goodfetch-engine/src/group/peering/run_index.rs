//! Group Run index feed and `runs reindex` re-reporting (ADR 0055):
//! Group members report each local Run index row to the Lead Daemon over
//! mutual TLS (`group.run_index.report`), where the Lead Daemon stamps the
//! reporting member's name as `owner` and writes the row into its group Run
//! index. `runs reindex` on a Group member also re-reports every rebuilt row
//! to the Lead Daemon.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::group::{
    PEER_PROTOCOL_VERSION, PeerRunIndexReportParams, PeerRunIndexReportResult, RUN_INDEX_REPORT,
};
use goodfetch_protocol::run::{RunIndexRow, RunOwner};
use rustls::pki_types::CertificateDer;
use tokio::sync::broadcast::error::RecvError;

use super::{Asked, Peering, ask_lead, presented_name};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::group::operations::internal;
use crate::group::{failure_line, result_line, store};
use crate::home::Home;
use crate::product::Identity;
use crate::run_index::IndexFile;
use crate::sqlite_store::StoreFile as _;
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::user_store::UserStore;

/// How long the live Run index feeder waits before retrying a failed report.
const FEED_RETRY: Duration = Duration::from_millis(250);

/// Streams local Run index writes on a non-lead Group member to its Lead
/// Daemon until `owner` stops or the Daemon leaves its group.
///
/// # Cancel safety
///
/// Dropping the future stops reporting further writes; each completed
/// report is idempotent (upsert by `run_id`).
pub(super) async fn feed_run_index(peering: Arc<Peering>, owner: TaskOwner) {
    let Some(run_index) = peering.services.run_index.clone() else {
        return;
    };
    let mut writes = run_index.subscribe_writes();
    if let Ok(existing) = run_index.rows().await
        && !existing.is_empty()
    {
        report_with_retry(&peering, &owner, existing).await;
    }
    loop {
        let received = tokio::select! {
            () = owner.token().cancelled() => return,
            received = writes.recv() => received,
        };
        let rows = match received {
            Ok(row) => vec![row],
            Err(RecvError::Lagged(_)) => match run_index.rows().await {
                Ok(rows) if !rows.is_empty() => rows,
                _ => continue,
            },
            Err(RecvError::Closed) => return,
        };
        report_with_retry(&peering, &owner, rows).await;
    }
}

/// Sends `rows` to the Lead Daemon, retrying briefly while `owner` stays live.
async fn report_with_retry(peering: &Peering, owner: &TaskOwner, rows: Vec<RunIndexRow>) {
    loop {
        let Ok(Some((kept, lead_epoch))) = store::identity(&peering.store).await else {
            return;
        };
        if kept.is_lead() {
            return;
        }
        let params = PeerRunIndexReportParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: peering.identity.product,
            product_version: peering.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            rows: rows.clone(),
            claim: false,
        };
        match ask_lead::<PeerRunIndexReportResult>(&kept, peering.clock, RUN_INDEX_REPORT, &params)
            .await
        {
            Ok(answered) => {
                if answered.lead_epoch > lead_epoch {
                    let _ = peering.admit_epoch(answered.lead_epoch).await;
                }
                return;
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    rows = rows.len(),
                    "failed to report Run index rows to the Lead Daemon; retrying"
                );
                tokio::select! {
                    () = owner.token().cancelled() => return,
                    () = peering.clock.sleep(FEED_RETRY) => {}
                }
            }
        }
    }
}

impl Peering {
    /// The answer to the `group.run_index.report` `request` of the Group
    /// member that presented `presented`.
    pub(super) async fn run_index_report(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerRunIndexReportParams>,
    ) -> Vec<u8> {
        match self
            .record_run_index(presented, request.settled, request.params)
            .await
        {
            Ok(reported) => result_line(&request.id, &reported),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    /// Stamps each reported row's `owner` with the authenticated Group member's
    /// name and writes it into the Lead Daemon's Run index.
    async fn record_run_index(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRunIndexReportParams,
    ) -> Result<PeerRunIndexReportResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let name = presented_name(presented, &params.member)?;
        let owner = name.as_str().to_owned();
        let mut reported = 0_u64;
        if let Some(run_index) = &self.services.run_index {
            for mut row in params.rows {
                // Passive reports (such as a restarted or re-added Group member
                // syncing its kept volume) must never hijack a Run already
                // assigned to another Group member in the Lead Daemon's group
                // Run index (ADR 0055). Only an explicit ownership claim from a
                // handoff or `runs adopt` transfers `owner`.
                if !params.claim
                    && let Ok(Some(existing)) = run_index.read(row.run_id.clone()).await
                    && matches!(
                        existing.owner,
                        Some(RunOwner::GroupMember(ref current)) if current != &owner
                    )
                {
                    continue;
                }
                row.owner = Some(RunOwner::GroupMember(owner.clone()));
                run_index
                    .write(row)
                    .await
                    .map_err(|error| internal(&error))?;
                reported = reported.saturating_add(1);
            }
        }
        Ok(PeerRunIndexReportResult {
            protocol_version: settled,
            lead_epoch,
            reported,
        })
    }
}

/// Re-reports every row in `home`'s rebuilt Run index for `identity`'s major
/// to its Lead Daemon if `home` is a non-lead Group member (ADR 0055).
///
/// Returns the number of rows the Lead Daemon recorded, or `0` when this home
/// is not a non-lead Group member or has no rows to report.
///
/// # Errors
///
/// When the group store, local Run index, or Lead Daemon peer link fails.
pub async fn reindex_to_lead(
    home: &Home,
    identity: &Identity,
    clock: Clock,
) -> Result<u64, String> {
    let group_path = home
        .per_major(&identity.product_version)
        .join(store::STORE.file());
    if !group_path.is_file() {
        return Ok(0);
    }
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store = UserStore::open(group_path, &store::STORE, None, Arc::clone(&owner));
    let kept = store::identity(&store)
        .await
        .map_err(|error| error.to_string())?;
    let Some((kept, lead_epoch)) = kept else {
        owner.stop().await;
        return Ok(0);
    };
    if kept.is_lead() {
        owner.stop().await;
        return Ok(0);
    }
    let index = IndexFile::of(
        home.run_index(&identity.product_version),
        &identity.product_version,
    );
    let connection = index.open().map_err(|error| error.to_string())?;
    let rows = index.rows(&connection).map_err(|error| error.to_string())?;
    drop(connection);
    if rows.is_empty() {
        owner.stop().await;
        return Ok(0);
    }
    let params = PeerRunIndexReportParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity.product,
        product_version: identity.product_version.clone(),
        lead_epoch,
        member: kept.name.clone(),
        rows,
        claim: false,
    };
    let answered = ask_lead::<PeerRunIndexReportResult>(&kept, clock, RUN_INDEX_REPORT, &params)
        .await
        .map_err(|error| error.to_string())?;
    if answered.lead_epoch > lead_epoch {
        let _ = store::admit_epoch(&store, answered.lead_epoch).await;
    }
    owner.stop().await;
    Ok(answered.reported)
}

#[cfg(test)]
mod tests;
