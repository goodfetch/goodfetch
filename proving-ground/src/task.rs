//! A Proving ground task (ADR 0063): its entry in the task list, its expected
//! result, and the task list itself, each entry with its digest.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::digest;

/// One task of a family: what a tool is asked to do, and what the oracle
/// expects back.
pub(crate) struct Task {
    /// The host name the task's pages are served on, its own, so every
    /// request the Proving ground observes names its task.
    pub(crate) host: String,
    pub(crate) entry: Entry,
    pub(crate) expected: Expected,
}

/// A task's entry in the task list, without its digest.
#[derive(Serialize)]
pub(crate) struct Entry {
    /// `<family>/<ordinal>`; ordinals are append-only and never reused.
    pub(crate) id: String,
    pub(crate) family: &'static str,
    pub(crate) kind: Kind,
    /// The page the task starts on, with the port `serve` bound.
    pub(crate) start_url: String,
    pub(crate) description: &'static str,
}

/// What a task asks a tool to do.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// Follow links from the start page and return Records.
    Crawl,
    /// Read Records from the start page alone.
    Extract,
    /// Carry out an Effect once, through the start page's form.
    Submit,
}

/// A task's expected result, derived from the seed data only: the Records
/// a tool returns, and the Effects the Proving ground records it carried
/// out.
#[derive(Serialize)]
pub(crate) struct Expected {
    pub(crate) records: Vec<Record>,
    pub(crate) effects: Vec<Effect>,
}

/// An Effect a task asks a tool to carry out (ADR 0010), as the site
/// records it when it carries one out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Effect {
    /// An order of one product, placed through its order form.
    Order { product: String },
}

/// A crawl's misstep the Proving ground saw on a task's host, as it
/// records it (ADR 0025's Crawl cases): the oracle refutes a result whose
/// task has one, whatever its Records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Finding {
    /// A page fetched again, at the same URL or another of its URLs.
    Duplicate { page: String, target: String },
    /// A request to a crawler trap beyond what its task allows.
    Trap { target: String },
    /// A request for a link hidden from people.
    Honeypot { target: String },
    /// A request sent before the `Retry-After` of a `429` had passed.
    RetryAfterIgnored { target: String },
    /// A request sent while the site already had its cap of requests in
    /// flight.
    OverInFlightCap { target: String },
}

/// One expected Record: its fields by name, `id` among them, each as the page
/// shows it.
pub(crate) type Record = BTreeMap<&'static str, String>;

/// The task list `GET /tasks` serves (ADR 0063).
#[derive(Serialize)]
pub(crate) struct TaskList<'a> {
    proving_ground: About<'a>,
    tasks: Vec<Listed<'a>>,
}

#[derive(Serialize)]
struct About<'a> {
    version: &'a str,
    seed: u64,
}

#[derive(Serialize)]
struct Listed<'a> {
    #[serde(flatten)]
    entry: &'a Entry,
    digest: String,
}

impl<'a> TaskList<'a> {
    pub(crate) fn new(version: &'a str, seed: u64, tasks: &'a [Task]) -> Self {
        Self {
            proving_ground: About { version, seed },
            tasks: tasks
                .iter()
                .map(|task| Listed {
                    entry: &task.entry,
                    digest: digest::of(&task.entry, &task.expected),
                })
                .collect(),
        }
    }
}
