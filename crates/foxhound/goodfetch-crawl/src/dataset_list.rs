//! `dataset.list` (spec #593, story 204; ADR 0103): the Datasets a crawl
//! declares, in the order declared, each with its JSON Schema, its key and
//! how many Records were committed, dropped and rejected, the counts the
//! Run report gives from the crawl requests' latest outcomes. Read from
//! the crawl's committed state through the engine's Run reader, never its
//! writer, so it works on a Run no program holds; a Run no crawl wrote
//! declares none.

use goodfetch_dataset::Declarations;
use goodfetch_engine::run::{run_error, through_owner};
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::dataset_view::{
    DATASET_LIST, DatasetList, DatasetListParams, DatasetSummary,
};
use rusqlite::Connection;

/// The handler of `dataset.list`.
pub(crate) fn handler() -> Handler {
    Handler::new(&DATASET_LIST, list)
}

async fn list(params: DatasetListParams, context: CallContext) -> Result<DatasetList, CallError> {
    if let Some(answer) = through_owner(&context, &DATASET_LIST, &params.run_id, &params).await? {
        return Ok(answer);
    }
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    reader.read(listed).await.map_err(failed)
}

/// The Datasets the run file `connection` declares, with their counts.
fn listed(connection: &Connection) -> rusqlite::Result<DatasetList> {
    let declarations = Declarations::read(connection)?;
    if declarations.declarations().is_empty() {
        return Ok(DatasetList {
            datasets: Vec::new(),
        });
    }
    let (_, counted) = crate::report::ended(connection)?;
    let datasets = declarations
        .declarations()
        .iter()
        .map(|declared| {
            let counts = counted
                .iter()
                .find(|counts| counts.dataset == declared.name);
            DatasetSummary {
                name: declared.name.clone(),
                schema: declared.schema.clone(),
                key: declared.key.clone(),
                committed: counts.map_or(0, |counts| counts.committed),
                dropped: counts.map_or(0, |counts| counts.dropped),
                rejected: counts.map_or(0, |counts| counts.rejected),
            }
        })
        .collect();
    Ok(DatasetList { datasets })
}
