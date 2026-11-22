//! A live Run's identity (ADR 0145): one small read-only value behind one
//! reference count that says which Run this is, its Run ID, its folder, its
//! clock, its resolved settings, its stop signal, its metrics, its
//! `tracing` span and its task owner, which a function takes without a
//! `Run`. No holder changes what it says of the Run: it hands out its task
//! owner's starting side alone (ADR 0145's note of 2026-11-10), and reads
//! its stop signal, which the Runs' registry alone sets, through the one
//! [`StopAsker`] it builds with the identity when the Run opens. What needs
//! nothing but the identity is read from it: a point in time from its
//! clock.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::run::{RunId, StopReason};
use tokio::sync::watch;

use crate::clock::Clock;
use crate::metrics::Metrics;
use crate::settings::RunSettings;
use crate::tasks::{TaskOwner, TaskStarter};

/// A live Run's identity. Cloning it is one reference count.
#[derive(Clone)]
pub struct RunIdentity(Arc<Identity>);

/// What a Run's identity says of it.
struct Identity {
    id: RunId,
    folder: PathBuf,
    clock: Clock,
    settings: Option<RunSettings>,
    /// The reason it was asked to stop at its next safe point, once asked,
    /// or the reason it stopped at once.
    stop: watch::Receiver<Option<StopReason>>,
    metrics: Metrics,
    span: tracing::Span,
    owner: Arc<TaskOwner>,
}

/// What the Runs' registry builds a Run's identity from as the Run opens.
pub(crate) struct Parts {
    pub(crate) id: RunId,
    pub(crate) folder: PathBuf,
    pub(crate) clock: Clock,
    pub(crate) settings: Option<RunSettings>,
    pub(crate) metrics: Metrics,
    /// Its `tracing` span, a root of its own (ADR 0132), which each of its
    /// tasks runs in.
    pub(crate) span: tracing::Span,
    /// Its task owner, which the registry alone cancels, waits for and
    /// stops.
    pub(crate) owner: Arc<TaskOwner>,
}

impl std::fmt::Debug for RunIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunIdentity")
            .field("id", &self.0.id)
            .finish_non_exhaustive()
    }
}

impl RunIdentity {
    /// The identity `parts` make, with no stop asked yet, and the one
    /// value that asks its Run to stop, which the Runs' registry keeps.
    pub(crate) fn new(parts: Parts) -> (Self, StopAsker) {
        let Parts {
            id,
            folder,
            clock,
            settings,
            metrics,
            span,
            owner,
        } = parts;
        let (asker, stop) = watch::channel(None);
        let identity = Self(Arc::new(Identity {
            id,
            folder,
            clock,
            settings,
            stop,
            metrics,
            span,
            owner,
        }));
        (identity, StopAsker(Arc::new(asker)))
    }

    /// Its Run ID.
    pub fn id(&self) -> &RunId {
        &self.0.id
    }

    /// Its folder, which holds its state and its Run trace.
    pub fn folder(&self) -> &Path {
        &self.0.folder
    }

    /// The engine's clock.
    pub fn clock(&self) -> Clock {
        self.0.clock
    }

    /// The settings it was started with, when its start resolved them.
    pub fn settings(&self) -> Option<&RunSettings> {
        self.0.settings.as_ref()
    }

    /// The runtime's metrics, which what the Run runs records in (ADR
    /// 0050).
    pub fn metrics(&self) -> &Metrics {
        &self.0.metrics
    }

    /// Its `tracing` span, whose field is its Run ID: the parent of what its
    /// driver and its pages record (ADR 0132).
    pub fn span(&self) -> &tracing::Span {
        &self.0.span
    }

    /// Its owner's starting side, whose tasks and blocking work stop with
    /// the Run: cancelling, waiting for and stopping the owner stay the
    /// Runs' registry's (ADR 0145's note of 2026-11-10).
    pub fn owner(&self) -> &TaskStarter {
        &self.0.owner
    }

    /// Whether the Run was asked to stop at its next safe point.
    pub(crate) fn is_stopping(&self) -> bool {
        self.0.stop.borrow().is_some()
    }

    /// Answers once something asks the Run to stop at its next safe point,
    /// with the reason it stops for; never, once the Run is gone without
    /// one.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing; a later call answers the same.
    pub async fn stopping(&self) -> StopReason {
        let mut stop = self.0.stop.clone();
        let asked = stop
            .wait_for(Option::is_some)
            .await
            .map(|reason| reason.clone());
        match asked {
            Ok(Some(reason)) => reason,
            _ => std::future::pending().await,
        }
    }

    /// The wall time now, as a point in time of the Run's (ADR 0102).
    pub fn point_in_time(&self) -> SystemTime {
        self.0.clock.point_in_time()
    }

    /// The wall time `wait` from now, as a point in time of the Run's, such
    /// as when a domain is next admitted (ADR 0102).
    pub fn point_in_time_after(&self, wait: Duration) -> SystemTime {
        self.0.clock.point_in_time_after(wait)
    }
}

/// The value of the Runs' registry's that asks one Run to stop, built with
/// its identity: the registry holds it, and hands it to a power that must
/// ask for a stop, such as charging at a Limit's refusal (ADR 0145). Cheap
/// to clone; every clone asks the same Run.
#[derive(Clone)]
pub struct StopAsker(Arc<watch::Sender<Option<StopReason>>>);

impl StopAsker {
    /// Asks the Run to stop at its next safe point for `reason`, waking
    /// whoever waits for it; the first reason asked stays.
    pub fn at_safe_point(&self, reason: StopReason) {
        self.0.send_if_modified(|stop| {
            let unset = stop.is_none();
            if unset {
                *stop = Some(reason);
            }
            unset
        });
    }

    /// Keeps `reason`, unless one was asked before, as what a call in
    /// flight reads once the Run's tasks are cancelled (ADR 0113), waking
    /// nothing that waits for a safe point: the Run ends now, not at one.
    pub(crate) fn at_once(&self, reason: StopReason) {
        self.0.send_if_modified(|stop| {
            if stop.is_none() {
                *stop = Some(reason);
            }
            false
        });
    }
}

#[cfg(test)]
mod tests;
