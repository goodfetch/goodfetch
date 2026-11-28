//! What one engine's Runs and handlers are given, built once at its start
//! (ADR 0059): the Goodfetch home, the clock, the engine's settings, the
//! user's Audit log, known-certificates store, Usage ledger, Run index and
//! Response cache, where their blocking work runs, the engine's "Needs
//! you", its sandbox probe and the person a single-user install acts for. The engine builds it beside its
//! Run registry and hands it down: to the registry, to each Connection's
//! calls and to its listeners, so no module reaches these values through the
//! Runs.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};

use goodfetch_confinement::deny_list::Places;
use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::connection::{Form, ProductVersion, Surface};
use goodfetch_protocol::response_cache::{CACHE_MODE, CacheMode};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;

use crate::audit::AuditLog;
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::endpoint::Refusals;
use crate::entropy::Entropy;
use crate::governor::Governor;
use crate::home::Home;
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::needs_you::NeedsYou;
use crate::plugin_provenance::AttestationTrustRoot;
use crate::response_cache::{Bounds, ResponseCache, ResponseSource};
use crate::run_index::RunIndex;
use crate::sandbox_probe::SandboxProbe;
use crate::settings::{Resolved, Settings};
use crate::signed_set::TrustRoot;
use crate::sync::Mutex;
use crate::tasks::TaskOwner;
use crate::user_store::{UserStore, UserStoreKind};

/// Who a Connection acts for: the OS user on a single-user install (ADR
/// 0102), the only caller until accounts arrive (area 12), and the
/// partition of the user's stores it reads and writes.
pub const CALLER: &str = "os_user";

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

/// Whether the engine's environment map names a display server, X11's
/// `DISPLAY` or Wayland's `WAYLAND_DISPLAY`, on which a headful browser
/// shows its window.
pub(crate) fn has_display(environment: &BTreeMap<OsString, OsString>) -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"].into_iter().any(|name| {
        environment
            .get(OsStr::new(name))
            .is_some_and(|value| !value.is_empty())
    })
}

/// The host's time zone as an IANA name, read once from the engine's
/// environment map as the C library reads `TZ` (with or without a leading
/// `:`, UTC when empty), the system's when `TZ` is unset; UTC, with a
/// warning, when neither names an IANA zone, such as a POSIX rule.
pub(crate) fn host_zone(environment: &BTreeMap<OsString, OsString>) -> String {
    let zone = match environment
        .get(OsStr::new("TZ"))
        .map(|value| value.to_str())
    {
        None => TimeZone::try_system().ok(),
        Some(Some(value)) => match value.strip_prefix(':').unwrap_or(value) {
            "" => Some(TimeZone::UTC),
            name => TimeZone::get(name).ok(),
        },
        Some(None) => None,
    };
    if let Some(name) = zone.as_ref().and_then(TimeZone::iana_name) {
        return name.to_owned();
    }
    tracing::warn!("the host's time zone has no IANA name; schedules default to UTC");
    "UTC".to_owned()
}

