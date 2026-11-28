//! Group Client routing, cached Run index lookups, `run_moved` retry-once,
//! governor headroom push, Lead Daemon placement, Live view single-use
//! redirect tokens and peer relay, and event stream redirect/forwarding
//! (ADRs 0055, 0056, 0132, 0140 and 0149, issue 244).
//!
//! Size: one concern, the Daemon group Client routing and placement layer
//! over `Peering` (cached routes, member directory, headroom push, unary
//! forwarding with `run_moved` retry-once, and Live view / event stream
//! redirect and peer relay), kept together so every peer RPC shares one
//! ownership check, route cache, and mTLS peer link helper.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime};

use data_encoding::HEXLOWER;
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::connection::{Form, Surface};
use goodfetch_protocol::daemon::{ADVERTISE_URL, DaemonToken};
use goodfetch_protocol::egress::EGRESS_LOCAL_ADDRESSES;
use goodfetch_protocol::events::{EventEnvelope, EventsSubscribeParams, LastRead, Position};
use goodfetch_protocol::group::{
    GROUP_EVENTS_SUBSCRIBE, GROUP_HEADROOM_REPORT, GROUP_LIVE_VIEW_MINT, GROUP_LIVE_VIEW_RELAY,
    GROUP_MEMBERS_ELIGIBLE, GROUP_PEER_EXTENSION, GROUP_RPC_FORWARD, GROUP_RUN_INDEX_LOOKUP,
    GROUP_RUN_PLACE, MemberCapabilities, MemberHeadroom, MemberRoute, PEER_PROTOCOL_VERSION,
    PeerEventsSubscribeParams, PeerExtensionParams, PeerExtensionResult, PeerHeadroomReportParams,
    PeerHeadroomReportResult, PeerLiveViewMintParams, PeerLiveViewMintResult,
    PeerLiveViewRelayParams, PeerMembersEligibleParams, PeerMembersEligibleResult,
    PeerRpcForwardParams, PeerRpcForwardResult, PeerRunIndexLookupParams, PeerRunIndexLookupResult,
    PeerRunPlaceParams, PeerRunPlaceResult, PlacementRequest, RUN_MOVED, RunMovedDetails,
};
use goodfetch_protocol::jsonrpc::{ErrorObject, ProtocolFailure, RequestId};
use goodfetch_protocol::live_view::LiveViewScreen;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::{RunId, RunOwner};
use rustls::pki_types::CertificateDer;
use serde_json::value::RawValue;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::mpsc;

use super::wire::{Lines, OPENING_TIMEOUT, connect_peer_tls, write};
use super::{Asked, Peering, ask_lead, ask_peer, presented_name};
use crate::call_error::CallError;
use crate::engine_context::{EventsRoute, ForwardedCall, LiveViewRelay, LiveViewRoute, RoutedCall};
use crate::forward::answered_failure;
use crate::framing::{Frame, Incoming, decode};
use crate::group::errors::{internal, store_failed};
use crate::group::placement::{
    MemberRecord, eligible_members, headroom_changed_noticeably, headroom_from_system_info,
    select_member,
};
use crate::group::{failure_line, result_line, store};
use crate::live_view::{Admission, LiveView, serve_relayed};
use crate::mask::Masks;
use crate::runs::Runs;
use crate::settings::get_or_default;
use crate::sync::Mutex;
use crate::tasks::TaskOwner;
use crate::wire::request_line;

/// How long a minted single-use Live view redirect token stays valid in the
/// owning Group member's memory (ADR 0149).
const LIVE_VIEW_TOKEN_TTL: Duration = Duration::from_secs(60);

/// Channel capacity for forwarded event batches and Live view relay lines.
const RELAY_CAPACITY: usize = 64;

/// How often the headroom pusher re-checks settings / retries after a failure.
const HEADROOM_RETRY: Duration = Duration::from_millis(250);

/// Cached Run and Session ownership routes held in memory by each Group member
/// (ADR 0055). Invalidated and refreshed from the Lead Daemon when a former
/// owner answers `run_moved`.
#[derive(Debug, Default)]
pub(super) struct RouteCache {
    runs: Mutex<BTreeMap<RunId, MemberRoute>>,
    sessions: Mutex<BTreeMap<SessionId, MemberRoute>>,
}

impl RouteCache {
    pub(super) fn get_run(&self, run_id: &RunId) -> Option<MemberRoute> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
    }

    pub(super) fn insert_run(&self, run_id: RunId, route: MemberRoute) {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id, route);
    }

    pub(super) fn invalidate_run(&self, run_id: &RunId) {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id);
    }

    pub(super) fn get_session(&self, session: &SessionId) -> Option<MemberRoute> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .cloned()
    }

    pub(super) fn insert_session(&self, session: SessionId, route: MemberRoute) {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session, route);
    }

    pub(super) fn invalidate_session(&self, session: &SessionId) {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session);
    }
}

/// Lead Daemon's live view of every Group member's headroom, capabilities,
/// `daemon.advertise_url`, and registered Sessions (ADR 0055).
#[derive(Debug, Default)]
pub(super) struct MemberDirectory {
    members: Mutex<BTreeMap<String, MemberRecord>>,
    sessions: Mutex<BTreeMap<SessionId, String>>,
}

impl MemberDirectory {
    pub(super) fn upsert(&self, record: MemberRecord) {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(record.name.clone(), record);
    }

    pub(super) fn get(&self, name: &str) -> Option<MemberRecord> {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    pub(super) fn remove(&self, name: &str) {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }

    #[cfg(test)]
    pub(super) fn register_session(&self, session: SessionId, owner: String) {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session, owner);
    }

    pub(super) fn session_owner(&self, session: &SessionId) -> Option<String> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .cloned()
    }
}

/// In-memory single-use Live view redirect tokens minted by the owning Group
/// member (ADRs 0055 and 0149). Never persisted or shared across the group.
#[derive(Debug, Default)]
pub(super) struct LiveViewTokens {
    tokens: Mutex<BTreeMap<String, (SessionId, SystemTime)>>,
}

