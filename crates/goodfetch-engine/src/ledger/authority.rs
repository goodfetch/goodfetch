//! Group-wide Usage ledger backend seam and *group redirect* row handling
//! (ADRs 0041 and 0055, issue 258):
//!
//! - [`SliceAuthority`] lets a non-lead Group member (over mutual TLS to the
//!   Lead Daemon) or an Attached engine beside a Group member (over the local
//!   loopback redirect endpoint authenticated with its `0600` token file)
//!   serve Limit reads, Slice takes and give-backs while keeping in-memory
//!   charging (`Charging::change_held`) untouched (`E3` Shared paths).
//! - A Group member writes a *group redirect* row (`scope_kind = "group_redirect"`,
//!   `must_understand = 1`) into the per-user `usage-ledger.sqlite` on its
//!   machine, naming its Product major, local endpoint, `0600` token file and
//!   renewed heartbeat. An older engine that cannot evaluate `group_redirect`
//!   refuses runs with `limit_not_understood`; a stale heartbeat or dead Group
//!   member makes new slices fail closed with `lead_unreachable`.
#![expect(
    clippy::disallowed_methods,
    reason = "RedirectClient runs inside the ledger's blocking policy, never on an async worker"
)]
#![expect(
    clippy::disallowed_types,
    reason = "RedirectClient dials the machine-local redirect loopback socket inside blocking ledger calls (ADR 0060)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::group::{
    GROUP_REDIRECT_FORMAT_VERSION, GROUP_REDIRECT_SCOPE, GroupRedirectRow, PeerLedgerAction,
    PeerLedgerReply, RedirectAction, RedirectConfig, RedirectReply, RedirectRequest, WireHolder,
    WireSlice, WireSliceSizing, WireStoredLimit,
};
use goodfetch_protocol::run::{ReachedLimit, RunLimit};
use goodfetch_protocol::usage::{Limit, LimitRaiseRefusedDetails, LimitUsage};
use jiff::{SignedDuration, Timestamp};

use super::{Holder, LedgerError, Slice, SliceId, SliceSizing, StoredLimit};

/// Why a run may not start, or may not charge more.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Refusal {
    /// A Limit covering it has nothing left in its period: `limit_reached`.
    #[error("a Limit on {} {} has nothing left", .0.scope.kind, .0.usage_kind)]
    Reached(Box<ReachedLimit>),
    /// A Limit flagged *must understand* that this engine cannot evaluate
    /// may cover it: `limit_not_understood`.
    #[error("a Limit on {} cannot be evaluated by this engine", .0.scope.kind)]
    NotUnderstood(Box<Limit>),
    /// The Daemon group's Lead Daemon (or local Group member named by a
    /// *group redirect* row) cannot be reached: `lead_unreachable`.
    #[error("the Lead Daemon cannot be reached: {0}")]
    LeadUnreachable(String),
    /// The ledger could not be read or written.
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

/// How long a blocking local redirect RPC waits before failing closed with
/// `lead_unreachable`.
const REDIRECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default heartbeat TTL (in milliseconds) written into a *group redirect* row.
pub const DEFAULT_REDIRECT_HEARTBEAT_TTL_MS: u64 = 5_000;

/// Backend that serves group-wide Usage ledger operations for a non-lead Group
/// member or an Attached engine redirected through its machine's Group member.
pub trait SliceAuthority: Send + Sync + std::fmt::Debug {
    /// Reads all Limit rows that apply across the group.
    ///
    /// On a non-lead Group member when the Lead Daemon is unreachable, this
    /// returns the last replicated `usage_limits` snapshot so live runs spend
    /// held Slices and new runs needing no Slice may start; if no snapshot has
    /// ever been received, fails closed with [`Refusal::LeadUnreachable`].
    fn limits(&self, now: Timestamp) -> Result<Vec<StoredLimit>, Refusal>;