/// The user's folders a Plugin's confinement denies paths under (ADR
/// 0033), from the engine's environment map: the home folder, `HOME` on
/// macOS and Linux, `USERPROFILE` on Windows, with `APPDATA` and
/// `LOCALAPPDATA` when set, and the Goodfetch home `home`. None without a
/// home folder, as for a Daemon started with an empty environment: no
/// Plugin starts then, since its deny list cannot be placed.
pub(crate) fn places(home: &Home, environment: &BTreeMap<OsString, OsString>) -> Option<Places> {
    let folder = |name: &str| {
        environment
            .get(&OsString::from(name))
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let user_home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    Some(Places {
        home: folder(user_home)?,
        goodfetch_home: home.root().to_owned(),
        app_data: folder("APPDATA"),
        local_app_data: folder("LOCALAPPDATA"),
    })
}

/// What sends the calls its major's Daemon keeps, such as `schedule.*`
/// (spec 589), from an Attached engine to that Daemon, starting it when
/// none runs: the forward module's, which this module cannot name.
pub(crate) trait ToTheDaemon: Send + Sync {
    /// Sends `operation` with `params` to the Daemon as a Client of the
    /// `client`'s surface and form would, running the Daemon's start on
    /// `owner`'s blocking threads, and answers the Daemon's result or
    /// failure.
    fn forward(
        &self,
        owner: Arc<TaskOwner>,
        operation: &'static Operation,
        params: Value,
        client: (Surface, Form),
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;
}

/// What the engine knows of its host beside its stores: the person it
/// acts for, its time zone, the stream its handlers draw from and the
/// person's folders a Plugin's confinement denies.
pub(crate) struct Host {
    /// The OS user's name ([`os_user`]).
    pub(crate) person: String,
    /// The host's time zone ([`host_zone`]).
    pub(crate) zone: String,
    /// Whether the host names a display server ([`has_display`]).
    pub(crate) display: bool,
    /// The handlers' own stream of the engine's entropy.
    pub(crate) entropy: Entropy,
    /// The person's folders ([`places`]).
    pub(crate) places: Option<Places>,
    /// What forwards the calls the major's Daemon keeps to it: an
    /// Attached engine's that can start its Daemon; none on a Daemon.
    pub(crate) daemon: Option<Arc<dyn ToTheDaemon>>,
    /// The trust root verifying signed Plugin index updates and offline
    /// imports (bundled in production, injected as a value in tests, ADR 0059).
    pub(crate) plugin_index_trust_root: TrustRoot,
    /// The trust root verifying Plugin artifact provenance attestations
    /// (bundled in production, injected as a value in tests, ADR 0059).
    pub(crate) plugin_attestation_trust_root: AttestationTrustRoot,
}

/// The user's known-certificates store, Usage ledger, Run index, Response
/// cache and the contributions' own stores as one engine opens them, and
/// the owner of their blocking work: built once, before the engine's "Needs
/// you", which reads them too.
pub(crate) struct Stores {
    owner: Arc<TaskOwner>,
    known_certificates: KnownCertificates,
    ledger: Ledger,
    run_index: RunIndex,
    response_cache: ResponseCache,
    /// The contributions' own stores.
    contributed: Vec<UserStore>,
}

impl Stores {
    /// `home`'s stores, as the engine of Product version `version` opens
    /// them through the SQLite VFS named `vfs` (none in production, ADR
    /// 0060), their blocking work under `owner` and the actors that keep
    /// their connections under `connections`, which the engine stops once
    /// its Runs and Connections have, the Response cache's work counted on
    /// `work` and its filesystem sampled by `governor`; with the
    /// contributions' `user_stores` in the major's folder.
    pub(crate) fn of(
        home: &Home,
        version: &ProductVersion,
        vfs: Option<String>,
        (owner, connections): (Arc<TaskOwner>, Arc<TaskOwner>),
        (work, governor): (CountedWork, &Governor),
        user_stores: &[&'static UserStoreKind],
    ) -> Self {
        let contributed = user_stores
            .iter()
            .map(|kind| {
                UserStore::open(
                    home.per_major(version).join(kind.file()),
                    kind,
                    vfs.clone(),
                    Arc::clone(&connections),
                )
            })
            .collect();
        let cache_folder = home.response_cache(version);
        Self {
            known_certificates: KnownCertificates::of(home, vfs.clone(), Arc::clone(&connections)),
            response_cache: ResponseCache::of(
                cache_folder.clone(),
                vfs.clone(),
                Arc::clone(&connections),
                work,
            )
            .watching(governor.sample(cache_folder), governor.storage()),
            run_index: RunIndex::of(home.run_index(version), version, vfs.clone(), connections),
            ledger: Ledger::at(home.usage_ledger(), vfs),
            owner,
            contributed,
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
/// Clones share its stores, held behind one pointer: each Run holds a copy,
/// and every future holding a Run carries its size.
#[derive(Clone)]
pub struct EngineContext {
    home: Home,
    clock: Clock,
    settings: Settings,
    audit: Arc<AuditLog>,
    stores: Arc<Stores>,
    needs_you: NeedsYou,
    sandbox_probe: SandboxProbe,
    refusals: Refusals,
    held: Arc<Held>,
}

/// What an engine's contexts share behind one pointer, which keeps every
/// future that holds a context small (clippy's `large_futures`): its host.
struct Held {
    person: String,
    zone: String,
    display: bool,
    entropy: Mutex<Entropy>,
    places: Option<Arc<Places>>,
    daemon: Option<Arc<dyn ToTheDaemon>>,
    plugin_index_trust_root: TrustRoot,
    plugin_attestation_trust_root: AttestationTrustRoot,
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
    /// `sandbox_probe`, acting for `host`'s person, whose folders are its
    /// places.
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
        host: Host,
    ) -> Self {
        let Host {
            person,
            zone,
            display,
            entropy,
            places,
            daemon,
            plugin_index_trust_root,
            plugin_attestation_trust_root,
        } = host;
        Self {
            stores: Arc::new(stores),
            home,
            clock,
            settings,
            audit,
            needs_you,
            sandbox_probe,
            refusals,
            held: Arc::new(Held {
                person,
                zone,
                display,
                entropy: Mutex::new(entropy),
                places: places.map(Arc::new),
                daemon,
                plugin_index_trust_root,
                plugin_attestation_trust_root,
            }),
        }
    }

    /// The trust root used to verify signed Plugin indexes (bundled with the
    /// release or injected in tests, ADR 0059).
    #[must_use]
    pub fn plugin_index_trust_root(&self) -> &TrustRoot {
        &self.held.plugin_index_trust_root
    }

    /// The trust root used to verify Plugin artifact provenance attestations
    /// (bundled with the release or injected in tests, ADR 0059).
    #[must_use]
    pub fn plugin_attestation_trust_root(&self) -> &AttestationTrustRoot {
        &self.held.plugin_attestation_trust_root
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
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// The user's known-certificates store (ADR 0042).
    pub fn known_certificates(&self) -> &KnownCertificates {
        &self.stores.known_certificates
    }

    /// The user's Usage ledger (ADR 0041).
    pub(crate) fn ledger(&self) -> &Ledger {
        &self.stores.ledger
    }

    /// The Run index of the engine's major (ADRs 0031 and 0098), where a
    /// Run's consent walls and refetch stops are recorded for "Needs you".
    pub fn run_index(&self) -> &RunIndex {
        &self.stores.run_index
    }

    /// The user's Response cache of the engine's major (ADR 0046), under
    /// the catalog's bounds: what the `cache.*` operations and a Run's
    /// discard count, clear and verify.
    pub(crate) fn response_cache(&self) -> &ResponseCache {
        &self.stores.response_cache
    }

    /// Where the responses of a Run whose settings are `settings` come
    /// from (ADR 0046): the one value the fetch transports and the
    /// interception consult, which reads and fills the user's Response
    /// cache of the engine's major unless the Run's `cache.mode` is `off`.
    pub fn response_source(&self, settings: &Resolved) -> ResponseSource {
        let mode = CacheMode::parse(settings.get(&CACHE_MODE)).unwrap_or(CacheMode::Off);
        ResponseSource::of(
            &self.stores.response_cache.bounded(Bounds::of(settings)),
            mode,
        )
    }

    /// Where reads and writes of these stores block, which the Run registry
    /// waits for before it stops.
    pub(crate) fn owner(&self) -> &Arc<TaskOwner> {
        &self.stores.owner
    }

    /// The engine's "Needs you" (spec #592).
    pub(crate) fn needs_you(&self) -> &NeedsYou {
        &self.needs_you
    }

    /// The engine's sandbox probe, whose answer each browser launch reads.
    pub fn sandbox_probe(&self) -> &SandboxProbe {
        &self.sandbox_probe
    }

    /// The OS user's name, from the engine's environment map: the person
    /// of a single-user install.
    pub fn person(&self) -> &str {
        &self.held.person
    }

    /// The host's time zone, an IANA name read once from the engine's
    /// environment map ([`host_zone`]).
    pub fn host_zone(&self) -> &str {
        &self.held.zone
    }

    /// Whether the engine's environment map names a display server
    /// ([`has_display`]), on which a headful browser shows its window.
    pub fn host_has_display(&self) -> bool {
        self.held.display
    }

    /// What forwards the calls the major's Daemon keeps to it; `None` on
    /// a Daemon and on an engine that cannot start one.
    pub(crate) fn daemon(&self) -> Option<&Arc<dyn ToTheDaemon>> {
        self.held.daemon.as_ref()
    }

    /// A draw of 64 random bits from the handlers' own stream of the
    /// engine's entropy (ADR 0059), such as the random part of a ULID.
    pub fn draw_u64(&self) -> u64 {
        self.held
            .entropy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_u64()
    }

    /// The user's store `kind`, which a contribution declared with
    /// [`Contribution::with_user_store`]; none when no contribution of
    /// this engine did.
    ///
    /// [`Contribution::with_user_store`]: crate::composition::Contribution::with_user_store
    pub fn user_store(&self, kind: &UserStoreKind) -> Option<&UserStore> {
        self.stores
            .contributed
            .iter()
            .find(|store| store.kind().file() == kind.file())
    }

    /// The person's folders a Plugin's confinement denies, none when the
    /// engine's environment names no home folder.
    pub(crate) fn places(&self) -> Option<&Arc<Places>> {
        self.held.places.as_ref()
    }

    /// Where the engine's listeners record their refusals: the user's
    /// Audit log, each listener's at most `audit.refusals_per_minute` in a
    /// minute (`endpoint`).
    pub fn refusals(&self) -> Refusals {
        self.refusals.clone()
    }
}

#[cfg(test)]
mod tests;
