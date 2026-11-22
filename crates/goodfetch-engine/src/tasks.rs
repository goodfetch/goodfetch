//! The ownership tree (ADR 0038): every task the engine starts belongs to an
//! owner (the engine process, a Connection, a call, a run, later a Session
//! and a page), which holds a cancellation token, a child of its parent's,
//! and a tracker of its tasks. Stopping an owner cancels first, then waits
//! for its tasks, then ends its actors once they handled what their
//! mailboxes hold, whoever still holds a sender. This is the only module
//! that starts tasks and threads (ADR 0060); the engine builds no runtime,
//! `main` does.
//!
//! No panic of a task goes unseen (ADR 0148): a task, an actor and blocking
//! work whose caller has gone each run inside a watch that catches a panic
//! where the task already runs, logs it at `error` with the owner's
//! [`OwnerKind`] and the name of its crash report, and tells the owner's
//! parent through [`TaskOwner::panicked`]. The parent carries out the
//! reaction, ending the owner through an end that already exists, never the
//! owner that failed: stopping an owner waits for its tasks, so none of
//! them can stop it. The task that serves a Connection ends it as when its
//! Client goes away. An owner of the engine's own kind under another tells
//! that one too, up to the engine's root, whose holder stops the engine: a
//! task that belongs to no Run and no Connection is a part every Run
//! shares. Until a Run's parent awaits it (spec #2425), the log is a Run's
//! panic's only trace. Nothing is restarted in place.
//!
//! Blocking work runs under the owner's [`BlockingPolicy`], a value: in
//! production on tracked blocking threads, and an actor that owns blocking
//! state (the run's SQLite writer) on a dedicated thread its owner joins; in
//! the Simulation and in-process tests, blocking calls run inline and an
//! actor's loop runs as a task on the host's runtime. A Page analysis runs
//! in production on a thread of the CPU pool's own (ADR 0138), at most one
//! a core, and on the owner's blocking threads, whose count the runtime
//! `main` builds caps at `governor.blocking_threads`, only when the pool
//! cannot start a thread, so that it runs at all. The system resolver's
//! lookups run on a pool of that kind of their own, never on a blocking
//! thread either (#2870).

use std::any::Any;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs as _};
use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::FutureExt as _;
use goodfetch_counted_work::{CountedWork, Row};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::crash_report;

mod cpu_pool;
mod state_actor;

pub(crate) use state_actor::{ActorStopped, StateActor};

/// How long a stop waits for its cancelled tasks before it logs that some
/// still run (issue 2525).
const STOP_TOLD_AFTER: Duration = Duration::from_secs(10);

/// Where an owner runs blocking work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockingPolicy {
    /// Production: tracked blocking threads, and a dedicated thread per
    /// actor, joined when its owner stops.
    Threads,
    /// The Simulation and in-process tests: blocking work runs inline, and
    /// an actor's loop runs as a task, so a run replays from its seed. In
    /// production, only work on a thread that runs no runtime's tasks, such
    /// as the OTLP exporter's own (ADR 0060's note of 2026-11-16).
    Inline,
}

/// The part a panic's log names for a task of the engine's root owner, or
/// of an owner under it that names no part of its own.
const ENGINES_OWN: &str = "engine";

/// What an owner is, which the log of a panic in one of its tasks names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    /// The engine process, and every owner under it that is no Connection
    /// and no Run.
    Engine,
    /// A Connection, whose own tasks are its writer and its log forwarder,
    /// and its calls.
    Connection,
    /// A Run, its writer, and the work the Run registry does for one Run,
    /// such as publishing its progress or recording its end.
    Run,
}

impl OwnerKind {
    /// Its name in a log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Connection => "connection",
            Self::Run => "run",
        }
    }
}

/// An owner of tasks: what it starts stops with it.
#[derive(Debug)]
pub struct TaskOwner {
    token: CancellationToken,
    tracker: TaskTracker,
    /// Its actors, which outlive its other tasks: a stop ends them only
    /// once those have ended.
    actors: TaskTracker,
    /// Cancelled once a stop's other tasks have ended: each actor then
    /// closes its mailbox, handles what it still holds and ends.
    closing: CancellationToken,
    policy: BlockingPolicy,
    /// The CPU pool, one thread a core at most, which every owner of the
    /// tree shares: each Page analysis runs on one of its threads.
    analyses: Arc<cpu_pool::CpuPool>,
    /// Where a panic of its tasks is told.
    panics: Panics,
}

