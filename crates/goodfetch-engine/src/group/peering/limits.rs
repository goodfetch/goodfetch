//! Group-wide Usage ledger slices, local *group redirect* server, heartbeat
//! renewal, Lead Daemon reachability metric/event and 10-minute token check
//! cache (ADRs 0041 and 0055, issue 258).
#![expect(
    clippy::disallowed_types,
    reason = "machine-local redirect endpoint binds 127.0.0.1:0 for Attached engines on the same host (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime};

use data_encoding::HEXLOWER;
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::group::{
    GROUP_LEAD_REACHABILITY_CHANGED, GROUP_LEDGER, GROUP_REDIRECT_FORMAT_VERSION,
    GROUP_TOKEN_CHECK, GroupLeadReachabilityChanged, GroupRedirectRow, LEAD_UNREACHABLE,
    PEER_PROTOCOL_VERSION, PeerLedgerAction, PeerLedgerParams, PeerLedgerReply, PeerLedgerResult,
    PeerTokenCheckParams, PeerTokenCheckResult, RedirectAction, RedirectConfig, RedirectReply,
    RedirectRequest, WireSlice, WireStoredLimit,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::politeness::POLITENESS_SHARING;
use goodfetch_protocol::politeness::store::POLITENESS_STORE;
use goodfetch_protocol::settings::SettingsLayer;
use jiff::Timestamp;
use rustls::pki_types::CertificateDer;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;

use super::{Asked, Peering, ask_lead, presented_name};
use crate::accounts::{AccountsStore, NamedTokenLookup};
use crate::call_error::CallError;
use crate::control_state::STORE_USAGE_LIMITS;
use crate::durable;
use crate::endpoint::same_token;
use crate::group::errors::{internal, store_failed};
use crate::group::{failure_line, result_line, store};
use crate::ledger::{
    DEFAULT_REDIRECT_HEARTBEAT_TTL_MS, Ledger, LedgerError, PeerLedgerClient, Refusal, StoredLimit,
    from_wire_holder, from_wire_limit, from_wire_sizing, to_wire_limit,
};
use crate::settings::get_or_default;
use crate::sync::Mutex;
use crate::tasks::TaskOwner;

/// Maximum time a cached named-token check stays valid while the Lead Daemon
/// is unreachable (ADR 0055): 10 minutes.
pub const TOKEN_CHECK_CACHE_MAX_TTL: Duration = Duration::from_secs(600);

/// File name inside the per-major folder holding the `0600` bearer token for
/// this Group member's local redirect endpoint.
pub(super) const REDIRECT_TOKEN_FILE: &str = "group-redirect.token";

/// One cached named-token verification result.
#[derive(Clone, Debug)]
struct CachedTokenCheck {
    valid: bool,
    token_name: Option<String>,
    expires_at: Option<SystemTime>,
    checked_at: SystemTime,
}

/// Cache of verified named-token checks (keyed by lowercase hex SHA-256)
/// bounded to [`TOKEN_CHECK_CACHE_MAX_TTL`] when the Lead Daemon is unreachable.
#[derive(Debug, Default)]
pub struct TokenCheckCache {
    entries: Mutex<BTreeMap<String, CachedTokenCheck>>,
}

impl TokenCheckCache {
    /// Records the result of checking `token_sha256` at `now`.
    pub fn record(
        &self,
        token_sha256: String,
        valid: bool,
        token_name: Option<String>,
        expires_at: Option<SystemTime>,
        now: SystemTime,
    ) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                token_sha256,
                CachedTokenCheck {
                    valid,
                    token_name,
                    expires_at,
                    checked_at: now,
                },
            );
    }

    /// Looks up a cached token check while the Lead Daemon is unreachable at
    /// `now`. Stays valid for what remains of the token, at most
    /// [`TOKEN_CHECK_CACHE_MAX_TTL`] (10 minutes); unknown or expired entries
    /// fail with `lead_unreachable`.
    pub fn lookup_while_unreachable(
        &self,
        token_sha256: &str,
        now: SystemTime,
    ) -> Result<Option<String>, CallError> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = entries.get(token_sha256) else {
            return Err(lead_unreachable_error(
                "the Lead Daemon is unreachable and this token has no cached check",
            ));
        };
        let elapsed = now
            .duration_since(entry.checked_at)
            .unwrap_or(Duration::ZERO);
        if elapsed > TOKEN_CHECK_CACHE_MAX_TTL {
            return Err(lead_unreachable_error(
                "the Lead Daemon is unreachable and the cached token check exceeded its 10-minute limit",
            ));
        }
        if let Some(expires_at) = entry.expires_at
            && now >= expires_at
        {
            return Err(lead_unreachable_error(
                "the Lead Daemon is unreachable and the cached token has expired",
            ));
        }
        if entry.valid {
            Ok(entry.token_name.clone())
        } else {
            Ok(None)
        }
    }
}

