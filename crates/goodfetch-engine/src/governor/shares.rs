//! Fair shares of the governor's targets (ADR 0040): each live Run joins
//! with its share, and takes admissions of each kind, given back when they
//! drop. Each kind is admitted on its own, up to its target. While other
//! Runs wait, capacity goes round-robin over the waiting Runs, never back
//! to the one that just gave it, so the first Run to grow never starves the
//! rest; within a Run, its gate in admission orders its domains (ADR 0004).
//! A Run holds at most its share of a kind's target, at least one.
#![expect(
    clippy::disallowed_types,
    reason = "a Permit gives its admission back as it drops, which cannot wait: one Mutex guards the counts for a few comparisons, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_protocol::governor::{GovernorLevel, WorkKind};
use tokio::sync::{Notify, watch};

use super::browsers::{Browsers, GovernedBrowser};
use super::ladder::Weight;

/// Every live Run's share of the governor's targets, and the browsers
/// their pools launched.
#[derive(Clone, Debug)]
pub(crate) struct Shares {
    state: Arc<Mutex<State>>,
    browsers: Browsers,
    /// The level of each sample, sent at every sample.
    level: Arc<watch::Sender<GovernorLevel>>,
}

#[derive(Debug)]
struct State {
    kinds: [Kind; 3],
    runs: BTreeMap<u64, Member>,
    next_run: u64,
}

/// One kind's admissions.
#[derive(Debug, Default)]
struct Kind {
    target: u32,
    /// Admissions held.
    in_use: u32,
    /// Admissions set aside for a waiting Run, which its next try takes.
    reserved: u32,
    /// The Runs that asked and were refused, by when they joined.
    waiting: BTreeSet<u64>,
    /// The Run last given an admission, after which the turn goes on.
    last: Option<u64>,
}

#[derive(Debug)]
struct Member {
    /// Its share, in percent of each kind's target.
    share: u64,
    in_use: [u32; 3],
    reserved: [u32; 3],
    /// Woken when an admission of each kind is set aside for it.
    turns: [Arc<Notify>; 3],
}

impl Member {
    /// The most of `kind` it may hold under `target`: its share, at least
    /// one.
    fn cap(&self, target: u32) -> u32 {
        let share = u64::from(target) * self.share.min(100) / 100;
        u32::try_from(share).unwrap_or(u32::MAX).max(1)
    }

    fn below_cap(&self, index: usize, target: u32) -> bool {
        self.in_use[index] + self.reserved[index] < self.cap(target)
    }
}

/// A Run's share: what its driver takes admissions through. Dropping it
/// gives back every admission set aside for it.
#[derive(Debug)]
pub struct RunShare {
    shares: Shares,
    run: u64,
}

/// One admission of a kind, given back when it drops.
#[derive(Debug)]
pub struct Permit {
    shares: Shares,
    run: u64,
    kind: WorkKind,
}

fn index(kind: WorkKind) -> usize {
    match kind {
        WorkKind::Http => 0,
        WorkKind::BrowserPage => 1,
        WorkKind::ModelCall => 2,
    }
}

