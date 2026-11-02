use serde_json::json;

use super::{check, work_of};
use crate::compare_spider::trials::{Comparison, Declared, InstructionCell, Machine, Tool, Trial};

/// The paths of the site of `size` pages.
fn site(size: u64) -> Vec<String> {
    (1..=size)
        .map(|page| format!("/sites/{size}/{page}"))
        .collect()
}

/// A trial of `tool` that fetched `pages`, each served 600 bytes.
fn trial(tool: Tool, number: u32, pages: Vec<String>) -> Trial {
    let requests = u64::try_from(pages.len()).unwrap();
    Trial {
        tool,
        number,
        seconds: 1.0,
        peak_bytes: 1,
        pages,
        requests,
        bytes: 600 * requests,
    }
}

/// Three trials of each tool over the 4-page site, `trials` in their
/// order.
fn comparison(trials: Vec<Trial>) -> Comparison {
    Comparison {
        declared: Declared {
            trials: 3,
            statistic: "median".to_owned(),
            concurrency: 16,
            site: "/sites/4/1".to_owned(),
            spider: "2.53.9".to_owned(),
        },
        machine: Machine {
            class: "local-controlled".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: "a CPU".to_owned(),
            threads: 16,
        },
        commit: "0".repeat(40),
        order_seed: 7,
        trials,
        instructions: InstructionCell::NotRun {
            reason: "a test".to_owned(),
        },
    }
}

/// Every trial of each tool fetched the 4-page site, in this order:
/// Foxhound's first.
fn same_work() -> Vec<Trial> {
    vec![
        trial(Tool::Foxhound, 1, site(4)),
        trial(Tool::Spider, 1, site(4)),
        trial(Tool::Foxhound, 2, site(4)),
        trial(Tool::Spider, 2, site(4)),
        trial(Tool::Foxhound, 3, site(4)),
        trial(Tool::Spider, 3, site(4)),
    ]
}

/// Each finding's kind and subject.
fn named(trials: &Comparison) -> Vec<(&'static str, String)> {
    check(trials)
        .into_iter()
        .map(|finding| (finding.kind, finding.subject))
        .collect()
}

#[test]
fn trials_that_did_the_same_work_pass() {
    assert_eq!(named(&comparison(same_work())), vec![]);
}

/// The first trial is held to the site's own pages like any other, so
/// when it is the one that did other work, it alone is named.
#[test]
fn a_first_trial_that_fetched_other_pages_is_the_one_named() {
    let mut trials = same_work();
    trials[0] = trial(Tool::Foxhound, 1, site(3));

    assert_eq!(
        named(&comparison(trials)),
        vec![("different_pages", "foxhound trial 1".to_owned())]
    );
}

/// Bytes are held to what most trials were served, so a first trial
/// served other bytes for the site's pages is the one named.
#[test]
fn a_first_trial_served_other_bytes_is_the_one_named() {
    let mut trials = same_work();
    trials[0].bytes += 1;

    assert_eq!(
        named(&comparison(trials)),
        vec![("different_bytes", "foxhound trial 1".to_owned())]
    );
}

/// Every trial that misses the same page is named, though they agree with
/// each other.
#[test]
fn trials_that_all_miss_a_page_are_each_named() {
    let trials = same_work()
        .into_iter()
        .map(|done| trial(done.tool, done.number, site(4)[..3].to_vec()))
        .collect();

    assert_eq!(named(&comparison(trials)).len(), 6);
}

/// The site's work is every `GET` of its pages the Proving ground answered
/// `200` on its host, a page fetched twice counted twice in requests and
/// bytes; the robots.txt, a `404`, another host and a `HEAD` are not.
#[test]
fn a_trials_work_is_its_answered_page_requests_on_the_sites_host() {
    let request = |host: &str, method: &str, target: &str, status: u16, sent: u64| json!({ "host": host, "method": method, "target": target, "status": status, "bytes": { "received": 90, "sent": sent } });
    let observations = json!({
        "requests": [
            request("scale.test:8080", "GET", "/robots.txt", 404, 50),
            request("scale.test:8080", "GET", "/sites/2/1", 200, 600),
            request("scale.test", "GET", "/sites/2/2", 200, 610),
            request("scale.test:8080", "GET", "/sites/2/2", 200, 610),
            request("scale.test:8080", "GET", "/sites/2/3", 404, 70),
            request("scale.test:8080", "HEAD", "/sites/2/1", 200, 120),
            request("catalog.test:8080", "GET", "/sites/2/1", 200, 600),
        ],
        "unrecorded_requests": 0,
    });

    let work = work_of(&observations, "scale.test", "/sites/2/").unwrap();

    assert_eq!(
        (
            work.pages.into_iter().collect::<Vec<_>>(),
            work.requests,
            work.bytes
        ),
        (
            vec!["/sites/2/1".to_owned(), "/sites/2/2".to_owned()],
            3,
            1820
        )
    );
}

#[test]
fn unrecorded_requests_make_a_trials_work_unknown() {
    let observations = json!({ "requests": [], "unrecorded_requests": 3 });

    assert!(work_of(&observations, "scale.test", "/sites/2/").is_err());
}
