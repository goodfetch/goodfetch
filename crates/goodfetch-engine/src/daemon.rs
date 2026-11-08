//! The Daemon's lifetime (ADRs 0024, 0036, 0078 and 0098): one engine of a
//! user's major that several Clients share, each Connection taken from its
//! listener and opened by a `hello` carrying its per-launch token, each
//! Client's Runs its own, stopped when that Client's Connection ends, but
//! for a CLI command's, which outlive it. It stops after
//! `daemon.idle_timeout` with no Client connected and no such Run live,
//! read on the `Clock`, unless it is kept alive (`--keep-alive`, a [`Held`]
//! Schedule, an installed service); it stops when `daemon.stop` asks; and when it is
//! not its major's pick, it hands over to the pick once it hosts no live
//! Run and no Client but a Console, or when it refuses a Client by Product
//! or version while idle, the refusal then telling that Client to wait for
//! the pick. Its state changes only as the engine's transition table says
//! (ADR 0128): an end that comes once another is decided changes nothing.
//!
//! Which executable is the pick, and what a handover takes on the machine
//! (the start lock, the runtime file), is the [`Succession`] its starter
//! gives it; [`files`] holds the machine's side: the runtime file, the
//! locks and the token.
//!
//! It answers `daemon.health` from the checks its ops listener's probes
//! make, and serves that listener when its starter opened one (ADR 0050);
//! it records its connected Clients and whether it drains in the runtime's
//! metrics.

