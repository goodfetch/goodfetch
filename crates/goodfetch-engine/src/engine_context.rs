//! What one engine's Runs and handlers are given, built once at its start
//! (ADR 0059): the Goodfetch home, the clock, the engine's settings, the
//! user's Audit log, known-certificates store, Usage ledger and Run index,
//! where their
//! blocking work runs, the engine's "Needs you", its sandbox probe and the
//! person a single-user install acts for. The engine builds it beside its
//! Run registry and hands it down: to the registry, to each Connection's
//! calls and to its listeners, so no module reaches these values through the
//! Runs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use goodfetch_protocol::connection::ProductVersion;
use jiff::Timestamp;

use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::endpoint::Refusals;
use crate::home::Home;
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::needs_you::NeedsYou;
use crate::run_index::RunIndex;
use crate::sandbox_probe::SandboxProbe;
use crate::settings::Settings;
use crate::tasks::TaskOwner;

/// Who a Connection acts for: the OS user on a single-user install (ADR
/// 0102), the only caller until accounts arrive (area 12), and the
/// partition of the user's stores it reads and writes.
pub(crate) const CALLER: &str = "os_user";

/// The OS user's name, as the engine's environment map gives it: `USER`
/// or `LOGNAME` on Unix, `USERNAME` on Windows; [`CALLER`] when none is
/// set, as for a Daemon started with an empty environment.
pub(crate) fn os_user(environment: &BTreeMap<OsString, OsString>) -> String {
    ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .find_map(|name| {
            environment
                .get(&OsString::from(name))
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or(CALLER)
        .to_owned()
}

/// The user's known-certificates store, Usage ledger and Run index as one
/// engine opens them, and the owner of their blocking work: built once,
/// before the engine's "Needs you", which reads them too.
pub(crate) struct Stores {
    owner: Arc<TaskOwner>,
    known_certificates: KnownCertificates,
    ledger: Ledger,
    run_index: RunIndex,
}

impl Stores {
    /// `home`'s stores, as the engine of Product version `version` opens
    /// them through the SQLite VFS named `vfs` (none in production, ADR
    /// 0060), their blocking work under `owner` and the actors that keep
    /// their connections under `connections`, which the engine stops once
    /// its Runs and Connections have.
    pub(crate) fn of(
        home: &Home,
        version: &ProductVersion,
        vfs: Option<String>,
        owner: Arc<TaskOwner>,
        connections: Arc<TaskOwner>,
    ) -> Self {
        Self {
            known_certificates: KnownCertificates::of(home, vfs.clone(), Arc::clone(&connections)),
            run_index: RunIndex::of(home.run_index(version), version, vfs.clone(), connections),
            ledger: Ledger::at(home.usage_ledger(), vfs),
            owner,
        }
    }

    /// The known-certificates store.
    pub(crate) fn known_certificates(&self) -> &KnownCertificates {
        &self.known_certificates
    }

    /// The Usage ledger.
    pub(crate) fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// The Run index of the engine's major.
    pub(crate) fn run_index(&self) -> &RunIndex {
        &self.run_index
    }
}

/// What every Run and call of one engine is given beside its own inputs.
/// Clones share its stores.
#[derive(Clone)]
pub struct EngineContext {
    home: Home,
    clock: Clock,
    settings: Settings,
    audit: Arc<AuditLog>,
    known_certificates: KnownCertificates,
    ledger: Ledger,
    run_index: RunIndex,
    owner: Arc<TaskOwner>,
    needs_you: NeedsYou,
    sandbox_probe: SandboxProbe,
    person: Arc<str>,
    refusals: Refusals,
}

impl std::fmt::Debug for EngineContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EngineContext")
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl EngineContext {
    /// The engine at `home` on `clock` under `settings`, recording in
    /// `audit` and its listeners' refusals in `refusals`, with `stores`, its
    /// blocking work under their owner, with `needs_you` and
    /// `sandbox_probe`, acting for `person`.
    #[expect(
        clippy::too_many_arguments,
        reason = "each is one of the engine's own values, built once at its start and held together here"
    )]
    pub(crate) fn new(
        home: Home,
        clock: Clock,
        settings: Settings,
        (audit, refusals): (Arc<AuditLog>, Refusals),
        stores: Stores,
        needs_you: NeedsYou,
        sandbox_probe: SandboxProbe,
        person: String,
    ) -> Self {
        Self {
            known_certificates: stores.known_certificates,
            ledger: stores.ledger,
            run_index: stores.run_index,
            home,
            clock,
            settings,
            audit,
            owner: stores.owner,
            needs_you,
            sandbox_probe,
            person: Arc::from(person),
            refusals,
        }
    }

    /// The Goodfetch home the engine runs in.
    pub fn home(&self) -> &Home {
        &self.home
    }

    /// The engine's clock, which a call's wait reads.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// The wall time now, as the Audit log and the Usage ledger record it.
    pub(crate) fn now(&self) -> Timestamp {
        self.clock.timestamp()
    }

    /// The engine's settings, which each Run resolves its own under.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The user's Audit log, to which the engine records its security
    /// events.
    pub(crate) fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// The user's known-certificates store (ADR 0042).
    pub(crate) fn known_certificates(&self) -> &KnownCertificates {
        &self.known_certificates
    }

    /// The user's Usage ledger (ADR 0041).
    pub(crate) fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// The Run index of the engine's major (ADRs 0031 and 0098).
    pub(crate) fn run_index(&self) -> &RunIndex {
        &self.run_index
    }

    /// Where reads and writes of these stores block, which the Run registry
    /// waits for before it stops.
    pub(crate) fn owner(&self) -> &Arc<TaskOwner> {
        &self.owner
    }

    /// The engine's "Needs you" (spec #592).
    pub(crate) fn needs_you(&self) -> &NeedsYou {
        &self.needs_you
    }

    /// The engine's sandbox probe, whose answer each browser launch reads.
    pub(crate) fn sandbox_probe(&self) -> &SandboxProbe {
        &self.sandbox_probe
    }

    /// The OS user's name, from the engine's environment map: the person
    /// of a single-user install.
    pub(crate) fn person(&self) -> &str {
        &self.person
    }

    /// Where the engine's listeners record their refusals: the user's
    /// Audit log, each listener's at most `audit.refusals_per_minute` in a
    /// minute (`endpoint`).
    pub(crate) fn refusals(&self) -> Refusals {
        self.refusals.clone()
    }
}

#[cfg(test)]
mod tests;