impl TaskOwner {
    /// The root of a tree, such as the engine process's, whose blocking
    /// work runs under `policy`, with a CPU pool sized to the machine's
    /// cores (one when the OS cannot tell).
    pub fn root(policy: BlockingPolicy) -> Self {
        let cores = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        Self::root_on_cores(policy, cores)
    }

    /// The root of a tree whose blocking work runs under `policy`, with a
    /// CPU pool of at most `cores` threads.
    pub fn root_on_cores(policy: BlockingPolicy, cores: NonZeroUsize) -> Self {
        Self {
            token: CancellationToken::new(),
            tracker: TaskTracker::new(),
            actors: TaskTracker::new(),
            closing: CancellationToken::new(),
            policy,
            analyses: Arc::new(cpu_pool::CpuPool::new(
                cpu_pool::THREAD_NAME,
                cores.get(),
                cpu_pool::KEEP_ALIVE,
            )),
            panics: Panics::new(OwnerKind::Engine),
        }
    }

    /// An owner under this one, of its kind, with its blocking policy:
    /// cancelling this one cancels it, never the other way round. Its tasks
    /// are its own, which its holder waits for, and whose panics are told
    /// to its holder ([`TaskOwner::panicked`]), and to this one's holder
    /// too when both are the engine's own ([`OwnerKind::Engine`]).
    #[must_use]
    pub fn child(&self) -> Self {
        self.under(self.panics.kind(), self.panics.part())
    }

    /// An owner under this one, as [`TaskOwner::child`], of `kind`, such
    /// as a Connection's or a Run's.
    #[must_use]
    pub fn child_for(&self, kind: OwnerKind) -> Self {
        self.under(kind, self.panics.part())
    }

    /// An owner under this one, as [`TaskOwner::child`], for `part`, a part
    /// every Run shares, such as the Run registry, which the log of a panic
    /// of its tasks or of those of its children names (ADR 0148).
    #[must_use]
    pub fn part(&self, part: &'static str) -> Self {
        self.under(self.panics.kind(), part)
    }

    fn under(&self, kind: OwnerKind, part: &'static str) -> Self {
        Self {
            token: self.token.child_token(),
            tracker: TaskTracker::new(),
            actors: TaskTracker::new(),
            closing: CancellationToken::new(),
            policy: self.policy,
            analyses: Arc::clone(&self.analyses),
            panics: self.panics.child(kind, part),
        }
    }

