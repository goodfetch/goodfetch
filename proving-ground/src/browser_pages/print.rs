//! The `print` family: a report (`/report`) whose stylesheet
//! (`/report.css`) shows some entries on screen only and others in print
//! only, with a seal (`/seal.svg`) both show. Each task asks for the page
//! saved another way: as it prints, as the screen shows it, and as one
//! MHTML file, whose parts are the page, its stylesheet and its image.

use std::fmt::Write as _;

use hyper::StatusCode;

use super::document;
use crate::exchange::Reply;
use crate::generator;
use crate::task::{self, Entry, Expected, Kind, Record, Task};

pub(super) const HOST: &str = "print.test";
pub(super) const FAMILY: &str = "print";
pub(super) const TASKS: u32 = 3;

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 23;

const ADJECTIVES: [&str; 8] = [
    "Coastal", "Eastern", "Highland", "Inland", "Northern", "Southern", "Upland", "Western",
];
const NOUNS: [&str; 8] = [
    "Harvest",
    "Freight",
    "Rainfall",
    "Orchards",
    "Ferries",
    "Quarries",
    "Fisheries",
    "Mills",
];

/// Where each entry is shown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shown {
    Both,
    Screen,
    Print,
}

/// The report's entries, in order, each with where it is shown.
const ENTRIES: [Shown; 6] = [
    Shown::Both,
    Shown::Screen,
    Shown::Print,
    Shown::Both,
    Shown::Print,
    Shown::Screen,
];

const STYLESHEET: &str = "body { font-family: sans-serif; color: #111827; background: #ffffff; }\n\
@media screen {\n  .print-only { display: none; }\n}\n\
@media print {\n  .screen-only { display: none; }\n}\n";

const SEAL: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\" viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"28\" fill=\"#1d4ed8\"/><circle cx=\"32\" cy=\"32\" r=\"18\" fill=\"#ffffff\"/></svg>\n";

/// Entry `number`'s ID and title for `seed`.
fn entry(seed: u64, number: u64) -> (String, String) {
    let draw = |place, bound: usize| {
        let value = generator::below(seed, &[FAMILY_PLACE, number, place], bound as u64);
        usize::try_from(value).expect("invariant: a word index is below 8")
    };
    (
        format!("P-{number}"),
        format!(
            "{} {}",
            ADJECTIVES[draw(1, ADJECTIVES.len())],
            NOUNS[draw(2, NOUNS.len())]
        ),
    )
}

/// The report's entries for `seed`: each one's ID, title and where it is
/// shown.
fn entries(seed: u64) -> impl Iterator<Item = (String, String, Shown)> {
    (1..).zip(ENTRIES).map(move |(number, shown)| {
        let (id, title) = entry(seed, number);
        (id, title, shown)
    })
}

/// A `GET` of `path` on one of the family's task hosts.
pub(super) fn page(seed: u64, path: &str) -> Option<Reply> {
    let reply = |content_type, body: &str| Reply {
        status: StatusCode::OK,
        content_type,
        headers: Vec::new(),
        body: body.to_owned().into(),
    };
    match path {
        "/report" => Some(Reply::html(report(seed))),
        "/report.css" => Some(reply("text/css; charset=utf-8", STYLESHEET)),
        "/seal.svg" => Some(reply("image/svg+xml", SEAL)),
        _ => None,
    }
}

fn report(seed: u64) -> String {
    let mut main = String::from(
        "<h1>Regional report</h1>\n<img src=\"/seal.svg\" alt=\"The report's seal\" width=\"64\" height=\"64\">\n",
    );
    for (id, title, shown) in entries(seed) {
        let class = match shown {
            Shown::Both => "entry",
            Shown::Screen => "entry screen-only",
            Shown::Print => "entry print-only",
        };
        write!(
            main,
            "<section class=\"{class}\" data-id=\"{id}\">\n<h2 class=\"title\">{title}</h2>\n<p>Entry {id} of the report.</p>\n</section>\n"
        )
        .expect("invariant: writing to a String succeeds");
    }
    document(
        "Regional report",
        "<link rel=\"stylesheet\" href=\"/report.css\">\n",
        &main,
        "",
    )
}

/// The family's tasks for `seed`. Ordinals are append-only (ADR 0063).
pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let shown_in = |medium: Shown| {
        entries(seed)
            .filter(|(_, _, shown)| *shown == Shown::Both || *shown == medium)
            .map(|(id, title, _)| Record::from([("id", id), ("title", title)]))
            .collect()
    };
    let task = |ordinal: u32, description: &'static str, records: Vec<Record>| {
        let host = task::task_host(ordinal, HOST);
        Task {
            entry: Entry {
                id: format!("{FAMILY}/{ordinal}"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}/report", task::origin(&host, port)),
                description,
                account: None,
                time_steps: None,
            },
            host,
            expected: Expected::records(records),
        }
    };
    let archived = |path: &str| Record::from([("id", path.to_owned())]);
    vec![
        task(
            1,
            "Save this page as it prints (print media, as a PDF of it holds it) and return one \
             Record for each entry the saved page shows, with the fields `id` (the entry's \
             `data-id`) and `title`.",
            shown_in(Shown::Print),
        ),
        task(
            2,
            "Save this page as the screen shows it (screen media) and return one Record for \
             each entry the saved page shows, with the fields `id` (the entry's `data-id`) and \
             `title`.",
            shown_in(Shown::Screen),
        ),
        task(
            3,
            "Save this page as one MHTML file and return one Record for each part the file \
             holds, with the field `id`: the path of the part's `Content-Location`, without its \
             scheme, host and port.",
            vec![
                archived("/report"),
                archived("/report.css"),
                archived("/seal.svg"),
            ],
        ),
    ]
}
