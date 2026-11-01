//! The storage entries of "Needs you" (ADR 0122; pitfall O03), the
//! engine's own sources, which its contribution passes in: each filesystem
//! the Runs write to at the `low` or `critical` storage level, with a
//! folder on it, from the Resource governor's last storage sample; and each
//! Run a storage stop left stopped, from the Run index, which a person may
//! dismiss. An item names no free space, which changes with every write:
//! only its level does. Both read again at each storage sample, when a level changes
//! and when a Run stops at its next safe point, or runs again.

use goodfetch_protocol::needs_you::{NeedsYouItem, RUN_STOPPED_BY_STORAGE, STORAGE_LEVEL};
use goodfetch_protocol::run::{Resource, RunIndexRow, StopReason};
use goodfetch_protocol::storage::StorageLevel;
use serde_json::{Map, Value, json};

use super::{Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::run_index;

/// Each filesystem at `low` or `critical`, with its folder.
pub fn levels() -> NeedsYouSource {
    NeedsYouSource::new(Levels)
}

/// Each Run a storage stop left stopped.
pub fn runs_stopped() -> NeedsYouSource {
    NeedsYouSource::new(RunsStopped)
}

/// At each storage sample.
fn each_sample(context: &SourceContext) -> Changes {
    Changes::of(context.storage())
}

struct Levels;

impl Primitives for Levels {
    fn dismissable(&self) -> bool {
        false
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let sample = std::sync::Arc::clone(&context.storage().borrow());
        let since = context.now();
        Box::pin(async move {
            Ok(sample
                .filesystems
                .iter()
                .filter(|filesystem| filesystem.level >= StorageLevel::Low)
                .filter_map(|filesystem| {
                    let folder = filesystem.folders.first()?.display().to_string();
                    let level = filesystem.level.as_str();
                    let summary = match filesystem.level {
                        StorageLevel::Critical => format!(
                            "The filesystem holding {folder} is at the critical storage level: the Runs writing there stopped. Free space on it, then start each again with its Run ID."
                        ),
                        StorageLevel::Low | StorageLevel::Normal => format!(
                            "The filesystem holding {folder} is at the low storage level: its Runs go on but shed what they can. Free space on it."
                        ),
                    };
                    Some(NeedsYouItem {
                        id: format!("{STORAGE_LEVEL}:{folder}"),
                        kind: STORAGE_LEVEL.to_owned(),
                        since: since.clone(),
                        run: None,
                        schedule: None,
                        summary,
                        actions: Vec::new(),
                        details: details(json!({ "level": level, "folder": folder })),
                    })
                })
                .collect())
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        each_sample(context)
    }
}

struct RunsStopped;

impl Primitives for RunsStopped {
    fn dismissable(&self) -> bool {
        true
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let path = context.run_index.clone();
        let version = context.version.clone();
        Box::pin(async move {
            let rows = context
                .blocking(move || run_index::stopped_by_storage(&path, &version))
                .await
                .map_err(SourceError::new)?;
            Ok(rows.iter().map(stopped).collect())
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        each_sample(context)
    }
}

/// The item of the Run `row` names, stopped by storage.
fn stopped(row: &RunIndexRow) -> NeedsYouItem {
    let folder = match &row.stop_reason {
        Some(StopReason::Resources {
            resource: Resource::Storage,
            folder,
        }) => folder.clone(),
        _ => None,
    };
    let since = row
        .ended_at
        .clone()
        .unwrap_or_else(|| row.started_at.clone());
    let run_id = row.run_id.as_str();
    let summary = match &folder {
        Some(folder) => format!(
            "The Run {run_id} stopped: the filesystem holding {folder} filled. Free space on it, then start the Run again with its Run ID to resume it."
        ),
        None => format!(
            "The Run {run_id} stopped: a filesystem it writes to filled. Free space, then start the Run again with its Run ID to resume it."
        ),
    };
    NeedsYouItem {
        id: format!("{RUN_STOPPED_BY_STORAGE}:{run_id}:{}", since.as_str()),
        kind: RUN_STOPPED_BY_STORAGE.to_owned(),
        since,
        run: Some(row.run_id.clone()),
        schedule: None,
        summary,
        actions: Vec::new(),
        details: details(folder.map_or_else(|| json!({}), |folder| json!({ "folder": folder }))),
    }
}

fn details(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
