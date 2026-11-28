//! Scale-down drain handoff, startup assigned-run resume, and `runs adopt`
//! across a Daemon group (ADRs 0055, 0056 and 0060, issue 245).
//!
//! Size: one concern, the Group member Run handoff and `runs adopt` pipeline
//! (drain, checkpoint transfer, receiver install/resume, hook delegation, and
//! offline volume adoption) kept together because every path shares the same
//! mTLS peer-link envelope, epoch check, and Run index ownership update.
//!
//! On removal (`group leave` on VMs or `StatefulSet` scale-down where
//! `pod_ordinal >= desired_replicas`), a Group member drains its live Runs
//! within `daemon.drain_grace` and sends each non-machine-bound Run's
//! Checkpoint over the mTLS peer link (`group.run.handoff`) to the Group member
//! the Lead Daemon places it on. The receiver installs the Checkpoint (with its
//! data key still wrapped by the shared Secret provider), resumes the Run at
//! the next run epoch, and claims ownership in the Lead Daemon's group Run
//! index. Runs that the grace period does not cover stay checkpointed on disk
//! and still assigned to the removed member so `runs adopt` can bring them into
//! another Group member, while machine-bound Runs stay assigned to the removed
//! member and are refused with `session_bound_to_machine`. On restart or
//! re-add, a Group member queries `group.run_index.list_assigned` and resumes
//! only the Runs the group Run index still assigns to it, so no Run runs twice.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use data_encoding::{BASE64, HEXLOWER};
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::checkpoint::{
    RunBinding, SESSION_BOUND_TO_MACHINE, SessionBoundToMachineDetails,
};
use goodfetch_protocol::group::{
    GROUP_PEER_EXTENSION, GROUP_RUN_HANDOFF, GROUP_RUN_INDEX_LIST_ASSIGNED, GROUP_RUN_INDEX_LOOKUP,
    GROUP_RUN_PLACE, LeadEpoch, MemberRoute, NOT_IN_A_GROUP, PEER_PROTOCOL_VERSION,
    PeerExtensionParams, PeerExtensionResult, PeerRunHandoffParams, PeerRunHandoffResult,
    PeerRunIndexListAssignedParams, PeerRunIndexListAssignedResult, PeerRunIndexLookupParams,
    PeerRunIndexLookupResult, PeerRunIndexReportParams, PeerRunIndexReportResult,
    PeerRunPlaceParams, PeerRunPlaceResult, PlacementRequest, RUN_INDEX_REPORT,
};
use goodfetch_protocol::home::RUN_FILE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{AdoptedRun, Lifecycle, RunId, RunIndexRow, RunOwner};
use rustls::pki_types::CertificateDer;
use sha2::{Digest as _, Sha256};
use tokio::time::Instant;

use super::wire::{ask_lead, ask_peer};
use super::{Asked, Peering, presented_name};
use crate::call_error::CallError;
use crate::checkpoint::{self, CHECKPOINT_FILE, Held};
use crate::clock::Clock;
use crate::group::errors::{internal, store_failed};
use crate::group::identity::KeptIdentity;
use crate::group::{failure_line, result_line, store};
use crate::home::Home;
use crate::product::Identity;
use crate::runs::{FromCheckpoint, Resume, Resumed, Runs};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::user_store::UserStore;

/// Result of a removal drain handoff on a leaving Group member (ADR 0055).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::group) struct RemovalSummary {
    /// Run IDs handed off and resumed on another Group member.
    pub(in crate::group) handed_off: Vec<String>,
    /// Run IDs left checkpointed and still assigned to this Group member
    /// because the drain grace expired or no target accepted them.
    pub(in crate::group) left_checkpointed: Vec<String>,
    /// Run IDs bound to this machine (ADR 0056) that stayed assigned here.
    pub(in crate::group) machine_bound: Vec<String>,
}

/// Result of `runs adopt` bringing checkpointed Runs from a removed Group
/// member's kept volume into another Group member (ADR 0055).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdoptedSummary {
    /// Runs successfully handed off, resumed, and reassigned.
    pub adopted: Vec<AdoptedRun>,
    /// Run IDs skipped because they were already ended or already owned by
    /// another active Group member.
    pub skipped: Vec<String>,
}