    /// Its cancellation token, which its tasks watch at each cancellation
    /// point.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Waits until one of its tasks, its actors or its blocking work whose
    /// caller has gone panicked, which its parent, its holder, awaits to end
    /// it: the panic was logged already, with the name of its crash report.
    ///
    /// The future, which holds no borrow of the owner, so a task can await
    /// it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting; a later wait still sees a panic
    /// told before it.
    pub fn panicked(&self) -> impl Future<Output = ()> + Send + 'static {
        self.panics.panicked().clone().cancelled_owned()
    }

    /// Whether one of its tasks, its actors or its blocking work whose
    /// caller has gone panicked already: what [`TaskOwner::panicked`] waits
    /// for, read where its holder no longer waits. A task's panic is told
    /// before the task ends, so once its tasks ended ([`TaskOwner::wait`])
    /// this reads every panic among them.
    pub fn has_panicked(&self) -> bool {
        self.panics.panicked().is_cancelled()
    }

    /// The part whose task panicked first among its own and its children's
    /// that tell it ([`TaskOwner::part`]): the name the engine's log gives
    /// when that panic stops it; none while none has panicked.
    pub fn panicked_part(&self) -> Option<&'static str> {
        self.panics.0.first.get().copied()
    }

    /// Its blocking policy.
    pub fn policy(&self) -> BlockingPolicy {
        self.policy
    }

    /// Starts `future` as one of its tasks, whose panic is told to its
    /// parent ([`TaskOwner::panicked`]).
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let panics = self.panics.clone();
        self.tracker.spawn(watched(future, panics));
    }

    /// Runs `work`, which blocks, under its blocking policy, and answers
    /// what it returns; a panic in `work` resumes in the caller.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops `work` once it started: on a
    /// blocking thread it runs to its end, and the owner waits for it; a
    /// panic in it then is told to the owner's parent
    /// ([`TaskOwner::panicked`]).
    pub async fn blocking<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        run_blocking(&self.tracker, self.policy, &self.panics, work).await
    }

    /// Its blocking threads, as a handle that runs blocking work as
    /// [`TaskOwner::blocking`] does for whoever holds it, such as a run
    /// file's readers, which its writer's handles hold; the owner waits for
    /// that work too.
    pub fn blocking_threads(&self) -> BlockingThreads {
        BlockingThreads {
            tracker: self.tracker.clone(),
            policy: self.policy,
            panics: self.panics.clone(),
        }
    }

    /// Runs `work`, a Page analysis's parsing or reading of a page, on the
    /// CPU pool, and answers what it returns: on a thread of the pool's
    /// own, at most one a core, started on first use and ended once idle,
    /// so no more analyses run at once than the machine has cores, whatever
    /// the requests in flight, and none waits behind the cap of Tokio's
    /// blocking threads (ADR 0138, #2200); inline under
    /// [`BlockingPolicy::Inline`]. A panic in `work` resumes in the caller.
    ///
    /// Each call is one hand-off to the pool, counted on `counted` as
    /// `F1.analysis_handoffs` under either policy: an owner holds no
    /// Counted work, so its caller gives it its Run's.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before a thread of the pool takes `work` runs
    /// nothing; once `work` started, as [`TaskOwner::blocking`]: it runs to
    /// its end, the owner waits for it, and a panic in it is told to the
    /// owner's parent.
    pub async fn analysis<T, F>(&self, counted: &CountedWork, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        counted.count(Row::PageAnalysisHandoffs);
        self.on_pool(work).await
    }

    /// [`TaskOwner::analysis`] of a handler's read of its page, counted
    /// apart from the page's own reads, on `F1.analysis_handoffs.handler`
    /// (#2189): a page makes at most one hand-off of its own, and its
    /// handler one for each read it makes.
    ///
    /// # Cancel safety
    ///
    /// As [`TaskOwner::analysis`].
    pub async fn handler_analysis<T, F>(&self, counted: &CountedWork, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        counted.count(Row::HandlerAnalysisHandoffs);
        self.on_pool(work).await
    }

    /// Runs `work` on a thread of the CPU pool, or inline under
    /// [`BlockingPolicy::Inline`]; a panic in `work` resumes in the caller.
    async fn on_pool<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        match self.policy {
            BlockingPolicy::Inline => work(),
            BlockingPolicy::Threads => {
                let (answer, answered) = oneshot::channel();
                let panics = self.panics.clone();
                // The owner waits for the job as for its tasks.
                let running = self.tracker.token();
                let job: cpu_pool::Job = Box::new(move || {
                    let _running = running;
                    if answer.is_closed() {
                        return;
                    }
                    let caught =
                        std::panic::catch_unwind(AssertUnwindSafe(work)).map_err(|panic| Caught {
                            panic: Some(panic),
                            crash_report: crash_report::take_written(),
                            panics,
                        });
                    // A caller that has gone drops the answer here, and a
                    // `Caught` dropped unresumed tells its panic.
                    let _ = answer.send(caught);
                });
                if let Err(cpu_pool::Stranded(stranded)) = self.analyses.run(job) {
                    // No thread of the pool's own: the analyses run on the
                    // owner's blocking threads rather than never.
                    for job in stranded {
                        self.tracker.spawn_blocking(job);
                    }
                }
                match answered
                    .await
                    .expect("invariant: the CPU pool runs every job it queued")
                {
                    Ok(answer) => answer,
                    Err(caught) => caught.resume(),
                }
            }
        }
    }

    /// Starts an actor named `name` that owns blocking state: `handle` gets
    /// every message `mailbox` holds at once, in order, so it can group
    /// them (the run's SQLite writer commits a group in one transaction).
    /// The actor ends once every sender of `mailbox` is gone, or once a
    /// stop of the owner saw its other tasks end: it closes its mailbox
    /// then, refusing any later message, whoever holds a sender, and
    /// handles what the mailbox still holds first. The owner waits for it.
    /// A panic in `handle` ends the actor, its mailbox dropped, and is told
    /// to the owner's parent ([`TaskOwner::panicked`]).
    ///
    /// # Errors
    ///
    /// When the OS refuses the dedicated thread.
    pub fn actor<M, H>(
        &self,
        name: &str,
        mailbox: mpsc::Receiver<M>,
        mut handle: H,
    ) -> io::Result<()>
    where
        M: Send + 'static,
        H: FnMut(Vec<M>) + Send + 'static,
    {
        self.actor_until(name, mailbox, move |group| {
            handle(group);
            ControlFlow::Continue(())
        })
    }

    /// Starts an actor as [`TaskOwner::actor`] does, which also ends once
    /// `handle` breaks: its mailbox is dropped then, with every message it
    /// still holds, so a sender's later message is refused, whoever holds a
    /// sender still.
    ///
    /// # Errors
    ///
    /// When the OS refuses the dedicated thread.
    pub fn actor_until<M, H>(
        &self,
        name: &str,
        mut mailbox: mpsc::Receiver<M>,
        mut handle: H,
    ) -> io::Result<()>
    where
        M: Send + 'static,
        H: FnMut(Vec<M>) -> ControlFlow<()> + Send + 'static,
    {
        let (closing, panics) = (self.closing.clone(), self.panics.clone());
        match self.policy {
            BlockingPolicy::Inline => {
                let handling = async move {
                    while let Some(first) = next(&mut mailbox, &closing).await {
                        if handle(drain(first, &mut mailbox)).is_break() {
                            break;
                        }
                    }
                };
                self.actors.spawn(watched(handling, panics));
            }
            BlockingPolicy::Threads => {
                // The thread waits for its next message on the runtime's
                // handle, outside any of its tasks, beside the stop's
                // closing.
                let runtime = tokio::runtime::Handle::current();
                let (finished, ended) = oneshot::channel::<()>();
                let joining = panics.clone();
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the tasks module is the one place that starts threads (ADR 0060)"
                )]
                let thread =
                    std::thread::Builder::new()
                        .name(name.to_owned())
                        .spawn(move || {
                            // The actor's watch, on its own thread.
                            let handled = std::panic::catch_unwind(AssertUnwindSafe(|| {
                                while let Some(first) =
                                    runtime.block_on(next(&mut mailbox, &closing))
                                {
                                    if handle(drain(first, &mut mailbox)).is_break() {
                                        break;
                                    }
                                }
                            }));
                            if handled.is_err() {
                                panics.tell(crash_report::take_written().as_deref());
                            }
                            drop(mailbox);
                            let _ = finished.send(());
                        })?;
                self.actors.spawn(async move {
                    let _ = ended.await;
                    // The thread has returned, its own panic caught and
                    // told: joining takes no time.
                    if thread.join().is_err() {
                        joining.tell(None);
                    }
                });
            }
        }
        Ok(())
    }

    /// Asks its tasks to stop at their next cancellation point.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// How many of the tasks it started still run, its actors aside.
    pub fn running(&self) -> usize {
        self.tracker.len()
    }

    /// Waits for every task it started, without cancelling them, its
    /// actors last, each ending once every sender of its mailbox is gone.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops waiting; the tasks keep running and no new
    /// one can be started.
    pub async fn wait(&self) {
        self.tracker.close();
        self.tracker.wait().await;
        self.actors.close();
        self.actors.wait().await;
    }

    /// Cancels its tasks, waits for them, then ends its actors once each
    /// handled what its mailbox holds, never waiting for a sender held
    /// anywhere else to go (#1763). Tasks still running
    /// [`STOP_TOLD_AFTER`] after the cancellation are logged once at
    /// `warn`, with how many, and still waited for.
    ///
    /// # Cancel safety
    ///
    /// As [`TaskOwner::wait`]: the tasks are already cancelled.
    pub async fn stop(&self) {
        self.cancel();
        self.tracker.close();
        if tokio::time::timeout(STOP_TOLD_AFTER, self.tracker.wait())
            .await
            .is_err()
        {
            tracing::warn!(
                owner = self.panics.kind().as_str(),
                running = self.tracker.len(),
                "a stop's tasks still run after their cancellation, and it waits for them"
            );
            self.tracker.wait().await;
        }
        self.closing.cancel();
        self.actors.close();
        self.actors.wait().await;
    }
}

