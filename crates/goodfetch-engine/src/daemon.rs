//! The Daemon's lifetime (ADRs 0024, 0036, 0078 and 0098): one engine of a
//! user's major that several Clients share, each Connection taken from its
//! listener and opened by a `hello` carrying its per-launch token, each
//! Client's Runs its own, stopped when that Client's Connection ends, but
//! for a CLI command's, which outlive it. It stops after
//! `daemon.idle_timeout` with no Client connected and no such Run live,
//! read on the `Clock`, unless it is kept alive (`--keep-alive`, a [`Held`]
//! Schedule, an installed service); it stops when `daemon.stop` asks; and when it is
//! not its major's pick, it hands over to the pick once it hosts no live
//! Run and no Client that holds it, or when it refuses a Client by Product
//! or version while idle, the refusal then telling that Client to wait for
//! the pick. Its state belongs to its loop, the one place it changes, as
//! the engine's transition table says (ADR 0128): a Connection asks the
//! loop to stop it or to hand over, on a bounded channel, and an end that
//! comes once another is decided changes nothing.
//!
//! A failed accept ends that one attempt: it is logged and the Daemon goes
//! on accepting, at once after a stream it had to drop, and waiting a
//! little longer before each new attempt while accepts keep failing, as
//! they do while the process has no descriptor left.
//! Only a listener closed for good ends the Daemon, as a listener failure,
//! once its Runs drained as on a requested drain, within the drain grace.
//! A panic in a part every Run shares ends it too (ADR 0148), once its
//! Runs drained the same way, or at once when its Run registry is what
//! panicked.
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
//!
//! Size: one concern, the Daemon's loop and each end it comes to, which
//! share its state; the machine's side is in [`files`] and [`service`].

pub mod files;
pub mod service;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{PanicEnded, ProductVersion, Surface};
use goodfetch_protocol::daemon::{
    DaemonEndpoint, DaemonHealth, DaemonParams, DaemonState, DaemonStopping, HEALTH, NOT_A_DAEMON,
    NotADaemonDetails, STATUS, STOP,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::composition::{Answering, CallContext, DaemonSide, Handler, Holding};
use crate::endpoint::Authorities;
use crate::engine_context::EngineContext;
use crate::events::Events;
use crate::metrics::Metrics;
use crate::ops::{self, Health, Ops};
use crate::product::{Channel, Identity};
use crate::rpc::{self, Dispatch};
use crate::run_state::install::InstallDatabase;
use crate::runs::{ConnectionId, Draining, Runs};
use crate::tasks::{OwnerKind, TaskOwner};
use crate::transition::engine::{self as table, EngineEvent, EngineState, InvalidEngineTransition};
use crate::transport::{Connection, DaemonListener, TransportError};

/// How long a Daemon asked to stop waits for the asking Client to read the
/// answer and close before it closes that Connection itself.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// How long a Daemon waits before it accepts again once a second accept in
/// a row failed, doubling with each further failure up to
/// [`LONGEST_ACCEPT_WAIT`].
const FIRST_ACCEPT_WAIT: Duration = Duration::from_millis(10);

/// The longest a Daemon waits between two attempts to accept.
const LONGEST_ACCEPT_WAIT: Duration = Duration::from_secs(1);

/// How many ended Connections wait for the Daemon's loop to receive them
/// before the next one's task waits for room.
const ENDED_CONNECTIONS: usize = 16;

/// How many asks of its Connections (`daemon.stop`, a refusal's handover)
/// wait for the Daemon's loop before the next asker waits for room.
const ASKS: usize = 1;

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
    /// How it drains: when its starter asks, as `daemon run` does on
    /// `SIGTERM` (ADR 0049), and once its listener is closed for good.
    pub drain: Drain,
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

/// How a Daemon drains (ADR 0049): once `asked` is cancelled, or its
/// listener is closed for good, it takes no Run, stops each live Run at its
/// next safe point with a Checkpoint, resumable by its Run ID, and ends
/// once every one stopped or `grace` passed on its `Clock`, whichever comes
/// first; a Run that reached no safe point by then stays resumable from its
/// run file.
#[derive(Clone, Debug)]
pub struct Drain {
    /// Cancelled when the Daemon must drain, as `daemon run` asks on
    /// `SIGTERM`; none for a Daemon on the machine, which no starter asks.
    pub asked: Option<CancellationToken>,
    /// `daemon.drain_grace`.
    pub grace: Duration,
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
    /// Its listener was closed for good, and it drained.
    ListenerFailed,
    /// It drained, as its starter asked.
    Drained,
    /// A part every Run shares panicked (ADR 0148): it drained, or, when
    /// its Runs' registry is what panicked, stopped at once.
    Panicked,
}

impl std::fmt::Debug for DaemonEnd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Idle => "Idle",
            Self::Stopped => "Stopped",
            Self::HandedOver(_) => "HandedOver",
            Self::ListenerFailed => "ListenerFailed",
            Self::Drained => "Drained",
            Self::Panicked => "Panicked",
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
    /// The runtime's metrics.
    pub(crate) metrics: Metrics,
}