impl LiveViewTokens {
    /// Mints one cryptographically random single-use token scoped to `session`.
    pub(super) fn mint(
        &self,
        session: SessionId,
        now: SystemTime,
    ) -> Result<DaemonToken, CallError> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| internal(&error))?;
        let token = HEXLOWER.encode(&bytes);
        let expires_at = now + LIVE_VIEW_TOKEN_TTL;
        let mut guard = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        guard.retain(|_, (_, exp)| *exp > now);
        guard.insert(token.clone(), (session, expires_at));
        Ok(DaemonToken::new(token))
    }

    /// Verifies and consumes `token` for `session` at `now`; returns `true` at
    /// most once while unexpired.
    pub(super) fn consume(&self, session: &SessionId, token: &str, now: SystemTime) -> bool {
        let mut guard = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        guard.retain(|_, (_, exp)| *exp > now);
        let Some((expected, expires_at)) = guard.get(token) else {
            return false;
        };
        if expected != session || *expires_at <= now {
            return false;
        }
        guard.remove(token);
        true
    }
}

/// Destination recorded when a Run moves off this Group member.
pub(super) struct MovedTarget(pub(super) Option<String>);

/// Runs that have moved off this Group member, so it immediately answers
/// `run_moved` rather than serving stale local state (ADR 0055).
#[derive(Debug, Default)]
pub(super) struct MovedRuns {
    runs: Mutex<BTreeMap<RunId, Option<String>>>,
}

impl MovedRuns {
    pub(super) fn mark_moved(&self, run_id: RunId, new_owner: Option<String>) {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id, new_owner);
    }

    pub(super) fn get(&self, run_id: &RunId) -> Option<MovedTarget> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
            .map(MovedTarget)
    }
}

/// Builds a `run_moved` [`CallError`] for `run_id`.
pub(super) fn run_moved_error(run_id: RunId, owner: Option<String>) -> CallError {
    let message = match &owner {
        Some(owner) => format!("Run `{run_id}` moved to Group member `{owner}`."),
        None => format!("Run `{run_id}` moved to another Group member."),
    };
    CallError::new(&RUN_MOVED, message, &RunMovedDetails { run_id, owner })
}

/// Background task that watches this Group member's Resource governor headroom
/// and pushes updates to the Lead Daemon (or updates the Lead Daemon's own
/// [`MemberDirectory`]) whenever headroom or capabilities change noticeably
/// (ADR 0055).
///
/// # Cancel safety
///
/// Dropping the future stops reporting further headroom updates.
pub(super) async fn push_headroom(peering: Arc<Peering>, owner: TaskOwner) {
    peering.run_push_headroom(owner).await;
}

impl Peering {
    /// Reads this Group member's current capabilities (`daemon.advertise_url`,
    /// `egress.local_addresses`, product version, and any registered capabilities).
    async fn read_local_capabilities(&self) -> MemberCapabilities {
        let mut caps = MemberCapabilities {
            product_version: self.identity.product_version.clone(),
            ..MemberCapabilities::default()
        };
        if let Some(settings) = &self.services.settings {
            let machine = settings.snapshot().await;
            if let Ok(resolved) = machine.resolved() {
                caps.advertise_url = get_or_default::<Option<&str>>(Some(resolved), &ADVERTISE_URL)
                    .map(str::to_owned)
                    .filter(|url| !url.trim().is_empty());
                if let Some(items) = resolved.value(&EGRESS_LOCAL_ADDRESSES).as_array() {
                    caps.local_addresses = items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                }
            }
        }
        caps
    }

    async fn run_push_headroom(self: Arc<Self>, owner: TaskOwner) {
        let mut info_rx = self
            .services
            .governor
            .as_ref()
            .map(crate::governor::Governor::system_info);
        let mut last_pushed: Option<(MemberHeadroom, MemberCapabilities)> = None;

        loop {
            let Ok(Some((kept, lead_epoch))) = store::identity(&self.store).await else {
                return;
            };
            let info = info_rx.as_ref().and_then(|rx| rx.borrow().clone());
            let headroom = headroom_from_system_info(info.as_ref(), true);
            let capabilities = self.read_local_capabilities().await;
            let current = (headroom, capabilities);

            if headroom_changed_noticeably(last_pushed.as_ref(), &current) {
                if kept.is_lead() {
                    self.directory.upsert(MemberRecord {
                        name: kept.name.clone(),
                        peer_address: kept.peer_address.clone(),
                        headroom: current.0,
                        capabilities: current.1.clone(),
                    });
                    last_pushed = Some(current);
                } else {
                    let params = PeerHeadroomReportParams {
                        protocol_version: PEER_PROTOCOL_VERSION,
                        product: self.identity.product,
                        product_version: self.identity.product_version.clone(),
                        lead_epoch,
                        member: kept.name.clone(),
                        headroom: current.0,
                        capabilities: current.1.clone(),
                    };
                    match ask_lead::<PeerHeadroomReportResult>(
                        &kept,
                        self.clock,
                        GROUP_HEADROOM_REPORT,
                        &params,
                    )
                    .await
                    {
                        Ok(answered) => {
                            if answered.lead_epoch > lead_epoch {
                                let _ = self.admit_epoch(answered.lead_epoch).await;
                            }
                            last_pushed = Some(current);
                        }
                        Err(error) => {
                            tracing::debug!(
                                %error,
                                "failed to report governor headroom to the Lead Daemon; retrying"
                            );
                            tokio::select! {
                                () = owner.token().cancelled() => return,
                                () = self.clock.sleep(HEADROOM_RETRY) => continue,
                            }
                        }
                    }
                }
            }

            if let Some(rx) = &mut info_rx {
                tokio::select! {
                    () = owner.token().cancelled() => return,
                    changed = rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            } else {
                // Unsampled governor (e.g. unit tests without a system sampler):
                // initial snapshot was pushed; stay parked until cancelled.
                owner.token().cancelled().await;
                return;
            }
        }
    }
}

impl Peering {
    /// Marks `run_id` as moved off this Group member to `new_owner` so any
    /// forwarded call arriving here answers `run_moved` immediately.
    pub(super) fn mark_run_moved(&self, run_id: &RunId, new_owner: Option<String>) {
        self.moved_runs.mark_moved(run_id.clone(), new_owner);
        self.route_cache.invalidate_run(run_id);
    }

