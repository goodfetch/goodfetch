//! A Proving ground task (ADR 0063): its entry in the task list, its expected
//! result, and the task list itself, each entry with its digest.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::digest;

/// The origin a task's start URL begins with: `http://<host>:<port>` as
/// `serve` lists it, or `http://<host>` when `tasks` lists it binding no port.
pub(crate) fn origin(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) => format!("http://{host}:{port}"),
        None => format!("http://{host}"),
    }
}

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
    /// The account a task behind a login logs in with, derived from the seed
    /// so each Journey variant has its own (ADR 0063).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account: Option<Account>,
    /// How many time steps a task spans, from step 0: the steps its harness
    /// sets, one after another, while the tool works on its pages.
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
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// Follow links from the start page and return Records.
    Crawl,
    /// Read Records from the start page alone.
    Extract,
    /// Carry out an Effect once, through the start page's form.
    Submit,
    /// Revisit the start page at each time step and return a Record per
    /// change seen.
    Watch,
    /// Carry out one action on the start page, such as a hover, a drag or a
    /// key press, which the page's own script reports as an Effect; or
    /// refuse it when its target is not the one the task names.
    Act,
    /// Download the file the start page offers and return its bytes.
    Download,
}

/// A task's expected result, derived from the seed data only or written by
/// hand: the Records a tool returns, the Effects the Proving ground records
/// it carried out, the files it downloads, and whether the task must be
/// refused.
#[derive(Default, Serialize)]
pub(crate) struct Expected {
    pub(crate) records: Vec<Record>,
    pub(crate) effects: Vec<Effect>,
    /// Records a result may hold or leave out, such as rows a table gained
    /// or lost while the tool read it, each correct when it is there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tolerated: Vec<Record>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) files: Vec<File>,
    /// Whether the only right result is a refusal: no Effect, and a tool that
    /// reports a failure, as when the control under the pointer is not the
    /// task's.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) refusal: bool,
}

impl Expected {
    /// The expected Records alone.
    pub(crate) fn records(records: Vec<Record>) -> Self {
        Self {
            records,
            ..Self::default()
        }
    }

    /// The expected Effects alone.
    pub(crate) fn effects(effects: Vec<Effect>) -> Self {
        Self {
            effects,
            ..Self::default()
        }
    }
}

/// A file a task asks a tool to download: its name and the SHA-256 of its
/// bytes, in lowercase hex.
#[derive(Clone, Serialize)]
pub(crate) struct File {
    pub(crate) name: String,
    pub(crate) sha256: String,
}

/// An Effect a task asks a tool to carry out (ADR 0010), as the site
/// records it when it carries one out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Effect {
    /// An order of one product, placed through its order form.
    Order { product: String },
    /// A sign-in that opened a new session on a site behind a login form.
    SignIn,
    /// An event a hand-made page's script reports when an action reaches one
    /// of its controls: the event, the control's ID, whether the browser
    /// marked the event trusted (`isTrusted`), and what the page read, such
    /// as a field's value.
    Event {
        event: String,
        target: String,
        trusted: bool,
        #[serde(skip_serializing_if = "BTreeMap::is_empty")]
        details: BTreeMap<String, String>,
    },
    /// A demo booked through the Agent page's form, at the time step the
    /// Proving ground received it.
    Booking {
        time_step: u64,
        name: String,
        email: String,
        plan: String,
        /// Whether the form's consent box was ticked.
        consent: bool,
        /// Whether the submit came from a trusted (user-agent) event.
        trusted: bool,
    },
    /// The Agent page's account deleted through its delete control, at the
    /// time step the Proving ground received it.
    DeleteAccount { time_step: u64, trusted: bool },
}

/// What a hand-made page's script posts when an action reaches one of its
/// controls, before the site records it as an [`Effect::Event`].
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reported {
    event: String,
    target: String,
    trusted: bool,
    #[serde(default)]
    details: BTreeMap<String, String>,
}

impl From<Reported> for Effect {
    fn from(reported: Reported) -> Self {
        Self::Event {
            event: reported.event,
            target: reported.target,
            trusted: reported.trusted,
            details: reported.details,
        }
    }
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

/// The host name of task `ordinal` of the family served on `family_host`,
/// on which the task's pages are served: `1.adversarial.test`.
pub(crate) fn task_host(ordinal: u32, family_host: &str) -> String {
    format!("{ordinal}.{family_host}")
}

/// Every host name of a family with `tasks` tasks, each with its task:
/// `family_host` itself, and each task's.
pub(crate) fn family_hosts(
    family: &str,
    family_host: &str,
    tasks: u32,
) -> Vec<(String, Option<String>)> {
    let mut hosts = vec![(family_host.to_owned(), None)];
    hosts.extend((1..=tasks).map(|ordinal| {
        (
            task_host(ordinal, family_host),
            Some(format!("{family}/{ordinal}")),
        )
    }));
    hosts
}

/// Whether `host`, a host name without its port, is `family_host` or the
/// host name of one of its family's first `tasks` tasks.
pub(crate) fn serves(host: &str, family_host: &str, tasks: u32) -> bool {
    host == family_host
        || host
            .strip_suffix(family_host)
            .and_then(|prefix| prefix.strip_suffix('.'))
            .filter(|ordinal| !ordinal.starts_with('0'))
            .and_then(|ordinal| ordinal.parse::<u32>().ok())
            .is_some_and(|ordinal| (1..=tasks).contains(&ordinal))
}