/// The Clients connected now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Presence {
    /// Every connected Client, as `daemon.status` counts them.
    clients: u32,
    /// Those that hold off the idle stop and a handover to the major's
    /// pick.
    holding: u32,
}

impl Presence {
    /// What one Client counts toward each figure. A Kennel that only
    /// observes holds nothing off (ADR 0141).
    fn of_one(holding: Holding) -> Self {
        Self {
            clients: 1,
            holding: u32::from(holding == Holding::Holds),
        }
    }

    fn add(&mut self, one: Self) {
        self.clients += one.clients;
        self.holding += one.holding;
    }

    fn remove(&mut self, one: Self) {
        self.clients = self.clients.saturating_sub(one.clients);
        self.holding = self.holding.saturating_sub(one.holding);
    }
}

/// What a Connection asks of the Daemon's loop.
enum Ask {
    /// `daemon.stop`, from this Connection.
    Stop(ConnectionId),
    /// A refusal by Product or version on this Connection: whether the
    /// Daemon hands over to its major's pick on it, which it has then
    /// begun.
    HandOver(ConnectionId, oneshot::Sender<bool>),
}

/// Why the Daemon's loop ends.
enum Ending {
    /// `daemon.stop`, from this Connection.
    Stop(ConnectionId),
    /// A handover, which [`Succession::begin_handover`] began, decided on
    /// a refusal on this Connection, if one asked it.
    HandOver(Box<dyn Send>, Option<ConnectionId>),
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
    websocket: Option<Arc<Authorities>>,
    presence: watch::Sender<Presence>,
    runs: Runs,
    health: Health,
    metrics: Metrics,
    blocking: TaskOwner,
    /// What the Connections ask of the Daemon's loop.
    asks: mpsc::Sender<Ask>,
    /// Where the Daemon is, as its loop, which alone changes it, last
    /// said.
    state: watch::Receiver<EngineState>,
}