    /// Registers a Session on `owner` in the Lead Daemon's directory (used by
    /// tests and Session registration).
    #[cfg(test)]
    pub(super) fn register_session_owner(&self, session: SessionId, owner: String) {
        self.directory.register_session(session, owner);
    }

    /// Records a Group member's headroom and capabilities in the Lead Daemon's
    /// directory.
    #[cfg(test)]
    pub(super) fn record_member_for_test(&self, record: MemberRecord) {
        self.directory.upsert(record);
    }

    /// Returns the cached route for `run_id`, if present.
    #[cfg(test)]
    pub(super) fn cached_run_route(&self, run_id: &RunId) -> Option<MemberRoute> {
        self.route_cache.get_run(run_id)
    }

    /// Answers `group.headroom.report` on the Lead Daemon.
    pub(super) async fn headroom_report(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerHeadroomReportParams>,
    ) -> Vec<u8> {
        match self
            .handle_headroom_report(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_headroom_report(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerHeadroomReportParams,
    ) -> Result<PeerHeadroomReportResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let name = presented_name(presented, &params.member)?;
        let peer_address = store::members(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .into_iter()
            .find(|m| m.name == name.as_str())
            .map(|m| m.peer_address)
            .unwrap_or_default();
        self.directory.upsert(MemberRecord {
            name: name.as_str().to_owned(),
            peer_address,
            headroom: params.headroom,
            capabilities: params.capabilities,
        });
        Ok(PeerHeadroomReportResult {
            protocol_version: settled,
            lead_epoch,
        })
    }

    /// Answers `group.run_index.lookup` on the Lead Daemon.
    pub(super) async fn run_index_lookup(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerRunIndexLookupParams>,
    ) -> Vec<u8> {
        match self
            .handle_run_index_lookup(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_run_index_lookup(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRunIndexLookupParams,
    ) -> Result<PeerRunIndexLookupResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let _ = presented_name(presented, &params.member)?;
        let route = if let Some(run_id) = &params.run_id {
            self.lead_lookup_run(run_id).await?
        } else if let Some(session_id) = &params.session_id {
            self.lead_lookup_session(session_id).await?
        } else {
            None
        };
        Ok(PeerRunIndexLookupResult {
            protocol_version: settled,
            lead_epoch,
            route,
        })
    }

    /// Looks up `run_id` in the Lead Daemon's group Run index and resolves the
    /// owning Group member's peer address and `daemon.advertise_url`.
    async fn lead_lookup_run(&self, run_id: &RunId) -> Result<Option<MemberRoute>, CallError> {
        let Some(run_index) = &self.services.run_index else {
            return Ok(None);
        };
        let Some(row) = run_index
            .read(run_id.clone())
            .await
            .map_err(|error| internal(&error))?
        else {
            return Ok(None);
        };
        let Some(RunOwner::GroupMember(owner)) = row.owner else {
            return Ok(None);
        };
        self.member_route_on_lead(&owner).await
    }

    /// Looks up `session_id` on the Lead Daemon: first in the session directory,
    /// then locally on the Lead Daemon's `LiveView`, then probing active Group
    /// members.
    async fn lead_lookup_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<MemberRoute>, CallError> {
        if let Some(owner) = self.directory.session_owner(session_id) {
            return self.member_route_on_lead(&owner).await;
        }
        if let Some(lv) = self.current_live_view()
            && lv.has_session(session_id)
        {
            return self.member_route_on_lead(&self.name).await;
        }
        Ok(None)
    }

    /// Resolves `member`'s [`MemberRoute`] from the Lead Daemon's `members`
    /// table and [`MemberDirectory`].
    async fn member_route_on_lead(&self, member: &str) -> Result<Option<MemberRoute>, CallError> {
        let stored = store::members(&self.store)
            .await
            .map_err(|error| store_failed(&error))?;
        let Some(entry) = stored.into_iter().find(|m| m.name == member) else {
            return Ok(None);
        };
        let advertise_url = if member == self.name {
            self.read_local_capabilities().await.advertise_url
        } else {
            self.directory
                .get(member)
                .and_then(|record| record.capabilities.advertise_url)
        };
        Ok(Some(MemberRoute {
            member: entry.name,
            peer_address: entry.peer_address,
            advertise_url,
        }))
    }

    /// Answers `group.run.place` on the Lead Daemon.
    pub(super) async fn run_place(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerRunPlaceParams>,
    ) -> Vec<u8> {
        match self
            .handle_run_place(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_run_place(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRunPlaceParams,
    ) -> Result<PeerRunPlaceResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let _ = presented_name(presented, &params.member)?;
        let placed = self.place_on_lead(&params.request).await?;
        Ok(PeerRunPlaceResult {
            protocol_version: settled,
            lead_epoch,
            placed,
        })
    }

    /// Runs placement on the Lead Daemon using the merged list of admitted
    /// members and their latest reported headroom and capabilities.
    pub(super) async fn place_on_lead(
        &self,
        request: &PlacementRequest,
    ) -> Result<MemberRoute, CallError> {
        let _ = self.require_confirmed_lead()?;
        let members = self.all_member_records().await?;
        select_member(&members, request)
    }

    /// Answers `group.members.eligible` on the Lead Daemon.
    pub(super) async fn members_eligible(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerMembersEligibleParams>,
    ) -> Vec<u8> {
        match self
            .handle_members_eligible(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_members_eligible(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerMembersEligibleParams,
    ) -> Result<PeerMembersEligibleResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let _ = presented_name(presented, &params.member)?;
        let members = self.all_member_records().await?;
        let eligible = eligible_members(&members, &params.request)?;
        Ok(PeerMembersEligibleResult {
            protocol_version: settled,
            lead_epoch,
            members: eligible,
        })
    }

    /// Answers `group.peer.extension` on any Group member by delegating to the
    /// contribution-registered peer extension handler.
    pub(super) async fn peer_extension(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerExtensionParams>,
    ) -> Vec<u8> {
        match self
            .handle_peer_extension(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_peer_extension(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerExtensionParams,
    ) -> Result<PeerExtensionResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let sender = presented_name(presented, &params.member)?;
        let result = self
            .dispatch_local_extension(
                sender.as_str().to_owned(),
                lead_epoch,
                &params.extension_method,
                params.payload,
            )
            .await?;
        Ok(PeerExtensionResult {
            protocol_version: settled,
            lead_epoch,
            result,
        })
    }

    async fn dispatch_local_extension(
        &self,
        sender_member: String,
        lead_epoch: goodfetch_protocol::group::LeadEpoch,
        method: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        let Some(handler) = self.services.peer_extensions.get(method) else {
            return Err(CallError::protocol(
                ProtocolFailure::MethodNotFound,
                format!("No peer extension handler is registered for `{method}`."),
            ));
        };
        handler.handle(sender_member, lead_epoch, payload).await
    }

    pub(super) async fn query_eligible_members(
        &self,
        request: &PlacementRequest,
    ) -> Result<Vec<MemberRoute>, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        if kept.is_lead() {
            let _ = self.require_confirmed_lead()?;
            let members = self.all_member_records().await?;
            return eligible_members(&members, request);
        }
        let params = PeerMembersEligibleParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            request: request.clone(),
        };
        let answered = ask_lead::<PeerMembersEligibleResult>(
            &kept,
            self.clock,
            GROUP_MEMBERS_ELIGIBLE,
            &params,
        )
        .await?;
        if answered.lead_epoch > lead_epoch {
            let _ = self.admit_epoch(answered.lead_epoch).await;
        }
        Ok(answered.members)
    }

    pub(super) async fn resolve_member_route(
        &self,
        member: &str,
    ) -> Result<Option<MemberRoute>, CallError> {
        let Some((kept, _)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Ok(None);
        };
        if kept.is_lead() {
            return self.member_route_on_lead(member).await;
        }
        let eligible = self
            .query_eligible_members(&PlacementRequest {
                receiving_member: self.name.clone(),
                group_member: Some(member.to_owned()),
                ..PlacementRequest::default()
            })
            .await;
        Ok(eligible.ok().and_then(|mut routes| routes.drain(..).next()))
    }

    pub(super) async fn invoke_member_extension(
        &self,
        member: &str,
        peer_address: &str,
        extension_method: String,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        if member == self.name {
            return self
                .dispatch_local_extension(self.name.clone(), lead_epoch, &extension_method, payload)
                .await;
        }
        let params = PeerExtensionParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            extension_method,
            payload,
        };
        let answered = ask_peer::<PeerExtensionResult>(
            &kept,
            member,
            peer_address,
            self.clock,
            GROUP_PEER_EXTENSION,
            &params,
        )
        .await?;
        if answered.lead_epoch > lead_epoch {
            let _ = self.admit_epoch(answered.lead_epoch).await;
        }
        Ok(answered.result)
    }

    pub(super) async fn invoke_lead_extension(
        &self,
        extension_method: String,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        if kept.is_lead() {
            return self
                .dispatch_local_extension(self.name.clone(), lead_epoch, &extension_method, payload)
                .await;
        }
        let params = PeerExtensionParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            extension_method,
            payload,
        };
        let answered =
            ask_lead::<PeerExtensionResult>(&kept, self.clock, GROUP_PEER_EXTENSION, &params)
                .await?;
        if answered.lead_epoch > lead_epoch {
            let _ = self.admit_epoch(answered.lead_epoch).await;
        }
        Ok(answered.result)
    }

    pub(super) async fn invoke_member_rpc(
        &self,
        member: &str,
        peer_address: &str,
        method: String,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        let params_raw = serde_json::to_string(&params).map_err(|error| internal(&error))?;
        let call = ForwardedCall {
            id: RequestId::Number(1),
            method,
            params_raw: Some(params_raw),
            surface: Surface::Rust,
            form: Form::Rust,
            behind_gateway: false,
            answers_as_a_person: false,
        };
        let raw_line = if member == self.name {
            let Some(dispatcher) = &self.services.local_dispatcher else {
                return Err(internal(&"local RPC dispatcher is not wired on this peer"));
            };
            dispatcher.dispatch(call).await
        } else {
            let route = MemberRoute {
                member: member.to_owned(),
                peer_address: peer_address.to_owned(),
                advertise_url: None,
            };
            self.forward_unary_once(&route, &call, None).await?
        };
        let reply: serde_json::Value =
            serde_json::from_slice(&raw_line).map_err(|error| internal(&error))?;
        if let Some(error_val) = reply.get("error") {
            let error: ErrorObject =
                serde_json::from_value(error_val.clone()).map_err(|e| internal(&e))?;
            return Err(
                match goodfetch_protocol::entries::error_code(&error.data.code) {
                    Some(def) => CallError::new(def, error.message, &error.data.details),
                    None => internal(&format!("{}: {}", error.data.code, error.message)),
                },
            );
        }
        Ok(reply
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// Merges the Lead Daemon's `members` table with reported [`MemberRecord`]s
    /// (ensuring every admitted member, including the Lead Daemon itself, has
    /// a record even before its first headroom push).
    async fn all_member_records(&self) -> Result<Vec<MemberRecord>, CallError> {
        let stored = store::members(&self.store)
            .await
            .map_err(|error| store_failed(&error))?;
        let mut records = Vec::with_capacity(stored.len());
        for member in stored {
            if let Some(mut record) = self.directory.get(&member.name) {
                if record.peer_address.is_empty() {
                    record.peer_address = member.peer_address;
                }
                records.push(record);
            } else if member.name == self.name {
                let info = self
                    .services
                    .governor
                    .as_ref()
                    .and_then(|gov| gov.system_info().borrow().clone());
                records.push(MemberRecord {
                    name: member.name,
                    peer_address: member.peer_address,
                    headroom: headroom_from_system_info(info.as_ref(), true),
                    capabilities: self.read_local_capabilities().await,
                });
            } else {
                records.push(MemberRecord {
                    name: member.name,
                    peer_address: member.peer_address,
                    headroom: MemberHeadroom::default(),
                    capabilities: MemberCapabilities {
                        product_version: self.identity.product_version.clone(),
                        ..MemberCapabilities::default()
                    },
                });
            }
        }
        Ok(records)
    }

    /// Answers `group.rpc.forward` on the owning Group member.
    pub(super) async fn rpc_forward(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerRpcForwardParams>,
    ) -> Vec<u8> {
        match self
            .handle_rpc_forward(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_rpc_forward(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerRpcForwardParams,
    ) -> Result<PeerRpcForwardResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = presented_name(presented, &params.member)?;

        if let Some(run_id) = &params.run_id {
            self.verify_local_run_ownership(run_id).await?;
        }

        let Some(dispatcher) = &self.services.local_dispatcher else {
            return Err(internal(&"local RPC dispatcher is not wired on this peer"));
        };
        let call = ForwardedCall {
            id: params.request_id,
            method: params.method,
            params_raw: params.params.map(|raw| raw.get().to_owned()),
            surface: params.surface,
            form: params.form,
            behind_gateway: params.behind_gateway,
            answers_as_a_person: params.answers_as_a_person,
        };
        let raw_line = dispatcher.dispatch(call).await;
        let trimmed = String::from_utf8(raw_line).map_err(|error| internal(&error))?;
        let trimmed = trimmed.trim_end_matches(['\r', '\n']);
        let response =
            RawValue::from_string(trimmed.to_owned()).map_err(|error| internal(&error))?;
        Ok(PeerRpcForwardResult {
            protocol_version: settled,
            lead_epoch,
            response,
        })
    }

    /// Verifies that `run_id` has not moved away from this Group member.
    /// Answers `run_moved` if `run_id` was marked moved or belongs to another
    /// Group member in the Run index.
    async fn verify_local_run_ownership(&self, run_id: &RunId) -> Result<(), CallError> {
        if let Some(MovedTarget(new_owner)) = self.moved_runs.get(run_id) {
            return Err(run_moved_error(run_id.clone(), new_owner));
        }
        if let Some(runs) = &self.services.runs
            && runs.live(run_id.clone()).await.is_ok()
        {
            return Ok(());
        }
        if let Some(run_index) = &self.services.run_index {
            match run_index.read(run_id.clone()).await {
                Ok(Some(row)) => match row.owner {
                    Some(RunOwner::GroupMember(ref owner)) if owner != &self.name => {
                        return Err(run_moved_error(run_id.clone(), Some(owner.clone())));
                    }
                    _ => return Ok(()),
                },
                Ok(None) => {
                    return Err(run_moved_error(run_id.clone(), None));
                }
                Err(error) => return Err(internal(&error)),
            }
        }
        Ok(())
    }

    /// Resolves the owning Group member for `run_id`, consulting the local
    /// cache first unless `refresh` is `true`.
    pub(super) async fn lookup_run_owner(
        &self,
        run_id: &RunId,
        refresh: bool,
    ) -> Result<Option<MemberRoute>, CallError> {
        if !refresh && let Some(cached) = self.route_cache.get_run(run_id) {
            return Ok(Some(cached));
        }
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Ok(None);
        };
        let route = if kept.is_lead() {
            self.lead_lookup_run(run_id).await?
        } else {
            let params = PeerRunIndexLookupParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                member: kept.name.clone(),
                run_id: Some(run_id.clone()),
                session_id: None,
            };
            let answered = ask_lead::<PeerRunIndexLookupResult>(
                &kept,
                self.clock,
                GROUP_RUN_INDEX_LOOKUP,
                &params,
            )
            .await?;
            if answered.lead_epoch > lead_epoch {
                let _ = self.admit_epoch(answered.lead_epoch).await;
            }
            answered.route
        };
        if let Some(route) = &route {
            self.route_cache.insert_run(run_id.clone(), route.clone());
        } else {
            self.route_cache.invalidate_run(run_id);
        }
        Ok(route)
    }

    /// Requests run placement from the Lead Daemon (locally if this Group
    /// member is the Lead Daemon, over `group.run.place` otherwise).
    pub(super) async fn place_run(
        &self,
        request: &PlacementRequest,
    ) -> Result<MemberRoute, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        if kept.is_lead() {
            return self.place_on_lead(request).await;
        }
        let params = PeerRunPlaceParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            request: request.clone(),
        };
        let answered =
            ask_lead::<PeerRunPlaceResult>(&kept, self.clock, GROUP_RUN_PLACE, &params).await?;
        if answered.lead_epoch > lead_epoch {
            let _ = self.admit_epoch(answered.lead_epoch).await;
        }
        Ok(answered.placed)
    }

    /// Routes a unary Client call for `run_id` (or places a new/resumed Run
    /// given `placement`): forwards raw request bytes over the peer link when
    /// another Group member owns it, and on `run_moved` refreshes the route
    /// cache and retries once (ADRs 0055, 0132 and 0140).
    pub(super) async fn route_unary_call(
        &self,
        call: ForwardedCall,
        run_id: Option<RunId>,
        placement: Option<PlacementRequest>,
    ) -> Result<RoutedCall, CallError> {
        let mut target = if let Some(ref run_id) = run_id {
            // Fast path: if this Group member owns the Run live right now and
            // has not moved it, handle locally with zero network round trips.
            if self.moved_runs.get(run_id).is_none()
                && let Some(runs) = &self.services.runs
                && runs.live(run_id.clone()).await.is_ok()
            {
                return Ok(RoutedCall::Local);
            }
            self.lookup_run_owner(run_id, false).await?
        } else {
            None
        };

        // For start/resume calls with no existing remote owner, ask the Lead
        // Daemon to place the run.
        let placed_new = target.is_none() && placement.is_some();
        if let (None, Some(mut req)) = (&target, placement) {
            req.receiving_member = self.name.clone();
            target = Some(self.place_run(&req).await?);
        }

        let Some(route) = target else {
            return Ok(RoutedCall::Local);
        };
        if route.member == self.name {
            return Ok(RoutedCall::Local);
        }

        let forward_run_id = if placed_new { None } else { run_id.clone() };
        match self
            .forward_unary_once(&route, &call, forward_run_id.clone())
            .await
        {
            Ok(bytes) => Ok(RoutedCall::Forwarded(bytes)),
            Err(error) if error.code() == RUN_MOVED.name => {
                let Some(ref moved_id) = run_id else {
                    return Err(error);
                };
                // Refresh cache from the Lead Daemon and retry ONCE (ADR 0055).
                let refreshed = self.lookup_run_owner(moved_id, true).await?;
                let Some(new_route) = refreshed else {
                    return Err(error);
                };
                if new_route.member == self.name {
                    return Ok(RoutedCall::Local);
                }
                let bytes = self
                    .forward_unary_once(&new_route, &call, Some(moved_id.clone()))
                    .await?;
                Ok(RoutedCall::Forwarded(bytes))
            }
            Err(error) => Err(error),
        }
    }

    async fn forward_unary_once(
        &self,
        route: &MemberRoute,
        call: &ForwardedCall,
        run_id: Option<RunId>,
    ) -> Result<Vec<u8>, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        let raw_params = call
            .params_raw
            .as_ref()
            .map(|text| RawValue::from_string(text.clone()))
            .transpose()
            .map_err(|error| internal(&error))?;
        let params = PeerRpcForwardParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            request_id: call.id.clone(),
            method: call.method.clone(),
            run_id,
            params: raw_params,
            surface: call.surface,
            form: call.form,
            behind_gateway: call.behind_gateway,
            answers_as_a_person: call.answers_as_a_person,
        };
        let answered = ask_peer::<PeerRpcForwardResult>(
            &kept,
            &route.member,
            &route.peer_address,
            self.clock,
            GROUP_RPC_FORWARD,
            &params,
        )
        .await?;
        if answered.lead_epoch > lead_epoch {
            let _ = self.admit_epoch(answered.lead_epoch).await;
        }
        let mut bytes = answered.response.get().as_bytes().to_vec();
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Resolves how `events.subscribe` for `run_id` should be served: locally,
    /// redirected to the owner's `daemon.advertise_url`, or forwarded over the
    /// peer link (ADR 0055).
    pub(super) async fn route_events_for(&self, run_id: &RunId) -> Result<EventsRoute, CallError> {
        if self.moved_runs.get(run_id).is_none()
            && let Some(runs) = &self.services.runs
            && runs.live(run_id.clone()).await.is_ok()
        {
            return Ok(EventsRoute::Local);
        }
        let Some(route) = self.lookup_run_owner(run_id, false).await? else {
            return Ok(EventsRoute::Local);
        };
        if route.member == self.name {
            return Ok(EventsRoute::Local);
        }
        if let Some(url) = route.advertise_url {
            Ok(EventsRoute::Redirect {
                url,
                owner: route.member,
            })
        } else {
            Ok(EventsRoute::Forward {
                owner: route.member,
                peer_address: route.peer_address,
            })
        }
    }