pub mod files;
pub mod service;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{ProductVersion, Surface};
use goodfetch_protocol::console::{ConsoleFailure, ConsoleState};
use goodfetch_protocol::daemon::{
    DaemonEndpoint, DaemonHealth, DaemonParams, DaemonState, DaemonStopping, HEALTH, NOT_A_DAEMON,
    NotADaemonDetails, STATUS, STOP,
};
use tokio::sync::{Mutex, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::composition::{Answering, CallContext, DaemonSide, Handler, Holding};
use crate::endpoint::Authorities;
use crate::engine_context::EngineContext;
use crate::events::Events;
use crate::metrics::Metrics;
use crate::ops::{self, Health, Ops};
use crate::people::{SignInDriver, SignInPolicy};
use crate::product::{Channel, Identity};
use crate::rpc::{self, ConsoleConnection, Dispatch};
use crate::run_state::install::InstallDatabase;
use crate::runs::{ConnectionId, Runs};
use crate::tasks::TaskOwner;
use crate::transition::engine::{self as table, EngineEvent, EngineState, InvalidEngineTransition};
use crate::transport::{Connection, ConsoleListener, DaemonListener, TransportError};

/// How long a Daemon asked to stop waits for the asking Client to read the
/// answer and close before it closes that Connection itself.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// What a Daemon runs with beside its engine's [`Dependencies`](crate::Dependencies).
pub struct DaemonOptions {
    /// The per-launch token every `hello` must carry, from the OS
    /// generator ([`crate::endpoint::new_token`]).
    pub token: String,
    /// Where it listens, as its runtime file names it.
    pub endpoint: DaemonEndpoint,
    /// Its process ID.
    pub pid: u32,
    /// `daemon.idle_timeout`.
    pub idle_timeout: Duration,
    /// Whether it never stops idle: `--keep-alive`, or an installed service.
    pub keep_alive: bool,
    /// What keeps it from stopping idle while held, such as an enabled
    /// Schedule of the active major (area 10).
    pub holds: Holds,
    /// Which executable is its major's pick, and how it hands over.
    pub succession: Arc<dyn Succession>,
    /// Its Console: the listener it serves its engine's Console build on,
    /// or why it has none.
    pub console: DaemonConsole,
    /// How it drains when its starter asks, as `daemon run` does on
    /// `SIGTERM` (ADR 0049); none for a Daemon on the machine.
    pub drain: Option<Drain>,
    /// The install database it keeps every Run's run state in, in the
    /// per-install layout (`run_state.layout`, ADR 0002); none in the
    /// per-run layout.
    pub install: Option<InstallDatabase>,
    /// What it checks its health with, and its ops listener (ADR 0050).
    pub ops: Ops,
    /// Whether its listener also carries the protocol over WebSocket, as
    /// an Exposed Daemon's address does (ADR 0049): the names a handshake's
    /// `Host` may carry, and its `Origin` over `http`; none for a Daemon
    /// on the machine.
    pub websocket: Option<Authorities>,
}

/// How a foreground Daemon drains (ADR 0049): once `asked` is cancelled it
/// takes no Run, stops each live Run at its next safe point with a
/// Checkpoint, resumable by its Run ID, and ends once every one stopped or
/// `grace` passed on its `Clock`, whichever comes first; a Run that reached
/// no safe point by then stays resumable from its run file.
#[derive(Clone, Debug)]
pub struct Drain {
    /// Cancelled when the Daemon must drain.
    pub asked: CancellationToken,
    /// `daemon.drain_grace`.
    pub grace: Duration,
}

/// A Daemon's Console, or the one an Attached engine serves for its
/// script, as its starter set it up (ADR 0031).
pub enum DaemonConsole {
    /// The Console's listener, bound at `console.address`, and the URL it
    /// is reached at.
    Listening {
        /// The listener.
        listener: ConsoleListener,
        /// Its URL, such as `http://127.0.0.1:7801/`.
        url: String,
        /// `console.public_url`: the address links to it name instead of
        /// `url`, when set.
        public_url: Option<String>,
        /// How long its sign-ins last and how often an address may try.
        sign_in: SignInPolicy,
        /// Its content origin, or none when it did not start.
        content: Option<ContentOrigin>,
    },
    /// The Console did not start; the Daemon still serves its Clients.
    Failed(ConsoleFailure),
    /// No Console, as for a Daemon a test runs in process.
    Absent,
}

/// The Console's content origin (ADR 0031): the second origin that shows
/// scraped HTML, screenshots, DOM snapshots and page bodies by signed
/// address, with no cookie.
pub struct ContentOrigin {
    /// Its own listener, at `console.content_address`; none when it shares
    /// the Console's listener, which tells the two apart by the `Host` of
    /// `url` (ADR 0049).
    pub listener: Option<ConsoleListener>,
    /// The origin its signed addresses name, such as
    /// `http://127.0.0.1:51234` or `console.content_public_url`.
    pub url: String,
}

/// Which executable is the Daemon's major's pick, and what a handover takes
/// on the machine (ADR 0098). Both are blocking, and run on the tasks
/// module's blocking policy.
pub trait Succession: Send + Sync {
    /// Whether this Daemon is its major's pick now.
    fn is_pick(&self) -> bool;
    /// Begins the handover: takes what keeps a starter from racing it (the
    /// start lock) and unpublishes this Daemon (its runtime file), so the
    /// next Client to look waits for, or starts, the pick. What it answers
    /// lives until the Daemon's end, which hands it back.
    fn begin_handover(&self) -> Box<dyn Send>;
}

/// Why a Daemon stopped.
pub enum DaemonEnd {
    /// No Client was connected for `daemon.idle_timeout`.
    Idle,
    /// `daemon.stop` asked it to.
    Stopped,
    /// It handed over to its major's pick, which its caller now starts with
    /// what [`Succession::begin_handover`] answered.
    HandedOver(Box<dyn Send>),
    /// Its listener failed.
    ListenerFailed,
    /// It drained, as its starter asked.
    Drained,
}

impl std::fmt::Debug for DaemonEnd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Idle => "Idle",
            Self::Stopped => "Stopped",
            Self::HandedOver(_) => "HandedOver",
            Self::ListenerFailed => "ListenerFailed",
            Self::Drained => "Drained",
        })
    }
}

