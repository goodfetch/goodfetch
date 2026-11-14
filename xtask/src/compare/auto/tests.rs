use serde_json::{Value, json};

use super::{Judged, Manifest, Read, Record, judged};

/// A Record of the rungs site.
fn record(id: &str, name: &str) -> Record {
    Record {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

/// A manifest of the root, a static page, a script-rendered page and a
/// blocked page, which holds no Record.
fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "pages": [
            { "path": "/", "kind": "root", "records": [] },
            { "path": "/static/1", "kind": "static", "number": 1,
              "records": [{ "id": "static-1", "name": "amber" }] },
            { "path": "/scripted/1", "kind": "scripted", "number": 1,
              "records": [{ "id": "scripted-1", "name": "birch" }] },
            { "path": "/blocked/4", "kind": "blocked", "number": 4, "records": [] },
        ]
    }))
    .unwrap()
}

/// Observations of requests for `targets` on `host`, none left
/// unrecorded.
fn observations(host: &str, targets: &[&str]) -> Value {
    json!({
        "requests": targets
            .iter()
            .map(|target| json!({ "host": host, "target": target, "status": 200 }))
            .collect::<Vec<_>>(),
        "unrecorded_requests": 0,
    })
}

fn read(url: &str, rendered: bool, records: Vec<Record>) -> Read {
    Read {
        url: url.to_owned(),
        rendered,
        records,
    }
}

/// The rungs site's requests the Proving ground saw for the manifest's
/// pages, and one for `robots.txt`.
fn every_page_requested() -> Value {
    observations(
        "rungs.localhost:4443",
        &["/robots.txt", "/", "/static/1", "/scripted/1", "/blocked/4"],
    )
}

/// A page whose Records a tool read match the manifest's is right, one it
/// read none on and that holds none included; each page counts once,
/// rendered when any of its reads was, its Records those of all its reads
/// together. Pages outside the manifest count for nothing, and the rate is
/// the manifest's pages over the crawl's seconds.
#[test]
fn a_page_is_right_when_its_reads_together_give_the_manifests_records() {
    let reads = [
        read("https://rungs.localhost:4443/", false, Vec::new()),
        read(
            "https://rungs.localhost:4443/static/1",
            false,
            vec![record("static-1", "amber")],
        ),
        read("https://rungs.localhost:4443/scripted/1", false, Vec::new()),
        read(
            "https://rungs.localhost:4443/scripted/1?again",
            true,
            vec![record("scripted-1", "birch")],
        ),
        read(
            "https://rungs.localhost:4443/elsewhere",
            true,
            vec![record("x", "y")],
        ),
    ];

    let judged = judged(&manifest(), &every_page_requested(), &reads, 2.0).unwrap();

    assert_eq!(
        (judged.clone(), judged.rate()),
        (
            Judged {
                seconds: 2.0,
                pages: 4,
                requested: 4,
                rendered: 1,
                wrong: 0,
            },
            2.0
        )
    );
}

/// A page is wrong when the Records a tool read on it differ from the
/// manifest's, and a page it never read counts as read with none.
#[test]
fn a_page_is_wrong_when_its_records_differ_from_the_manifests() {
    let reads = [
        read(
            "https://rungs.localhost:4443/static/1#top",
            false,
            vec![record("static-1", "ambre")],
        ),
        read("https://rungs.localhost:4443/blocked/4", true, Vec::new()),
    ];

    let judged = judged(&manifest(), &every_page_requested(), &reads, 2.0).unwrap();

    assert_eq!(
        judged,
        Judged {
            seconds: 2.0,
            pages: 4,
            requested: 4,
            rendered: 1,
            // The static page read wrong and the script-rendered page,
            // never read, hold Records the tool did not give.
            wrong: 2,
        }
    );
}

/// The same work is every page of the manifest requested on the rungs
/// site's host, as the Proving ground saw it: a page requested only on
/// another host, or not at all, is not.
#[test]
fn the_same_work_counts_the_manifests_pages_requested_on_the_rungs_host() {
    let mut seen = observations("rungs.localhost:4443", &["/", "/static/1", "/static/1"]);
    seen["requests"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "host": "elsewhere.localhost", "target": "/scripted/1", "status": 200 }));

    let judged = judged(&manifest(), &seen, &[], 1.0).unwrap();

    assert_eq!((judged.pages, judged.requested), (4, 2));
    assert!(!judged.same_work());
}

/// Observations that left requests unrecorded cannot show the same work.
#[test]
fn observations_that_left_requests_unrecorded_are_an_error() {
    let mut seen = observations("rungs.localhost:4443", &["/"]);
    seen["unrecorded_requests"] = json!(1);

    assert!(judged(&manifest(), &seen, &[], 1.0).is_err());
}
