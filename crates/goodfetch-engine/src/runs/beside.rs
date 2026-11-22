//! What the Runs' registry does beside its loop (ADR 0145, issue 2258):
//! the loop keeps every decision, and a Run's disk work, such as a start's
//! read of its run file, runs in a task beside it, which reports back by a
//! command. Meanwhile the loop answers the calls on every other Run, and
//! keeps those on that Run, the busy one, in the order they came, so a Run
//! still takes its calls one at a time.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;

use goodfetch_protocol::run::RunId;

/// An answer the loop gives now, or one whose disk work goes beside it.
pub(super) enum Reply<T> {
    /// Answered without waiting on a disk.
    Now(T),
    /// Answered once this work, which owns what it reads, is done.
    Later(Pin<Box<dyn Future<Output = T> + Send>>),
}

impl<T: Send + 'static> Reply<T> {
    /// The answer `work` gives, beside the loop.
    pub(super) fn later(work: impl Future<Output = T> + Send + 'static) -> Self {
        Self::Later(Box::pin(work))
    }

    /// This answer, changed by `change` once given.
    pub(super) fn map<U>(self, change: impl FnOnce(T) -> U + Send + 'static) -> Reply<U> {
        match self {
            Self::Now(answer) => Reply::Now(change(answer)),
            Self::Later(work) => Reply::Later(Box::pin(async move { change(work.await) })),
        }
    }

    /// The answer, waiting on its disk work here: for the calls on every
    /// live Run, such as a drain, whose answers come without it.
    pub(super) async fn resolve(self) -> T {
        match self {
            Self::Now(answer) => answer,
            Self::Later(work) => work.await,
        }
    }
}

/// Which calls a command waits behind.
pub(super) enum Reach {
    /// None: it reads or reports what no call on a Run changes.
    Never,
    /// Every Run's: it waits for the disk work of every Run, as a drain
    /// waits for the starts in flight.
    Every,
    /// None, but it keeps its place behind a command that waits for every
    /// Run, as a start under a Run ID the registry generates does.
    InOrder,
    /// The Run's whose ID it names.
    Run(RunId),
}

/// Where a command comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Order {
    /// From the mailbox, or from the commands that waited for every Run:
    /// after those that wait still.
    Arrived,
    /// From the calls a busy Run kept, which came before any command that
    /// waits for every Run.
    Released,
}

/// The commands that wait, and those the loop answers next.
pub(super) struct Queues<C> {
    /// The busy Runs, each with the commands on it that came meanwhile.
    busy: BTreeMap<RunId, VecDeque<C>>,
    /// The commands that wait for every Run, and those that came after
    /// one of them, in the order they came.
    held: VecDeque<C>,
    /// The commands the loop answers before it reads its mailbox again.
    ready: VecDeque<(C, Order)>,
}

impl<C> Queues<C> {
    /// No Run busy, and no command waiting.
    pub(super) fn new() -> Self {
        Self {
            busy: BTreeMap::new(),
            held: VecDeque::new(),
            ready: VecDeque::new(),
        }
    }

    /// The command the loop answers next, before any in its mailbox.
    pub(super) fn next(&mut self) -> Option<(C, Order)> {
        self.ready.pop_front()
    }

    /// `command`, which reaches `reach`, unless it waits: behind its busy
    /// Run, or behind a command that waits for every Run.
    pub(super) fn unless_waiting(&mut self, command: C, reach: Reach, order: Order) -> Option<C> {
        let behind_held = order == Order::Arrived && !self.held.is_empty();
        match reach {
            Reach::Never => Some(command),
            Reach::Every if behind_held || !self.busy.is_empty() => {
                self.held.push_back(command);
                None
            }
            Reach::InOrder | Reach::Run(_) if behind_held => {
                self.held.push_back(command);
                None
            }
            Reach::Run(run_id) => match self.busy.get_mut(&run_id) {
                Some(kept) => {
                    kept.push_back(command);
                    None
                }
                None => Some(command),
            },
            Reach::Every | Reach::InOrder => Some(command),
        }
    }

    /// Marks the Run `run_id` busy: the calls on it wait until its disk
    /// work reports back.
    pub(super) fn busy(&mut self, run_id: &RunId) {
        self.busy.entry(run_id.clone()).or_default();
    }

    /// Whether the Run `run_id` is busy.
    #[cfg(test)]
    pub(super) fn is_busy(&self, run_id: &RunId) -> bool {
        self.busy.contains_key(run_id)
    }

    /// Takes the busy mark of `run_id` as its disk work ends: the commands
    /// on it that waited go next, in the order they came, and once no Run
    /// is busy, the commands that waited for every Run go after them.
    pub(super) fn done(&mut self, run_id: &RunId) {
        let kept = self.busy.remove(run_id).unwrap_or_default();
        self.ready
            .extend(kept.into_iter().map(|command| (command, Order::Released)));
        if self.busy.is_empty() {
            self.ready
                .extend(self.held.drain(..).map(|command| (command, Order::Arrived)));
        }
    }
}

#[cfg(test)]
mod tests;
