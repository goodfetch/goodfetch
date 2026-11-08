//! `runs reindex` (ADRs 0031 and 0036): rebuilds the Run index of the
//! executable's major from the configured run locations (the per-major data
//! folder's `runs`, and `run_state.location` when user configuration or the
//! Operator policy sets it) and the paths it is given, each a Run's folder
//! or a folder of them. A Run it does not find stays as a *missing* row,
//! and an *erased* row stays erased.

use std::path::PathBuf;
use std::process::ExitCode;

use goodfetch_engine::runs::reindex as rebuild;
use goodfetch_protocol::run::{RUN_STATE_LOCATION, RunsReindexOutput};

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