/// An owner's blocking threads, under its policy, for whoever holds the
/// handle ([`TaskOwner::blocking_threads`]).
#[derive(Clone, Debug)]
pub struct BlockingThreads {
    tracker: TaskTracker,
    policy: BlockingPolicy,
    panics: Panics,
}

impl BlockingThreads {
    /// Runs `work`, which blocks, as [`TaskOwner::blocking`] does.
    ///
    /// # Cancel safety
    ///
    /// As [`TaskOwner::blocking`]'s.
    pub async fn run<T, F>(&self, work: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        run_blocking(&self.tracker, self.policy, &self.panics, work).await
    }
}

/// The name the lookup pool's threads carry, which `/proc/<pid>/task/*/comm`
/// shows apart from Tokio's workers and blocking threads and from the CPU
/// pool's (#2870).
pub(crate) const LOOKUP_THREAD: &str = "gf-lookup";

/// The lookup pool: the system resolver's lookups run on its threads, at
/// most one a core, never on Tokio's blocking pool, so none waits behind
/// the work `governor.blocking_threads` caps (#2870). The process's
/// lookups share it; a thread ends once idle for its keep-alive.
static LOOKUPS: OnceLock<cpu_pool::CpuPool> = OnceLock::new();

/// The addresses the operating system's resolver (`getaddrinfo`) gives
/// `host` on `port`, looked up on a thread of the lookup pool
/// ([`LOOKUP_THREAD`]).
///
/// # Errors
///
/// The resolver's, and one of kind `Other` when the pool has no thread to
/// run the lookup on.
///
/// # Cancel safety
///
/// Dropping the future before a thread of the pool takes the lookup runs
/// nothing; once it started, it runs to its end, since `getaddrinfo`
/// cannot be cancelled, and its answer is dropped.
pub(crate) async fn system_lookup(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    // An address is its own answer, as Tokio's `lookup_host` has it.
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(address, port)]);
    }
    let host = host.to_owned();
    on_lookup_thread(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(Iterator::collect)
    })
    .await
    .unwrap_or_else(|| {
        Err(io::Error::other(
            "the lookup pool has no thread to run a lookup on",
        ))
    })
}

