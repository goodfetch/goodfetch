//! The Runs the State follows (ADR 0141): the live ones, from the
//! subscription's events and the Run index's rows; those that stopped, until
//! their stop reason is read; and those that ended in a state that needs a
//! look, which stay listed, and in the verdict, until the menu that showed
//! them closes. Their ends send the desktop notices the settings turn on.
//! The menu's rows of the Runs come from here too.

use std::collections::BTreeSet;
use std::time::Instant;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_protocol::needs_you::{NeedsYouChange, NeedsYouItem, RUN_STOPPED_BY_LIMIT};
use goodfetch_protocol::run::{Lifecycle, RunAxes, RunId, RunKind, RunProgress, StopReason};

use super::controls::control_label;
use super::notices::{self, Ended, Kind};
use super::{Choice, MENU_RUNS, NAME_MOST, Row, State};
use crate::client::DaemonEvent;
use crate::text;

/// Where a Run the State follows is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// It runs.
    Running,
    /// It stopped, and its stop reason is read.
    Stopping,
    /// It was paused, to be resumed: it stays listed until it runs again
    /// or ends.
    Paused,
    /// It failed.
    Failed,
    /// It was stopped by a Limit.
    StoppedByLimit,
}

impl Phase {
    /// Whether it ended in a state that needs the person.
    pub(super) fn needs_a_look(self) -> bool {
        matches!(self, Self::Failed | Self::StoppedByLimit)
    }
}

/// A Run the State follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub(super) phase: Phase,
    /// Its progress message, from its last progress read.
    pub(super) message: Option<String>,
    /// A crawl's counts, from its last progress read.
    pub(super) counts: Option<Counts>,
    /// When an open menu first showed it since it ended.
    pub(super) seen: Option<Instant>,
    /// Whether its phase came from a resync's replayed state, whose end
    /// sends no desktop notice.
    pub(super) replayed: bool,
}

impl Run {
    pub(super) fn new(phase: Phase) -> Self {
        Self {
            phase,
            message: None,
            counts: None,
            seen: None,
            replayed: false,
        }
    }
}

/// A crawl's requests the menu counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Counts {
    succeeded: u64,
    failed: u64,
    pending: u64,
}

/// What the Run index's row says of a live Run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Facts {
    pub(super) kind: RunKind,
    pub(super) recipe: Option<String>,
    pub(super) message: Option<String>,
}

