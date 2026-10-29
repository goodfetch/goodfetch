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

/// [`origin`] over TLS, which the same address serves: `https://<host>:<port>`,
/// or `https://<host>` binding no port.
pub(crate) fn secure_origin(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
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
    /// The error a refusal must report, when only one is right.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ExpectedError>,
    /// The path prefixes the task must never request, such as those its
    /// site's robots.txt disallows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) forbidden_paths: Vec<&'static str>,
    /// The robots.txt the task's site serves, which judges each request of
    /// the task by its `User-Agent` (RFC 9309).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) robots_txt: Option<&'static str>,
    /// Whether every request of the task must carry a Web Bot Auth signature
    /// its agent's key directory verifies (`true`), or none may be signed
    /// (`false`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) signed: Option<bool>,
    /// The site of a task whose Records hold a page's formats (ADR 0007;
    /// spec #580): its `markdown` and `markdown_*` fields are Markdown,
    /// judged by structure, each link on it by its path; its `html_*` fields
    /// HTML, judged by the text of its blocks; its `text` field text, judged
    /// white space aside.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) markdown_site: Option<&'static str>,
    /// A decompression bomb's declared sizes, present on a `bombs` task:
    /// the codings its body carries and the bytes it sends compressed
    /// against the bytes it would decode to, held in the task's expected
    /// result, which its digest hashes, so a refusal at the ratio limit, not
    /// the decoded content, is the only right result (ADR 0067).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bomb: Option<Bomb>,
}

/// A decompression bomb's declared sizes (ADR 0067): a small compressed body
/// that decodes to far more, which a correct tool refuses at the engine's
/// decompression-ratio limit rather than reading to its end.
#[derive(Clone, Serialize)]
pub(crate) struct Bomb {
    /// The `Content-Encoding` codings the body carries, in the order they
    /// were applied (`["gzip"]`, or `["gzip", "gzip"]` for a double coding).
    pub(crate) content_encodings: Vec<&'static str>,
    /// The bytes the page sends, compressed.
    pub(crate) compressed_bytes: u64,
    /// The bytes the body decodes to if fully decompressed.
    pub(crate) expanded_bytes: u64,
}

/// The error a refusal must report: its Error code and, for a navigation
/// that failed, the browser's network error, or, for an egress refusal, the
/// rule that refused it (ADR 0067).
#[derive(Clone, Default, Serialize)]
pub(crate) struct ExpectedError {
    pub(crate) code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rule: Option<&'static str>,
    /// The engine setting whose limit a `response_limit_exceeded` names, such
    /// as `network.max_decompression_ratio` for a decompression bomb
    /// (ADR 0067).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) setting: Option<&'static str>,
}

impl Expected {
    /// A refusal alone: nothing carried out, and a reported failure that
    /// names `error` when one is given.
    pub(crate) fn refused(error: Option<ExpectedError>) -> Self {
        Self {
            refusal: true,
            error,
            ..Self::default()
        }
    }

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
    /// The name the tool saves it under; for a file whose naming is
    /// [`Naming::Safe`], the hostile name the page suggests.
    pub(crate) name: String,
    pub(crate) sha256: String,
    #[serde(skip_serializing_if = "Naming::is_exact")]
    pub(crate) naming: Naming,
}

/// How the oracle judges the name a tool saved a file under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Naming {
    /// The name is exactly the expected one.
    #[default]
    Exact,
    /// The page suggests a hostile name (a path, a reserved name, a control
    /// character): any name is right that is one safe file name on every OS,
    /// so the file stays in the folder it is saved to (ADR 0067).
    Safe,
}

impl Naming {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `skip_serializing_if` passes the field by reference"
    )]
    fn is_exact(&self) -> bool {
        *self == Self::Exact
    }
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
    /// A request for what a relative link names when resolved against the
    /// wrong base: the URL before a redirect, or the document's URL where a
    /// `<base href>` names another (pitfall U08).
    WrongBase { target: String },
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