impl Shared {
    fn new(
        serving: &Serving,
        options: DaemonOptions,
        blocking: TaskOwner,
        asks: mpsc::Sender<Ask>,
        state: watch::Receiver<EngineState>,
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
            websocket: options.websocket.map(Arc::new),
            presence: watch::Sender::new(Presence::default()),
            runs: serving.runs.clone(),
            health,
            metrics: serving.metrics.clone(),
            blocking,
            asks,
            state,
        }
    }

    fn kept_alive(&self) -> bool {
        self.keep_alive || self.holds.held()
    }

    /// Whether no Client that holds off a handover is connected and no Run
    /// is live.
    async fn nothing_holds_off_a_handover(&self) -> bool {
        self.presence.borrow().holding == 0 && self.runs.live_count().await == 0
    }

    /// Hands over when this Daemon, in `state`, is idle but for Kennels
    /// that only observe, and is not its major's pick: what the handover
    /// began with, which ends the Daemon's loop, whose step this is.
    async fn hand_over_if_not_the_pick(
        &self,
        state: &watch::Sender<EngineState>,
    ) -> Option<Box<dyn Send>> {
        if !self.nothing_holds_off_a_handover().await {
            return None;
        }
        self.hand_over_unless_the_pick(state).await
    }

    /// Hands over when this Daemon, in `state`, found the Run index raised
    /// past itself and has no live Run, whatever Clients are connected
    /// (ADR 0098): it takes no new Run, so only its major's pick serves
    /// them. What the handover began with, as
    /// [`Self::hand_over_if_not_the_pick`].
    async fn hand_over_if_locked_out(
        &self,
        state: &watch::Sender<EngineState>,
        lockout: &Lockout,
    ) -> Option<Box<dyn Send>> {
        if !lockout.with_no_live_run() {
            return None;
        }
        self.hand_over_unless_the_pick(state).await
    }

    /// Hands over when this Daemon, in `state`, is not its major's pick.
    async fn hand_over_unless_the_pick(
        &self,
        state: &watch::Sender<EngineState>,
    ) -> Option<Box<dyn Send>> {
        let succession = Arc::clone(&self.succession);
        if self.blocking.blocking(move || succession.is_pick()).await {
            return None;
        }
        match transition(state, EngineEvent::HandOver) {
            Ok(EngineState::HandingOver) => {}
            // A draining or stopping Daemon hands nothing over.
            Ok(
                EngineState::Attached
                | EngineState::Serving
                | EngineState::Draining
                | EngineState::Stopping,
            )
            | Err(_) => return None,
        }
        let succession = Arc::clone(&self.succession);
        Some(
            self.blocking
                .blocking(move || succession.begin_handover())
                .await,
        )
    }
}

/// What a Daemon locked out of the Run index watches to hand over once it
/// has no live Run: its count of live Runs and whether it is locked out.
struct Lockout {
    live: watch::Receiver<usize>,
    locked_out: watch::Receiver<bool>,
}

impl Lockout {
    fn of(runs: &Runs) -> Self {
        Self {
            live: runs.live_changes(),
            locked_out: runs.locked_out_of_the_index(),
        }
    }

    /// Once either changes; never once neither can.
    async fn changed(&mut self) {
        tokio::select! {
            Ok(()) = self.live.changed() => {}
            Ok(()) = self.locked_out.changed() => {}
            else => std::future::pending().await,
        }
    }

    /// Whether the Daemon is locked out and has no live Run.
    fn with_no_live_run(&self) -> bool {
        *self.locked_out.borrow() && *self.live.borrow() == 0
    }
}