    /// Reads the remaining budget of `stored` in the period holding `now`.
    /// Must reach the Lead Daemon; fails closed with [`Refusal::LeadUnreachable`]
    /// when unreachable.
    fn left(&self, stored: &StoredLimit, now: Timestamp) -> Result<u64, Refusal>;

    /// Takes one Slice of `stored` for `holder` in one transaction on the Lead
    /// Daemon's ledger (one peer round trip).
    fn take_slice(
        &self,
        stored: &StoredLimit,
        holder: &Holder,
        now: Timestamp,
        sizing: SliceSizing,
        needing: u64,
    ) -> Result<Option<Slice>, Refusal>;

    /// Gives back unused portions of held Slices, charging what was used.
    fn give_back(&self, used: &[(SliceId, u64)]) -> Result<(), Refusal>;

    /// Lists all group Limits with their current Usage (`usage.list`).
    fn usage(&self, product: Product, now: Timestamp) -> Result<Vec<LimitUsage>, Refusal>;

    /// Adds a Limit row on the Lead Daemon (`usage.set`).
    fn add_limit(
        &self,
        limit: &Limit,
        product: Product,
        now: Timestamp,
    ) -> Result<LimitUsage, Refusal>;

    /// Raises or sets a person Limit on the Lead Daemon (`usage.raise`).
    fn raise(&self, limit: &Limit, product: Product, now: Timestamp)
    -> Result<LimitUsage, Refusal>;

    /// Checks Run-level Limits against standing ones on the Lead Daemon.
    fn check_run_limits(
        &self,
        run_id: &str,
        given: &[RunLimit],
    ) -> Result<Result<(), LimitRaiseRefusedDetails>, Refusal>;

    /// Records Run-level Limits on start on the Lead Daemon.
    fn give_run_limits(
        &self,
        run_id: &str,
        given: &[RunLimit],
        began: bool,
        now: Timestamp,
    ) -> Result<Result<(), LimitRaiseRefusedDetails>, Refusal>;
}

/// Converts a [`StoredLimit`] to its wire representation.
pub fn to_wire_limit(stored: &StoredLimit) -> WireStoredLimit {
    WireStoredLimit {
        id: stored.id,
        scope: stored.scope.clone(),
        usage_kind: stored.usage_kind.clone(),
        currency: stored.currency.clone(),
        period: stored.period.clone(),
        time_zone: stored.time_zone.clone(),
        amount: stored.amount,
        must_understand: stored.must_understand,
        set_by: stored.set_by.clone(),
        managed_major: stored.managed_major.clone(),
        set_at: stored.set_at.clone(),
    }
}

/// Converts a [`WireStoredLimit`] back into a [`StoredLimit`].
pub fn from_wire_limit(wire: WireStoredLimit) -> StoredLimit {
    StoredLimit {
        id: wire.id,
        scope: wire.scope,
        usage_kind: wire.usage_kind,
        currency: wire.currency,
        period: wire.period,
        time_zone: wire.time_zone,
        amount: wire.amount,
        must_understand: wire.must_understand,
        set_by: wire.set_by,
        managed_major: wire.managed_major,
        set_at: wire.set_at,
    }
}

/// Converts a [`Holder`] to its wire representation.
pub fn to_wire_holder(holder: &Holder) -> WireHolder {
    WireHolder {
        run_id: holder.run_id.clone(),
        epoch: holder.epoch,
        folder: holder.folder.to_string_lossy().into_owned(),
    }
}

/// Converts a [`WireHolder`] back to a [`Holder`].
pub fn from_wire_holder(wire: WireHolder) -> Holder {
    Holder {
        run_id: wire.run_id,
        epoch: wire.epoch,
        folder: PathBuf::from(wire.folder),
    }
}

/// Converts [`SliceSizing`] to its wire representation.
pub fn to_wire_sizing(sizing: SliceSizing) -> WireSliceSizing {
    WireSliceSizing {
        share: sizing.share,
        minimum: sizing.minimum,
    }
}