impl Shares {
    /// Shares of `targets`, in [`WorkKind::ALL`]'s order, with no Run yet.
    pub(crate) fn new(targets: [u32; 3]) -> Self {
        let kinds = targets.map(|target| Kind {
            target,
            ..Kind::default()
        });
        Self {
            state: Arc::new(Mutex::new(State {
                kinds,
                runs: BTreeMap::new(),
                next_run: 0,
            })),
            browsers: Browsers::default(),
            level: Arc::new(watch::Sender::new(GovernorLevel::Normal)),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A Run joins with `share`, in percent of each kind's target.
    pub(crate) fn join(&self, share: u64) -> RunShare {
        let mut state = self.state();
        let run = state.next_run;
        state.next_run += 1;
        state.runs.insert(
            run,
            Member {
                share,
                in_use: [0; 3],
                reserved: [0; 3],
                turns: [(); 3].map(|()| Arc::new(Notify::new())),
            },
        );
        RunShare {
            shares: self.clone(),
            run,
        }
    }

    /// Sets each kind's target, and gives the room it made to the Runs
    /// that wait.
    pub(crate) fn set_targets(&self, targets: [u32; 3]) {
        let mut state = self.state();
        for (index, target) in targets.into_iter().enumerate() {
            state.kinds[index].target = target;
            state.distribute(index);
        }
    }

    /// Each kind's admissions held now.
    pub(crate) fn in_use(&self) -> [u32; 3] {
        self.state().kinds.each_ref().map(|kind| kind.in_use)
    }

    /// The browsers the Runs' pools launched.
    pub(crate) fn browsers(&self) -> &Browsers {
        &self.browsers
    }

    /// A sample found the governor at `level`.
    pub(crate) fn sampled(&self, level: GovernorLevel) {
        self.level.send_replace(level);
    }
}

impl State {
    /// Sets room of kind `index` aside for the waiting Runs, in turn after
    /// the one last given an admission, waking each.
    fn distribute(&mut self, index: usize) {
        loop {
            let kind = &self.kinds[index];
            if kind.in_use + kind.reserved >= kind.target {
                return;
            }
            let eligible = |run: &u64| {
                self.runs
                    .get(run)
                    .is_some_and(|member| member.below_cap(index, kind.target))
            };
            let after = kind.last.map_or(0, |last| last + 1);
            let Some(next) = kind
                .waiting
                .range(after..)
                .find(|run| eligible(run))
                .or_else(|| kind.waiting.range(..after).find(|run| eligible(run)))
                .copied()
            else {
                return;
            };
            let kind = &mut self.kinds[index];
            kind.waiting.remove(&next);
            kind.reserved += 1;
            kind.last = Some(next);
            if let Some(member) = self.runs.get_mut(&next) {
                member.reserved[index] += 1;
                member.turns[index].notify_one();
            }
        }
    }

    /// An admission of kind `index` for `run`, when its turn allows.
    fn admit(&mut self, run: u64, index: usize) -> bool {
        let Some(member) = self.runs.get_mut(&run) else {
            return false;
        };
        let kind = &mut self.kinds[index];
        if member.reserved[index] > 0 {
            member.reserved[index] -= 1;
            member.in_use[index] += 1;
            kind.reserved -= 1;
            kind.in_use += 1;
            return true;
        }
        let room = kind.in_use + kind.reserved < kind.target;
        let below_cap = member.below_cap(index, kind.target);
        let others_wait = kind.waiting.iter().any(|waiting| {
            *waiting != run
                && self
                    .runs
                    .get(waiting)
                    .is_some_and(|other| other.below_cap(index, kind.target))
        });
        if room && below_cap && !others_wait {
            let member = self
                .runs
                .get_mut(&run)
                .expect("invariant: the Run was found above");
            member.in_use[index] += 1;
            kind.in_use += 1;
            kind.waiting.remove(&run);
            kind.last = Some(run);
            return true;
        }
        kind.waiting.insert(run);
        false
    }

    /// `run` gave an admission of kind `index` back.
    fn give_back(&mut self, run: u64, index: usize) {
        self.kinds[index].in_use -= 1;
        if let Some(member) = self.runs.get_mut(&run) {
            member.in_use[index] -= 1;
        }
        self.distribute(index);
    }

    /// `run` left: what was set aside for it goes to the others; each
    /// admission it still holds is given back as it drops.
    fn leave(&mut self, run: u64) {
        let Some(member) = self.runs.remove(&run) else {
            return;
        };
        for index in 0..self.kinds.len() {
            let kind = &mut self.kinds[index];
            kind.reserved -= member.reserved[index];
            kind.waiting.remove(&run);
            self.distribute(index);
        }
    }
}

impl RunShare {
    /// An admission of `kind` now, when the Run's turn and share allow;
    /// otherwise none, and [`RunShare::turn`] answers once it may try
    /// again.
    pub fn try_admit(&self, kind: WorkKind) -> Option<Permit> {
        self.shares
            .state()
            .admit(self.run, index(kind))
            .then(|| Permit {
                shares: self.shares.clone(),
                run: self.run,
                kind,
            })
    }

    /// Whether the Run waits for its turn for `kind`: a try found none, and
    /// none was set aside for it since.
    pub fn waits(&self, kind: WorkKind) -> bool {
        let state = self.shares.state();
        let index = index(kind);
        state.kinds[index].waiting.contains(&self.run)
            && state
                .runs
                .get(&self.run)
                .is_some_and(|member| member.reserved[index] == 0)
    }

    /// Answers once an admission of `kind` was set aside for the Run, after
    /// a [`RunShare::try_admit`] that found none.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: the admission stays set aside for
    /// the Run's next try.
    pub async fn turn(&self, kind: WorkKind) {
        let notify = {
            let state = self.shares.state();
            match state.runs.get(&self.run) {
                Some(member) => Arc::clone(&member.turns[index(kind)]),
                None => return,
            }
        };
        notify.notified().await;
    }

    /// An admission of `kind`, once the Run's turn comes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: an admission set aside meanwhile
    /// stays for the Run's next try.
    pub async fn admit(&self, kind: WorkKind) -> Permit {
        loop {
            if let Some(permit) = self.try_admit(kind) {
                return permit;
            }
            self.turn(kind).await;
        }
    }
}

impl RunShare {
    /// The browser running as `process`, which the Run's pool launched,
    /// joins the governor, which counts it in the Run's weight and sheds it
    /// at critical (ADR 0040).
    pub fn browser(&self, process: u32) -> GovernedBrowser {
        self.shares.browsers.join(self.run, process)
    }

    /// The level the governor found at each sample, which changes at every
    /// sample, even to the same level: at critical the Run's pages past
    /// their cap end (ADR 0040).
    pub fn samples(&self) -> watch::Receiver<GovernorLevel> {
        self.shares.level.subscribe()
    }

    /// What the last resort weighs of the Run: its browsers' memory at the
    /// governor's last sample, and whether a person holds one of its pages.
    pub fn weight(&self) -> Weight {
        self.shares.browsers.weight(self.run)
    }
}

impl Drop for RunShare {
    fn drop(&mut self) {
        self.shares.state().leave(self.run);
    }
}

impl Permit {
    /// Its kind.
    pub fn kind(&self) -> WorkKind {
        self.kind
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.shares.state().give_back(self.run, index(self.kind));
    }
}

#[cfg(test)]
mod tests;
