//! `recipe.start` and `catalog_run.start`, the starts of Runs with no
//! handler in the program (ADRs 0036 and 0102). This engine runs neither a
//! Recipe nor a code-free target yet, so each refuses with
//! `run_target_unbuilt` once its params read, and starts no Run. The
//! tickets that build those runtimes remove the refusal and its Error code.

use goodfetch_protocol::run::{
    CATALOG_RUN_START, CatalogRunStartParams, RECIPE_START, RUN_TARGET_UNBUILT, RecipeStartParams,
    RunStarted, RunTargetUnbuiltDetails,
};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};

/// The two handlers.
pub(super) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&RECIPE_START, recipe_start),
        Handler::new(&CATALOG_RUN_START, catalog_run_start),
    ]
}

async fn recipe_start(
    _params: RecipeStartParams,
    _context: CallContext,
) -> Result<RunStarted, CallError> {
    Err(unbuilt(RECIPE_START.name, "recipe"))
}

async fn catalog_run_start(
    params: CatalogRunStartParams,
    _context: CallContext,
) -> Result<RunStarted, CallError> {
    Err(unbuilt(CATALOG_RUN_START.name, params.target.kind()))
}

/// `run_target_unbuilt`: `operation` names a `target` this engine does not
/// run yet.
fn unbuilt(operation: &str, target: &str) -> CallError {
    CallError::new(
        &RUN_TARGET_UNBUILT,
        format!(
            "This engine does not run a `{target}` start yet, so `{operation}` started no Run."
        ),
        &RunTargetUnbuiltDetails {
            operation: operation.to_owned(),
            target: target.to_owned(),
        },
    )
}
