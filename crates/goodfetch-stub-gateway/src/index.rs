//! The gateway's own index of runs (ADR 0096): each run it started, by its
//! Run ID, with the engine that owns it, which is what `run.list` lists and
//! where every call naming the run goes. And the keys a start's settings
//! name, which the routing rule reads.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::engines::Engine;

/// The runs the gateway started, in the order it started them.
#[derive(Default)]
pub(crate) struct RunIndex {
    runs: Vec<(String, Arc<Engine>)>,
}

impl RunIndex {
    /// The engine that owns `run_id`, if the gateway started it.
    pub(crate) fn owner(&self, run_id: &str) -> Option<Arc<Engine>> {
        self.runs
            .iter()
            .find(|(indexed, _)| indexed == run_id)
            .map(|(_, engine)| Arc::clone(engine))
    }

    /// Records that `engine` owns `run_id`, once.
    pub(crate) fn insert(&mut self, run_id: &str, engine: &Arc<Engine>) {
        if self.owner(run_id).is_none() {
            self.runs.push((run_id.to_owned(), Arc::clone(engine)));
        }
    }

    /// Whether the gateway started `run_id`.
    pub(crate) fn holds(&self, run_id: &str) -> bool {
        self.owner(run_id).is_some()
    }

    /// Each engine that owns a run, once, in the order of their first run.
    pub(crate) fn engines(&self) -> Vec<Arc<Engine>> {
        let mut engines: Vec<Arc<Engine>> = Vec::new();
        for (_, engine) in &self.runs {
            if !engines.iter().any(|seen| seen.number() == engine.number()) {
                engines.push(Arc::clone(engine));
            }
        }
        engines
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
