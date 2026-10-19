//! A Proving ground task (ADR 0063): its entry in the task list, its expected
//! result, and the task list itself, each entry with its digest.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::digest;

/// One task of a family: what a tool is asked to do, and what the oracle
/// expects back.
pub(crate) struct Task {
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
    /// The account a task behind a login logs in with, derived from the seed
    /// so each Journey variant has its own (ADR 0063).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account: Option<Account>,
    /// How many time steps a watch task spans, from step 0: the steps its
    /// harness sets, one after another, while the tool watches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) time_steps: Option<u64>,
}

/// A login account of the task list.
#[derive(Serialize)]
pub(crate) struct Account {
    pub(crate) username: String,
    pub(crate) password: String,
    /// The account's authenticator secret in base32 (RFC 4648, unpadded),
    /// from which a tool computes its TOTP codes (RFC 6238); absent when the
    /// account has no MFA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) totp_secret: Option<String>,
}

/// What a task asks a tool to do.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// Follow links from the start page and return Records.
    Crawl,
    /// Read Records from the start page alone.
    Extract,
    /// Revisit the start page at each time step and return a Record per
    /// change seen.
    Watch,
}

/// A task's expected result, derived from the seed data only.
#[derive(Serialize)]
pub(crate) struct Expected {
    pub(crate) records: Vec<Record>,
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