/// Tracks the last reported Lead Daemon reachability so metrics and lossless
/// events emit cleanly on transitions.
#[derive(Debug, Default)]
pub(super) struct LeadReachability {
    last_reachable: Mutex<Option<bool>>,
}

impl LeadReachability {
    /// Returns `Some(changed)` when `reachable` differs from the last reported
    /// state (or is the first unreachable observation).
    pub(super) fn observe(&self, reachable: bool) -> bool {
        let mut guard = self
            .last_reachable
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let prev = *guard;
        if prev == Some(reachable) {
            return false;
        }
        *guard = Some(reachable);
        // Do not emit a spurious "reachable" event on initial healthy startup;
        // emit on first unreachable transition and on every subsequent change.
        !(prev.is_none() && reachable)
    }
}

pub(super) fn lead_unreachable_error(message: impl Into<String>) -> CallError {
    CallError::new(&LEAD_UNREACHABLE, message.into(), &serde_json::json!({}))
}

impl Peering {
    /// Records whether this Group member can reach its Lead Daemon, updating
    /// `goodfetch.group.lead_unreachable` and emitting
    /// `group.lead_reachability_changed` on transitions.
    pub(super) async fn report_lead_reachable(&self, reachable: bool, reason: Option<String>) {
        if let Some(metrics) = &self.services.metrics {
            metrics.group_lead_unreachable(!reachable);
        }
        if !self.lead_reachability.observe(reachable) {
            return;
        }
        let lead_name = store::identity(&self.store)
            .await
            .ok()
            .flatten()
            .map_or_else(|| "lead".to_owned(), |(kept, _)| kept.lead_name);
        if let Some(events) = &self.services.events {
            let payload = GroupLeadReachabilityChanged {
                reachable,
                lead_name,
                reason: if reachable { None } else { reason },
            };
            events
                .publish(&GROUP_LEAD_REACHABILITY_CHANGED, Some(&self.name), &payload)
                .await;
        }
    }

    /// Runs blocking work `work` under the engine's task owner blocking policy
    /// when wired, or inline in isolated tests.
    pub(super) async fn run_blocking<T, F>(&self, work: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        match &self.services.owner {
            Some(owner) => owner.blocking(work).await,
            None => work(),
        }
    }

    /// Publishes the Lead Daemon's current non-redirect Limit rows into the
    /// `usage_limits` read-mostly control-state store whenever they change.
    pub(super) async fn sync_usage_limits(&self) {
        let Some(ledger) = &self.services.ledger else {
            return;
        };
        let ledger = ledger.clone();
        let Ok(limits) = self
            .run_blocking(move || ledger.non_redirect_limits())
            .await
        else {
            return;
        };
        let wire: Vec<WireStoredLimit> = limits.iter().map(to_wire_limit).collect();
        let Ok(json) = serde_json::to_string(&wire) else {
            return;
        };
        let _ = self
            .publish_control_store(STORE_USAGE_LIMITS, Some(json))
            .await;
    }

    /// Reads the cached `usage_limits` snapshot replicated from the Lead Daemon.
    pub(super) async fn cached_usage_limits(&self) -> Option<Vec<StoredLimit>> {
        let state = self.control_state.lock().await;
        let entry = state.get(STORE_USAGE_LIMITS)?;
        let text = entry.content.as_deref()?;
        let wire: Vec<WireStoredLimit> = serde_json::from_str(text).ok()?;
        Some(wire.into_iter().map(from_wire_limit).collect())
    }