/// Prepared Checkpoint payload read from a Run folder.
struct PreparedCheckpoint {
    run_id: RunId,
    held: Held,
    bytes: Vec<u8>,
    sha256: String,
}

impl Peering {
    /// Drains live Runs for a removal (`group leave` or `StatefulSet` scale-down)
    /// and hands every non-machine-bound Run's Checkpoint over the peer link to
    /// the Group member the Lead Daemon places it on within `grace` (ADR 0055).
    /// Contributions (such as Partitioned crawls, ADR 0054) run their removal
    /// drain hooks first so their Partitions hand off by domain rather than by
    /// Checkpoint transfer.
    pub(in crate::group) async fn drain_and_handoff_for_removal(
        &self,
        runs: &Runs,
        grace: Duration,
    ) -> RemovalSummary {
        let now = self.clock.now();
        let deadline = now + grace;
        let mut skip_checkpoint_handoff = BTreeSet::new();
        if !self.services.group_membership_hooks.is_empty()
            && let Some(ctx) = self.membership_context()
        {
            for hook in self.services.group_membership_hooks.iter() {
                let handled = hook
                    .on_removal_drain(self.name.clone(), deadline, ctx.clone())
                    .await;
                skip_checkpoint_handoff.extend(handled);
            }
        }
        let remaining_now = self.clock.now();
        let effects_by = remaining_now + (deadline.saturating_duration_since(remaining_now) / 2);
        let draining = runs.drain(effects_by).await;
        tokio::select! {
            () = draining.ended() => {}
            () = self.clock.sleep_until(deadline) => {
                tracing::warn!(
                    member = %self.name,
                    remaining = draining.not_ended(),
                    "removal drain grace passed before every Run stopped; remaining Runs stay checkpointed for `runs adopt`"
                );
            }
        }
        self.handoff_local_runs_excluding(deadline, &skip_checkpoint_handoff)
            .await
    }

    /// Hands off local non-ended Runs assigned to this Group member before
    /// `deadline`, leaving machine-bound Runs and any Run not covered before
    /// `deadline` checkpointed and assigned to this member (ADR 0055).
    pub(super) async fn handoff_local_runs(&self, deadline: Instant) -> RemovalSummary {
        self.handoff_local_runs_excluding(deadline, &BTreeSet::new())
            .await
    }

