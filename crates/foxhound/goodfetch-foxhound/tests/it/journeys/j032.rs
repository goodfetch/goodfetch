//! J-032, "Crawl a site whose certificate is invalid by listing its domain
//! for trust on first use, and see the crawl stop when the key changes
//! until I accept it", on the `cli` surface (ADRs 0042, 0063 and 0075;
//! spec #565, ticket #680): a program of the person's, a Client speaking
//! the protocol to Foxhound's shipped executable as an Attached engine,
//! crawls the Proving ground's expired host on the `http` tier, listing
//! `certificates.test` for trust on first use in the crawl's settings. A
//! second Proving ground, on another seed, then serves the same host with
//! another key; the person accepts it with `certs accept` in a
//! pseudo-terminal, and the next crawl fetches the host again. The result
//! document is read back as every reader reads it.

use std::path::Path;

use goodfetch_protocol::journey::Journey;
use goodfetch_protocol::result_document::{
    LoadedDocument, ResultDocument, read, read_journey_document,
};
use goodfetch_test_client::Attached;
use goodfetch_test_support::{JourneyRun, ProvingGround, ReceivedRequests, pty};
use serde_json::{Value, json};

use crate::attached_spawned;
use crate::fetch_tiers::settings;
use crate::frontier::crawl_from;
use crate::provenance::{push, records};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The site: the Proving ground's host whose certificate expired.
const HOST: &str = "expired.certificates.test";

/// The seed of the Proving ground that serves the site under another key,
/// which no Journey uses.
const CHANGED_SEED: u64 = 1032;

/// The site's root on `proving_ground`.
fn site(proving_ground: &ProvingGround) -> String {
    format!("https://{HOST}:{}/", proving_ground.address().port())
}

/// What one crawl of the site left: its request's outcome, as its kind and
/// error codes, and its Records' Provenance marks.
#[derive(Debug, Default)]
struct Crawled {
    outcome: Value,
    marks: Vec<Value>,
}

