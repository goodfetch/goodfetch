//! The replay proxy, ported from the lab's: a frozen archive of real pages,
//! each served at its original host and path, to browsers that send every
//! request through the Proving ground as their proxy. Plain HTTP arrives in
//! absolute form; HTTPS arrives as a `CONNECT` to an archived host's port
//! 443, which the Proving ground terminates with a leaf certificate of its
//! seeded authority, since browsers load HSTS-preloaded hosts only over
//! HTTPS. Every other request is refused and recorded: one outside the
//! archive, a `CONNECT` to another host or port, a page the archive lacks.
//! It stays a page proxy, never a model recorder (ADR 0065).
//!
//! Each archived page is a file of at most 256 KiB in `fixtures/archive/`
//! with a `.source.toml` naming how it was captured (ADR 0062), embedded in
//! the binary so the Proving ground stays one file. The family's tasks
//! extract each page's headline and author, their expected values written
//! by hand from the page as served and WCXB's annotation (CC-BY-4.0).

use serde::Serialize;

use crate::sync::Mutex;
use crate::task::{Entry, Expected, Kind, Record, Task};

const FAMILY: &str = "replay";

/// One archived page.
struct Page {
    host: &'static str,
    /// The path and query it was captured at.
    path: &'static str,
    html: &'static str,
    /// Its headline, as the page shows it.
    title: &'static str,
    /// Its author, as WCXB's annotation names it.
    author: &'static str,
}

/// The archive, in task order: `replay/1` is its first page. Append only.
const ARCHIVE: [Page; 2] = [
    Page {
        host: "blog.rust-lang.org",
        path: "/2025/12/19/what-do-people-love-about-rust",
        html: include_str!("../fixtures/archive/wcxb-0379.html"),
        title: "What do people love about Rust?",
        author: "Niko Matsakis",
    },
    Page {
        host: "www.fda.gov",
        path: "/drugs/postmarket-drug-safety-information-patients-and-providers/fdas-concerns-unapproved-glp-1-drugs-used-weight-loss",
        html: include_str!("../fixtures/archive/wcxb-0064.html"),
        title: "FDA\u{2019}s Concerns with Unapproved GLP-1 Drugs Used for Weight Loss",
        author: "Center for Drug Evaluation and Research",
    },
];

/// The port a `CONNECT` to an archived host must name.
const HTTPS_PORT: &str = "443";

/// Whether `host` is an archived page's host.
pub(crate) fn serves(host: &str) -> bool {
    ARCHIVE.iter().any(|page| page.host == host)
}

/// The archived page at `host` and `target` (path and query), if any.
pub(crate) fn page(host: &str, target: &str) -> Option<&'static str> {
    ARCHIVE
        .iter()
        .find(|page| page.host == host && page.path == target)
        .map(|page| page.html)
}

/// Whether a `CONNECT` to `authority` reaches the archive: an archived
/// host's port 443.
pub(crate) fn tunnels(authority: &str) -> bool {
    authority
        .rsplit_once(':')
        .is_some_and(|(host, port)| port == HTTPS_PORT && serves(host))
}

/// The family's tasks: each extracts one archived page's headline and
/// author. Their start URLs are the pages' original URLs over HTTP, with no
/// port: a tool reaches them through the Proving ground as its proxy.
pub(crate) fn tasks() -> Vec<Task> {
    ARCHIVE
        .iter()
        .zip(1_u32..)
        .map(|(page, ordinal)| {
            let start_url = format!("http://{}{}", page.host, page.path);
            Task {
                host: page.host.to_owned(),
                entry: Entry {
                    id: format!("{FAMILY}/{ordinal}"),
                    family: FAMILY,
                    kind: Kind::Extract,
                    start_url: start_url.clone(),
                    description: "Through the Proving ground as your proxy, extract the article \
                        this frozen page shows as one Record with the fields `id` (the start URL), \
                        `title` (its headline, as the page shows it) and `author`.",
                    time_steps: None,
                },
                expected: Expected {
                    records: vec![Record::from([
                        ("id", start_url),
                        ("title", page.title.to_owned()),
                        ("author", page.author.to_owned()),
                    ])],
                    effects: Vec::new(),
                },
            }
        })
        .collect()
}

/// One request the replay proxy refused.
#[derive(Serialize)]
struct Refusal {
    method: String,
    /// The target as the request named it: an absolute URL, a `CONNECT`
    /// authority, or an archived host's path.
    target: String,
}

/// The requests the replay proxy refused, in order.
#[derive(Default)]
pub(crate) struct Refusals(Mutex<Vec<Refusal>>);

impl Refusals {
    pub(crate) fn record(&self, method: &str, target: String) {
        self.0.lock().push(Refusal {
            method: method.to_owned(),
            target,
        });
    }

    /// The refusals as JSON: `{"refused": […]}`.
    pub(crate) fn document(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "refused": &*self.0.lock() }))
            .expect("invariant: the refusals serialize")
    }
}