    async fn handoff_local_runs_excluding(
        &self,
        deadline: Instant,
        skip_runs: &BTreeSet<RunId>,
    ) -> RemovalSummary {
        let mut summary = RemovalSummary::default();
        let Some((kept, _)) = store::identity(&self.store).await.ok().flatten() else {
            return summary;
        };
        let candidates = self.collect_local_handoff_folders().await;
        for folder in candidates {
            let prepared = match self.prepare_checkpoint_blocking(folder).await {
                Ok(Some(prepared)) => prepared,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(%error, "skipping unreadable Run folder during removal handoff");
                    continue;
                }
            };
            if skip_runs.contains(&prepared.run_id) {
                continue;
            }
            if matches!(
                prepared.held.lifecycle,
                Lifecycle::Finished | Lifecycle::Failed
            ) {
                continue;
            }
            if matches!(prepared.held.binding, Some(RunBinding::Machine { .. })) {
                summary
                    .machine_bound
                    .push(prepared.run_id.as_str().to_owned());
                continue;
            }
            if self.clock.now() >= deadline {
                summary
                    .left_checkpointed
                    .push(prepared.run_id.as_str().to_owned());
                continue;
            }
            let remaining = deadline.saturating_duration_since(self.clock.now());
            let handoff_fut = self.handoff_one_run(&kept, &prepared, false);
            let outcome = tokio::select! {
                res = handoff_fut => Some(res),
                () = self.clock.sleep(remaining) => None,
            };
            match outcome {
                Some(Ok(answered)) => {
                    self.mark_run_moved(&prepared.run_id, Some(answered.owner));
                    summary.handed_off.push(prepared.run_id.as_str().to_owned());
                }
                Some(Err(error)) => {
                    tracing::warn!(
                        run_id = %prepared.run_id,
                        %error,
                        "handoff failed during removal drain; Run stays checkpointed for `runs adopt`"
                    );
                    summary
                        .left_checkpointed
                        .push(prepared.run_id.as_str().to_owned());
                }
                None => {
                    summary
                        .left_checkpointed
                        .push(prepared.run_id.as_str().to_owned());
                }
            }
        }
        summary
    }

    async fn collect_local_handoff_folders(&self) -> Vec<PathBuf> {
        let mut folders = BTreeSet::new();
        if let Some(home) = &self.services.home {
            let per_major = home.per_major(&self.identity.product_version);
            if let Some(run_index) = &self.services.run_index
                && let Ok(rows) = run_index.rows().await
            {
                for row in rows {
                    if !matches!(row.lifecycle, Lifecycle::Finished | Lifecycle::Failed) {
                        folders.insert(per_major.join(&row.location));
                    }
                }
            }
            let runs_root = home.runs(&self.identity.product_version);
            for folder in scan_run_folders(&[runs_root]) {
                folders.insert(folder);
            }
        }
        folders.into_iter().collect()
    }

    async fn prepare_checkpoint_blocking(
        &self,
        folder: PathBuf,
    ) -> Result<Option<PreparedCheckpoint>, CallError> {
        let nonce = self.clock.wall_nonce();
        if let Some(owner) = &self.services.owner {
            owner
                .blocking(move || prepare_checkpoint(&folder, nonce))
                .await
        } else {
            prepare_checkpoint(&folder, nonce)
        }
    }

    async fn handoff_one_run(
        &self,
        kept: &KeptIdentity,
        prepared: &PreparedCheckpoint,
        adopt: bool,
    ) -> Result<PeerRunHandoffResult, CallError> {
        let request = PlacementRequest {
            receiving_member: self.name.clone(),
            exclude_members: vec![self.name.clone()],
            ..PlacementRequest::default()
        };
        let route = self.place_run(&request).await?;
        let lead_epoch = self.highest_epoch().await;
        send_handoff_to_route(
            kept,
            &self.identity,
            self.clock,
            lead_epoch,
            &route,
            prepared,
            Some(self.name.clone()),
            adopt,
        )
        .await
    }

    /// Answers an incoming `group.run.handoff` peer RPC on the destination
    /// Group member: verifies integrity and machine binding, installs the
    /// Checkpoint, resumes the Run at the next run epoch, and claims ownership
    /// in the Lead Daemon's group Run index (ADR 0055).
    pub(super) async fn receive_handoff(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerRunHandoffParams>,
    ) -> Vec<u8> {
        match self
            .handle_receive_handoff(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_receive_handoff(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRunHandoffParams,
    ) -> Result<PeerRunHandoffResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = presented_name(presented, &params.member)?;
        let bytes = BASE64
            .decode(params.checkpoint_base64.as_bytes())
            .map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid Checkpoint base64 payload: {error}"),
                )
            })?;
        let digest = sha256_hex(&bytes);
        if digest != params.sha256 {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "Checkpoint SHA-256 mismatch: expected `{}`, got `{digest}`",
                    params.sha256
                ),
            ));
        }
        let Some(runs) = &self.services.runs else {
            return Err(internal(&"Runs registry is not wired on this Group member"));
        };
        let Some(home) = &self.services.home else {
            return Err(internal(
                &"Goodfetch home is not wired on this Group member",
            ));
        };
        let temp_path = self
            .write_and_validate_temp_checkpoint(home, &params.run_id, bytes)
            .await?;

        let resume_settings = self.open_resume_settings().await;
        let resumed = runs
            .resume(Resume {
                run_id: params.run_id.clone(),
                checkpoint: Some(FromCheckpoint {
                    path: temp_path.clone(),
                    rollback: None,
                }),
                checkpoint_interval: None,
                timeout: None,
                settings: resume_settings,
                retry_failed: false,
                redeclared: false,
            })
            .await;
        remove_temp_file(self.services.owner.as_ref(), temp_path).await;

        let run_epoch = match resumed {
            Ok(Resumed::New { run, .. }) => run.state.epoch(),
            Ok(Resumed::Live { .. }) => 1,
            Ok(Resumed::Unchanged { lifecycle, .. }) => {
                return Err(internal(&format!(
                    "Run `{}` could not resume from Checkpoint (lifecycle: {lifecycle:?})",
                    params.run_id
                )));
            }
            Err(crate::runs::RunError::BoundToMachine(run_id)) => {
                return Err(session_bound_for_run(
                    run_id,
                    "This Run is bound to another machine and cannot be handed off or adopted.",
                ));
            }
            Err(error) => return Err(internal(&error)),
        };

        self.claim_run_ownership(&params.run_id).await?;
        self.route_cache.invalidate_run(&params.run_id);

        Ok(PeerRunHandoffResult {
            protocol_version: settled,
            lead_epoch,
            run_id: params.run_id,
            owner: self.name.clone(),
            run_epoch,
        })
    }

    async fn write_and_validate_temp_checkpoint(
        &self,
        home: &Home,
        run_id: &RunId,
        bytes: Vec<u8>,
    ) -> Result<PathBuf, CallError> {
        let runs_dir = home.runs(&self.identity.product_version);
        let nonce = self.clock.wall_nonce();
        let temp_path = runs_dir.join(format!(".{}.handoff-{nonce}.sqlite", run_id.as_str()));
        let expected_id = run_id.clone();
        let path_for_io = temp_path.clone();
        let work = move || -> Result<PathBuf, CallError> {
            #[expect(
                clippy::disallowed_methods,
                reason = "runs under owner.blocking or sync handoff setup to stage received Checkpoint bytes"
            )]
            {
                std::fs::create_dir_all(&runs_dir).map_err(|error| internal(&error))?;
                std::fs::write(&path_for_io, &bytes).map_err(|error| internal(&error))?;
            }
            let conn = checkpoint::open_checkpoint(&path_for_io).map_err(|error| {
                let _ = remove_file_sync(&path_for_io);
                internal(&error)
            })?;
            let held = checkpoint::read(&conn, &path_for_io).map_err(|error| {
                let _ = remove_file_sync(&path_for_io);
                internal(&error)
            })?;
            drop(conn);
            if matches!(held.binding, Some(RunBinding::Machine { .. })) {
                let _ = remove_file_sync(&path_for_io);
                return Err(session_bound_for_run(
                    expected_id,
                    "This Run is bound to another machine and cannot be handed off or adopted.",
                ));
            }
            Ok(path_for_io)
        };
        if let Some(owner) = &self.services.owner {
            owner.blocking(work).await
        } else {
            work()
        }
    }

    async fn claim_run_ownership(&self, run_id: &RunId) -> Result<(), CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Ok(());
        };
        let Some(run_index) = &self.services.run_index else {
            return Ok(());
        };
        let Some(mut row) = run_index
            .read(run_id.clone())
            .await
            .map_err(|error| internal(&error))?
        else {
            return Ok(());
        };
        row.owner = Some(RunOwner::GroupMember(self.name.clone()));
        if kept.is_lead() {
            run_index
                .write(row)
                .await
                .map_err(|error| internal(&error))?;
        } else {
            let params = PeerRunIndexReportParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                member: kept.name.clone(),
                rows: vec![row],
                claim: true,
            };
            let answered: PeerRunIndexReportResult =
                ask_lead(&kept, self.clock, RUN_INDEX_REPORT, &params).await?;
            if answered.lead_epoch > lead_epoch {
                let _ = self.admit_epoch(answered.lead_epoch).await;
            }
        }
        Ok(())
    }

    /// Answers `group.run_index.list_assigned` on the Lead Daemon: lists every
    /// non-ended Run whose group Run index row currently assigns `for_member`
    /// as its owner (ADR 0055).
    pub(super) async fn list_assigned_runs(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerRunIndexListAssignedParams>,
    ) -> Vec<u8> {
        match self
            .handle_list_assigned(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_list_assigned(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRunIndexListAssignedParams,
    ) -> Result<PeerRunIndexListAssignedResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let _ = presented_name(presented, &params.member)?;
        let rows = self.assigned_rows_on_lead(&params.for_member).await?;
        Ok(PeerRunIndexListAssignedResult {
            protocol_version: settled,
            lead_epoch,
            rows,
        })
    }

    async fn assigned_rows_on_lead(&self, member: &str) -> Result<Vec<RunIndexRow>, CallError> {
        let Some(run_index) = &self.services.run_index else {
            return Ok(Vec::new());
        };
        let all = run_index.rows().await.map_err(|error| internal(&error))?;
        Ok(all
            .into_iter()
            .filter(|row| {
                !matches!(row.lifecycle, Lifecycle::Finished | Lifecycle::Failed)
                    && matches!(row.owner, Some(RunOwner::GroupMember(ref o)) if o == member)
            })
            .collect())
    }

    /// On startup (restart or re-add), queries the group Run index for Runs
    /// still assigned to this Group member and resumes only those present on
    /// local disk, so a restarted member resumes its own Runs while a re-added
    /// member never resumes a Run already handed off or adopted (ADR 0055).
    pub(super) async fn resume_assigned_runs(&self) -> Vec<RunId> {
        let Some(runs) = &self.services.runs else {
            return Vec::new();
        };
        let Some(home) = &self.services.home else {
            return Vec::new();
        };
        let Ok(Some((kept, lead_epoch))) = store::identity(&self.store).await else {
            return Vec::new();
        };
        let assigned = if kept.is_lead() {
            self.assigned_rows_on_lead(&self.name)
                .await
                .unwrap_or_default()
        } else {
            let params = PeerRunIndexListAssignedParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                member: kept.name.clone(),
                for_member: self.name.clone(),
            };
            match ask_lead::<PeerRunIndexListAssignedResult>(
                &kept,
                self.clock,
                GROUP_RUN_INDEX_LIST_ASSIGNED,
                &params,
            )
            .await
            {
                Ok(answered) => {
                    if answered.lead_epoch > lead_epoch {
                        let _ = self.admit_epoch(answered.lead_epoch).await;
                    }
                    answered.rows
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        member = %self.name,
                        "could not query Lead Daemon for assigned Runs on startup"
                    );
                    return Vec::new();
                }
            }
        };

        let per_major = home.per_major(&self.identity.product_version);
        let resume_settings = self.open_resume_settings().await;
        let mut resumed_ids = Vec::new();
        for row in assigned {
            let folder = per_major.join(&row.location);
            let run_file = folder.join(RUN_FILE);
            if !path_is_file(self.services.owner.as_ref(), run_file).await {
                continue;
            }
            match runs
                .resume(Resume {
                    run_id: row.run_id.clone(),
                    checkpoint: None,
                    checkpoint_interval: None,
                    timeout: None,
                    settings: resume_settings.clone(),
                    retry_failed: false,
                    redeclared: false,
                })
                .await
            {
                Ok(Resumed::New { .. }) => {
                    resumed_ids.push(row.run_id);
                }
                Ok(Resumed::Live { .. } | Resumed::Unchanged { .. }) => {}
                Err(error) => {
                    tracing::warn!(
                        run_id = %row.run_id,
                        %error,
                        "failed to resume Run still assigned to this Group member on startup"
                    );
                }
            }
        }
        resumed_ids
    }

    async fn open_resume_settings(&self) -> Option<crate::settings::RunSettings> {
        let settings = self.services.settings.as_ref()?;
        settings
            .open(crate::settings::Opening::default())
            .await
            .ok()
            .map(|(opened, _)| opened)
    }
}