/// Runs `work` on a thread of the lookup pool, and answers what it
/// returns, or none when the pool has no thread to run it on.
async fn on_lookup_thread<T, F>(work: F) -> Option<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let pool = LOOKUPS.get_or_init(|| {
        let cores = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        cpu_pool::CpuPool::new(LOOKUP_THREAD, cores.get(), cpu_pool::KEEP_ALIVE)
    });
    let (answer, answered) = oneshot::channel();
    let job: cpu_pool::Job = Box::new(move || {
        if !answer.is_closed() {
            let _ = answer.send(work());
        }
    });
    // A pool with no thread drops the jobs it gives back, whose callers
    // then find their answer gone.
    let _ = pool.run(job);
    answered.await.ok()
}

/// Where an owner's panics are told: logged at `error` with its kind, then
/// told to whoever awaits [`TaskOwner::panicked`], its parent.
///
/// One pointer, which every task's watch holds: a call's task (ADR 0140's
/// E1 starts one a call) grows by no more, and starting one copies no token.
#[derive(Clone, Debug)]
struct Panics(Arc<Telling>);

/// What an owner's tasks tell, shared by their watches.
#[derive(Debug)]
struct Telling {
    kind: OwnerKind,
    /// The part every Run shares it is, or is under, which the log of a
    /// panic names: its parent's unless it was named.
    part: &'static str,
    /// Cancelled once one of the owner's tasks panicked.
    panicked: CancellationToken,
    /// The part whose task panicked first among its own and those of the
    /// owners that tell it.
    first: OnceLock<&'static str>,
    /// Its parent's, told too when both owners are the engine's own: a
    /// task under the engine's owner that belongs to no Run and no
    /// Connection is a part every Run shares (ADR 0148).
    parent: Option<Panics>,
}

impl Panics {
    fn new(kind: OwnerKind) -> Self {
        Self(Arc::new(Telling {
            kind,
            part: ENGINES_OWN,
            panicked: CancellationToken::new(),
            first: OnceLock::new(),
            parent: None,
        }))
    }

    /// The tell of a child owner of `kind`, of the part `part`, which tells
    /// this one too when both are the engine's own.
    fn child(&self, kind: OwnerKind, part: &'static str) -> Self {
        let engines_own = kind == OwnerKind::Engine && self.kind() == OwnerKind::Engine;
        Self(Arc::new(Telling {
            kind,
            part,
            panicked: CancellationToken::new(),
            first: OnceLock::new(),
            parent: engines_own.then(|| self.clone()),
        }))
    }

    fn kind(&self) -> OwnerKind {
        self.0.kind
    }