/// Converts [`WireSliceSizing`] back to [`SliceSizing`].
pub fn from_wire_sizing(wire: WireSliceSizing) -> SliceSizing {
    SliceSizing {
        share: wire.share,
        minimum: wire.minimum,
    }
}

/// Converts [`WireSlice`] to [`Slice`].
pub fn from_wire_slice(wire: WireSlice) -> Slice {
    Slice {
        id: wire.id,
        size: wire.size,
    }
}

/// Whether `stored` is a *group redirect* row in `usage-ledger.sqlite`.
pub fn is_group_redirect(stored: &StoredLimit) -> bool {
    stored.scope.kind == GROUP_REDIRECT_SCOPE
}

/// Parses a *group redirect* row from `stored`.
///
/// Fails closed with [`Refusal::NotUnderstood`] (`limit_not_understood`) when
/// the payload cannot be read or carries an unsupported format version.
pub fn parse_redirect(stored: &StoredLimit) -> Result<GroupRedirectRow, Refusal> {
    let row: GroupRedirectRow = serde_json::from_str(&stored.scope.id)
        .map_err(|_| Refusal::NotUnderstood(Box::new(stored.limit())))?;
    if row.format_version != GROUP_REDIRECT_FORMAT_VERSION
        || row.endpoint.is_empty()
        || row.token_file.is_empty()
        || row.member_name.is_empty()
        || row.heartbeat_ttl_ms == 0
    {
        return Err(Refusal::NotUnderstood(Box::new(stored.limit())));
    }
    Ok(row)
}

/// Verifies that `row`'s heartbeat is fresh at `now`.
///
/// A stale heartbeat fails closed with [`Refusal::LeadUnreachable`]
/// (`lead_unreachable`); an unparsable timestamp fails closed with
/// [`Refusal::NotUnderstood`].
pub fn check_redirect_heartbeat(
    stored: &StoredLimit,
    row: &GroupRedirectRow,
    now: Timestamp,
) -> Result<(), Refusal> {
    let heartbeat: Timestamp = row
        .heartbeat_at
        .parse()
        .map_err(|_| Refusal::NotUnderstood(Box::new(stored.limit())))?;
    let ttl = SignedDuration::from_millis(i64::try_from(row.heartbeat_ttl_ms).unwrap_or(i64::MAX));
    let age = now.duration_since(heartbeat);
    if age > ttl {
        return Err(Refusal::LeadUnreachable(format!(
            "the group redirect heartbeat for Group member `{}` expired ({}ms > {}ms); wait for the Group member to return or run `group leave`",
            row.member_name,
            age.as_millis(),
            row.heartbeat_ttl_ms
        )));
    }
    Ok(())
}

/// Blocking loopback client used by an Attached engine (or embedded Rust
/// engine) on a Group member's machine to route Usage ledger slices and
/// configuration reads through that Group member.
#[derive(Clone, Debug)]
pub struct RedirectClient {
    row: GroupRedirectRow,
}

impl RedirectClient {
    /// Creates a client talking to the local redirect endpoint named by `row`.
    pub fn new(row: GroupRedirectRow) -> Self {
        Self { row }
    }

    /// The parsed redirect row this client targets.
    pub fn row(&self) -> &GroupRedirectRow {
        &self.row
    }

    /// Fetches the Group member's cached Operator policy, user configuration
    /// and Politeness configuration.
    pub fn fetch_config(&self) -> Result<RedirectConfig, Refusal> {
        match self.exchange(RedirectAction::Config)? {
            RedirectReply::Config { config } => Ok(config),
            RedirectReply::Refused { code, message } => Err(map_refused(&code, message)),
            other @ RedirectReply::Ledger { .. } => Err(Refusal::LeadUnreachable(format!(
                "unexpected redirect config answer from `{}`: {other:?}",
                self.row.member_name
            ))),
        }
    }