/// Crawls the site on `proving_ground` as the Run `run_id` from a program
/// driving `engine`, on the `http` tier, listing `certificates.test` for
/// trust on first use; its handler pushes one Record naming the URL.
fn crawl(engine: &mut Attached, proving_ground: &ProvingGround, run_id: &str) -> Crawled {
    let finished = crawl_from(
        engine,
        &json!({
            "labels": { "page": {} },
            "run_id": run_id,
            "seeds": [{ "url": site(proving_ground), "label": "page" }],
            "settings": settings(proving_ground, &json!({
                "fetch": { "tier": "http" },
                "politeness": { "domain": { "delay": 0 } },
                "site_preferences": { "respect_robots_txt": false },
                "tls": { "accept_invalid_certs": { "certificates.test": null } },
            })),
            "datasets": [{ "name": "pages" }],
        }),
        &mut |inside, handle| {
            let url = handle["crawl_request"]["url"].clone();
            let pushed = push(inside, handle, "pages", &json!({ "url": url }));
            assert!(pushed.get("result").is_some(), "{pushed}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let Some(outcome) = read["result"]["outcomes"]
        .as_array()
        .and_then(|outcomes| outcomes.first())
    else {
        panic!("the crawl's one outcome: {read}");
    };
    let outcome = json!({
        "kind": outcome["kind"],
        "errors": outcome["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|error| error["code"].clone())
            .collect::<Vec<_>>(),
    });
    let marks = records(engine, run_id, "pages")
        .iter()
        .map(|record| record["provenance"]["certificate_exception"].clone())
        .collect();
    Crawled { outcome, marks }
}

/// The requests `proving_ground` received for the site.
fn requests_to_the_site(proving_ground: &ProvingGround) -> usize {
    ReceivedRequests::of(&proving_ground.observations().unwrap())
        .every()
        .filter(|request| request.host == HOST)
        .count()
}

/// A Record's mark of trust on first use, with the key it carries.
fn pinned_key(crawled: &Crawled) -> Option<String> {
    match crawled.marks.as_slice() {
        [mark] if mark["kind"] == "trust_on_first_use" => {
            mark["spki_sha256"].as_str().map(str::to_owned)
        }
        _ => None,
    }
}

/// Covers: story:4-35, story:4-36, story:4-104, row:SC-06, command:certs.accept
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-032", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-032's cli variant starts");
    let mut engine = {
        let mut command = run.env().process_command(run.engine());
        command.arg("attached-engine");
        attached_spawned(run.env(), None, &mut command)
    };
    let mut changed_server: Option<ProvingGround> = None;
    let mut first = Crawled::default();
    let mut changed = Crawled::default();
    let mut changed_requests = usize::MAX;
    let mut accepted = Crawled::default();

    run.step(
        "Crawl the site from a program, listing its domain for trust on first use in the crawl's settings",
        |run| {
            let proving_ground = run.proving_ground().expect("J-032 has a Proving ground");
            first = crawl(&mut engine, proving_ground, "first-use");
        },
    );
    run.step(
        "Crawl the site the same way once its server presents another key",
        |run| {
            let server = ProvingGround::start(run.env(), CHANGED_SEED)
                .expect("the server with another key starts");
            changed = crawl(&mut engine, &server, "changed-key");
            changed_requests = requests_to_the_site(&server);
            changed_server = Some(server);
        },
    );
    run.step(
        "Accept the site's new key in a terminal (`certs accept`)",
        |run| {
            let mut accept = pty(run.env(), run.engine())
                .args(["certs", "accept", HOST])
                .spawn()
                .unwrap();
            accept.expect("[y/N]").unwrap();
            accept.send_line("y").unwrap();
            let answered = accept.finish().unwrap();
            assert_eq!(answered.exit, Some(0), "{}", answered.stripped());
        },
    );
    run.step("Crawl the site the same way again", |_| {
        let server = changed_server
            .as_ref()
            .expect("the server with another key runs");
        accepted = crawl(&mut engine, server, "accepted");
    });

    run.check(
        "The first crawl fetched the site through its exception, and its Record's Provenance carries the exception and the key it pinned",
        |_| {
            assert_eq!(
                (&first.outcome, pinned_key(&first).is_some()),
                (&json!({ "kind": "succeeded", "errors": [] }), true),
                "{first:?}"
            );
        },
    );
    run.check(
        "The crawl after the key changed failed the site's request with `certificate_changed`, and no request reached the site",
        |_| {
            assert_eq!(
                (&changed.outcome, changed.marks.len(), changed_requests),
                (
                    &json!({ "kind": "failed", "errors": ["certificate_changed"] }),
                    0,
                    0
                ),
                "{changed:?}"
            );
        },
    );
    run.check(
        "Once the new key was accepted, the crawl fetched the site under the new key",
        |_| {
            let (old, new) = (pinned_key(&first), pinned_key(&accepted));
            assert_eq!(
                (&accepted.outcome, new.is_some(), new != old),
                (&json!({ "kind": "succeeded", "errors": [] }), true, true),
                "{accepted:?}"
            );
        },
    );
    assert!(engine.finished(), "the Attached engine ends cleanly");
    drop(changed_server);

    let finished = run.finish().expect("J-032's document is written");
    finished.assert_proved();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test reads the written document and the Journey's file as every reader does"
    )]
    let (bytes, journey) = (
        std::fs::read(finished.path()).unwrap(),
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../journeys/J-032.toml"),
        )
        .unwrap(),
    );
    let journey: Journey = toml::from_str(&journey).unwrap();
    let LoadedDocument::Journey(loaded) = read(&bytes).unwrap() else {
        panic!("J-032's document is a journey document");
    };
    read_journey_document(loaded, &journey).expect("the reader accepts J-032's document");
}
