//! Each Run's controls in its submenu (ADR 0141), labelled with what they
//! do: Pause, Stop and Abort for a running Run, and Retry failed for a
//! running crawl on a Foxhound Daemon; Resume for a paused or Limit-stopped
//! Run that a Recipe runs, and "Copy resume command" for a script's (ADR
//! 0031). Stop asks first as the settings say, Abort and Retry failed
//! always, Pause never. A read-only Kennel offers none.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::RunResumeParams;
use goodfetch_protocol::foxhound::outcomes::RunRetryFailedParams;
use goodfetch_protocol::run::{RunAbortParams, RunId, RunKind, RunParams, RunStopParams};

use super::runs::Phase;
use super::{Daemon, Effect, LINE_MOST, State};
use crate::client::Call;
use crate::text;

/// A Run's control in its submenu, each labelled with what it does (ADR
/// 0141).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    /// A Checkpoint and a stop, resumable: `run.pause`.
    Pause,
    /// A Checkpoint, and the Run can resume: `run.stop`.
    Stop,
    /// No attempt in flight commits; not immediate: `run.abort`.
    Abort,
    /// The crawl's failed requests run again: `run.retry_failed`.
    RetryFailed,
    /// The Daemon runs the stopped Run again: `run.resume`, for a Run the
    /// engine can run itself, a Recipe's.
    Resume,
    /// Copies the command that resumes a script's Run, which only its
    /// program can run (ADR 0031).
    CopyResumeCommand,
}

/// A question asked before a control or a Needs you action acts: it names
/// the Run and the consequence (ADR 0141).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    /// The question, naming the Run.
    pub title: String,
    /// What the control or the action does.
    pub consequence: String,
    /// The label of the button that confirms.
    pub confirm: String,
    pub(super) asked: Asked,
}

impl Confirmation {
    /// The question `title`, its `consequence` and its `confirm` button,
    /// made safe to show by the Text function, asked before `asked`.
    pub(super) fn new(title: &str, consequence: &str, confirm: &str, asked: Asked) -> Self {
        Self {
            title: text::shown(title, LINE_MOST),
            consequence: text::shown(consequence, LINE_MOST * 2),
            confirm: text::shown(confirm, LINE_MOST),
            asked,
        }
    }
}

/// What a confirmation asks before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Asked {
    /// The control `control` of the Run `run`.
    Control { run: RunId, control: Control },
    /// The call that resolves the Needs you item `item`, as it was when
    /// the question was asked.
    NeedsYou { item: String, call: Call },
}

impl State {
    /// The controls the menu offers for `id`, in their order: none in a
    /// read-only Kennel, for a Run it does not list, or with no Daemon.
    pub(super) fn offered(&self, id: &RunId) -> Vec<Control> {
        let Daemon::Reached(identity, _) = &self.daemon else {
            return Vec::new();
        };
        let listed = self
            .listed()
            .0
            .into_iter()
            .find(|(listed, _)| *listed == id);
        let (Some((_, run)), true) = (listed, self.settings.controls) else {
            return Vec::new();
        };
        let facts = self.facts.get(id);
        match run.phase {
            Phase::Running => {
                let mut controls = vec![Control::Pause, Control::Stop, Control::Abort];
                // A crawl's retry is Foxhound's.
                let crawl = facts.is_some_and(|facts| facts.kind == RunKind::Crawl);
                if crawl && identity.product == Product::Foxhound {
                    controls.push(Control::RetryFailed);
                }
                controls
            }
            // The engine runs a Recipe's Run itself; only its program runs
            // a script's (ADR 0031).
            Phase::Paused | Phase::StoppedByLimit => {
                if facts.is_some_and(|facts| facts.recipe.is_some()) {
                    vec![Control::Resume]
                } else {
                    vec![Control::CopyResumeCommand]
                }
            }
            Phase::Stopping | Phase::Failed => Vec::new(),
        }
    }