    fn part(&self) -> &'static str {
        self.0.part
    }

    fn panicked(&self) -> &CancellationToken {
        &self.0.panicked
    }

    /// Tells a panic of one of the owner's tasks or actors, whose crash
    /// report is `crash_report`, when one was written, to the owner and to
    /// every owner of the engine's kind it tells.
    fn tell(&self, crash_report: Option<&str>) {
        self.log(crash_report);
        let mut told = Some(self);
        while let Some(panics) = told {
            panics.told(self.part());
            told = panics.0.parent.as_ref();
        }
    }

    /// Tells a panic of blocking work whose caller has gone, whose crash
    /// report is `crash_report`, to the owner alone: the work was its
    /// caller's, such as a call's read on the engine's stores, not a part
    /// every Run shares (ADR 0148).
    fn tell_alone(&self, crash_report: Option<&str>) {
        self.log(crash_report);
        self.told(self.part());
    }

    fn log(&self, crash_report: Option<&str>) {
        // A Run's or a Connection's task is no part every Run shares, so
        // only the engine's own name one.
        let part = (self.kind() == OwnerKind::Engine).then(|| self.part());
        tracing::error!(
            owner = self.kind().as_str(),
            part,
            crash_report,
            "a task panicked, and its owner's parent is told"
        );
    }

    fn told(&self, part: &'static str) {
        let _ = self.0.first.set(part);
        self.panicked().cancel();
    }
}

/// `future`, whose panic is caught where it runs and told to `panics`,
/// on the thread it panicked on, so its crash report's name goes with it.
/// A combinator, not an `async fn`, which would hold `future` twice.
fn watched<F>(future: F, panics: Panics) -> impl Future<Output = ()>
where
    F: Future<Output = ()>,
{
    AssertUnwindSafe(future).catch_unwind().map(move |caught| {
        if caught.is_err() {
            panics.tell(crash_report::take_written().as_deref());
        }
    })
}

/// A panic that blocking work caught on its thread, with the name of its
/// crash report, on its way to the caller: dropped there unresumed, the
/// caller having gone, it is told to `panics` alone.
struct Caught {
    panic: Option<Box<dyn Any + Send>>,
    crash_report: Option<String>,
    panics: Panics,
}

impl Caught {
    /// Resumes the panic in the caller, with its crash report's name.
    fn resume(mut self) -> ! {
        let panic = self
            .panic
            .take()
            .expect("invariant: a caught panic is resumed once");
        crash_report::resume_unwind(panic, self.crash_report.take())
    }
}

impl Drop for Caught {
    fn drop(&mut self) {
        if self.panic.take().is_some() {
            self.panics.tell_alone(self.crash_report.as_deref());
        }
    }
}

/// Runs `work` under `policy`, on a blocking thread `tracker` tracks, and
/// answers what it returns; a panic in `work` resumes in the caller, with
/// the name of its crash report, or, once the caller has gone, which only
/// a blocking thread allows, is told to `panics` (ADR 0148).
async fn run_blocking<T, F>(
    tracker: &TaskTracker,
    policy: BlockingPolicy,
    panics: &Panics,
    work: F,
) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match policy {
        BlockingPolicy::Inline => work(),
        BlockingPolicy::Threads => {
            let panics = panics.clone();
            // A caller that has gone drops its handle: the runtime then
            // drops what the work returned on its thread, and a `Caught`
            // dropped unresumed tells its panic.
            let caught = tracker.spawn_blocking(move || {
                std::panic::catch_unwind(AssertUnwindSafe(work)).map_err(|panic| Caught {
                    panic: Some(panic),
                    crash_report: crash_report::take_written(),
                    panics,
                })
            });
            match caught.await {
                Ok(Ok(answer)) => answer,
                Ok(Err(caught)) => caught.resume(),
                Err(error) => panic!("invariant: a blocking task is never cancelled: {error}"),
            }
        }
    }
}

/// The next message of `mailbox`; once `closing` is cancelled, the mailbox
/// is closed first, so it answers what it holds, then none.
async fn next<M>(mailbox: &mut mpsc::Receiver<M>, closing: &CancellationToken) -> Option<M> {
    tokio::select! {
        biased;
        message = mailbox.recv() => message,
        () = closing.cancelled() => {
            mailbox.close();
            mailbox.recv().await
        }
    }
}

/// `first`, then every message `mailbox` holds now.
fn drain<M>(first: M, mailbox: &mut mpsc::Receiver<M>) -> Vec<M> {
    let mut batch = vec![first];
    while let Ok(next) = mailbox.try_recv() {
        batch.push(next);
    }
    batch
}

#[cfg(test)]
mod tests;
