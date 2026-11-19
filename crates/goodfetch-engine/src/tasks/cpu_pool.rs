//! The CPU pool's own threads (ADR 0138, #2200): every Page analysis runs
//! on one of them, never on Tokio's blocking pool, whose cap
//! (`governor.blocking_threads`) then covers only the work that waits. At
//! most one thread a core runs at once; a thread starts when a job finds
//! none idle, and ends once it has waited its keep-alive with nothing to
//! run, so cheap parses keep few threads alive. The `tasks` module runs a
//! second pool of the kind, of threads named apart, for the system
//! resolver's lookups (#2870).
//!
//! A job queues under one lock, as Tokio's blocking pool does: a submitter
//! wakes an idle thread it counts as notified, or starts one while fewer
//! than the most run; a thread that wakes takes its notification, or ends
//! after its keep-alive. Both decide under the lock, so a job is never left
//! behind by a thread ending as it arrives (the loom tests below).

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use crate::sync::{Arc, Condvar, Mutex, MutexGuard};

/// A job of the pool: it answers its caller itself.
pub(super) type Job = Box<dyn FnOnce() + Send + 'static>;

/// How long a thread with nothing to run waits for a job before it ends:
/// Tokio's own for its blocking threads.
pub(super) const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// The name the pool's threads carry, which `/proc/<pid>/task/*/comm`
/// shows (15 bytes at most on Linux).
pub(super) const THREAD_NAME: &str = "gf-analysis";

/// The jobs a pool with no thread of its own gives back, for its caller
/// to run elsewhere.
pub(super) struct Stranded(pub(super) Vec<Job>);

impl std::fmt::Debug for Stranded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("Stranded")
            .field(&self.0.len())
            .finish()
    }
}

/// The pool, shared by every owner of a tree; dropping it ends its idle
/// threads, and the others once the queue is empty.
#[derive(Debug)]
pub(super) struct CpuPool {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// The most threads that run at once: the machine's cores.
    most: usize,
    keep_alive: Duration,
    name: &'static str,
}

struct State {
    queue: VecDeque<Job>,
    /// Threads waiting for a job, notified ones aside.
    idle: usize,
    /// Wake-ups sent that no thread has taken yet.
    notified: usize,
    /// Threads alive, started or starting.
    threads: usize,
    /// Set once the pool is dropped.
    closed: bool,
}

impl std::fmt::Debug for State {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("State")
            .field("queued", &self.queue.len())
            .field("idle", &self.idle)
            .field("notified", &self.notified)
            .field("threads", &self.threads)
            .field("closed", &self.closed)
            .finish()
    }
}

impl CpuPool {
    /// A pool of at most `most` threads named `name`, none started yet,
    /// each ending after `keep_alive` with nothing to run.
    pub(super) fn new(name: &'static str, most: usize, keep_alive: Duration) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    idle: 0,
                    notified: 0,
                    threads: 0,
                    closed: false,
                }),
                wake: Condvar::new(),
                most: most.max(1),
                keep_alive,
                name,
            }),
        }
    }

    /// Queues `job` for the next thread free: an idle one, else a new one
    /// while fewer than the most run, else the first to finish its job.
    ///
    /// # Errors
    ///
    /// When the OS refuses a thread and none of the pool's is alive to run
    /// what is queued: every queued job, `job` among them, given back.
    pub(super) fn run(&self, job: Job) -> Result<(), Stranded> {
        let mut state = self.shared.lock();
        state.queue.push_back(job);
        if state.idle > 0 {
            state.idle -= 1;
            state.notified += 1;
            self.shared.wake.notify_one();
            return Ok(());
        }
        if state.threads >= self.shared.most {
            return Ok(());
        }
        state.threads += 1;
        drop(state);
        let Err(error) = start(self.shared.name, Arc::clone(&self.shared)) else {
            return Ok(());
        };
        let mut state = self.shared.lock();
        state.threads -= 1;
        if state.threads > 0 {
            return Ok(());
        }
        tracing::warn!(
            %error,
            pool = self.shared.name,
            "the OS refused the pool a thread, and none of its own runs"
        );
        Err(Stranded(state.queue.drain(..).collect()))
    }
}

impl Drop for CpuPool {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.wake.notify_all();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("invariant: no job runs under the CPU pool's lock, so none poisons it")
    }

    /// A thread's loop: it runs what is queued, then waits for a job up to
    /// its keep-alive, and ends with nothing to run or once the pool is
    /// closed and its queue empty.
    fn work(&self) {
        let mut state = self.lock();
        loop {
            if let Some(job) = state.queue.pop_front() {
                drop(state);
                job();
                state = self.lock();
                continue;
            }
            if state.closed || self.keep_alive.is_zero() {
                break;
            }
            state.idle += 1;
            let (woken, waited) = self
                .wake
                .wait_timeout(state, self.keep_alive)
                .expect("invariant: no job runs under the CPU pool's lock, so none poisons it");
            state = woken;
            if state.notified > 0 {
                // A submitter counted this thread out of the idle ones.
                state.notified -= 1;
                continue;
            }
            state.idle -= 1;
            if waited.timed_out() && state.queue.is_empty() {
                break;
            }
        }
        state.threads -= 1;
    }
}

/// Starts a thread of the pool named `name`.
#[cfg(not(all(test, goodfetch_loom)))]
fn start(name: &'static str, shared: Arc<Shared>) -> io::Result<()> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the tasks module is the one place that starts threads (ADR 0060)"
    )]
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || shared.work())
        .map(drop)
}

/// Starts a thread of the pool on loom's scheduler.
#[cfg(all(test, goodfetch_loom))]
fn start(name: &'static str, shared: Arc<Shared>) -> io::Result<()> {
    loom::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || shared.work())
        .map(drop)
}

#[cfg(all(test, goodfetch_loom))]
mod loom_tests;

#[cfg(test)]
mod tests;
