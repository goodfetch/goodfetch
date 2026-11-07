//! Politeness conformance cases for the AI-use signals (spec #581's seam:
//! the protocol against the Proving ground; ADRs 0016, 0086 and 0093):
//! crawls under `fetch.tier: http` on Foxhound's executable against the
//! `access-policy` family. A user who honours the signals and declares a
//! Purpose a signal refuses has the request end `skipped` with
//! `use_signals`, its handler never run: never sent when its host declared
//! the refusal before it, in robots.txt's `Content-Signal` line (task 3) or
//! `TDMRep`'s well-known file (task 4); closed unhandled when its own
//! response refuses it, in a `tdm-reservation` header (task 5) or in its
//! document's markup, a `TDMRep` `<meta>` or an RSL `Link` (the
//! `markup.access-policy.test` host). A user who does not honour them, or
//! whose Purpose is `search`, has every page handled.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::crawl_with;

/// The Proving ground's seed.
const SEED: u64 = 883;

/// The `access-policy` family's Content-Signal task's host, whose
/// robots.txt says `Content-Signal: … ai-train=no` to every crawler.
const CONTENT_SIGNAL_HOST: &str = "3.access-policy.test";

/// The `access-policy` family's TDMRep-file task's host, whose
/// `/.well-known/tdmrep.json` reserves `/catalog*`.
const TDMREP_FILE_HOST: &str = "4.access-policy.test";

/// The `access-policy` family's TDMRep-header task's host.
const TDMREP_HOST: &str = "5.access-policy.test";

/// The `access-policy` family's named host whose pages declare their
/// signals in markup: none on page 1, a `TDMRep` `<meta>` on page 2, an RSL
/// `Link` header on page 3 and an RSL `<link>` on page 4, the license
/// prohibiting every AI use.
const MARKUP_HOST: &str = "markup.access-policy.test";

/// What one crawl gave: the paths the handler was given, each outcome's
/// path, kind and skip reason, by path, and the paths the site received
/// that are no robots.txt or signal file.
struct Crawled {
    handled: Vec<String>,
    outcomes: Vec<(String, Value, Value)>,
    sent: Vec<String>,
}

/// Crawls `paths` of `host` under `site_preferences`.
fn crawl(host: &str, paths: &[&str], site_preferences: &Value) -> Crawled {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip().to_string();
    let settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { host: [ip] } },
        "egress": { "private_ranges": [host] },
        "site_preferences": site_preferences,
    });
    let seeds: Vec<Value> = paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url(host, path), "label": "page" }))
        .collect();
    let mut engine = attached(&env);
    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        "signals",
        &seeds,
        &settings,
        &mut |_, handle| {
            handled.push(path_of(&handle["crawl_request"]["url"]));
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "signals" }),
        &mut |_| None,
    );
    assert!(engine.finished());
    let mut outcomes: Vec<(String, Value, Value)> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                path_of(&outcome["request"]["url"]),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    handled.sort();
    let observations = proving_ground.observations().unwrap();
    let mut sent: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request["host"] == host)
        .filter_map(|request| request["target"].as_str().map(str::to_owned))
        .filter(|target| target.starts_with("/catalog"))
        .collect();
    sent.sort();
    sent.dedup();
    Crawled {
        handled,
        outcomes,
        sent,
    }
}

/// The path of `url`.
fn path_of(url: &Value) -> String {
    let rest = url.as_str().unwrap().split_once("://").unwrap().1;
    rest[rest.find('/').unwrap()..].to_owned()
}