/// What keeps a Daemon from stopping idle while any [`Held`] lives: the
/// hook through which Schedules (area 10) keep it running.
#[derive(Clone, Debug)]
pub struct Holds(Arc<watch::Sender<u32>>);

impl Default for Holds {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(0)))
    }
}

impl Holds {
    /// No hold yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// A hold, which keeps the Daemon from stopping idle until it drops.
    #[must_use]
    pub fn hold(&self) -> Held {
        self.0.send_modify(|count| *count += 1);
        Held(Arc::clone(&self.0))
    }

    fn held(&self) -> bool {
        *self.0.borrow() > 0
    }
}

/// One hold on a Daemon.
#[derive(Debug)]
pub struct Held(Arc<watch::Sender<u32>>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// What the engine gives the Daemon to serve its Connections with.
pub(crate) struct Serving {
    pub(crate) dispatch: Arc<Dispatch>,
    pub(crate) runs: Runs,
    pub(crate) events: Events,
    /// What the engine gives every call and listener, its clock among
    /// them.
    pub(crate) engine: EngineContext,
    /// Where its Console is served, or why it is not.
    pub(crate) console: ConsoleState,
    /// The Connections its Console's server opens: `WebSocket`s and event streams.
    pub(crate) console_connections: mpsc::Receiver<ConsoleConnection>,
    /// The runtime's metrics.
    pub(crate) metrics: Metrics,
    /// The sign-in driver of its Console, if it serves one.
    pub(crate) sign_ins: Option<SignInDriver>,
}

/// The Clients connected now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Presence {
    /// Every connected Client, as `daemon.status` counts them.
    clients: u32,
    /// Those that hold off the idle stop.
    holding_off_idle_stop: u32,
    /// Those that hold off a handover to the major's pick.
    holding_off_handover: u32,
}

impl Presence {
    /// What one Client of `surface` counts toward each figure.
    ///
    /// A Console holds off the idle stop, as a visible tab does, but not a
    /// handover (ADR 0098). A Kennel that only observes holds off neither
    /// (ADR 0141).
    fn of_one(surface: Surface, holding: Holding) -> Self {
        let holds = holding == Holding::Holds;
        Self {
            clients: 1,
            holding_off_idle_stop: u32::from(holds),
            holding_off_handover: u32::from(holds && surface.holds_off_handover()),
        }
    }

    fn add(&mut self, one: Self) {
        self.clients += one.clients;
        self.holding_off_idle_stop += one.holding_off_idle_stop;
        self.holding_off_handover += one.holding_off_handover;
    }

    fn remove(&mut self, one: Self) {
        self.clients = self.clients.saturating_sub(one.clients);
        self.holding_off_idle_stop = self
            .holding_off_idle_stop
            .saturating_sub(one.holding_off_idle_stop);
        self.holding_off_handover = self
            .holding_off_handover
            .saturating_sub(one.holding_off_handover);
    }
}

/// Why the Daemon's loop ends, as its Connections ask.
enum Ending {
    Stop(ConnectionId),
    HandOver(Box<dyn Send>),
}

/// The Daemon's state that each of its Connections reaches.
struct Shared {
    identity: Arc<Identity>,
    token: String,
    endpoint: DaemonEndpoint,
    pid: u32,
    idle_timeout: Duration,
    keep_alive: bool,
    holds: Holds,
    /// One hold for each Connection whose live Runs outlive it, until
    /// they end; not the kept-alive `daemon status` shows, which those
    /// Runs show as live.
    runs_held: Holds,
    succession: Arc<dyn Succession>,
    console: ConsoleState,
    websocket: Option<Arc<Authorities>>,
    presence: watch::Sender<Presence>,
    runs: Runs,
    health: Health,
    metrics: Metrics,
    blocking: TaskOwner,
    endings: mpsc::Sender<Ending>,
    sign_ins: Option<SignInDriver>,
    /// Where the Daemon is, which only the engine's table changes; held
    /// while a handover is decided, so at most one begins.
    state: Mutex<EngineState>,
}