    /// Verifies a named token's SHA-256 digest: locally on the Lead Daemon,
    /// over `group.token.check` on a non-lead Group member with a 10-minute
    /// cache fallback while the Lead Daemon is unreachable (ADR 0055).
    pub(super) async fn check_named_token(
        &self,
        token_sha256: &str,
    ) -> Result<Option<String>, CallError> {
        let kept = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(|| lead_unreachable_error("this Daemon is not in a group"))?;
        if kept.0.is_lead() {
            return self.check_named_token_local(token_sha256);
        }
        let lead_epoch = self.highest_epoch().await;
        let params = PeerTokenCheckParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: self.name.clone(),
            token_sha256: token_sha256.to_owned(),
        };
        match ask_lead::<PeerTokenCheckResult>(&kept.0, self.clock, GROUP_TOKEN_CHECK, &params)
            .await
        {
            Ok(answered) => {
                if answered.lead_epoch > lead_epoch {
                    let _ = self.admit_epoch(answered.lead_epoch).await;
                }
                self.report_lead_reachable(true, None).await;
                self.token_cache.record(
                    token_sha256.to_owned(),
                    answered.valid,
                    answered.token_name.clone(),
                    answered.expires_at,
                    self.clock.wall(),
                );
                Ok(if answered.valid {
                    answered.token_name
                } else {
                    None
                })
            }
            Err(error) => {
                self.report_lead_reachable(false, Some(error.to_string()))
                    .await;
                self.token_cache
                    .lookup_while_unreachable(token_sha256, self.clock.wall())
            }
        }
    }

    fn check_named_token_local(&self, token_sha256: &str) -> Result<Option<String>, CallError> {
        let Some(home) = &self.services.home else {
            return Ok(None);
        };
        if !home
            .accounts_store(&self.identity.product_version)
            .is_file()
        {
            return Ok(None);
        }
        let store = AccountsStore::of(home, &self.identity.product_version)
            .map_err(|error| internal(&error))?;
        store
            .named_token(NamedTokenLookup::Sha256(token_sha256))
            .map_err(|error| internal(&error))
    }

    /// Answers an incoming `group.token.check` peer RPC on the Lead Daemon.
    pub(super) async fn token_check_request(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerTokenCheckParams>,
    ) -> Vec<u8> {
        match self
            .handle_token_check(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_token_check(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerTokenCheckParams,
    ) -> Result<PeerTokenCheckResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let _ = presented_name(presented, &params.member)?;
        let token_name = self.check_named_token_local(&params.token_sha256)?;
        Ok(PeerTokenCheckResult {
            protocol_version: settled,
            lead_epoch,
            valid: token_name.is_some(),
            token_name,
            expires_at: None,
        })
    }

    /// Answers an incoming `group.ledger` peer RPC on the Lead Daemon.
    pub(super) async fn ledger_request(
        &self,
        presented: &CertificateDer<'_>,
        request: Asked<PeerLedgerParams>,
    ) -> Vec<u8> {
        match self
            .handle_ledger(presented, request.settled, request.params)
            .await
        {
            Ok(result) => result_line(&request.id, &result),
            Err(error) => failure_line(Some(request.id), &error),
        }
    }

    async fn handle_ledger(
        &self,
        presented: &CertificateDer<'_>,
        settled: ProtocolVersion,
        params: PeerLedgerParams,
    ) -> Result<PeerLedgerResult, CallError> {
        let lead_epoch = self.admit_epoch(params.lead_epoch).await?;
        let _ = self.require_confirmed_lead()?;
        let _ = presented_name(presented, &params.member)?;
        let reply = self.execute_local_ledger_action(params.action).await?;
        Ok(PeerLedgerResult {
            protocol_version: settled,
            lead_epoch,
            reply,
        })
    }

    /// Executes `action` against this Lead Daemon's own SQLite Usage ledger.
    pub(super) async fn execute_local_ledger_action(
        &self,
        action: PeerLedgerAction,
    ) -> Result<PeerLedgerReply, CallError> {
        let ledger = self
            .services
            .ledger
            .clone()
            .ok_or_else(|| internal(&"no Usage ledger is wired on this Lead Daemon"))?;
        let mutates_limits = matches!(
            action,
            PeerLedgerAction::AddLimit { .. } | PeerLedgerAction::Raise { .. }
        );
        let reply = self
            .run_blocking(move || run_local_ledger_action(&ledger, action))
            .await?;
        if mutates_limits {
            self.control_wake.notify_one();
        }
        Ok(reply)
    }

    /// Executes `action` on behalf of a local Attached engine: directly on the
    /// Lead Daemon, or forwarded over mutual TLS to the Lead Daemon on a
    /// non-lead Group member.
    async fn execute_redirect_ledger_action(
        &self,
        action: PeerLedgerAction,
    ) -> Result<PeerLedgerReply, CallError> {
        let (kept, _) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(|| lead_unreachable_error("this Daemon left its group"))?;
        if kept.is_lead() {
            let _ = self.require_confirmed_lead()?;
            return self.execute_local_ledger_action(action).await;
        }
        self.ask_lead_ledger(action).await
    }

    /// Forwards `action` to the Lead Daemon over mutual TLS.
    pub(super) async fn ask_lead_ledger(
        &self,
        action: PeerLedgerAction,
    ) -> Result<PeerLedgerReply, CallError> {
        let (kept, _) = store::identity(&self.store)
            .await
            .map_err(|error| store_failed(&error))?
            .ok_or_else(|| lead_unreachable_error("this Daemon left its group"))?;
        if kept.is_lead() {
            return self.execute_local_ledger_action(action).await;
        }
        let lead_epoch = self.highest_epoch().await;
        let params = PeerLedgerParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: self.identity.product,
            product_version: self.identity.product_version.clone(),
            lead_epoch,
            member: self.name.clone(),
            action: action.clone(),
        };
        match ask_lead::<PeerLedgerResult>(&kept, self.clock, GROUP_LEDGER, &params).await {
            Ok(answered) => {
                if answered.lead_epoch > lead_epoch {
                    let _ = self.admit_epoch(answered.lead_epoch).await;
                }
                self.report_lead_reachable(true, None).await;
                Ok(answered.reply)
            }
            Err(error) => {
                self.report_lead_reachable(false, Some(error.to_string()))
                    .await;
                if matches!(action, PeerLedgerAction::Limits)
                    && let Some(cached) = self.cached_usage_limits().await
                {
                    return Ok(PeerLedgerReply::Limits {
                        limits: cached.iter().map(to_wire_limit).collect(),
                    });
                }
                Err(lead_unreachable_error(error.to_string()))
            }
        }
    }

    /// Builds the [`RedirectConfig`] served to local Attached engines.
    async fn local_redirect_config(&self) -> RedirectConfig {
        let Some(settings) = &self.services.settings else {
            return RedirectConfig {
                member_name: self.name.clone(),
                operator_policy: None,
                user_configuration: None,
                politeness_sharing: None,
                politeness_store: None,
            };
        };
        let operator_policy = settings
            .read_file_text(SettingsLayer::OperatorPolicy)
            .await
            .ok()
            .flatten();
        let user_configuration = settings
            .read_file_text(SettingsLayer::UserConfiguration)
            .await
            .ok()
            .flatten();
        let snapshot = settings.snapshot().await;
        let resolved = snapshot.resolved().ok();
        let politeness_sharing =
            resolved.map(|r| get_or_default::<&str>(Some(r), &POLITENESS_SHARING).to_owned());
        let politeness_store = resolved
            .map(|r| r.value(&POLITENESS_STORE))
            .filter(|v| !v.is_null())
            .map(serde_json::Value::to_string);
        RedirectConfig {
            member_name: self.name.clone(),
            operator_policy,
            user_configuration,
            politeness_sharing,
            politeness_store,
        }
    }
}