/// `paths` as owned strings.
fn owned(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

/// With the signals honoured and the Purpose `ai-input`, the page whose
/// response reserves its text and data mining rights ends `skipped` with
/// `use_signals`, and its handler is never run.
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn a_page_whose_tdmrep_header_refuses_the_purpose_ends_skipped_with_use_signals() {
    let crawled = crawl(
        TDMREP_HOST,
        &["/catalog/1", "/catalog/2"],
        &json!({ "respect_use_signals": true, "purpose": "ai-input" }),
    );

    assert_eq!(
        (crawled.handled, crawled.outcomes),
        (
            owned(&["/catalog/1"]),
            vec![
                ("/catalog/1".to_owned(), json!("succeeded"), Value::Null),
                (
                    "/catalog/2".to_owned(),
                    json!("skipped"),
                    json!("use_signals")
                ),
            ],
        )
    );
}

/// With the signals honoured and the Purpose `ai-train`, a request whose
/// host's robots.txt says `Content-Signal: ai-train=no` is never sent: it
/// ends `skipped` with `use_signals`, its handler never run.
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn a_request_whose_robots_txt_content_signal_refuses_the_purpose_ends_skipped_unsent() {
    let crawled = crawl(
        CONTENT_SIGNAL_HOST,
        &["/catalog/1"],
        &json!({ "respect_use_signals": true, "purpose": "ai-train" }),
    );

    assert_eq!(
        (crawled.handled, crawled.outcomes, crawled.sent),
        (
            Vec::<String>::new(),
            vec![(
                "/catalog/1".to_owned(),
                json!("skipped"),
                json!("use_signals")
            )],
            Vec::<String>::new(),
        )
    );
}

/// With the signals honoured and the Purpose `collect`, text and data
/// mining, a request under a location the host's
/// `/.well-known/tdmrep.json` reserves is never sent: it ends `skipped`
/// with `use_signals`, its handler never run.
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn a_request_whose_tdmrep_file_reserves_it_ends_skipped_unsent() {
    let crawled = crawl(
        TDMREP_FILE_HOST,
        &["/catalog/1"],
        &json!({ "respect_use_signals": true, "purpose": "collect" }),
    );

    assert_eq!(
        (crawled.handled, crawled.outcomes, crawled.sent),
        (
            Vec::<String>::new(),
            vec![(
                "/catalog/1".to_owned(),
                json!("skipped"),
                json!("use_signals")
            )],
            Vec::<String>::new(),
        )
    );
}

/// With the signals honoured and the Purpose `ai-train`, a document whose
/// `TDMRep` `<meta>` reserves its rights, or whose RSL license, named by
/// a `Link` header or a `<link>` element, prohibits AI use, ends `skipped`
/// with `use_signals`, unhandled; the page declaring nothing is handled.
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn a_document_whose_markup_refuses_the_purpose_ends_skipped_unhandled() {
    let crawled = crawl(
        MARKUP_HOST,
        &["/catalog/1", "/catalog/2", "/catalog/3", "/catalog/4"],
        &json!({ "respect_use_signals": true, "purpose": "ai-train" }),
    );
    let skipped = |path: &str| (path.to_owned(), json!("skipped"), json!("use_signals"));

    assert_eq!(
        (crawled.handled, crawled.outcomes),
        (
            owned(&["/catalog/1"]),
            vec![
                ("/catalog/1".to_owned(), json!("succeeded"), Value::Null),
                skipped("/catalog/2"),
                skipped("/catalog/3"),
                skipped("/catalog/4"),
            ],
        )
    );
}

/// Every page of every host is sent and handled while the signals are not
/// honoured, the default, and while the Purpose is `search`, which no
/// signal is judged against (ADR 0086).
///
/// Covers: setting:site_preferences.respect_use_signals, setting:site_preferences.purpose
#[test]
fn every_page_is_handled_unless_the_signals_are_honoured_for_a_declared_purpose() {
    let hosts: [(&str, &[&str]); 4] = [
        (TDMREP_HOST, &["/catalog/1", "/catalog/2"]),
        (CONTENT_SIGNAL_HOST, &["/catalog/1"]),
        (TDMREP_FILE_HOST, &["/catalog/1"]),
        (
            MARKUP_HOST,
            &["/catalog/1", "/catalog/2", "/catalog/3", "/catalog/4"],
        ),
    ];
    for (host, paths) in hosts {
        for site_preferences in [
            json!({ "purpose": "ai-train" }),
            json!({ "respect_use_signals": true }),
        ] {
            let crawled = crawl(host, paths, &site_preferences);

            assert_eq!(
                (crawled.handled, crawled.sent),
                (owned(paths), owned(paths)),
                "{host} under {site_preferences}"
            );
        }
    }
}