    /// The question `control` asks before it acts on `run`: Abort and
    /// Retry failed always ask, Stop as the settings say, the others never.
    pub(super) fn confirmation(&self, run: &RunId, control: Control) -> Option<Confirmation> {
        let name = self.name(run);
        let (title, consequence, confirm) = match control {
            Control::Stop if self.settings.confirm_stop => (
                format!("Stop {name}?"),
                "It stops at its next safe point with a Checkpoint, and it can resume.",
                "Stop",
            ),
            Control::Abort => (
                format!("Abort {name}?"),
                "It stops at its next safe point, not at once. No attempt in flight commits, and its work items return to pending.",
                "Abort",
            ),
            Control::RetryFailed => (
                format!("Retry the failed requests of {name}?"),
                "Each request whose latest outcome is failed runs again in this crawl, with its retries counted afresh.",
                "Retry failed",
            ),
            Control::Stop | Control::Pause | Control::Resume | Control::CopyResumeCommand => {
                return None;
            }
        };
        Some(Confirmation::new(
            &title,
            consequence,
            confirm,
            Asked::Control {
                run: run.clone(),
                control,
            },
        ))
    }

    /// What `control` does to `run` once it may act.
    pub(super) fn act(&self, run: RunId, control: Control) -> Option<Effect> {
        let call = match control {
            Control::Pause => Call::Pause(RunParams { run_id: run }),
            Control::Stop => Call::Stop(RunStopParams {
                run_id: run,
                timeout: None,
            }),
            Control::Abort => Call::Abort(RunAbortParams {
                run_id: run,
                forwarded: false,
            }),
            Control::RetryFailed => Call::RetryFailed(RunRetryFailedParams {
                run_id: run,
                blocked: false,
                settings: serde_json::Map::new(),
                project: None,
            }),
            Control::Resume => Call::Resume(RunResumeParams {
                run_id: run,
                checkpoint: None,
                rollback: false,
                settings: serde_json::Map::new(),
                project: None,
            }),
            Control::CopyResumeCommand => {
                let Daemon::Reached(identity, _) = &self.daemon else {
                    return None;
                };
                // A Run ID holds only lowercase letters, digits, `.`, `_`
                // and `-`: the command needs no quoting.
                return Some(Effect::Copy(format!(
                    "goodfetch-{} run resume {}",
                    identity.product.as_str(),
                    run.as_str()
                )));
            }
        };
        Some(Effect::Call(call))
    }
}

/// What the menu says a failed `call` was trying to do, naming a Run by
/// `name`.
pub(super) fn attempt(call: &Call, name: impl Fn(&RunId) -> String) -> String {
    match call {
        Call::RaiseLimit(params) => format!("Raising the Limit on {}", params.usage_kind),
        Call::AllowConsentDomain(params) => {
            format!("Accepting the consent walls of {}", params.domain)
        }
        Call::DenyWallDomain(params) => format!("Denying {}", params.domain),
        Call::RemoveRefetchDomain(params) => {
            format!("Removing {} from the refetch domains", params.domain)
        }
        Call::Dismiss(_) => "Dismissing a Needs you item".to_owned(),
        Call::Pause(params) => format!("Pausing {}", name(&params.run_id)),
        Call::Stop(params) => format!("Stopping {}", name(&params.run_id)),
        Call::Abort(params) => format!("Aborting {}", name(&params.run_id)),
        Call::Resume(params) => format!("Resuming {}", name(&params.run_id)),
        Call::RetryFailed(params) => {
            format!("Retrying the failed requests of {}", name(&params.run_id))
        }
        Call::AddSeeds(seeds) => format!("Adding seeds to {}", name(&seeds.run_id)),
    }
}

/// What a control's line in a Run's submenu says it does (ADR 0141).
pub(super) fn control_label(control: Control) -> &'static str {
    match control {
        Control::Pause => "Pause: a Checkpoint and a stop",
        Control::Stop => "Stop: a Checkpoint, and the Run can resume",
        Control::Abort => "Abort: no attempt in flight commits; not immediate",
        Control::RetryFailed => "Retry failed: its failed requests run again",
        Control::Resume => "Resume: the Daemon runs it again from where it stopped",
        Control::CopyResumeCommand => "Copy resume command",
    }
}
