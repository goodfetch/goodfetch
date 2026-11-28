//! What one engine's Runs and handlers are given, built once at its start
//! (ADR 0059): the Goodfetch home, the clock, the engine's settings, the
//! user's Audit log, known-certificates store, Usage ledger, Run index and
//! Response cache, where their blocking work runs, the engine's "Needs
//! you", its sandbox probe and the person a single-user install acts for. The engine builds it beside its
//! Run registry and hands it down: to the registry, to each Connection's
//! calls and to its listeners, so no module reaches these values through the
//! Runs.
//!
//! Size: one concern, the engine's shared per-instance handles and peer-link
//! traits (`GroupRouter`, `GroupPeer`, `PeerExtensionHandler`, `LocalDispatcher`)
//! held behind `Arc<Held>` so every handler, listener and Run shares one context
//! without cyclic ownership.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use goodfetch_confinement::deny_list::Places;
use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::connection::{Form, ProductVersion, Surface};
use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::events::{EventEnvelope, EventsSubscribeParams};
use goodfetch_protocol::group::{ErasureTarget, LeadEpoch, MemberRoute, PlacementRequest};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::live_view::LiveViewScreen;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::response_cache::{CACHE_MODE, CacheMode};
use goodfetch_protocol::run::RunId;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::audit::AuditLog;
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::control_state::ControlStateRegistry;
use crate::endpoint::Refusals;
use crate::entropy::Entropy;
use crate::events::Events;
use crate::governor::Governor;
use crate::home::Home;
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::metrics::Metrics;
use crate::needs_you::NeedsYou;
use crate::response_cache::{Bounds, ResponseCache, ResponseSource};
use crate::run_index::RunIndex;
use crate::sandbox_probe::SandboxProbe;
use crate::settings::{Resolved, Settings};
use crate::sync::{AtomicBool, Mutex};
use crate::tasks::TaskOwner;
use crate::user_store::{UserStore, UserStoreKind};

/// Who a Connection acts for: the OS user on a single-user install (ADR
/// 0102), the only caller until accounts arrive (area 12), and the
/// partition of the user's stores it reads and writes.
pub const CALLER: &str = "os_user";

/// Boxed future answered by a [`PeerExtensionHandler`].
pub type PeerExtensionAnswer = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

