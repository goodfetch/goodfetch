//! The page crate's record of a live Run (ADR 0145): its identity, the
//! powers its Sessions, pages and browsers use, and the engine's context
//! they read. A Session, an open page and a browser pool each hold one in
//! place of the Run; the fetch tier holds one for each Run it serves, which
//! its served documents carry.

use std::sync::Arc;

use goodfetch_engine::EngineContext;
use goodfetch_engine::entropy::RunEntropy;
use goodfetch_engine::governor::RunGovernor;
use goodfetch_engine::ledger::RunCharging;
use goodfetch_engine::plugin_kinds::run_plugins::RunPlugins;
use goodfetch_engine::politeness::RunPoliteness;
use goodfetch_engine::robots_txt::OwnCopies;
use goodfetch_engine::run_files::RunFiles;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::run_trace::RunTrace;
use goodfetch_engine::runs::{Progress, ReportWarnings, RunKeys, RunParts, RunSendWindow};

/// A live Run, as the page crate holds it: no method of its own reaches a
/// power, each field is the power's value (ADR 0145).
pub struct BrowserRun {
    /// Which Run it is.
    pub identity: RunIdentity,
    /// Its run state's writer, where its Sessions and pages are recorded.
    pub state: RunState,
    /// Its charging, which its pages' requests and crashes are charged to.
    pub charging: RunCharging,
    /// Its Secrets and its data key.
    pub keys: RunKeys,
    /// Its Run report's warnings.
    pub warnings: ReportWarnings,
    /// Its part of the Resource governor, which admits its browser pages.
    pub governor: RunGovernor,
    /// Its entropy, which its browsers' fingerprints fork.
    pub entropy: RunEntropy,
    /// The Politeness it shares, which its Sessions' proxies open.
    pub politeness: RunPoliteness,
    /// Its own robots.txt copies, which its program's navigation reads.
    pub own_robots_txt: OwnCopies,
    /// Its files, which store its pages' bodies.
    pub files: RunFiles,
    /// Its Run trace.
    pub trace: RunTrace,
    /// Its progress, where live Human pauses appear (ADRs 0092 and 0102).
    pub progress: Progress,
    /// Its send window, which its pages' commands mark before they go out.
    pub send_window: RunSendWindow,
    /// Its process Plugins, which its pages' Extractors start.
    pub plugins: Arc<RunPlugins>,
    /// Its compiled Wall detection rules and per-Run Wall allowance state
    /// (ADR 0052; ticket #757).
    pub walls: crate::wall_state::RunWalls,
    /// The engine's context: its sandbox probe, its Audit log, its
    /// known-certificates store and where its listeners record refusals.
    pub engine: EngineContext,
}

impl BrowserRun {
    /// The page crate's record of the Run whose `parts` the registry handed
    /// out, reading the engine's `engine`.
    pub fn of(parts: &RunParts, engine: &EngineContext) -> Arc<Self> {
        Arc::new(Self {
            identity: parts.identity.clone(),
            state: parts.state.clone(),
            charging: parts.charging.clone(),
            keys: parts.keys.clone(),
            warnings: parts.warnings.clone(),
            governor: parts.governor.clone(),
            entropy: parts.entropy.clone(),
            politeness: parts.politeness.clone(),
            own_robots_txt: parts.own_robots_txt.clone(),
            files: parts.files.clone(),
            trace: parts.trace.clone(),
            progress: parts.progress.clone(),
            send_window: parts.send_window.clone(),
            plugins: Arc::clone(&parts.plugins),
            walls: crate::wall_state::RunWalls::default(),
            engine: engine.clone(),
        })
    }
}

impl std::fmt::Debug for BrowserRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserRun")
            .field("id", self.identity.id())
            .finish_non_exhaustive()
    }
}