    /// Opens a forwarded event stream over the peer link from `owner` at
    /// `peer_address`, refreshing the cache and retrying once on `run_moved`.
    pub(super) async fn forward_events_stream(
        &self,
        owner: String,
        peer_address: String,
        params: EventsSubscribeParams,
    ) -> Result<mpsc::Receiver<Vec<EventEnvelope>>, CallError> {
        match self
            .open_events_peer_stream(&owner, &peer_address, &params)
            .await
        {
            Ok(rx) => Ok(rx),
            Err(error) if error.code() == RUN_MOVED.name => {
                let Some(ref run_id) = params.run_id else {
                    return Err(error);
                };
                let Some(refreshed) = self.lookup_run_owner(run_id, true).await? else {
                    return Err(error);
                };
                self.open_events_peer_stream(&refreshed.member, &refreshed.peer_address, &params)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn open_events_peer_stream(
        &self,
        peer_name: &str,
        peer_address: &str,
        params: &EventsSubscribeParams,
    ) -> Result<mpsc::Receiver<Vec<EventEnvelope>>, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        let mut stream = connect_peer_tls(&kept, peer_name, peer_address, self.clock).await?;
        let wire_params = PeerEventsSubscribeParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            run_id: params.run_id.clone(),
            after_engine_instance: params.after.as_ref().map(|a| a.engine_instance.clone()),
            after_position: params.after.as_ref().map(|a| a.position.0),
            system_info: params.system_info,
        };
        let req_line = request_line(&RequestId::Number(1), GROUP_EVENTS_SUBSCRIBE, &wire_params)
            .map_err(|error| internal(&error))?;
        if !write(&mut stream, &req_line).await {
            return Err(internal(
                &"peer link closed before event subscription opened",
            ));
        }
        let mut lines = Lines::new();
        let first = tokio::select! {
            first = lines.next(&mut stream) => first,
            () = self.clock.sleep(OPENING_TIMEOUT) => None,
        };
        let Some(Frame::Line(reply)) = first else {
            return Err(internal(
                &"peer link closed before acknowledging event subscription",
            ));
        };
        match decode(reply) {
            Ok(Incoming::Reply { answer: Ok(_), .. }) => {}
            Ok(Incoming::Reply {
                answer: Err(failure),
                ..
            }) => return Err(answered_failure(&failure)),
            _ => {
                return Err(internal(&"unexpected reply opening forwarded event stream"));
            }
        }
        let (tx, rx) = mpsc::channel(RELAY_CAPACITY);
        let Some(task_owner) = &self.services.owner else {
            return Err(internal(&"task owner is not wired for forwarded events"));
        };
        let cancel = task_owner.token().clone();
        task_owner.spawn(async move {
            loop {
                let frame = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    () = tx.closed() => break,
                    frame = lines.next(&mut stream) => frame,
                };
                let Some(Frame::Line(line)) = frame else {
                    break;
                };
                let Ok(batch) = serde_json::from_slice::<Vec<EventEnvelope>>(&line) else {
                    break;
                };
                if tx.send(batch).await.is_err() {
                    break;
                }
            }
            let _ = stream.shutdown().await;
        });
        Ok(rx)
    }

