//! What one engine's Runs and handlers are given, built once at its start
//! (ADR 0059): the Goodfetch home, the clock, the engine's settings, the
//! user's Audit log, known-certificates store and Usage ledger, where their
//! blocking work runs, the engine's "Needs you", its sandbox probe and the
//! person a single-user install acts for. The engine builds it beside its
//! Run registry and hands it down: to the registry, to each Connection's
//! calls and to its listeners, so no module reaches these values through the
//! Runs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use jiff::Timestamp;

use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::endpoint::Refusals;
use crate::home::Home;
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::needs_you::NeedsYou;
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
    owner: Arc<TaskOwner>,
    needs_you: NeedsYou,
    sandbox_probe: SandboxProbe,
    person: Arc<str>,
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
    /// `audit`, its blocking work under `owner`, with `needs_you` and
    /// `sandbox_probe`, acting for `person`; its known-certificates store
    /// and Usage ledger are `home`'s, opened through the SQLite VFS named
    /// `vfs` (none in production, ADR 0060).
    #[expect(
        clippy::too_many_arguments,
        reason = "each is one of the engine's own values, built once at its start and held together here"
    )]
    pub(crate) fn new(
        home: Home,
        clock: Clock,
        settings: Settings,
        audit: Arc<AuditLog>,
        owner: Arc<TaskOwner>,
        needs_you: NeedsYou,
        sandbox_probe: SandboxProbe,
        person: String,
        vfs: Option<String>,
    ) -> Self {
        Self {
            known_certificates: KnownCertificates::of(&home, vfs.clone()),
            ledger: Ledger::at(home.usage_ledger(), vfs),
            home,
            clock,
            settings,
            audit,
            owner,
            needs_you,
            sandbox_probe,
            person: Arc::from(person),
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
    /// Audit log (`endpoint`).
    pub(crate) fn refusals(&self) -> Refusals {
        Refusals::new(self.audit().clone(), self.clock)
    }
}

#[cfg(test)]
mod tests;