    fn ledger_call(&self, action: PeerLedgerAction) -> Result<PeerLedgerReply, Refusal> {
        match self.exchange(RedirectAction::Ledger {
            action: Box::new(action),
        })? {
            RedirectReply::Ledger { reply } => Ok(reply),
            RedirectReply::Refused { code, message } => Err(map_refused(&code, message)),
            other @ RedirectReply::Config { .. } => Err(Refusal::LeadUnreachable(format!(
                "unexpected redirect ledger answer from `{}`: {other:?}",
                self.row.member_name
            ))),
        }
    }

    fn exchange(&self, action: RedirectAction) -> Result<RedirectReply, Refusal> {
        let token = std::fs::read_to_string(&self.row.token_file)
            .map(|value| value.trim().to_owned())
            .map_err(|error| {
                Refusal::LeadUnreachable(format!(
                    "cannot read group redirect token file `{}` for Group member `{}`: {error}",
                    self.row.token_file, self.row.member_name
                ))
            })?;
        let mut stream = TcpStream::connect(&self.row.endpoint).map_err(|error| {
            Refusal::LeadUnreachable(format!(
                "cannot reach local Group member `{}` at `{}`: {error}",
                self.row.member_name, self.row.endpoint
            ))
        })?;
        let _ = stream.set_read_timeout(Some(REDIRECT_TIMEOUT));
        let _ = stream.set_write_timeout(Some(REDIRECT_TIMEOUT));
        let request = RedirectRequest { token, action };
        let mut payload = serde_json::to_vec(&request)
            .map_err(|error| Refusal::LeadUnreachable(error.to_string()))?;
        payload.push(b'\n');
        stream.write_all(&payload).map_err(|error| {
            Refusal::LeadUnreachable(format!(
                "failed sending to local Group member `{}`: {error}",
                self.row.member_name
            ))
        })?;
        stream.flush().map_err(|error| {
            Refusal::LeadUnreachable(format!(
                "failed flushing to local Group member `{}`: {error}",
                self.row.member_name
            ))
        })?;
        let mut line = String::new();
        let mut reader = BufReader::new(stream);
        let read = reader.read_line(&mut line).map_err(|error| {
            Refusal::LeadUnreachable(format!(
                "local Group member `{}` closed before answering: {error}",
                self.row.member_name
            ))
        })?;
        if read == 0 {
            return Err(Refusal::LeadUnreachable(format!(
                "local Group member `{}` closed the redirect link before answering",
                self.row.member_name
            )));
        }
        serde_json::from_str(&line).map_err(|error| {
            Refusal::LeadUnreachable(format!(
                "invalid redirect answer from local Group member `{}`: {error}",
                self.row.member_name
            ))
        })
    }
}

fn map_refused(code: &str, message: String) -> Refusal {
    if code == "lead_unreachable" || code == "group_lead_unreachable" || code == "lead_unconfirmed"
    {
        Refusal::LeadUnreachable(message)
    } else {
        Refusal::Ledger(LedgerError::Sqlite {
            path: PathBuf::from("group-redirect"),
            message: format!("{code}: {message}"),
        })
    }
}

/// Blocking transport that sends a [`PeerLedgerAction`] to the Lead Daemon or
/// local Group member redirect endpoint and returns its [`PeerLedgerReply`].
pub trait PeerLedgerClient: Send + Sync + std::fmt::Debug {
    /// Executes one group-ledger action over its transport.
    fn send_ledger_action(&self, action: PeerLedgerAction) -> Result<PeerLedgerReply, Refusal>;
}

impl PeerLedgerClient for RedirectClient {
    fn send_ledger_action(&self, action: PeerLedgerAction) -> Result<PeerLedgerReply, Refusal> {
        self.ledger_call(action)
    }
}

