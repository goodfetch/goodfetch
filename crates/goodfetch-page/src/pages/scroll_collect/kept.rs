//! What a collection keeps and when it ends: its bounds, from its call or
//! by default, the rows it kept by key, and what the list declares.

use std::collections::HashSet;
use std::time::Duration;

use goodfetch_engine::{CallError, named};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::functions::{Declared, LocatorScrollCollectParams};
use serde_json::Value;

use crate::page_functions::Collected;

/// The rows at which a collection ends when its call names none.
const MAX_ROWS: u64 = 10_000;

/// The steps in a row without a new key at which a collection ends when
/// its call names none.
const IDLE_STEPS: u64 = 3;

/// How long a step waits for a new key when its call names no wait.
const STEP_WAIT: Duration = Duration::from_millis(2_000);

/// When a collection ends, as its call gave it or by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Bounds {
    pub(super) max_rows: u64,
    pub(super) idle_steps: u64,
    pub(super) step_wait: Duration,
}

impl Bounds {
    /// The bounds `params` give, each 0 refused as `invalid_params`.
    pub(super) fn of(params: &LocatorScrollCollectParams) -> Result<Self, CallError> {
        Ok(Self {
            max_rows: at_least_one("max_rows", "rows", params.max_rows)?.unwrap_or(MAX_ROWS),
            idle_steps: at_least_one("idle_steps", "steps", params.idle_steps)?
                .unwrap_or(IDLE_STEPS),
            step_wait: at_least_one("step_wait", "ms", params.step_wait)?
                .map_or(STEP_WAIT, Duration::from_millis),
        })
    }
}

/// `value`, or `invalid_params` naming the parameter `name` as its Client
/// passes it when it is 0.
fn at_least_one(name: &str, unit: &str, value: Option<u64>) -> Result<Option<u64>, CallError> {
    match value {
        Some(0) => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{}` is 0 {unit}: it is at least 1", named::parameter(name)),
        )),
        other => Ok(other),
    }
}

/// What a collection kept so far: each new key's value in first-seen
/// order, the keys seen, and the last one kept, which its
/// `deadline_exceeded` names.
#[derive(Default)]
pub(super) struct Kept {
    pub(super) rows: Vec<Value>,
    keys: HashSet<String>,
    last: Option<String>,
}

impl Kept {
    /// Keeps the rows of `collected` whose key is new, up to `max_rows`
    /// rows in all; answers how many it kept.
    pub(super) fn keep(&mut self, collected: Collected, max_rows: u64) -> usize {
        let mut added = 0;
        for row in collected.rows {
            if self.full(max_rows) {
                break;
            }
            if self.keys.insert(row.key.clone()) {
                self.rows.push(row.value);
                self.last = Some(row.key);
                added += 1;
            }
        }
        added
    }

    /// Whether it holds `max_rows` rows.
    pub(super) fn full(&self, max_rows: u64) -> bool {
        u64::try_from(self.rows.len()).unwrap_or(u64::MAX) >= max_rows
    }

    /// What a `deadline_exceeded` says the collection kept.
    pub(super) fn observed(&self) -> String {
        match &self.last {
            Some(key) => format!("{} rows kept, the last key `{key}`", self.rows.len()),
            None => "no rows kept".to_owned(),
        }
    }
}

/// The rows a list declares, from what `collect.js` read: `-1` says it
/// does not know; another negative is no declaration.
pub(super) fn declared(read: Option<i64>) -> Option<Declared> {
    match read {
        Some(-1) => Some(Declared::Unknown),
        Some(rows) => u64::try_from(rows).ok().map(Declared::Rows),
        None => None,
    }
}

#[cfg(test)]
mod tests;