/// Brings checkpointed Runs from a removed Group member's kept volume (`paths`)
/// into another Group member of the Daemon group `home` belongs to (ADR 0055).
///
/// Refuses any Run bound to the removed Group member's machine with
/// `session_bound_to_machine` (ADR 0056), leaving it assigned to the removed
/// member. Skips Runs that have already ended or are already owned by another
/// live Group member so no Run runs twice.
///
/// # Errors
///
/// `not_in_a_group` when `home` is not in a Daemon group,
/// `session_bound_to_machine` when any Run in `paths` is machine-bound, or
/// peer/placement failures from the Lead Daemon or destination Group member.
/// Peer client backed by `KeptIdentity` for offline `runs adopt` hooks.
struct KeptAdoptPeerClient {
    kept: KeptIdentity,
    identity: Identity,
    clock: Clock,
    lead_epoch: LeadEpoch,
}

impl crate::engine_context::AdoptPeerClient for KeptAdoptPeerClient {
    fn member_name(&self) -> &str {
        &self.kept.name
    }

    fn call_lead_extension(
        &self,
        extension_method: String,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CallError>> + Send>> {
        self.call_member_extension(
            self.kept.lead_name.clone(),
            self.kept.lead_address.clone(),
            extension_method,
            payload,
        )
    }

    fn call_member_extension(
        &self,
        member: String,
        peer_address: String,
        extension_method: String,
        payload: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CallError>> + Send>> {
        let kept = self.kept.clone();
        let identity = self.identity.clone();
        let clock = self.clock;
        let lead_epoch = self.lead_epoch;
        Box::pin(async move {
            let params = PeerExtensionParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: identity.product,
                product_version: identity.product_version,
                lead_epoch,
                member: kept.name.clone(),
                extension_method,
                payload,
            };
            let answered: PeerExtensionResult = ask_peer(
                &kept,
                &member,
                &peer_address,
                clock,
                GROUP_PEER_EXTENSION,
                &params,
            )
            .await?;
            Ok(answered.result)
        })
    }
}

pub async fn adopt_runs(
    home: &Home,
    identity: &Identity,
    clock: Clock,
    paths: &[PathBuf],
    from_member: Option<&str>,
) -> Result<AdoptedSummary, CallError> {
    adopt_runs_with_hooks(home, identity, clock, paths, from_member, &[]).await
}

/// Brings checkpointed Runs from `paths` on a removed Group member's kept
/// volume into another Group member, letting contribution `hooks` intercept
/// their own Run folders (such as Partitioned crawl Partitions, ADR 0054)
/// before generic Checkpoint handoff.
///
/// # Errors
///
/// As [`adopt_runs`].
pub async fn adopt_runs_with_hooks(
    home: &Home,
    identity: &Identity,
    clock: Clock,
    paths: &[PathBuf],
    from_member: Option<&str>,
    hooks: &[Arc<dyn crate::engine_context::RunAdoptHook>],
) -> Result<AdoptedSummary, CallError> {
    let group_path = home
        .per_major(&identity.product_version)
        .join(store::STORE.file());
    if !is_file_sync(&group_path) {
        return Err(CallError::new(
            &NOT_IN_A_GROUP,
            "This Daemon is not in a Daemon group, so it cannot adopt Runs into one.",
            &(),
        ));
    }
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store = UserStore::open(group_path, &store::STORE, None, owner);
    let Some((kept, lead_epoch)) = store::identity(&store)
        .await
        .map_err(|error| store_failed(&error))?
    else {
        return Err(CallError::new(
            &NOT_IN_A_GROUP,
            "This Daemon is not in a Daemon group, so it cannot adopt Runs into one.",
            &(),
        ));
    };

    let peer_client: Arc<dyn crate::engine_context::AdoptPeerClient> =
        Arc::new(KeptAdoptPeerClient {
            kept: kept.clone(),
            identity: identity.clone(),
            clock,
            lead_epoch,
        });

    let folders = scan_run_folders(paths);
    let mut summary = AdoptedSummary::default();
    let nonce_base = clock.wall_nonce();

    for (idx, folder) in folders.into_iter().enumerate() {
        if try_adopt_with_hooks(hooks, &folder, from_member, &peer_client, &mut summary).await? {
            continue;
        }
        let Some(prepared) = prepare_checkpoint(&folder, nonce_base.wrapping_add(idx as u64))?
        else {
            continue;
        };
        if matches!(
            prepared.held.lifecycle,
            Lifecycle::Finished | Lifecycle::Failed
        ) {
            summary.skipped.push(prepared.run_id.as_str().to_owned());
            continue;
        }
        if matches!(prepared.held.binding, Some(RunBinding::Machine { .. })) {
            return Err(session_bound_for_run(
                prepared.run_id,
                "This Run is bound to the removed Group member's machine and cannot be adopted.",
            ));
        }
        let current_owner =
            lookup_owner_for_adopt(&kept, identity, clock, lead_epoch, &prepared.run_id).await?;
        if let (Some(expected_from), Some(current)) = (from_member, &current_owner)
            && current != expected_from
        {
            // Already adopted or owned by another Group member: do not run twice.
            summary.skipped.push(prepared.run_id.as_str().to_owned());
            continue;
        }
        let mut exclude = Vec::new();
        if let Some(from) = from_member {
            exclude.push(from.to_owned());
        } else if let Some(current) = &current_owner {
            exclude.push(current.clone());
        }
        let placed = place_for_adopt(
            &kept,
            identity,
            clock,
            lead_epoch,
            PlacementRequest {
                receiving_member: kept.name.clone(),
                exclude_members: exclude,
                ..PlacementRequest::default()
            },
        )
        .await?;
        let answered = send_handoff_to_route(
            &kept,
            identity,
            clock,
            lead_epoch,
            &placed,
            &prepared,
            from_member.map(str::to_owned).or(current_owner),
            true,
        )
        .await?;
        summary.adopted.push(AdoptedRun {
            run_id: answered.run_id,
            owner: answered.owner,
            run_epoch: answered.run_epoch,
        });
    }
    Ok(summary)
}

async fn try_adopt_with_hooks(
    hooks: &[Arc<dyn crate::engine_context::RunAdoptHook>],
    folder: &Path,
    from_member: Option<&str>,
    peer_client: &Arc<dyn crate::engine_context::AdoptPeerClient>,
    summary: &mut AdoptedSummary,
) -> Result<bool, CallError> {
    for hook in hooks {
        match hook
            .try_adopt_folder(
                folder.to_path_buf(),
                from_member.map(str::to_owned),
                Arc::clone(peer_client),
            )
            .await?
        {
            crate::engine_context::AdoptHookOutcome::NotMine => {}
            crate::engine_context::AdoptHookOutcome::Handled { adopted, skipped } => {
                summary.adopted.extend(adopted);
                summary.skipped.extend(skipped);
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn lookup_owner_for_adopt(
    kept: &KeptIdentity,
    identity: &Identity,
    clock: Clock,
    lead_epoch: LeadEpoch,
    run_id: &RunId,
) -> Result<Option<String>, CallError> {
    let params = PeerRunIndexLookupParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity.product,
        product_version: identity.product_version.clone(),
        lead_epoch,
        member: kept.name.clone(),
        run_id: Some(run_id.clone()),
        session_id: None,
    };
    let answered: PeerRunIndexLookupResult = ask_peer(
        kept,
        &kept.lead_name,
        &kept.lead_address,
        clock,
        GROUP_RUN_INDEX_LOOKUP,
        &params,
    )
    .await?;
    Ok(answered.route.map(|r| r.member))
}

async fn place_for_adopt(
    kept: &KeptIdentity,
    identity: &Identity,
    clock: Clock,
    lead_epoch: LeadEpoch,
    request: PlacementRequest,
) -> Result<MemberRoute, CallError> {
    let params = PeerRunPlaceParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity.product,
        product_version: identity.product_version.clone(),
        lead_epoch,
        member: kept.name.clone(),
        request,
    };
    let answered: PeerRunPlaceResult = ask_peer(
        kept,
        &kept.lead_name,
        &kept.lead_address,
        clock,
        GROUP_RUN_PLACE,
        &params,
    )
    .await?;
    Ok(answered.placed)
}

#[expect(
    clippy::too_many_arguments,
    reason = "peer handoff helper forwarding identity, clock, epoch, route, and prepared Checkpoint"
)]
async fn send_handoff_to_route(
    kept: &KeptIdentity,
    identity: &Identity,
    clock: Clock,
    lead_epoch: LeadEpoch,
    route: &MemberRoute,
    prepared: &PreparedCheckpoint,
    source_member: Option<String>,
    adopt: bool,
) -> Result<PeerRunHandoffResult, CallError> {
    let params = PeerRunHandoffParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity.product,
        product_version: identity.product_version.clone(),
        lead_epoch,
        member: kept.name.clone(),
        run_id: prepared.run_id.clone(),
        source_member,
        checkpoint_base64: BASE64.encode(&prepared.bytes),
        sha256: prepared.sha256.clone(),
        adopt,
    };
    ask_peer(
        kept,
        &route.member,
        &route.peer_address,
        clock,
        GROUP_RUN_HANDOFF,
        &params,
    )
    .await
}