    /// Serves an incoming `group.events.subscribe` peer link on the owning
    /// Group member: verifies Run ownership (answering `run_moved` if moved),
    /// sends an acknowledgement line, and streams event batches as newline JSON.
    pub(super) async fn events_subscribe_peer(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerEventsSubscribeParams>,
        stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    ) {
        let params = request.params;
        if let Err(error) = self.admit_epoch(params.lead_epoch).await {
            let _ = write(stream, &failure_line(Some(request.id), &error)).await;
            return;
        }
        if let Err(error) = presented_name(presented, &params.member) {
            let _ = write(stream, &failure_line(Some(request.id), &error)).await;
            return;
        }
        if let Some(ref run_id) = params.run_id
            && let Err(error) = self.verify_local_run_ownership(run_id).await
        {
            let _ = write(stream, &failure_line(Some(request.id), &error)).await;
            return;
        }
        let Some(events) = &self.services.events else {
            let _ = write(
                stream,
                &failure_line(
                    Some(request.id),
                    &internal(&"event stream is not wired on this Group member"),
                ),
            )
            .await;
            return;
        };
        let after = params.after_engine_instance.zip(params.after_position).map(
            |(engine_instance, position)| LastRead {
                engine_instance,
                position: Position(position),
            },
        );
        let Some(mut subscription) = events.subscribe(after, params.system_info).await else {
            let _ = write(
                stream,
                &failure_line(Some(request.id), &internal(&"event stream stopped")),
            )
            .await;
            return;
        };
        let ack = result_line(&request.id, &serde_json::json!({ "subscribed": true }));
        if !write(stream, &ack).await {
            return;
        }
        let mut eof_buf = [0u8; 16];
        loop {
            let batch = tokio::select! {
                biased;
                read = stream.read(&mut eof_buf) => match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) => continue,
                },
                batch = subscription.next(64) => match batch {
                    Some(batch) => batch,
                    None => break,
                },
            };
            let mut wire_batch = Vec::with_capacity(batch.len());
            for env in batch {
                let data = serde_json::from_str(env.data.json()).unwrap_or(serde_json::Value::Null);
                wire_batch.push(EventEnvelope {
                    position: env.position,
                    event: env.event.to_owned(),
                    data,
                    dropped: env.dropped,
                });
            }
            let Ok(mut encoded) = serde_json::to_vec(&wire_batch) else {
                break;
            };
            encoded.push(b'\n');
            if !write(stream, &encoded).await {
                break;
            }
        }
    }

    /// Answers `group.live_view.mint_token` on the owning Group member: verifies
    /// the Session is open locally, mints a single-use token in memory (ADR
    /// 0149), and returns it alongside this member's `daemon.advertise_url`.
    pub(super) async fn live_view_mint(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerLiveViewMintParams>,
    ) -> Vec<u8> {
        match self
            .handle_live_view_mint(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_live_view_mint(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerLiveViewMintParams,
    ) -> Result<PeerLiveViewMintResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = presented_name(presented, &params.member)?;
        let Some(live_view) = self.current_live_view() else {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("no open Session of this Daemon is `{}`", params.session.0),
            ));
        };
        if !live_view.has_session(&params.session) {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("no open Session of this Daemon is `{}`", params.session.0),
            ));
        }
        let token = self
            .live_view_tokens
            .mint(params.session, self.clock.wall())?;
        let advertise_url = self.read_local_capabilities().await.advertise_url;
        Ok(PeerLiveViewMintResult {
            protocol_version: settled,
            lead_epoch,
            token,
            advertise_url,
        })
    }

    /// Resolves how a Live view channel for `session` should be served when
    /// `session` is not open locally: redirects with a single-use channel token
    /// when the owning Group member has `daemon.advertise_url`, or relays over
    /// the peer link otherwise (ADRs 0055 and 0149).
    pub(super) async fn route_live_view_for(
        &self,
        session: &SessionId,
    ) -> Result<LiveViewRoute, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Ok(LiveViewRoute::Local);
        };
        let route = if let Some(cached) = self.route_cache.get_session(session) {
            Some(cached)
        } else if kept.is_lead() {
            self.lead_lookup_session(session).await?
        } else {
            let params = PeerRunIndexLookupParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch,
                member: kept.name.clone(),
                run_id: None,
                session_id: Some(session.clone()),
            };
            let answered = ask_lead::<PeerRunIndexLookupResult>(
                &kept,
                self.clock,
                GROUP_RUN_INDEX_LOOKUP,
                &params,
            )
            .await?;
            if answered.lead_epoch > lead_epoch {
                let _ = self.admit_epoch(answered.lead_epoch).await;
            }
            answered.route
        };
        let Some(route) = route else {
            return Ok(LiveViewRoute::Local);
        };
        self.route_cache
            .insert_session(session.clone(), route.clone());
        if route.member == self.name {
            return Ok(LiveViewRoute::Local);
        }
        if let Some(url) = route.advertise_url {
            let mint_params = PeerLiveViewMintParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                lead_epoch: self.highest_epoch().await,
                member: kept.name.clone(),
                session: session.clone(),
            };
            match ask_peer::<PeerLiveViewMintResult>(
                &kept,
                &route.member,
                &route.peer_address,
                self.clock,
                GROUP_LIVE_VIEW_MINT,
                &mint_params,
            )
            .await
            {
                Ok(minted) => {
                    let redirect_url = minted.advertise_url.unwrap_or(url);
                    Ok(LiveViewRoute::Redirect {
                        url: redirect_url,
                        token: minted.token,
                        owner: route.member,
                    })
                }
                Err(error) => {
                    self.route_cache.invalidate_session(session);
                    Err(error)
                }
            }
        } else {
            Ok(LiveViewRoute::Relay {
                owner: route.member,
                peer_address: route.peer_address,
            })
        }
    }

    /// Opens a relayed Live view channel over the peer link to `owner` when it
    /// has no `daemon.advertise_url` (ADR 0055).
    pub(super) async fn open_live_view_relay_stream(
        &self,
        peer_name: String,
        peer_address: String,
        session: SessionId,
        screen: LiveViewScreen,
    ) -> Result<LiveViewRelay, CallError> {
        let Some((kept, lead_epoch)) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
        else {
            return Err(internal(&"this Daemon is not in a group"));
        };
        let mut stream = connect_peer_tls(&kept, &peer_name, &peer_address, self.clock).await?;
        let wire_params = PeerLiveViewRelayParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: kept.name.clone(),
            session,
            screen,
        };
        let req_line = request_line(&RequestId::Number(1), GROUP_LIVE_VIEW_RELAY, &wire_params)
            .map_err(|error| internal(&error))?;
        if !write(&mut stream, &req_line).await {
            return Err(internal(&"peer link closed before Live view relay opened"));
        }
        let mut lines = Lines::new();
        let first = tokio::select! {
            first = lines.next(&mut stream) => first,
            () = self.clock.sleep(OPENING_TIMEOUT) => None,
        };
        let Some(Frame::Line(reply)) = first else {
            return Err(internal(
                &"peer link closed before acknowledging Live view relay",
            ));
        };
        match decode(reply) {
            Ok(Incoming::Reply { answer: Ok(_), .. }) => {}
            Ok(Incoming::Reply {
                answer: Err(failure),
                ..
            }) => return Err(answered_failure(&failure)),
            _ => return Err(internal(&"unexpected reply opening Live view relay")),
        }

        let (to_owner, mut from_viewer) = mpsc::channel::<String>(RELAY_CAPACITY);
        let (to_viewer, from_owner) = mpsc::channel::<String>(RELAY_CAPACITY);
        let Some(task_owner) = &self.services.owner else {
            return Err(internal(&"task owner is not wired for Live view relay"));
        };
        let cancel = task_owner.token().clone();
        task_owner.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    () = to_viewer.closed() => break,
                    frame = lines.next(&mut stream) => match frame {
                        Some(Frame::Line(bytes)) => {
                            let Ok(text) = String::from_utf8(bytes) else { break; };
                            if to_viewer.send(text).await.is_err() {
                                break;
                            }
                        }
                        _ => break,
                    },
                    viewer_msg = from_viewer.recv() => match viewer_msg {
                        Some(mut text) => {
                            if !text.ends_with('\n') {
                                text.push('\n');
                            }
                            if !write(&mut stream, text.as_bytes()).await {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
            let _ = stream.shutdown().await;
        });
        Ok(LiveViewRelay {
            to_owner,
            from_owner,
        })
    }

    /// Serves an incoming `group.live_view.relay` peer link on the owning
    /// Group member (ADR 0055).
    pub(super) async fn live_view_relay_peer(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerLiveViewRelayParams>,
        stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
        lines: &mut Lines,
    ) {
        let params = request.params;
        if let Err(error) = self.admit_epoch(params.lead_epoch).await {
            let _ = write(stream, &failure_line(Some(request.id), &error)).await;
            return;
        }
        if let Err(error) = presented_name(presented, &params.member) {
            let _ = write(stream, &failure_line(Some(request.id), &error)).await;
            return;
        }
        let Some(live_view) = self.current_live_view() else {
            let _ = write(
                stream,
                &failure_line(
                    Some(request.id),
                    &CallError::protocol(
                        ProtocolFailure::InvalidParams,
                        format!("no open Session of this Daemon is `{}`", params.session.0),
                    ),
                ),
            )
            .await;
            return;
        };
        let admission = if let Some(settings) = &self.services.settings {
            let machine = settings.snapshot().await;
            Admission::of(machine.resolved().ok())
        } else {
            Admission::of(None)
        };
        let (to_peer, mut from_live_view) = mpsc::channel::<String>(RELAY_CAPACITY);
        let (to_live_view, from_peer) = mpsc::channel::<String>(RELAY_CAPACITY);
        let masks = Masks::new();
        let ack = result_line(&request.id, &serde_json::json!({ "opened": true }));
        if !write(stream, &ack).await {
            return;
        }
        let relay_fut = serve_relayed(
            live_view,
            params.session,
            params.screen,
            admission,
            masks,
            to_peer,
            from_peer,
        );
        tokio::pin!(relay_fut);
        loop {
            tokio::select! {
                biased;
                () = &mut relay_fut => {
                    while let Ok(mut text) = from_live_view.try_recv() {
                        if !text.ends_with('\n') {
                            text.push('\n');
                        }
                        if !write(stream, text.as_bytes()).await {
                            break;
                        }
                    }
                    break;
                }
                incoming = lines.next(stream) => match incoming {
                    Some(Frame::Line(bytes)) => {
                        let Ok(text) = String::from_utf8(bytes) else { break; };
                        if to_live_view.send(text).await.is_err() {
                            break;
                        }
                    }
                    _ => break,
                },
                outgoing = from_live_view.recv() => match outgoing {
                    Some(mut text) => {
                        if !text.ends_with('\n') {
                            text.push('\n');
                        }
                        if !write(stream, text.as_bytes()).await {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

/// Type-erased accessor used by `Peering::from_engine` to extract `Runs` and
/// `LiveView` handles stored on [`EngineContext`].
pub(super) fn runs_from_engine(engine: &crate::engine_context::EngineContext) -> Option<Runs> {
    engine.runs_handle::<Runs>().map(|r| (*r).clone())
}

pub(super) fn live_view_from_engine(
    engine: &crate::engine_context::EngineContext,
) -> Option<LiveView> {
    engine
        .live_view_handle::<LiveView>()
        .map(|lv| (*lv).clone())
}

#[cfg(test)]
mod tests;
