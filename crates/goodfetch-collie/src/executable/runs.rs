//! `runs reindex` (ADRs 0031 and 0036): rebuilds the Run index of the
//! executable's major from the configured run locations (the per-major data
//! folder's `runs`, and `run_state.location` when user configuration or the
//! Operator policy sets it) and the paths it is given, each a Run's folder
//! or a folder of them. A Run it does not find stays as a *missing* row,
//! and an *erased* row stays erased.

use goodfetch_engine::Engine;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::group::reindex_to_lead;
use goodfetch_engine::runs::reindex as rebuild;
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::run::{RUN_STATE_LOCATION, RunsAdoptOutput, RunsReindexOutput};
use std::path::PathBuf;
use std::process::ExitCode;

use super::daemon::Machine;
use super::output::{self, Human};

/// Rebuilds the Run index from the configured locations and `paths`,
/// relative ones read from `folder`; machine settings it cannot read fail
/// it as every command's failure prints, with their Error code's row.
pub(super) fn reindex(
    machine: &Machine<'_>,
    folder: &std::path::Path,
    paths: Vec<PathBuf>,
    json: bool,
    human: &Human,
) -> ExitCode {
    let (identity, home) = (machine.identity, machine.home);
    let settings = match machine.settings() {
        Ok(settings) => settings,
        Err(failure) => return failure.print(json, human),
    };
    let configured = settings
        .effective(&RUN_STATE_LOCATION)
        .and_then(|location| location.value.as_str().map(PathBuf::from));
    let locations: Vec<PathBuf> = std::iter::once(home.runs(&identity.product_version))
        .chain(configured)
        .chain(paths.into_iter().map(|path| folder.join(path)))
        .collect();
    match rebuild(home, &identity.product_version, &locations) {
        Ok(rebuilt) => {
            if let Ok(runtime) = goodfetch_runtime::current_thread().enable_all().build()
                && let Err(error) =
                    runtime.block_on(reindex_to_lead(home, identity, Clock::system()))
            {
                tracing::warn!(%error, "rebuilt Run index could not be reported to the Lead Daemon");
            }
            let run_index = rebuilt.run_index.display().to_string();
            if json {
                output::json(&RunsReindexOutput {
                    run_index,
                    present: rebuilt.present,
                    missing: rebuilt.missing,
                    erased: rebuilt.erased,
                });
            } else {
                output::line(&format!(
                    "Rebuilt the Run index {run_index}: {} found, {} missing, {} erased.",
                    rebuilt.present, rebuilt.missing, rebuilt.erased
                ));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "the Run index cannot be rebuilt");
            ExitCode::FAILURE
        }
    }
}

/// Brings checkpointed Runs from `paths` (relative to `folder`) on a removed
/// Group member's kept volume into another Group member of the Daemon group
/// (ADR 0055).
pub(super) fn adopt(
    machine: &Machine<'_>,
    folder: &std::path::Path,
    paths: Vec<PathBuf>,
    from_member: Option<&str>,
    engine: &Engine,
    json: bool,
    human: &Human,
) -> ExitCode {
    let (identity, home) = (machine.identity, machine.home);
    let locations: Vec<PathBuf> = if paths.is_empty() {
        vec![home.runs(&identity.product_version)]
    } else {
        paths.into_iter().map(|path| folder.join(path)).collect()
    };
    let Ok(runtime) = goodfetch_runtime::current_thread().enable_all().build() else {
        tracing::error!("the runtime for `runs adopt` could not be started");
        return ExitCode::FAILURE;
    };
    match runtime.block_on(engine.adopt_runs(
        home,
        identity,
        Clock::system(),
        &locations,
        from_member,
    )) {
        Ok(adopted) => {
            let result = RunsAdoptOutput {
                adopted: adopted.adopted,
                skipped: adopted.skipped,
            };
            if json {
                output::json(&result);
            } else {
                output::line(&format!(
                    "Adopted {} Run(s) ({} skipped).",
                    result.adopted.len(),
                    result.skipped.len()
                ));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            let error_object = error.to_error_object_in(Form::Catalog);
            output::fail(error_object, Some("runs.adopt"), json, human)
        }
    }
}
