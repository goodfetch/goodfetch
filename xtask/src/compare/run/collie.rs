//! The Collie scenario's trials in a run: its cells' (issue 2282), then
//! its page-load cell's (issue 2319), each set in its own random order,
//! each trial waiting for a quiet machine as a timed trial does.

use anyhow::{Context as _, Result};

use crate::output;

use super::super::collie::{self, page_load};
use super::super::trial::Outcome;
use super::super::trials::{CollieScenario, CollieStopped, LoadStopped};
use super::machine::{load, quiet};
use super::watch::{Retried, retried};
use super::{Options, next, waits_for_quiet};

/// One trial by `run`, handed the load it started at once the machine is
/// quiet (or at once, in a quicker try), run once more when the comparison
/// stopped it; the run's line names it `subject` when it was retried or
/// stopped twice.
fn attempted<T>(
    options: &Options,
    subject: &str,
    mut run: impl FnMut(Option<f64>) -> Result<Outcome<T>>,
) -> Result<Retried<T>> {
    let attempt = retried(|| {
        let load = if waits_for_quiet(options) {
            quiet()
        } else {
            load()
        };
        run(load)
    })
    .with_context(|| format!("running {subject}"))?;
    match &attempt {
        Retried::Done {
            retried: Some(reason),
            ..
        } => output::line(&format!("{subject} was stopped, then retried: {reason}")),
        Retried::Done { retried: None, .. } => {}
        Retried::Stopped(reason) => output::line(&format!(
            "{subject} was stopped twice, and the cell measures it no further: {reason}"
        )),
    }
    Ok(attempt)
}

/// The Collie scenario's trials, when the run asked for it (`tools`):
/// `options.trials` of each tool in each of its cells, then of each row on
/// each page of its page-load cell, each set in one random order drawn from
/// `state`.
pub(super) fn collie_trials(
    tools: Option<&collie::Tools<'_>>,
    options: &Options,
    state: &mut u64,
) -> Result<CollieScenario> {
    let Some(tools) = tools else {
        return Ok(CollieScenario::NotRun {
            reason: "the run asked for no Collie cell".to_owned(),
        });
    };
    let mut trials_run = Vec::new();
    let mut stopped: Vec<CollieStopped> = Vec::new();
    for (exercise, tool, number) in collie::order(options.trials, state, next) {
        if stopped
            .iter()
            .any(|run| run.exercise == exercise && run.tool == tool)
        {
            continue;
        }
        let subject = format!("collie {} {} trial {number}", exercise.name(), tool.name());
        match attempted(options, &subject, |load| {
            collie::trial(tools, exercise, tool, number, load)
        })? {
            Retried::Done {
                measured: mut done,
                retried,
            } => {
                output::line(&format!(
                    "{subject}: {} actions, {} CDP commands ({} awaited one after another) in {:.3} s, peak {} bytes",
                    done.actions, done.commands, done.rounds, done.wall_seconds, done.peak_bytes,
                ));
                done.retried = retried;
                trials_run.push(done);
            }
            Retried::Stopped(reason) => stopped.push(CollieStopped {
                exercise,
                tool,
                number,
                reason,
            }),
        }
    }
    let ad_hosts = page_load::served_ad_hosts(tools.proving_ground)?;
    let mut loads = Vec::new();
    let mut loads_stopped: Vec<LoadStopped> = Vec::new();
    for (page, row, number) in page_load::order(options.trials, state, next) {
        if loads_stopped
            .iter()
            .any(|run| run.page == page && run.row == row)
        {
            continue;
        }
        let subject = page_load::subject(page, row, number);
        match attempted(options, &subject, |load| {
            page_load::trial(tools, &ad_hosts, page, row, number, load)
        })? {
            Retried::Done {
                measured: mut done,
                retried,
            } => {
                output::line(&format!(
                    "{subject}: load in {:.3} s, {} own subresources of {} requests, peak {} bytes",
                    done.wall_seconds, done.own, done.sent, done.peak_bytes,
                ));
                done.retried = retried;
                loads.push(done);
            }
            Retried::Stopped(reason) => loads_stopped.push(LoadStopped {
                page,
                row,
                number,
                reason,
            }),
        }
    }
    Ok(CollieScenario::Measured {
        trials: options.trials,
        trials_run,
        stopped,
        loads,
        loads_stopped,
    })
}