fn prepare_checkpoint(folder: &Path, nonce: u64) -> Result<Option<PreparedCheckpoint>, CallError> {
    let checkpoint_path = folder.join(CHECKPOINT_FILE);
    let run_file = folder.join(RUN_FILE);
    let (source_path, temp_to_remove) = if is_file_sync(&checkpoint_path) {
        (checkpoint_path, None)
    } else if is_file_sync(&run_file) {
        let temp = folder.join(format!(".adopt-{nonce}.sqlite"));
        checkpoint::write(&run_file, None, &temp, nonce).map_err(|error| internal(&error))?;
        (temp.clone(), Some(temp))
    } else {
        return Ok(None);
    };

    let result = (|| -> Result<PreparedCheckpoint, CallError> {
        let conn = checkpoint::open_checkpoint(&source_path).map_err(|error| internal(&error))?;
        let held = checkpoint::read(&conn, &source_path).map_err(|error| internal(&error))?;
        drop(conn);
        let run_id = RunId::parse(&held.run_id).map_err(|error| internal(&error))?;
        let bytes = read_file_sync(&source_path)?;
        let sha256 = sha256_hex(&bytes);
        Ok(PreparedCheckpoint {
            run_id,
            held,
            bytes,
            sha256,
        })
    })();

    if let Some(temp) = temp_to_remove {
        let _ = remove_file_sync(&temp);
    }
    result.map(Some)
}