/// Handles a contribution-registered Daemon group peer-link extension RPC
/// (`group.peer.extension`), authenticated by mTLS and fenced by `lead_epoch`
/// (ADRs 0054, 0055 and 0087).
pub trait PeerExtensionHandler: Send + Sync {
    /// Handles one authenticated peer-extension request from `sender_member`
    /// carrying `lead_epoch` and JSON `payload`.
    fn handle(
        &self,
        sender_member: String,
        lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer;
}

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

/// Broadcasts an erasure (a discarded Run or deleted Saved identity) and
/// verifies named tokens across a Daemon group (ADRs 0046 and 0055).
pub(crate) trait ErasureBroadcaster: Send + Sync {
    /// Broadcasts `target` through the Lead Daemon to every Group member.
    fn broadcast(
        &self,
        target: ErasureTarget,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send>>;

    /// Verifies a named token's SHA-256 digest against the Lead Daemon (or
    /// cached token verification valid for at most 10 minutes while the Lead
    /// Daemon is unreachable).
    fn check_named_token(
        &self,
        token_sha256: String,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, CallError>> + Send>>;
}

/// One unary Client JSON-RPC call forwarded across a Daemon group peer link
/// without decoding `params_raw` into a `Value` copy (ADRs 0055, 0132 and 0140).
#[derive(Clone, Debug)]
pub(crate) struct ForwardedCall {
    /// The Client's JSON-RPC request ID.
    pub(crate) id: RequestId,
    /// The catalog operation method name.
    pub(crate) method: String,
    /// Raw JSON text of the call's params (preserving `_meta` Trace context).
    pub(crate) params_raw: Option<String>,
    /// The Client's surface.
    pub(crate) surface: Surface,
    /// The Client's error-name spelling form.
    pub(crate) form: Form,
    /// Whether the Client reached the receiving Group member behind a gateway.
    pub(crate) behind_gateway: bool,
    /// Whether the Client's Connection answers as a person.
    pub(crate) answers_as_a_person: bool,
}

/// Executes a forwarded unary JSON-RPC call locally on the owning Group
/// member and returns its raw JSON-RPC response line (ADR 0055).
pub(crate) trait LocalDispatcher: Send + Sync {
    /// Dispatches `call` locally, opening its `SERVER` span from `_meta` and
    /// returning the encoded JSON-RPC reply line (newline-terminated).
    fn dispatch(&self, call: ForwardedCall) -> Pin<Box<dyn Future<Output = Vec<u8>> + Send>>;
}

/// Outcome of routing a unary Client call in a Daemon group (ADR 0055).
pub(crate) enum RoutedCall {
    /// Handle the call locally on this Group member.
    Local,
    /// Handled by the owning Group member: exact JSON-RPC reply line bytes to
    /// pass straight to the Client without decoding or encoding again (ADR 0140).
    Forwarded(Vec<u8>),
}

/// Outcome of routing an `events.subscribe` stream in a Daemon group (ADR 0055).
pub(crate) enum EventsRoute {
    /// Serve locally on this Group member.
    Local,
    /// Redirect the Client to the owning Group member's `daemon.advertise_url`.
    Redirect { url: String, owner: String },
    /// Forward events over the peer link from the owning Group member.
    Forward { owner: String, peer_address: String },
}

/// Outcome of routing a Live view channel in a Daemon group (ADRs 0055 and 0149).
pub(crate) enum LiveViewRoute {
    /// Serve locally (or no remote Group member owns this Session).
    Local,
    /// Redirect the viewer to the owning Group member's `daemon.advertise_url`
    /// using the single-use channel token minted in that owner's memory.
    Redirect {
        url: String,
        token: DaemonToken,
        owner: String,
    },
    /// Relay the Live view channel over the peer link to the owning Group member.
    Relay { owner: String, peer_address: String },
}

/// Bidirectional newline-JSON relay handle for a Live view channel forwarded
/// over a peer link when the owning Group member has no `daemon.advertise_url`.
pub(crate) struct LiveViewRelay {
    /// Sends viewer JSON messages to the owning Group member.
    pub(crate) to_owner: mpsc::Sender<String>,
    /// Receives engine JSON messages and frames from the owning Group member.
    pub(crate) from_owner: mpsc::Receiver<String>,
}

/// Shared type-erased slot holding the Daemon's `LiveView` driver once
/// `daemon::serve` constructs it (avoids an `Arc` cycle back to `EngineContext`).
pub(crate) type LiveViewSlot = Arc<Mutex<Option<Arc<dyn std::any::Any + Send + Sync>>>>;

/// Boxed future yielding a peer-forwarded event batch receiver.
pub(crate) type ForwardedEventsFuture =
    Pin<Box<dyn Future<Output = Result<mpsc::Receiver<Vec<EventEnvelope>>, CallError>> + Send>>;

/// Routes unary Client calls, event streams, Live view channels and run
/// placement across a Daemon group (ADRs 0055, 0056, 0132, 0140 and 0149).
pub(crate) trait GroupRouter: Send + Sync {
    /// Routes a unary call for `run_id` (or places a new/resumed run given
    /// `placement`). Answers `RoutedCall::Local` when this Group member handles
    /// it, or `RoutedCall::Forwarded(line)` with the owner's raw response line
    /// (retrying once after a cache refresh on `run_moved`).
    fn route_unary(
        &self,
        call: ForwardedCall,
        run_id: Option<RunId>,
        placement: Option<PlacementRequest>,
    ) -> Pin<Box<dyn Future<Output = Result<RoutedCall, CallError>> + Send>>;

    /// Resolves how `events.subscribe` for `run_id` should be served: locally,
    /// redirected to `daemon.advertise_url`, or forwarded over the peer link.
    fn route_events(
        &self,
        run_id: RunId,
    ) -> Pin<Box<dyn Future<Output = Result<EventsRoute, CallError>> + Send>>;

    /// Opens a forwarded event stream over the peer link from `owner` at
    /// `peer_address` for `params`, retrying once after a cache refresh on
    /// `run_moved`.
    fn forward_events(
        &self,
        owner: String,
        peer_address: String,
        params: EventsSubscribeParams,
    ) -> ForwardedEventsFuture;

    /// Resolves how a Live view channel for `session` should be served when
    /// not open locally: redirected with a single-use token or relayed over
    /// the peer link.
    fn route_live_view(
        &self,
        session: SessionId,
    ) -> Pin<Box<dyn Future<Output = Result<LiveViewRoute, CallError>> + Send>>;

    /// Opens a relayed Live view channel over the peer link to `owner`.
    fn open_live_view_relay(
        &self,
        owner: String,
        peer_address: String,
        session: SessionId,
        screen: LiveViewScreen,
    ) -> Pin<Box<dyn Future<Output = Result<LiveViewRelay, CallError>> + Send>>;

    /// Verifies and consumes a single-use Live view redirect token minted in
    /// this Group member's memory for `session` (ADR 0149).
    fn verify_live_view_token(&self, session: &SessionId, token: &str) -> bool;

    /// Hands off checkpointed Runs over the peer link before `deadline` when a
    /// `SIGTERM` drain is a scale-down removal rather than a restart (ADR 0055).
    fn handoff_after_drain(
        &self,
        deadline: tokio::time::Instant,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;

    /// This Group member's name.
    fn member_name(&self) -> &str;
}

/// Public Daemon group peer-communication interface exposed on
/// [`EngineContext`] for contributions (such as Foxhound's Partitioned crawls)
/// without `goodfetch-engine` naming any contribution types (ADRs 0054, 0055
/// and 0087).
pub trait GroupPeer: Send + Sync {
    /// This Group member's name.
    fn member_name(&self) -> &str;

    /// Whether this Group member is currently the Lead Daemon.
    fn is_lead(&self) -> Pin<Box<dyn Future<Output = Result<bool, CallError>> + Send>>;

    /// Queries the Lead Daemon for all eligible ready Group members satisfying
    /// `request`, ordered by descending headroom.
    fn eligible_members(
        &self,
        request: PlacementRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MemberRoute>, CallError>> + Send>>;

    /// Resolves `member`'s current route (`member`, `peer_address`,
    /// `advertise_url`), or `None` if no such member exists in the group.
    fn member_route(
        &self,
        member: String,
    ) -> Pin<Box<dyn Future<Output = Result<Option<MemberRoute>, CallError>> + Send>>;

    /// Invokes a contribution-registered peer extension `extension_method` on
    /// `member` (dispatched locally when `member == self.member_name()`, over
    /// mTLS `group.peer.extension` otherwise).
    fn call_member_extension(
        &self,
        member: String,
        peer_address: String,
        extension_method: String,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

    /// Invokes a contribution-registered peer extension `extension_method` on
    /// the Lead Daemon (locally when this member is the Lead Daemon).
    fn call_lead_extension(
        &self,
        extension_method: String,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

    /// Invokes a catalog JSON-RPC `method` with `params` on `member` (locally
    /// when `member == self.member_name()`, over `group.rpc.forward`
    /// otherwise), returning its parsed JSON `result` or `CallError`.
    fn call_member_rpc(
        &self,
        member: String,
        peer_address: String,
        method: String,
        params: Value,
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
    /// The Daemon group control-state store registry (ADR 0055).
    pub(crate) control_stores: Arc<ControlStateRegistry>,
    /// The engine's event stream (ADR 0024).
    pub(crate) events: Events,
    /// The engine's metrics instruments (ADR 0050).
    pub(crate) metrics: Metrics,
    /// The engine's Resource governor (ADR 0040).
    pub(crate) governor: Governor,
    /// Whether a `SIGTERM` drain on this engine classifies as a Kubernetes
    /// `StatefulSet` scale-down removal rather than a restart (ADR 0055).
    pub(crate) sigterm_is_removal: bool,
    /// Contribution-registered peer-link extension handlers (ADR 0087).
    pub(crate) peer_extensions: Arc<BTreeMap<&'static str, Arc<dyn PeerExtensionHandler>>>,
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

struct HeldPeerLink {
    owner: TaskOwner,
    state: Arc<dyn std::any::Any + Send + Sync>,
}

/// What an engine's contexts share behind one pointer, which keeps every
/// future that holds a context small (clippy's `large_futures`): its host,
/// and the owner of its Daemon group's peer link while it is in a group.
struct Held {
    person: String,
    zone: String,
    display: bool,
    entropy: Mutex<Entropy>,
    places: Option<Arc<Places>>,
    daemon: Option<Arc<dyn ToTheDaemon>>,
    peer_link: Mutex<Option<HeldPeerLink>>,
    control_stores: Arc<ControlStateRegistry>,
    events: Events,
    metrics: Metrics,
    governor: Governor,
    sigterm_is_removal: bool,
    erasure: Mutex<Option<Arc<dyn ErasureBroadcaster>>>,
    has_group_router: AtomicBool,
    group_router: Mutex<Option<Arc<dyn GroupRouter>>>,
    group_peer: Mutex<Option<Arc<dyn GroupPeer>>>,
    peer_extensions: Arc<BTreeMap<&'static str, Arc<dyn PeerExtensionHandler>>>,
    local_dispatcher: Mutex<Option<Arc<dyn LocalDispatcher>>>,
    runs_handle: Mutex<Option<Arc<dyn std::any::Any + Send + Sync>>>,
    live_view_handle: Arc<Mutex<Option<Arc<dyn std::any::Any + Send + Sync>>>>,
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
            control_stores,
            events,
            metrics,
            governor,
            sigterm_is_removal,
            peer_extensions,
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
                peer_link: Mutex::new(None),
                control_stores,
                events,
                metrics,
                governor,
                sigterm_is_removal,
                erasure: Mutex::new(None),
                has_group_router: AtomicBool::new(false),
                group_router: Mutex::new(None),
                group_peer: Mutex::new(None),
                peer_extensions,
                local_dispatcher: Mutex::new(None),
                runs_handle: Mutex::new(None),
                live_view_handle: Arc::new(Mutex::new(None)),
            }),
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

    /// The starting side of the engine's task owner, for contributions that
    /// start background tasks or tracked blocking reads without stopping the
    /// engine (ADR 0038 and ADR 0145).
    pub fn task_starter(&self) -> &crate::tasks::TaskStarter {
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

    /// Holds `link`, the owner of the tasks of the Daemon group's peer
    /// link, and its running `state`, in place of the one held before, which
    /// it cancels.
    pub(crate) fn hold_peer_link<T: Send + Sync + 'static>(&self, link: TaskOwner, state: Arc<T>) {
        let before = self
            .held
            .peer_link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(HeldPeerLink { owner: link, state });
        if let Some(before) = before {
            before.owner.cancel();
        }
    }

    /// The running state of the Daemon group's peer link, if held as `T`.
    pub(crate) fn peer_link_state<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.held
            .peer_link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|held| Arc::clone(&held.state).downcast::<T>().ok())
    }

    /// Stops the tasks of the Daemon group's peer link, if an owner of them
    /// is held: once this answers, they have ended.
    pub(crate) async fn stop_peer_link(&self) {
        let held = self
            .held
            .peer_link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(link) = held {
            link.owner.stop().await;
        }
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

    /// The Daemon group control-state store registry declaring each area's
    /// store as single-writer at the Lead Daemon or read-mostly and cached
    /// (ADR 0055).
    pub fn control_stores(&self) -> &Arc<ControlStateRegistry> {
        &self.held.control_stores
    }

    /// The engine's event stream (ADR 0024).
    pub(crate) fn events(&self) -> &Events {
        &self.held.events
    }

    /// The engine's metrics instruments (ADR 0050).
    pub(crate) fn metrics(&self) -> &Metrics {
        &self.held.metrics
    }

    /// The engine's Resource governor (ADR 0040).
    pub(crate) fn governor(&self) -> &Governor {
        &self.held.governor
    }

    /// Whether a `SIGTERM` drain on this engine classifies as a Kubernetes
    /// `StatefulSet` scale-down removal rather than a restart (ADR 0055).
    pub(crate) fn sigterm_is_removal(&self) -> bool {
        self.held.sigterm_is_removal
    }

    /// Fast lock-free check for whether this engine is currently routing calls
    /// as part of a Daemon group (ADR 0140's E1: zero cost for local calls
    /// outside a group).
    pub(crate) fn has_group_router(&self) -> bool {
        self.held.has_group_router.load(Ordering::Relaxed)
    }

    /// Registers or clears the Daemon group Client router (ADR 0055).
    pub(crate) fn set_group_router(&self, router: Option<Arc<dyn GroupRouter>>) {
        let present = router.is_some();
        *self
            .held
            .group_router
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = router;
        self.held.has_group_router.store(present, Ordering::Release);
    }

    /// The running Daemon group router, if this engine is in a Daemon group.
    pub(crate) fn group_router(&self) -> Option<Arc<dyn GroupRouter>> {
        if !self.has_group_router() {
            return None;
        }
        self.held
            .group_router
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Registers or clears the public Daemon group peer handle (ADR 0054/0087).
    pub(crate) fn set_group_peer(&self, peer: Option<Arc<dyn GroupPeer>>) {
        *self
            .held
            .group_peer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = peer;
    }

    /// The running Daemon group peer handle, if this engine belongs to a Daemon
    /// group.
    #[must_use]
    pub fn group_peer(&self) -> Option<Arc<dyn GroupPeer>> {
        self.held
            .group_peer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The contribution-registered peer-link extension handlers.
    pub(crate) fn peer_extensions(
        &self,
    ) -> &Arc<BTreeMap<&'static str, Arc<dyn PeerExtensionHandler>>> {
        &self.held.peer_extensions
    }

    /// Registers the local RPC dispatcher used to execute forwarded unary peer
    /// calls on the owning Group member (ADR 0055).
    pub(crate) fn set_local_dispatcher(&self, dispatcher: Option<Arc<dyn LocalDispatcher>>) {
        *self
            .held
            .local_dispatcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = dispatcher;
    }

    /// The local RPC dispatcher, if wired.
    pub(crate) fn local_dispatcher(&self) -> Option<Arc<dyn LocalDispatcher>> {
        self.held
            .local_dispatcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Stores the engine's `Runs` registry handle for group peer handlers.
    pub(crate) fn set_runs_handle<T: Send + Sync + 'static>(&self, handle: Option<Arc<T>>) {
        *self
            .held
            .runs_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            handle.map(|h| h as Arc<dyn std::any::Any + Send + Sync>);
    }

    /// Retrieves the engine's `Runs` registry handle as `T`.
    pub(crate) fn runs_handle<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.held
            .runs_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|held| Arc::clone(held).downcast::<T>().ok())
    }

    /// Stores the Daemon's `LiveView` driver handle for group peer handlers.
    pub(crate) fn set_live_view_handle<T: Send + Sync + 'static>(&self, handle: Option<Arc<T>>) {
        *self
            .held
            .live_view_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            handle.map(|h| h as Arc<dyn std::any::Any + Send + Sync>);
    }

    /// Retrieves the Daemon's `LiveView` driver handle as `T`.
    pub(crate) fn live_view_handle<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.held
            .live_view_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|held| Arc::clone(held).downcast::<T>().ok())
    }

    /// Returns a shared slot handle for reading the Daemon's `LiveView` once
    /// `daemon::serve` initializes it, without creating an `Arc` cycle back to
    /// [`EngineContext`].
    pub(crate) fn live_view_slot(&self) -> LiveViewSlot {
        Arc::clone(&self.held.live_view_handle)
    }

    /// Clears the runtime handles (`group_router`, `local_dispatcher`,
    /// `runs_handle`, `live_view_handle`, `erasure`, `peer_link`, and ledger
    /// group authority) when an engine shuts down so no `Arc` cycle keeps
    /// `Runs`, `Events`, `Peering`, or `EngineContext` alive past shutdown.
    pub(crate) fn clear_runtime_handles(&self) {
        self.held.has_group_router.store(false, Ordering::Release);
        *self
            .held
            .group_router
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .held
            .group_peer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .held
            .local_dispatcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .held
            .runs_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .held
            .live_view_handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .held
            .erasure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .held
            .peer_link
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        self.ledger().set_authority(None);
    }

    /// Registers or clears the Daemon group erasure broadcaster (ADR 0055).
    pub(crate) fn set_erasure_broadcaster(&self, broadcaster: Option<Arc<dyn ErasureBroadcaster>>) {
        *self
            .held
            .erasure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = broadcaster;
    }

    /// Broadcasts `target` to every Group member if this engine belongs to a
    /// Daemon group; a no-op outside a group.
    pub(crate) async fn broadcast_erasure(&self, target: ErasureTarget) -> Result<(), CallError> {
        let broadcaster = self
            .held
            .erasure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(broadcaster) = broadcaster {
            broadcaster.broadcast(target).await?;
        }
        Ok(())
    }

    /// Checks a named token's lowercase hex SHA-256 digest across the Daemon
    /// group when this engine is a Group member, respecting the 10-minute
    /// cached-token cap when the Lead Daemon is unreachable (ADR 0055).
    ///
    /// # Errors
    ///
    /// Returns `lead_unreachable` when the Lead Daemon cannot be reached and
    /// no fresh cached verification exists, or a store failure.
    pub async fn check_group_named_token(
        &self,
        token_sha256: &str,
    ) -> Result<Option<String>, CallError> {
        let broadcaster = self
            .held
            .erasure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match broadcaster {
            Some(broadcaster) => broadcaster.check_named_token(token_sha256.to_owned()).await,
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;