impl Shared {
    fn new(
        serving: &Serving,
        options: DaemonOptions,
        blocking: TaskOwner,
        endings: mpsc::Sender<Ending>,
    ) -> Self {
        let health = Health {
            runs: serving.runs.clone(),
            browsers: options.ops.browsers,
            blocking: Arc::new(blocking.child()),
        };
        Self {
            identity: Arc::clone(&serving.dispatch.identity),
            token: options.token,
            endpoint: options.endpoint,
            pid: options.pid,
            idle_timeout: options.idle_timeout,
            keep_alive: options.keep_alive,
            holds: options.holds,
            runs_held: Holds::new(),
            succession: options.succession,
            console: serving.console.clone(),
            websocket: options.websocket.map(Arc::new),
            presence: watch::Sender::new(Presence::default()),
            runs: serving.runs.clone(),
            health,
            metrics: serving.metrics.clone(),
            blocking,
            endings,
            sign_ins: serving.sign_ins.clone(),
            state: Mutex::new(EngineState::Serving),
        }
    }

    /// Applies `event` to the Daemon's state, as the engine's table says:
    /// its next state, or why it is refused, logged when only a bug causes
    /// it.
    async fn transition(&self, event: EngineEvent) -> Result<EngineState, InvalidEngineTransition> {
        let mut state = self.state.lock().await;
        let next = table::engine(*state, event).map_err(InvalidEngineTransition::logged)?;
        *state = next;
        Ok(next)
    }

    fn kept_alive(&self) -> bool {
        self.keep_alive || self.holds.held()
    }

    /// Whether no Client that holds off a handover is connected and no Run
    /// is live.
    async fn nothing_holds_off_a_handover(&self) -> bool {
        self.presence.borrow().holding_off_handover == 0 && self.runs.live_count().await == 0
    }

    /// Hands over when this Daemon is idle but for Consoles and Kennels that
    /// only observe, and is not its major's pick; whether it began to.
    async fn hand_over_if_not_the_pick(&self) -> bool {
        let mut state = self.state.lock().await;
        if *state == EngineState::HandingOver {
            // A handover began already: the Client waits for it too.
            return true;
        }
        if !self.nothing_holds_off_a_handover().await {
            return false;
        }
        let succession = Arc::clone(&self.succession);
        if self.blocking.blocking(move || succession.is_pick()).await {
            return false;
        }
        match table::engine(*state, EngineEvent::HandOver) {
            Ok(EngineState::HandingOver) => {}
            // A draining or stopping Daemon hands nothing over.
            Ok(
                next @ (EngineState::Attached
                | EngineState::Serving
                | EngineState::Draining
                | EngineState::Stopping),
            ) => {
                *state = next;
                return false;
            }
            Err(refused) => {
                let _ = refused.logged();
                return false;
            }
        }
        let succession = Arc::clone(&self.succession);
        let begun = self
            .blocking
            .blocking(move || succession.begin_handover())
            .await;
        *state = EngineState::HandingOver;
        let _ = self.endings.send(Ending::HandOver(begun)).await;
        true
    }
}

/// One Connection's view of the Daemon.
struct ConnectionSide {
    shared: Arc<Shared>,
    connection: ConnectionId,
}

/// A Client counted while it is open.
struct Opened {
    presence: watch::Sender<Presence>,
    metrics: Metrics,
    surface: Surface,
    counted: Presence,
}

impl Drop for Opened {
    fn drop(&mut self) {
        self.metrics.clients(self.surface, -1);
        self.presence
            .send_modify(|presence| presence.remove(self.counted));
    }
}

impl DaemonSide for ConnectionSide {
    fn token(&self) -> &str {
        &self.shared.token
    }