fn parse_ts(text: &str) -> Result<Timestamp, CallError> {
    text.parse().map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("invalid timestamp `{text}`: {error}"),
        )
    })
}

fn run_local_ledger_action(
    ledger: &Ledger,
    action: PeerLedgerAction,
) -> Result<PeerLedgerReply, CallError> {
    let map_err = |error: LedgerError| internal(&error);
    match action {
        PeerLedgerAction::Limits => {
            let limits = ledger.non_redirect_limits().map_err(map_err)?;
            Ok(PeerLedgerReply::Limits {
                limits: limits.iter().map(to_wire_limit).collect(),
            })
        }
        PeerLedgerAction::Left { stored, at } => {
            let now = parse_ts(&at)?;
            let left = ledger
                .left_direct(&from_wire_limit(stored), now)
                .map_err(map_err)?;
            Ok(PeerLedgerReply::Left { left })
        }
        PeerLedgerAction::TakeSlice {
            stored,
            holder,
            at,
            sizing,
            needing,
        } => {
            let now = parse_ts(&at)?;
            let slice = ledger
                .take_slice_direct(
                    &from_wire_limit(stored),
                    &from_wire_holder(holder),
                    now,
                    from_wire_sizing(sizing),
                    needing,
                )
                .map_err(map_err)?;
            Ok(PeerLedgerReply::Slice {
                slice: slice.map(|s| WireSlice {
                    id: s.id,
                    size: s.size,
                }),
            })
        }
        PeerLedgerAction::GiveBack { used } => {
            ledger.give_back_direct(&used).map_err(map_err)?;
            Ok(PeerLedgerReply::GivenBack)
        }
        PeerLedgerAction::Usage { product, at } => {
            let now = parse_ts(&at)?;
            let limits = ledger.usage(product, now).map_err(map_err)?;
            Ok(PeerLedgerReply::Usage { limits })
        }
        PeerLedgerAction::AddLimit { limit, product, at } => {
            let now = parse_ts(&at)?;
            let id = ledger.add_limit(&limit, now).map_err(map_err)?;
            let usage = ledger
                .usage_of(id, product, now)
                .map_err(map_err)?
                .ok_or_else(|| internal(&"the Limit written is gone"))?;
            Ok(PeerLedgerReply::Limit { limit: usage })
        }
        PeerLedgerAction::Raise { limit, product, at } => {
            let now = parse_ts(&at)?;
            let id = ledger.raise(&limit, now).map_err(map_err)?;
            let usage = ledger
                .usage_of(id, product, now)
                .map_err(map_err)?
                .ok_or_else(|| internal(&"the Limit written is gone"))?;
            Ok(PeerLedgerReply::Limit { limit: usage })
        }
        PeerLedgerAction::CheckRunLimits { run_id, given } => {
            let res = ledger.check_run_limits(&run_id, &given).map_err(map_err)?;
            Ok(PeerLedgerReply::RunLimits { refused: res.err() })
        }
        PeerLedgerAction::GiveRunLimits {
            run_id,
            given,
            began,
            at,
        } => {
            let now = parse_ts(&at)?;
            let res = ledger
                .give_run_limits(&run_id, &given, began, now)
                .map_err(map_err)?;
            Ok(PeerLedgerReply::RunLimits { refused: res.err() })
        }
        PeerLedgerAction::Reclaim { charges } => {
            ledger.apply_reclaim_charges(&charges).map_err(map_err)?;
            Ok(PeerLedgerReply::Reclaimed)
        }
    }
}