/// Applies `event` to the Daemon's `state`, as the engine's table says:
/// its next state, or why it is refused, logged when only a bug causes it.
/// Only the Daemon's loop holds `state`.
fn transition(
    state: &watch::Sender<EngineState>,
    event: EngineEvent,
) -> Result<EngineState, InvalidEngineTransition> {
    let next = table::engine(*state.borrow(), event).map_err(InvalidEngineTransition::logged)?;
    state.send_replace(next);
    Ok(next)
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
        let counted = Presence::of_one(holding);
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
        Box::pin(async {
            let (answer, answered) = oneshot::channel();
            let asked = Ask::HandOver(self.connection, answer);
            if self.shared.asks.send(asked).await.is_ok()
                && let Ok(began) = answered.await
            {
                return began;
            }
            // The loop decides nothing more: a handover it decided goes
            // on, and the Client waits for it too.
            *self.shared.state.borrow() == EngineState::HandingOver
        })
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
                heap: crate::heap::live(),
            }
        })
    }

    fn health(&self) -> Answering<'_, DaemonHealth> {
        Box::pin(self.shared.health.check())
    }

    fn stop(&self) -> Answering<'_, DaemonStopping> {
        Box::pin(async {
            let live_runs = count(self.shared.runs.live_count().await);
            // Once the loop decides nothing more, the Daemon ends anyway.
            let _ = self.shared.asks.send(Ask::Stop(self.connection)).await;
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
    serving: Serving,
    listener: DaemonListener,
    mut options: DaemonOptions,
    owner: &TaskOwner,
) -> DaemonEnd {
    let Drain { asked, grace } = options.drain.clone();
    let ops = options.ops.listener.take();
    let asked = drain_asked(asked);
    tokio::pin!(asked);
    let (asks, mut asked_of_the_loop) = mpsc::channel(ASKS);
    // Where the Daemon is: only this loop changes it, and a handover is
    // one of its steps, so at most one begins.
    let (state, seen) = watch::channel(EngineState::Serving);
    let shared = Arc::new(Shared::new(
        &serving,
        options,
        owner.part("Daemon's shared state"),
        asks,
        seen,
    ));
    serving.metrics.draining(false);
    if let Some(ops) = ops {
        let (health, engine) = (shared.health.clone(), serving.engine.clone());
        owner.spawn(ops::serve(ops, health, owner.part("ops endpoint"), engine));
    }
    let connections_owner = owner.part("Daemon's listener");
    let (ended, mut endings_of_connections) = mpsc::channel::<ConnectionId>(ENDED_CONNECTIONS);
    let mut open: BTreeMap<ConnectionId, CancellationToken> = BTreeMap::new();
    let mut next: ConnectionId = 0;
    let clock = serving.engine.clock();
    let idle = idle(&shared, clock);
    tokio::pin!(idle);
    let mut accepts = Accepts::new(listener, clock);
    let mut idled = false;
    let mut drain_asked = false;
    let mut panicked = std::pin::pin!(owner.panicked());
    let mut panic_told = false;
    let mut lockout = Lockout::of(&shared.runs);
    let end = loop {
        tokio::select! {
            biased;
            Some(ask) = asked_of_the_loop.recv() => {
                if let Some(ending) = decide(ask, &shared, &state).await {
                    break ending;
                }
            }
            Some(connection) = endings_of_connections.recv() => {
                let _ = transition(&state, EngineEvent::ClientLost);
                open.remove(&connection);
                if shared.presence.borrow().holding == 0
                    && let Some(begun) = shared.hand_over_if_not_the_pick(&state).await
                {
                    break Ending::HandOver(begun, None);
                }
            }
            () = lockout.changed() => {
                if let Some(begun) = shared.hand_over_if_locked_out(&state, &lockout).await {
                    break Ending::HandOver(begun, None);
                }
            }
            () = &mut idle, if !idled => {
                idled = true;
                if let Ok(EngineState::Stopping) = transition(&state, EngineEvent::Idle) {
                    decide_no_more(&mut asked_of_the_loop);
                    return finish(DaemonEnd::Idle, &connections_owner, &shared).await;
                }
            }
            () = &mut asked, if !drain_asked => {
                drain_asked = true;
                // A drain once a handover is decided changes nothing.
                if let Ok(EngineState::Draining) = transition(&state, EngineEvent::Drain) {
                    decide_no_more(&mut asked_of_the_loop);
                    let until = drain_runs(&serving, &state, grace, serving.runs.drain()).await;
                    drop(accepts);
                    return finish_by(DaemonEnd::Drained, &connections_owner, &shared, clock, until).await;
                }
            }
            () = &mut panicked, if !panic_told => {
                panic_told = true;
                if let Some(until) = stopped_on_panic(&serving, owner, &state, grace, &mut asked_of_the_loop).await {
                    drop(accepts);
                    return finish_by(DaemonEnd::Panicked, &connections_owner, &shared, clock, until).await;
                }
            }
            accepted = accepts.next() => match accepted {
                Ok(connection) => admit(&mut open, &mut next, |id| serve_connection(&serving, &shared, &connections_owner, connection, id, ended.clone())),
                // It goes on with its Clients.
                Err(NoMoreConnections::DialersGone) => {}
                Err(NoMoreConnections::ListenerClosed(error)) => {
                    tracing::error!(%error, "the Daemon's listener is closed: it drains its runs and stops");
                    // A listener closed once a handover is decided changes nothing.
                    let failed = transition(&state, EngineEvent::ListenerFailed);
                    if let Ok(EngineState::Draining) = failed {
                        decide_no_more(&mut asked_of_the_loop);
                        let until = drain_runs(&serving, &state, grace, serving.runs.drain()).await;
                        drop(accepts);
                        return finish_by(DaemonEnd::ListenerFailed, &connections_owner, &shared, clock, until).await;
                    }
                }
            },
        }
    };
    decide_no_more(&mut asked_of_the_loop);
    drop(accepts);
    let end = closed_for(end, &open, &mut endings_of_connections, clock).await;
    finish(end, &connections_owner, &shared).await
}

/// Once `asked` asks the Daemon to drain; never without it.
async fn drain_asked(asked: Option<CancellationToken>) {
    match asked {
        Some(asked) => asked.cancelled().await,
        None => std::future::pending().await,
    }
}

/// The Daemon's end for `ending`, once the Connections in `open` it closes
/// for it, all but the one that asked, have closed: told on `ended`, each
/// within its time on `clock`.
async fn closed_for(
    ending: Ending,
    open: &BTreeMap<ConnectionId, CancellationToken>,
    ended: &mut mpsc::Receiver<ConnectionId>,
    clock: Clock,
) -> DaemonEnd {
    match ending {
        Ending::HandOver(begun, asking) => {
            // The refused Client that asked reads its refusal, which tells
            // it to wait for the pick, before its Connection closes.
            if let Some(asking) = asking {
                close_all_but(open, asking, ended, clock).await;
            }
            DaemonEnd::HandedOver(begun)
        }
        Ending::Stop(asking) => {
            close_all_but(open, asking, ended, clock).await;
            DaemonEnd::Stopped
        }
    }
}

/// What a panic in a part every Run shares under `owner` does to the
/// Daemon in `state`, as a step of its loop (ADR 0148): it logs the part
/// and counts the panic, decides no more of what its Connections ask, and
/// drains its Runs within `grace`, unless its Runs' registry is what
/// panicked, when it stops at once; if it now stops, the moment its stop
/// ends by, `grace` after the panic.
async fn stopped_on_panic(
    serving: &Serving,
    owner: &TaskOwner,
    state: &watch::Sender<EngineState>,
    grace: Duration,
    asked_of_the_loop: &mut mpsc::Receiver<Ask>,
) -> Option<Instant> {
    let part = owner.panicked_part();
    let draining = match transition(
        state,
        EngineEvent::panicked(serving.runs.registry_panicked()),
    ) {
        Ok(EngineState::Draining) => true,
        Ok(EngineState::Stopping) => false,
        // Its loop serves until a handover is decided, which ends it.
        Ok(EngineState::Attached | EngineState::Serving | EngineState::HandingOver) => {
            return None;
        }
        // Refused, which was logged.
        Err(_) => return None,
    };
    tracing::error!(
        part,
        draining,
        "a part every Run shares panicked: the Daemon stops, draining its runs while its Run registry runs, else at once for its next start to reconcile them"
    );
    serving.metrics.panicked(PanicEnded::Engine);
    decide_no_more(asked_of_the_loop);
    if draining {
        Some(drain_runs(serving, state, grace, serving.runs.drain_on_panic()).await)
    } else {
        Some(serving.engine.clock().now() + grace)
    }
}

/// Decides what a Connection asked, as a step of the Daemon's loop in
/// `state`: the loop's end, if it ends it.
async fn decide(ask: Ask, shared: &Shared, state: &watch::Sender<EngineState>) -> Option<Ending> {
    match ask {
        Ask::Stop(asking) => match transition(state, EngineEvent::StopAsked) {
            Ok(EngineState::Stopping) => Some(Ending::Stop(asking)),
            // A stop once a handover or a drain is decided changes nothing.
            Ok(
                EngineState::Attached
                | EngineState::Serving
                | EngineState::HandingOver
                | EngineState::Draining,
            )
            | Err(_) => None,
        },
        Ask::HandOver(asking, answer) => {
            let begun = shared.hand_over_if_not_the_pick(state).await;
            let _ = answer.send(begun.is_some());
            begun.map(|begun| Ending::HandOver(begun, Some(asking)))
        }
    }
}

/// Ends what the Daemon's loop decides, once it decided its end: a later
/// ask fails at once, and one still waiting is dropped, so its asker reads
/// the Daemon's state instead of an answer.
fn decide_no_more(asks: &mut mpsc::Receiver<Ask>) {
    asks.close();
    while asks.try_recv().is_ok() {}
}

/// Why a listener gives no Connection any more.
pub(crate) enum NoMoreConnections {
    /// Every in-memory dialer is gone.
    DialersGone,
    /// It was closed for good, as the OS said with this failure.
    ListenerClosed(TransportError),
}

/// The Connections an engine takes from a listener, a Daemon's or an
/// Attached engine's private endpoint's, past each failed accept.
pub(crate) struct Accepts {
    listener: DaemonListener,
    clock: Clock,
    /// Whether the listener gives no Connection any more.
    ended: bool,
    /// How many accepts failed in a row.
    failed: u32,
    /// When it accepts again, after accepts that failed in a row.
    again_at: Option<Instant>,
}

impl Accepts {
    pub(crate) fn new(listener: DaemonListener, clock: Clock) -> Self {
        Self {
            listener,
            clock,
            ended: false,
            failed: 0,
            again_at: None,
        }
    }

    /// The next Connection. A failed accept ends that attempt alone: it is
    /// logged, and the next attempt follows, at once after a dropped
    /// stream or one failure, and after a wait once failures repeat. Answers once why the listener
    /// gives no Connection any more, and never resolves after that.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no Connection, and a wait it began ends
    /// when it would have.
    pub(crate) async fn next(&mut self) -> Result<Connection, NoMoreConnections> {
        loop {
            if self.ended {
                return std::future::pending().await;
            }
            if let Some(again_at) = self.again_at {
                self.clock.sleep_until(again_at).await;
                self.again_at = None;
            }
            match self.listener.accept().await {
                Ok(connection) => {
                    self.failed = 0;
                    return Ok(connection);
                }
                Err(TransportError::DialerClosed) => {
                    self.ended = true;
                    return Err(NoMoreConnections::DialersGone);
                }
                // It says nothing of the next: no wait, however many follow.
                Err(TransportError::Stream(error)) => {
                    tracing::warn!(%error, "an accepted stream was dropped: the listener goes on accepting");
                }
                Err(TransportError::Accept(error)) => {
                    self.failed = self.failed.saturating_add(1);
                    let wait = wait_after(self.failed);
                    tracing::warn!(
                        %error,
                        retry_in_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                        "an accept failed: the listener goes on accepting"
                    );
                    self.again_at = (!wait.is_zero()).then(|| self.clock.now() + wait);
                }
                // `EngineGone` is a dialer's, never an accept's.
                Err(error @ (TransportError::ListenerClosed(_) | TransportError::EngineGone)) => {
                    self.ended = true;
                    return Err(NoMoreConnections::ListenerClosed(error));
                }
            }
        }
    }
}

/// How long a Daemon waits before it accepts again once `failed` accepts
/// failed in a row: no time after the first, since one failed attempt says
/// nothing of the next, then [`FIRST_ACCEPT_WAIT`] doubled by each further
/// failure, up to [`LONGEST_ACCEPT_WAIT`].
fn wait_after(failed: u32) -> Duration {
    match failed.checked_sub(2) {
        None => Duration::ZERO,
        Some(doublings) => 1_u32
            .checked_shl(doublings)
            .and_then(|factor| FIRST_ACCEPT_WAIT.checked_mul(factor))
            .map_or(LONGEST_ACCEPT_WAIT, |wait| wait.min(LONGEST_ACCEPT_WAIT)),
    }
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

/// Serves `connection` as the Daemon's Connection `id`, a task of
/// `connections`, telling `ended` once it closes; the token that closes it.
fn serve_connection(
    serving: &Serving,
    shared: &Arc<Shared>,
    connections: &TaskOwner,
    connection: Connection,
    id: ConnectionId,
    ended: mpsc::Sender<ConnectionId>,
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
    let its_owner = connections.child_for(OwnerKind::Connection);
    let cancelled = token.clone();
    connections.spawn(async move {
        let served = async {
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
        // Once the Daemon's loop stops receiving, during a drain or at the
        // Daemon's end, its channel may stay full: the Daemon's end cancels
        // this Connection's token, which ends the wait for room.
        tokio::select! {
            biased;
            _ = ended.send(id) => {}
            () = cancelled.cancelled() => {}
        }
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

/// Closes every Connection of `open` but `asking`, which asked for the
/// Daemon's end, with `daemon.stop` or a refused `hello`: that Client reads
/// the answer and closes, and one that does not is closed after the grace.
async fn close_all_but(
    open: &BTreeMap<ConnectionId, CancellationToken>,
    asking: ConnectionId,
    endings_of_connections: &mut mpsc::Receiver<ConnectionId>,
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

/// Drains the Daemon's Runs through `drain`, waiting at most `grace` for
/// them to stop, after which the Daemon, which was `Draining`, is
/// `Stopping`; answers when that grace ends. Once the grace passed it asks
/// its Runs' registry nothing more, which a panic may have wedged (ADR
/// 0148): it counts the Runs not stopped from the drain's own answer.
async fn drain_runs(
    serving: &Serving,
    state: &watch::Sender<EngineState>,
    grace: Duration,
    drain: impl Future<Output = Draining>,
) -> Instant {
    serving.metrics.draining(true);
    let clock = serving.engine.clock();
    let until = clock.now() + grace;
    let mut grace_ends = std::pin::pin!(clock.sleep_until(until));
    tokio::select! {
        draining = drain => {
            tracing::info!(
                live_runs = draining.count(),
                "the Daemon drains: it takes no run and checkpoints each live run"
            );
            tokio::select! {
                () = draining.ended() => tracing::info!("the Daemon drained every run"),
                () = &mut grace_ends => tracing::warn!(
                    live_runs = draining.not_ended(),
                    "the drain grace passed: each run not yet at a safe point stays resumable from its run file"
                ),
            }
        }
        () = &mut grace_ends => tracing::warn!(
            "the drain grace passed and the Daemon's Run registry did not answer its drain: each live run stays resumable from its run file"
        ),
    }
    let _ = transition(state, EngineEvent::Drained);
    until
}

/// Closes every Connection, then answers `end`.
async fn finish(end: DaemonEnd, connections: &TaskOwner, shared: &Shared) -> DaemonEnd {
    connections.stop().await;
    shared.blocking.stop().await;
    tracing::info!(end = ?end, "the Daemon stops");
    end
}

/// [`finish`], waiting for its Connections and its shared blocking work
/// only until `until` on `clock`, the end of its drain grace: a Connection
/// still closing then, as one whose Runs' stop waits on a wedged registry,
/// keeps the Daemon no longer; it and the blocking work, cancelled, are
/// left to its process's exit.
async fn finish_by(
    end: DaemonEnd,
    connections: &TaskOwner,
    shared: &Shared,
    clock: Clock,
    until: Instant,
) -> DaemonEnd {
    let closed = async {
        connections.stop().await;
        shared.blocking.stop().await;
    };
    tokio::select! {
        () = closed => {}
        () = clock.sleep_until(until) => {
            shared.blocking.cancel();
            tracing::warn!(
                connections = connections.running(),
                blocking = shared.blocking.running(),
                "the drain grace passed before the Daemon's Connections and blocking work stopped: it stops without them"
            );
        }
    }
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
        let holding = presence.borrow_and_update().holding;
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