    fn opened(&self, surface: Surface, holding: Holding) -> Box<dyn Send + Sync> {
        let counted = Presence::of_one(surface, holding);
        self.shared
            .presence
            .send_modify(|presence| presence.add(counted));
        self.shared.metrics.clients(surface, 1);
        Box::new(Opened {
            presence: self.shared.presence.clone(),
            metrics: self.shared.metrics.clone(),
            surface,
            counted,
        })
    }

    fn refusing(&self) -> Answering<'_, bool> {
        Box::pin(self.shared.hand_over_if_not_the_pick())
    }

    fn status(&self) -> Answering<'_, DaemonState> {
        Box::pin(async {
            let shared = &self.shared;
            let presence = *shared.presence.borrow();
            DaemonState {
                product: shared.identity.product,
                product_version: shared.identity.product_version.clone(),
                pid: shared.pid,
                endpoint: shared.endpoint.clone(),
                clients: presence.clients,
                live_runs: count(shared.runs.live_count().await),
                kept_alive: shared.kept_alive(),
                idle_timeout: u64::try_from(shared.idle_timeout.as_millis()).unwrap_or(u64::MAX),
                console: shared.console.clone(),
                heap: crate::heap::live(),
            }
        })
    }

    fn health(&self) -> Answering<'_, DaemonHealth> {
        Box::pin(self.shared.health.check())
    }

    fn sign_ins(&self) -> Option<SignInDriver> {
        self.shared.sign_ins.clone()
    }

    fn stop(&self) -> Answering<'_, DaemonStopping> {
        Box::pin(async {
            let live_runs = count(self.shared.runs.live_count().await);
            let _ = self
                .shared
                .endings
                .send(Ending::Stop(self.connection))
                .await;
            DaemonStopping { live_runs }
        })
    }
}

fn count(live: usize) -> u32 {
    u32::try_from(live).unwrap_or(u32::MAX)
}