/// [`SliceAuthority`] implementation installed on a non-lead Group member's
/// [`Ledger`], forwarding Limit reads and Slice takes over mutual TLS to the
/// Lead Daemon (1 peer round trip per Slice taken; 0 calls on `change_held`).
pub(super) struct GroupMemberLedger {
    peering: Arc<Peering>,
    handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for GroupMemberLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupMemberLedger")
            .field("member", &self.peering.name)
            .finish_non_exhaustive()
    }
}

impl GroupMemberLedger {
    pub(super) fn new(peering: Arc<Peering>, handle: tokio::runtime::Handle) -> Self {
        Self { peering, handle }
    }
}

impl PeerLedgerClient for GroupMemberLedger {
    fn send_ledger_action(&self, action: PeerLedgerAction) -> Result<PeerLedgerReply, Refusal> {
        let peering = Arc::clone(&self.peering);
        self.handle
            .block_on(async move { peering.ask_lead_ledger(action).await })
            .map_err(|error| Refusal::LeadUnreachable(error.to_string()))
    }
}

/// Active local redirect endpoint resources started for a Group member.
pub(super) struct LocalRedirect {
    listener: TcpListener,
    token: String,
    row: GroupRedirectRow,
}

/// Binds the Group member's local redirect endpoint on `127.0.0.1:0`, writes
/// its `0600` bearer token file, and writes the initial *group redirect* row
/// into `usage-ledger.sqlite`.
pub(super) async fn start_local_redirect(peering: &Peering) -> Option<LocalRedirect> {
    peering.start_local_redirect_server().await
}