impl State {
    pub(super) fn event(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::Resync(resync) => {
                let live: BTreeSet<&RunId> = resync.runs.iter().map(|axes| &axes.run_id).collect();
                // A Run held as running that the resync leaves out ended.
                for (id, run) in &mut self.runs {
                    if run.phase == Phase::Running && !live.contains(id) {
                        run.phase = Phase::Stopping;
                    }
                }
                for axes in resync.runs {
                    self.axes(axes, true);
                }
                self.needs_you = resync
                    .needs_you
                    .into_iter()
                    .map(|item| (item.id.clone(), item))
                    .collect();
                self.live_runs.bump();
            }
            DaemonEvent::RunAxes(axes) => {
                let known = self.facts.contains_key(&axes.run_id);
                if axes.lifecycle == Lifecycle::Running && !known {
                    self.live_runs.bump();
                }
                self.axes(axes, false);
            }
            DaemonEvent::NeedsYou(change) => match change {
                NeedsYouChange::Listed { item } => {
                    if !self.needs_you.contains_key(&item.id) && !self.limit_noticed(&item) {
                        let summary = item.summary.clone();
                        self.notices
                            .push(Kind::NeedsYou, || notices::needs_you(&summary));
                    }
                    self.needs_you.insert(item.id.clone(), item);
                }
                NeedsYouChange::Removed { id, .. } => {
                    self.needs_you.remove(&id);
                }
            },
        }
    }

    /// A Run's axes as they are now, `replayed` by a resync or changed.
    pub(super) fn axes(&mut self, axes: RunAxes, replayed: bool) {
        let phase = match axes.lifecycle {
            Lifecycle::Running => Phase::Running,
            Lifecycle::Failed => Phase::Failed,
            // Its stop reason, which says whether a Limit stopped it, is
            // read from its progress.
            Lifecycle::Stopped => Phase::Stopping,
            Lifecycle::Finished => {
                // A Run the State does not follow ended before it learned
                // of it, or its progress already said so.
                if let Some(run) = self.runs.remove(&axes.run_id)
                    && !replayed
                    && !run.replayed
                {
                    self.ended(&axes.run_id, Some(run.phase), Ended::Finished);
                }
                return;
            }
        };
        let id = axes.run_id;
        let was = self.runs.get(&id).map(|run| run.phase);
        let run = self
            .runs
            .entry(id.clone())
            .or_insert_with(|| Run::new(phase));
        if was != Some(phase) {
            run.phase = phase;
            run.seen = None;
            // A running Run's end is a change, whatever listed it.
            run.replayed = replayed && phase != Phase::Running;
            if phase == Phase::Failed && !replayed {
                self.ended(&id, was, Ended::Failed);
            }
        }
    }

    /// Sends the notice of the Run `id` ending as `ended` says, from the
    /// phase `was` it was followed in, if any; none when it had already
    /// ended in a state that needs a look, or when the notice of its Needs
    /// you item tells the same.
    pub(super) fn ended(&mut self, id: &RunId, was: Option<Phase>, ended: Ended) {
        if was.is_some_and(Phase::needs_a_look) {
            return;
        }
        if ended == Ended::StoppedByLimit && self.notices.wants(Kind::NeedsYou) {
            let told = self
                .needs_you
                .values()
                .any(|item| item.kind == RUN_STOPPED_BY_LIMIT && item.run.as_ref() == Some(id));
            if told {
                return;
            }
        }
        let name = self.name(id);
        self.notices
            .push(ended.kind(), || notices::run_ended(ended, &name));
    }

    /// Whether `item` says a Run stopped by a Limit whose own notice was
    /// already sent.
    pub(super) fn limit_noticed(&self, item: &NeedsYouItem) -> bool {
        item.kind == RUN_STOPPED_BY_LIMIT
            && self.notices.wants(Kind::RunStoppedByLimit)
            && item
                .run
                .as_ref()
                .and_then(|id| self.runs.get(id))
                .is_some_and(|run| run.phase == Phase::StoppedByLimit && !run.replayed)
    }

    pub(super) fn progress(&mut self, progress: &RunProgress) {
        let Some(run) = self.runs.get_mut(&progress.run_id) else {
            return;
        };
        if let Some(message) = &progress.message {
            run.message = Some(message.as_str().to_owned());
        }
        if let Some(requests) = progress
            .kind_parts
            .get("requests")
            .and_then(|requests| serde_json::from_value::<RequestProgress>(requests.clone()).ok())
        {
            run.counts = Some(Counts {
                succeeded: requests.succeeded,
                failed: requests.failed,
                pending: requests.pending,
            });
        }
        let phase = match (progress.lifecycle, &progress.stop_reason, progress.last) {
            (Lifecycle::Running, _, _) => Phase::Running,
            (Lifecycle::Failed, _, _) => Phase::Failed,
            (Lifecycle::Stopped, Some(StopReason::Limit(_)), _) => Phase::StoppedByLimit,
            (Lifecycle::Stopped, Some(StopReason::Pause), _) => Phase::Paused,
            (Lifecycle::Stopped, _, false) => Phase::Stopping,
            (Lifecycle::Stopped, _, true) => {
                self.runs.remove(&progress.run_id);
                return;
            }
            (Lifecycle::Finished, _, _) => {
                let (was, replayed) = (run.phase, run.replayed);
                self.runs.remove(&progress.run_id);
                if !replayed {
                    self.ended(&progress.run_id, Some(was), Ended::Finished);
                }
                return;
            }
        };
        let (was, replayed) = (run.phase, run.replayed);
        if was != phase {
            run.phase = phase;
            run.seen = None;
            run.replayed = replayed && phase != Phase::Running;
            let ended = match phase {
                Phase::Failed => Some(Ended::Failed),
                Phase::StoppedByLimit => Some(Ended::StoppedByLimit),
                Phase::Running | Phase::Stopping | Phase::Paused => None,
            };
            if let Some(ended) = ended
                && !replayed
            {
                self.ended(&progress.run_id, Some(was), ended);
            }
        }
    }

    /// The Runs the menu lists, those that need a look first, then by Run
    /// ID, and how many more it leaves out.
    pub(super) fn listed(&self) -> (Vec<(&RunId, &Run)>, usize) {
        let mut listed: Vec<(&RunId, &Run)> = self
            .runs
            .iter()
            .filter(|(_, run)| run.phase != Phase::Stopping)
            .collect();
        listed.sort_by_key(|(id, run)| (!run.phase.needs_a_look(), *id));
        let more = listed.len().saturating_sub(MENU_RUNS);
        listed.truncate(MENU_RUNS);
        (listed, more)
    }

    /// The menu's rows of the Runs, under a Daemon of `product`.
    pub(super) fn run_rows(&self, product: Product, rows: &mut Vec<Row>) {
        let (listed, more) = self.listed();
        if listed.is_empty() {
            return;
        }
        rows.push(Row::Separator);
        for (id, run) in listed {
            let facts = self.facts.get(id);
            let label = format!("{} — {}", self.name(id), lifecycle(run.phase));
            let controls = self.offered(id);
            rows.push(if controls.is_empty() {
                Row::Text(label)
            } else {
                Row::Submenu {
                    label,
                    rows: controls
                        .into_iter()
                        .map(|control| Row::Choice {
                            label: control_label(control).to_owned(),
                            choice: Choice::Control {
                                run: id.clone(),
                                control,
                            },
                        })
                        .collect(),
                }
            });
            let message = run
                .message
                .as_deref()
                .or_else(|| facts.and_then(|facts| facts.message.as_deref()));
            if let Some(message) = message {
                rows.push(Row::Detail(message.to_owned()));
            }
            // A crawl's counts are Foxhound's.
            if let (Product::Foxhound, Some(counts)) = (product, run.counts) {
                rows.push(Row::Detail(format!(
                    "{} succeeded, {} failed, {} pending",
                    counts.succeeded, counts.failed, counts.pending
                )));
            }
        }
        if more > 0 {
            rows.push(Row::Text(format!(
                "{more} more {}",
                if more == 1 { "Run" } else { "Runs" }
            )));
        }
    }

    /// What the menu calls `id`: its Recipe, or its Run ID when it ran
    /// none.
    pub(super) fn name(&self, id: &RunId) -> String {
        self.facts
            .get(id)
            .and_then(|facts| facts.recipe.as_deref())
            .map_or_else(
                || id.as_str().to_owned(),
                |recipe| text::shown(recipe, NAME_MOST),
            )
    }

    /// Marks each Run that needs a look and that the open menu lists as
    /// seen at `at`, and the failed call it shows, each from the first
    /// time an open menu showed it.
    pub(super) fn mark_seen(&mut self, at: Instant) {
        let listed: Vec<RunId> = self
            .listed()
            .0
            .into_iter()
            .filter(|(_, run)| run.phase.needs_a_look())
            .map(|(id, _)| id.clone())
            .collect();
        for id in listed {
            if let Some(run) = self.runs.get_mut(&id) {
                run.seen.get_or_insert(at);
            }
        }
        if let Some(failed) = &mut self.failed {
            failed.seen.get_or_insert(at);
        }
    }
}

/// What the menu says of a Run's Lifecycle.
pub(super) fn lifecycle(phase: Phase) -> &'static str {
    match phase {
        Phase::Running | Phase::Stopping => "Running",
        Phase::Paused => "Paused",
        Phase::Failed => "Failed",
        Phase::StoppedByLimit => "Stopped by a Limit",
    }
}