/// Serves every Connection of `listener` until the Daemon stops, its tasks
/// those of children of `owner`.
pub(crate) async fn serve(
    mut serving: Serving,
    mut listener: DaemonListener,
    mut options: DaemonOptions,
    owner: &TaskOwner,
) -> DaemonEnd {
    let drain = options.drain.take();
    let ops = options.ops.listener.take();
    let asked = async {
        match &drain {
            Some(drain) => drain.asked.cancelled().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(asked);
    let (endings, mut ending) = mpsc::channel(1);
    let shared = Arc::new(Shared::new(&serving, options, owner.child(), endings));
    serving.metrics.draining(false);
    if let Some(ops) = ops {
        let (health, engine) = (shared.health.clone(), serving.engine.clone());
        owner.spawn(ops::serve(ops, health, owner.child(), engine));
    }
    let connections_owner = owner.child();
    let (ended, mut endings_of_connections) = mpsc::unbounded_channel::<ConnectionId>();
    let mut open: BTreeMap<ConnectionId, CancellationToken> = BTreeMap::new();
    let mut next: ConnectionId = 0;
    let clock = serving.engine.clock();
    let idle = idle(&shared, clock);
    tokio::pin!(idle);
    let mut accepting = true;
    let mut console_open = true;
    let mut idled = false;
    let mut drain_asked = false;
    let end = loop {
        tokio::select! {
            biased;
            Some(asked) = ending.recv() => match asked {
                Ending::HandOver(begun) => break Ending::HandOver(begun),
                Ending::Stop(asking) => match shared.transition(EngineEvent::StopAsked).await {
                    Ok(EngineState::Stopping) => break Ending::Stop(asking),
                    // A stop once a handover or a drain is decided changes nothing.
                    Ok(
                        EngineState::Attached
                        | EngineState::Serving
                        | EngineState::HandingOver
                        | EngineState::Draining,
                    )
                    | Err(_) => {}
                },
            },
            Some(connection) = endings_of_connections.recv() => {
                let _ = shared.transition(EngineEvent::ClientLost).await;
                open.remove(&connection);
                if shared.presence.borrow().holding_off_handover == 0 {
                    let shared = Arc::clone(&shared);
                    owner.spawn(async move {
                        shared.hand_over_if_not_the_pick().await;
                    });
                }
            }
            () = &mut idle, if !idled => {
                idled = true;
                if let Ok(EngineState::Stopping) = shared.transition(EngineEvent::Idle).await {
                    return finish(DaemonEnd::Idle, &connections_owner, &shared).await;
                }
            }
            () = &mut asked, if !drain_asked => {
                drain_asked = true;
                // A drain once a handover is decided changes nothing.
                if let Ok(EngineState::Draining) = shared.transition(EngineEvent::Drain).await {
                    serving.metrics.draining(true);
                    let grace = drain.as_ref().map_or(Duration::ZERO, |drain| drain.grace);
                    drain_runs(&serving, grace).await;
                    drop(listener);
                    let _ = shared.transition(EngineEvent::Drained).await;
                    return finish(DaemonEnd::Drained, &connections_owner, &shared).await;
                }
            }
            console = serving.console_connections.recv(), if console_open => match console {
                Some(console) => admit(&mut open, &mut next, |id| serve_connection(&serving, &shared, &connections_owner, Arriving::Console(console), id, ended.clone())),
                // The Console's server stopped, or there is none.
                None => console_open = false,
            },
            accepted = listener.accept(), if accepting => match accepted {
                Ok(connection) => admit(&mut open, &mut next, |id| serve_connection(&serving, &shared, &connections_owner, Arriving::Listener(connection), id, ended.clone())),
                // Every in-memory dialer is gone: it goes on with its Clients.
                Err(TransportError::DialerClosed) => accepting = false,
                Err(error) => {
                    tracing::error!(%error, "the Daemon's listener failed");
                    accepting = false;
                    let failed = shared.transition(EngineEvent::ListenerFailed).await;
                    if let Ok(EngineState::Stopping) = failed {
                        return finish(DaemonEnd::ListenerFailed, &connections_owner, &shared).await;
                    }
                }
            },
        }
    };
    drop(listener);
    let end = match end {
        Ending::HandOver(begun) => DaemonEnd::HandedOver(begun),
        Ending::Stop(asking) => {
            close_all_but(&open, asking, &mut endings_of_connections, clock).await;
            DaemonEnd::Stopped
        }
    };
    finish(end, &connections_owner, &shared).await
}

/// Serves the next Connection, `next`, with `serve`, keeping the token
/// that closes it in `open`.
fn admit(
    open: &mut BTreeMap<ConnectionId, CancellationToken>,
    next: &mut ConnectionId,
    serve: impl FnOnce(ConnectionId) -> CancellationToken,
) {
    let id = *next;
    *next += 1;
    open.insert(id, serve(id));
}

/// A Connection that reached the Daemon: through its listener, or as a
/// Connection its Console's server opened.
enum Arriving {
    Listener(Connection),
    Console(ConsoleConnection),
}

/// Serves `connection` as the Daemon's Connection `id`, a task of
/// `connections`, telling `ended` once it closes; the token that closes it.
fn serve_connection(
    serving: &Serving,
    shared: &Arc<Shared>,
    connections: &TaskOwner,
    connection: Arriving,
    id: ConnectionId,
    ended: mpsc::UnboundedSender<ConnectionId>,
) -> CancellationToken {
    let token = connections.token().child_token();
    let side: Arc<dyn DaemonSide> = Arc::new(ConnectionSide {
        shared: Arc::clone(shared),
        connection: id,
    });
    let (dispatch, runs, events, engine) = (
        Arc::clone(&serving.dispatch),
        serving.runs.on_connection(id),
        serving.events.clone(),
        serving.engine.clone(),
    );
    let websocket = shared.websocket.clone();
    let runs_held = shared.runs_held.clone();
    let its_owner = connections.child();
    let cancelled = token.clone();
    connections.spawn(async move {
        let served = async {
            match connection {
                Arriving::Listener(connection) => {
                    let access = match websocket {
                        Some(authorities) => rpc::Access::Exposed(side, authorities),
                        None => rpc::Access::Daemon(side),
                    };
                    rpc::serve(
                        dispatch,
                        connection,
                        &its_owner,
                        runs.clone(),
                        events,
                        access,
                        engine,
                    )
                    .await;
                }
                Arriving::Console(console) => {
                    let Some(sign_ins) = side.sign_ins() else {
                        // Only a Console's server opens one, and it has a driver.
                        return;
                    };
                    rpc::serve_console(
                        dispatch,
                        console,
                        &its_owner,
                        runs.clone(),
                        events,
                        rpc::ConsoleSide {
                            daemon: Some(side),
                            sign_ins,
                        },
                        engine,
                    )
                    .await;
                }
            }
        };
        tokio::select! {
            biased;
            () = cancelled.cancelled() => {}
            () = served => {}
        }
        its_owner.stop().await;
        let outliving = runs.stop_connection(id).await;
        // A Run that outlives the CLI command that started it holds the
        // Daemon from its idle stop until it ends, so it runs to its end
        // with no Client connected (ADR 0024's #1183 note).
        let held = (!outliving.is_empty()).then(|| runs_held.hold());
        let _ = ended.send(id);
        if let Some(held) = held {
            tokio::select! {
                biased;
                () = cancelled.cancelled() => {}
                () = outliving.ended() => {}
            }
            drop(held);
        }
    });
    token
}

/// Closes every Connection of `open` but `asking`, which `daemon.stop`
/// asked: that Client reads the answer and closes, and one that does not
/// is closed after the grace.
async fn close_all_but(
    open: &BTreeMap<ConnectionId, CancellationToken>,
    asking: ConnectionId,
    endings_of_connections: &mut mpsc::UnboundedReceiver<ConnectionId>,
    clock: Clock,
) {
    for (connection, token) in open {
        if *connection != asking {
            token.cancel();
        }
    }
    let closed = async {
        while let Some(connection) = endings_of_connections.recv().await {
            if connection == asking {
                return;
            }
        }
    };
    tokio::select! {
        () = closed => {}
        () = clock.sleep(STOP_GRACE) => {}
    }
}

/// Drains the Daemon's Runs, waiting at most `grace` for them to stop.
async fn drain_runs(serving: &Serving, grace: Duration) {
    let clock = serving.engine.clock();
    let drained = async {
        let draining = serving.runs.drain().await;
        tracing::info!(
            live_runs = draining.count(),
            "the Daemon drains: it takes no run and checkpoints each live run"
        );
        draining.ended().await;
    };
    tokio::select! {
        () = drained => tracing::info!("the Daemon drained every run"),
        () = clock.sleep(grace) => tracing::warn!(
            live_runs = serving.runs.live_count().await,
            "the drain grace passed: each run not yet at a safe point stays resumable from its run file"
        ),
    }
}

/// Closes every Connection, then answers `end`.
async fn finish(end: DaemonEnd, connections: &TaskOwner, shared: &Shared) -> DaemonEnd {
    connections.stop().await;
    shared.blocking.stop().await;
    tracing::info!(end = ?end, "the Daemon stops");
    end
}

/// Resolves once no Client that holds off the idle stop has been connected
/// (none but Kennels that only observe), and nothing has kept the Daemon
/// alive, neither a hold nor a live Run that outlived the CLI command that
/// started it, for `daemon.idle_timeout` on `clock`; never when it is kept
/// alive by `--keep-alive` or a service.
async fn idle(shared: &Shared, clock: Clock) {
    if shared.keep_alive {
        return std::future::pending().await;
    }
    let mut presence = shared.presence.subscribe();
    let mut holds = shared.holds.0.subscribe();
    let mut runs_held = shared.runs_held.0.subscribe();
    loop {
        let holding = presence.borrow_and_update().holding_off_idle_stop;
        let held = *holds.borrow_and_update() + *runs_held.borrow_and_update();
        if holding == 0 && held == 0 {
            tokio::select! {
                () = clock.sleep(shared.idle_timeout) => return,
                _ = presence.changed() => {}
                _ = holds.changed() => {}
                _ = runs_held.changed() => {}
            }
        } else {
            tokio::select! {
                _ = presence.changed() => {}
                _ = holds.changed() => {}
                _ = runs_held.changed() => {}
            }
        }
    }
}

/// How a Client the Daemon refused runs its own Product's executable.
#[derive(Clone, Copy, Debug)]
pub enum RefusedExecutable<'a> {
    /// The executable it would run attached, at this path: an SDK's, the
    /// CLI's or the MCP server's.
    Path(&'a std::path::Path),
    /// None: a Rust program, which links the engine through its Product's
    /// crate.
    Linked,
}

/// A Client a Daemon refused by Product or version while it could not hand
/// over (ADR 0098).
#[derive(Clone, Copy, Debug)]
pub struct RefusedClient<'a> {
    /// The Client's own Product.
    pub product: Product,
    /// Its Product version.
    pub version: &'a ProductVersion,
    /// Its executable.
    pub executable: RefusedExecutable<'a>,
    /// The channel its Product came from, whose install commands the fix
    /// names.
    pub channel: Channel,
    /// Whether its executable would be its major's pick once it ran.
    pub would_be_pick: bool,
}

/// What a refusal's `fix` adds for `client` (ADR 0098): its own Product's
/// `daemon restart` at its version, given by its path, a Rust Client's
/// install of that executable first; and, where that executable would
/// still not be the pick, Foxhound at its version or newer.
pub fn refusal_fix(client: &RefusedClient<'_>) -> String {
    let name = client.product.name();
    let version = client.version;
    let mut fix = match client.executable {
        RefusedExecutable::Path(path) => {
            let path = path.display().to_string();
            let path = if path.contains(char::is_whitespace) {
                format!("\"{path}\"")
            } else {
                path
            };
            format!("Run `{path} daemon restart` to restart the Daemon on its major's pick.")
        }
        RefusedExecutable::Linked => format!(
            "Install {name}'s executable at {version} (`{}`), then run its `daemon restart` to restart the Daemon on its major's pick.",
            client.channel.install_command(client.product)
        ),
    };
    if !client.would_be_pick {
        let install = client.channel.install_command(Product::Foxhound);
        let _ = write!(
            fix,
            " {name}'s executable would still not run the Daemon while a Foxhound of the major is installed: install Foxhound {version} or newer (`{install}`) and run its `daemon restart`."
        );
    }
    fix
}

/// The `daemon.*` handlers.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(
            &STATUS,
            |_: DaemonParams, context: CallContext| async move {
                Ok::<_, CallError>(daemon_of(&context, STATUS.name)?.status().await)
            },
        ),
        Handler::new(&STOP, |_: DaemonParams, context: CallContext| async move {
            Ok::<_, CallError>(daemon_of(&context, STOP.name)?.stop().await)
        }),
        Handler::new(
            &HEALTH,
            |_: DaemonParams, context: CallContext| async move {
                Ok::<_, CallError>(daemon_of(&context, HEALTH.name)?.health().await)
            },
        ),
    ]
}

fn daemon_of<'a>(
    context: &'a CallContext,
    operation: &str,
) -> Result<&'a Arc<dyn DaemonSide>, CallError> {
    context.daemon().ok_or_else(|| {
        CallError::new(
            &NOT_A_DAEMON,
            format!("`{operation}` asks a Daemon, and this engine is an Attached engine."),
            &NotADaemonDetails {
                operation: operation.to_owned(),
            },
        )
    })
}

#[cfg(test)]
mod tests;
