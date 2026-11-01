//! The gateway's own index of runs (ADR 0096): each run it started, by its
//! Run ID, with the engine that owns it, which is what `run.list` lists and
//! where every call naming the run goes. A start that names its Run ID
//! reserves it before it starts an engine, so a second start of that ID
//! joins the first's engine instead of starting another (`run.start` and
//! `crawl.start` are idempotent writes). And the keys a start's settings
//! name, which the routing rule reads.

use std::sync::Arc;

use serde_json::{Map, Value};
use tokio::sync::OnceCell;

use crate::engines::Engine;

/// A Run ID's place in the index: the engine that owns its run, once the
/// first start that named it has one.
pub(crate) type Slot = Arc<OnceCell<Arc<Engine>>>;

/// The runs the gateway started, in the order it started them.
#[derive(Default)]
pub(crate) struct RunIndex {
    runs: Vec<Indexed>,
}

/// A Run ID the index holds.
struct Indexed {
    run_id: String,
    slot: Slot,
    /// Whether a start of it answered with its run, which then stays.
    started: bool,
}

impl RunIndex {
    /// The engine that owns `run_id`, once a start of it has one.
    pub(crate) fn owner(&self, run_id: &str) -> Option<Arc<Engine>> {
        self.entry(run_id)
            .and_then(|indexed| indexed.slot.get())
            .map(Arc::clone)
    }

    /// The slot of `run_id`: the one a start already reserved, which this
    /// start joins, or a new one, which it fills.
    pub(crate) fn reserve(&mut self, run_id: &str) -> Slot {
        if let Some(indexed) = self.entry(run_id) {
            return Arc::clone(&indexed.slot);
        }
        let slot = Slot::default();
        self.runs.push(Indexed {
            run_id: run_id.to_owned(),
            slot: Arc::clone(&slot),
            started: false,
        });
        slot
    }

    /// Forgets `slot`, the reservation of `run_id` by a start that started
    /// no run, unless a start of it did or another start still holds it.
    pub(crate) fn release(&mut self, run_id: &str, slot: &Slot) {
        // The index holds one reference and the releasing start another;
        // a start that joined holds a third, taken under the index's lock.
        let joined = Arc::strong_count(slot) > 2;
        self.runs.retain(|indexed| {
            joined
                || indexed.started
                || indexed.run_id != run_id
                || !Arc::ptr_eq(&indexed.slot, slot)
        });
    }

    /// Records that `engine` owns `run_id`, whose start answered with its
    /// run, unless a start already gave it an engine.
    pub(crate) fn insert(&mut self, run_id: &str, engine: &Arc<Engine>) {
        let _ = self.reserve(run_id).set(Arc::clone(engine));
        if let Some(indexed) = self
            .runs
            .iter_mut()
            .find(|indexed| indexed.run_id == run_id)
        {
            indexed.started = true;
        }
    }

    /// Whether the gateway started `run_id`.
    pub(crate) fn holds(&self, run_id: &str) -> bool {
        self.entry(run_id).is_some_and(|indexed| indexed.started)
    }

    /// Each engine that owns a run, once, in the order of their first run.
    pub(crate) fn engines(&self) -> Vec<Arc<Engine>> {
        let mut engines: Vec<Arc<Engine>> = Vec::new();
        for engine in self
            .runs
            .iter()
            .filter(|indexed| indexed.started)
            .filter_map(|indexed| indexed.slot.get())
        {
            if !engines.iter().any(|seen| seen.number() == engine.number()) {
                engines.push(Arc::clone(engine));
            }
        }
        engines
    }

    fn entry(&self, run_id: &str) -> Option<&Indexed> {
        self.runs.iter().find(|indexed| indexed.run_id == run_id)
    }
}

/// Every key `settings` names, nested by key as a start carries them, with
/// each of its prefixes: `{"crawl": {"max_requests": 10}}` names `crawl`
/// and `crawl.max_requests`, so a setting whose value is itself a map is
/// named too.
pub(crate) fn setting_keys(settings: &Map<String, Value>) -> Vec<String> {
    let mut keys = Vec::new();
    let mut stack: Vec<(String, &Map<String, Value>)> = vec![(String::new(), settings)];
    while let Some((prefix, map)) = stack.pop() {
        for (name, value) in map {
            let key = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            if let Value::Object(nested) = value {
                stack.push((key.clone(), nested));
            }
            keys.push(key);
        }
    }
    keys
}

#[cfg(test)]
mod tests;