impl<T: PeerLedgerClient> SliceAuthority for T {
    fn limits(&self, _now: Timestamp) -> Result<Vec<StoredLimit>, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::Limits)? {
            PeerLedgerReply::Limits { limits } => {
                Ok(limits.into_iter().map(from_wire_limit).collect())
            }
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected limits reply: {other:?}"
            ))),
        }
    }

    fn left(&self, stored: &StoredLimit, now: Timestamp) -> Result<u64, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::Left {
            stored: to_wire_limit(stored),
            at: now.to_string(),
        })? {
            PeerLedgerReply::Left { left } => Ok(left),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected left reply: {other:?}"
            ))),
        }
    }

    fn take_slice(
        &self,
        stored: &StoredLimit,
        holder: &Holder,
        now: Timestamp,
        sizing: SliceSizing,
        needing: u64,
    ) -> Result<Option<Slice>, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::TakeSlice {
            stored: to_wire_limit(stored),
            holder: to_wire_holder(holder),
            at: now.to_string(),
            sizing: to_wire_sizing(sizing),
            needing,
        })? {
            PeerLedgerReply::Slice { slice } => Ok(slice.map(from_wire_slice)),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected take_slice reply: {other:?}"
            ))),
        }
    }

    fn give_back(&self, used: &[(SliceId, u64)]) -> Result<(), Refusal> {
        if used.is_empty() {
            return Ok(());
        }
        match self.send_ledger_action(PeerLedgerAction::GiveBack {
            used: used.to_vec(),
        })? {
            PeerLedgerReply::GivenBack => Ok(()),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected give_back reply: {other:?}"
            ))),
        }
    }

    fn usage(&self, product: Product, now: Timestamp) -> Result<Vec<LimitUsage>, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::Usage {
            product,
            at: now.to_string(),
        })? {
            PeerLedgerReply::Usage { limits } => Ok(limits),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected usage reply: {other:?}"
            ))),
        }
    }

    fn add_limit(
        &self,
        limit: &Limit,
        product: Product,
        now: Timestamp,
    ) -> Result<LimitUsage, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::AddLimit {
            limit: limit.clone(),
            product,
            at: now.to_string(),
        })? {
            PeerLedgerReply::Limit { limit } => Ok(limit),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected add_limit reply: {other:?}"
            ))),
        }
    }

    fn raise(
        &self,
        limit: &Limit,
        product: Product,
        now: Timestamp,
    ) -> Result<LimitUsage, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::Raise {
            limit: limit.clone(),
            product,
            at: now.to_string(),
        })? {
            PeerLedgerReply::Limit { limit } => Ok(limit),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected raise reply: {other:?}"
            ))),
        }
    }

    fn check_run_limits(
        &self,
        run_id: &str,
        given: &[RunLimit],
    ) -> Result<Result<(), LimitRaiseRefusedDetails>, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::CheckRunLimits {
            run_id: run_id.to_owned(),
            given: given.to_vec(),
        })? {
            PeerLedgerReply::RunLimits { refused } => Ok(refused.map_or(Ok(()), Err)),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected check_run_limits reply: {other:?}"
            ))),
        }
    }

    fn give_run_limits(
        &self,
        run_id: &str,
        given: &[RunLimit],
        began: bool,
        now: Timestamp,
    ) -> Result<Result<(), LimitRaiseRefusedDetails>, Refusal> {
        match self.send_ledger_action(PeerLedgerAction::GiveRunLimits {
            run_id: run_id.to_owned(),
            given: given.to_vec(),
            began,
            at: now.to_string(),
        })? {
            PeerLedgerReply::RunLimits { refused } => Ok(refused.map_or(Ok(()), Err)),
            other => Err(Refusal::LeadUnreachable(format!(
                "unexpected give_run_limits reply: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::run::LimitScope;
    use goodfetch_protocol::usage::{INSTALL, LimitStanding, REQUESTS};
    use goodfetch_test_support::TestEnv;

    use super::*;
    use crate::ledger::{Ledger, Placement, standing};

    fn sample_redirect_row(heartbeat_at: &str) -> GroupRedirectRow {
        GroupRedirectRow {
            format_version: GROUP_REDIRECT_FORMAT_VERSION,
            product: Product::Collie,
            product_major: "0.0".to_owned(),
            member_name: "worker-a".to_owned(),
            endpoint: "127.0.0.1:49999".to_owned(),
            token_file: "/tmp/nonexistent-token".to_owned(),
            heartbeat_at: heartbeat_at.to_owned(),
            heartbeat_ttl_ms: 5_000,
        }
    }

    /// An older engine evaluating a `group_redirect` row flagged *must
    /// understand* classifies it as `NotUnderstood` and refuses runs with
    /// `limit_not_understood`; a newer/malformed `group_redirect` payload also
    /// fails closed with `limit_not_understood`; and a stale heartbeat fails
    /// closed with `lead_unreachable` until renewed or removed by `group leave`.
    ///
    /// Covers: story:3-112, story:3-115
    #[test]
    fn group_redirect_row_fails_closed_on_unknown_version_and_stale_heartbeat() {
        let env = TestEnv::new();
        let ledger = Ledger::at(env.folder().join("usage-ledger.sqlite"), None);
        let t0: Timestamp = "2026-11-27T12:00:00Z".parse().unwrap();
        let row = sample_redirect_row("2026-11-27T12:00:00Z");
        ledger.write_group_redirect(&row, t0).unwrap();

        // An older engine that does not special-case `group_redirect` sees
        // `LimitStanding::NotUnderstood` with `must_understand = true` and
        // refuses any placement with `Err(stored)` -> `limit_not_understood`.
        let stored = ledger.limits().unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].must_understand);
        assert_eq!(
            standing(&stored[0], Product::Collie),
            LimitStanding::NotUnderstood
        );
        let placement = Placement {
            product: Product::Collie,
            scopes: vec![LimitScope {
                kind: INSTALL.to_owned(),
                id: String::new(),
            }],
        };
        assert!(placement.holding(&stored).is_err());

        // `usage()` omits internal `group_redirect` bookkeeping rows.
        assert_eq!(ledger.usage(Product::Collie, t0).unwrap(), Vec::new());

        // Fresh heartbeat at t0 + 3s passes heartbeat check.
        let t_fresh: Timestamp = "2026-11-27T12:00:03Z".parse().unwrap();
        check_redirect_heartbeat(&stored[0], &row, t_fresh).unwrap();

        // Stale heartbeat at t0 + 6s (> 5s TTL) fails closed with LeadUnreachable.
        let t_stale: Timestamp = "2026-11-27T12:00:06Z".parse().unwrap();
        let stale_err = check_redirect_heartbeat(&stored[0], &row, t_stale).unwrap_err();
        assert!(
            matches!(stale_err, Refusal::LeadUnreachable(_)),
            "{stale_err:?}"
        );

        // Future/unreadable redirect format_version fails closed with NotUnderstood.
        let mut future_row = row.clone();
        future_row.format_version = 99;
        let mut future_stored = stored[0].clone();
        future_stored.scope.id = serde_json::to_string(&future_row).unwrap();
        let future_err = parse_redirect(&future_stored).unwrap_err();
        assert!(
            matches!(future_err, Refusal::NotUnderstood(_)),
            "{future_err:?}"
        );

        // Removing the row (as `group leave` does) leaves no redirect row.
        ledger.remove_group_redirect("worker-a").unwrap();
        assert_eq!(ledger.limits().unwrap(), Vec::new());
        // Adding a normal Limit works cleanly after removal.
        let _ = ledger
            .add_limit(
                &Limit {
                    scope: LimitScope {
                        kind: INSTALL.to_owned(),
                        id: String::new(),
                    },
                    usage_kind: REQUESTS.to_owned(),
                    currency: None,
                    period: None,
                    amount: 100,
                    must_understand: true,
                    set_by: goodfetch_protocol::run::LimitSetter::Person,
                },
                t_stale,
            )
            .unwrap();
        assert_eq!(ledger.usage(Product::Collie, t_stale).unwrap().len(), 1);
    }
}
