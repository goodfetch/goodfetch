use std::collections::BTreeMap;
use std::convert::Infallible;

use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use url::Url;

use super::{Candidate, admit};

/// Where a test keeps its crawl's requests: each identity with its ID, the
/// IDs numbered from 1.
#[derive(Default)]
struct Requests(BTreeMap<String, String>);

impl Requests {
    fn admit(&mut self, candidate: Candidate<'_>) -> Enqueued {
        let added = admit(candidate, |request| {
            let next = (self.0.len() + 1).to_string();
            Ok::<_, Infallible>(match self.0.entry(request.identity.to_owned()) {
                std::collections::btree_map::Entry::Occupied(_) => None,
                std::collections::btree_map::Entry::Vacant(vacant) => {
                    Some(vacant.insert(next).clone())
                }
            })
        });
        match added {
            Ok(enqueued) => enqueued,
            Err(never) => match never {},
        }
    }
}

fn added(id: &str, url: &str) -> Enqueued {
    Enqueued::Added {
        id: id.to_owned(),
        url: url.to_owned(),
    }
}

fn not_added(reason: NotAddedReason, url: &str) -> Enqueued {
    Enqueued::NotAdded {
        reason,
        url: url.to_owned(),
    }
}

/// Covers: story:6-201, story:6-202, story:6-211
#[test]
fn each_seed_is_answered_added_with_its_id_or_not_added_with_its_reason() {
    let mut requests = Requests::default();
    let answers: Vec<Enqueued> = [
        "http://bücher.example/a",
        "http://xn--bcher-kva.example/a",
        "/a",
        "mailto:books@example.com",
        "http://0xc0.0.2.1/",
        "HTTP://192.0.2.1:80/",
        "http://",
    ]
    .into_iter()
    .map(|url| requests.admit(Candidate::Seed { url }))
    .collect();

    assert_eq!(
        answers,
        [
            added("1", "http://xn--bcher-kva.example/a"),
            not_added(NotAddedReason::Duplicate, "http://xn--bcher-kva.example/a"),
            not_added(NotAddedReason::InvalidUrl, "/a"),
            not_added(
                NotAddedReason::UnsupportedScheme,
                "mailto:books@example.com"
            ),
            added("2", "http://192.0.2.1/"),
            not_added(NotAddedReason::Duplicate, "HTTP://192.0.2.1:80/"),
            not_added(NotAddedReason::InvalidUrl, "http://"),
        ]
    );
}

/// Covers: story:6-231
#[test]
fn an_enqueue_resolves_against_its_documents_base_and_is_answered_as_a_seed_is() {
    let base = Url::parse("http://links.test/library/shelves/").unwrap();
    let mut requests = Requests::default();
    requests.admit(Candidate::Seed {
        url: "http://links.test/library/shelves/book/1",
    });

    let answers: Vec<Enqueued> = ["book/1", "book/2", "../book/2", "javascript:void(0)"]
        .into_iter()
        .map(|url| requests.admit(Candidate::Enqueue { url, base: &base }))
        .collect();

    assert_eq!(
        answers,
        [
            not_added(NotAddedReason::Duplicate, "book/1"),
            added("2", "http://links.test/library/shelves/book/2"),
            added("3", "http://links.test/library/book/2"),
            not_added(NotAddedReason::UnsupportedScheme, "javascript:void(0)"),
        ]
    );
}