impl Peering {
    async fn start_local_redirect_server(&self) -> Option<LocalRedirect> {
        let ledger = self.services.ledger.clone()?;
        let home = self.services.home.clone()?;
        let listener = TcpListener::bind("127.0.0.1:0").await.ok()?;
        let endpoint = listener.local_addr().ok()?.to_string();
        let mut raw = [0_u8; 32];
        if getrandom::fill(&mut raw).is_err() {
            return None;
        }
        let token = HEXLOWER.encode(&raw);
        let token_path: PathBuf = home
            .per_major(&self.identity.product_version)
            .join(REDIRECT_TOKEN_FILE);
        if let Some(parent) = token_path.parent() {
            let _ = durable::create_folder(parent);
        }
        if let Err(error) = write_token_file(&token_path, &token) {
            tracing::warn!(%error, path = %token_path.display(), "failed to write group redirect token file");
            return None;
        }
        let now = self.clock.timestamp();
        let row = GroupRedirectRow {
            format_version: GROUP_REDIRECT_FORMAT_VERSION,
            product: self.identity.product,
            product_major: self.identity.product_version.line().to_string(),
            member_name: self.name.clone(),
            endpoint,
            token_file: token_path.to_string_lossy().into_owned(),
            heartbeat_at: now.to_string(),
            heartbeat_ttl_ms: DEFAULT_REDIRECT_HEARTBEAT_TTL_MS,
        };
        let write_row = row.clone();
        let written = self
            .run_blocking(move || ledger.write_group_redirect(&write_row, now))
            .await;
        if let Err(error) = written {
            tracing::warn!(%error, "failed to write initial group redirect row");
            return None;
        }
        Some(LocalRedirect {
            listener,
            token,
            row,
        })
    }
}

fn write_token_file(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    durable::replace(path, token.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Serves the Group member's local redirect endpoint for Attached engines on
/// its machine and renews the *group redirect* row heartbeat until `owner` stops.
pub(super) async fn serve_local_redirect(
    peering: Arc<Peering>,
    redirect: LocalRedirect,
    owner: TaskOwner,
) {
    let LocalRedirect {
        listener,
        token,
        row,
    } = redirect;
    let heartbeat_peering = Arc::clone(&peering);
    let heartbeat_owner = owner.part("group redirect heartbeat");
    owner.spawn(renew_redirect_heartbeat(
        heartbeat_peering,
        row,
        heartbeat_owner,
    ));

    let token = Arc::new(token);
    loop {
        let accepted = tokio::select! {
            () = owner.token().cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _)) = accepted else {
            continue;
        };
        let peering = Arc::clone(&peering);
        let token = Arc::clone(&token);
        owner.spawn(async move {
            handle_redirect_connection(peering, token, stream).await;
        });
    }
}

async fn renew_redirect_heartbeat(peering: Arc<Peering>, row: GroupRedirectRow, owner: TaskOwner) {
    peering.run_redirect_heartbeat(row, owner).await;
}

impl Peering {
    async fn run_redirect_heartbeat(self: Arc<Self>, mut row: GroupRedirectRow, owner: TaskOwner) {
        let interval = Duration::from_millis((row.heartbeat_ttl_ms / 3).max(250));
        loop {
            tokio::select! {
                () = owner.token().cancelled() => return,
                () = self.clock.sleep(interval) => {}
            }
            let Some(ledger) = self.services.ledger.clone() else {
                return;
            };
            let now = self.clock.timestamp();
            row.heartbeat_at = now.to_string();
            let next = row.clone();
            let _ = self
                .run_blocking(move || ledger.write_group_redirect(&next, now))
                .await;
        }
    }
}

async fn handle_redirect_connection(
    peering: Arc<Peering>,
    expected_token: Arc<String>,
    stream: tokio::net::TcpStream,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let Ok(read) = reader.read_line(&mut line).await else {
        return;
    };
    if read == 0 {
        return;
    }
    let reply = match serde_json::from_str::<RedirectRequest>(&line) {
        Ok(request) => {
            if same_token(&request.token, &expected_token) {
                match request.action {
                    RedirectAction::Ledger { action } => {
                        match peering.execute_redirect_ledger_action(*action).await {
                            Ok(reply) => RedirectReply::Ledger { reply },
                            Err(error) => RedirectReply::Refused {
                                code: error.code().to_owned(),
                                message: error.to_string(),
                            },
                        }
                    }
                    RedirectAction::Config => RedirectReply::Config {
                        config: peering.local_redirect_config().await,
                    },
                }
            } else {
                RedirectReply::Refused {
                    code: "unauthorized".to_owned(),
                    message: "group redirect token mismatch".to_owned(),
                }
            }
        }
        Err(error) => RedirectReply::Refused {
            code: "invalid_request".to_owned(),
            message: error.to_string(),
        },
    };
    if let Ok(mut out) = serde_json::to_vec(&reply) {
        out.push(b'\n');
        let _ = write_half.write_all(&out).await;
        let _ = write_half.flush().await;
    }
}

#[cfg(test)]
mod tests;