fn scan_run_folders(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders = BTreeSet::new();
    for path in paths {
        if is_file_sync(&path.join(CHECKPOINT_FILE)) || is_file_sync(&path.join(RUN_FILE)) {
            folders.insert(path.clone());
            continue;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "scans kept volume folders synchronously in CLI or blocking handoff task"
        )]
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path();
            if is_file_sync(&candidate.join(CHECKPOINT_FILE))
                || is_file_sync(&candidate.join(RUN_FILE))
            {
                folders.insert(candidate);
            }
        }
    }
    folders.into_iter().collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    HEXLOWER.encode(&digest)
}

fn session_bound_for_run(run_id: RunId, message: &str) -> CallError {
    CallError::new(
        &SESSION_BOUND_TO_MACHINE,
        message,
        &SessionBoundToMachineDetails { run_id },
    )
}

fn is_file_sync(path: &Path) -> bool {
    path.is_file()
}

#[expect(
    clippy::disallowed_methods,
    reason = "synchronous file read called only from blocking handoff/adopt paths"
)]
fn read_file_sync(path: &Path) -> Result<Vec<u8>, CallError> {
    std::fs::read(path).map_err(|error| internal(&error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "synchronous temp file cleanup called only from blocking handoff/adopt paths"
)]
fn remove_file_sync(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

async fn remove_temp_file(owner: Option<&Arc<TaskOwner>>, path: PathBuf) {
    if let Some(owner) = owner {
        let _ = owner.blocking(move || remove_file_sync(&path)).await;
    } else {
        let _ = remove_file_sync(&path);
    }
}

async fn path_is_file(owner: Option<&Arc<TaskOwner>>, path: PathBuf) -> bool {
    if let Some(owner) = owner {
        owner.blocking(move || is_file_sync(&path)).await
    } else {
        is_file_sync(&path)
    }
}

trait ClockNonce {
    fn wall_nonce(&self) -> u64;
}

impl ClockNonce for Clock {
    fn wall_nonce(&self) -> u64 {
        u64::try_from(
            self.wall()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        )
        .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests;
