use serde_json::{Value, json};

use super::{Broken, LIMITS_IN_FLIGHT, LIMITS_RATE, TRAPS, broken};

/// Observations holding `requests` and `findings`, each `(task, …)`, none
/// left unrecorded.
fn observations(requests: &[(&str, &str, u16)], findings: &[(&str, &str)]) -> Value {
    json!({
        "requests": requests
            .iter()
            .map(|(task, target, status)| json!({ "task": task, "target": target, "status": status }))
            .collect::<Vec<_>>(),
        "unrecorded_requests": 0,
        "findings": findings
            .iter()
            .map(|(task, kind)| json!({ "task": task, "kind": kind, "target": "/" }))
            .collect::<Vec<_>>(),
        "unrecorded_findings": 0,
    })
}

/// On `traps/1`, each item counts once whatever URL of it was fetched, by
/// the family's rule, a refused or missing page counts for nothing, and
/// another task's requests and findings count for none of it; a honeypot
/// flags the address, and duplicates and trap requests are its waste.
#[test]
fn traps_counts_each_item_once_and_flags_on_a_honeypot() {
    let seen = observations(
        &[
            ("traps/1", "/robots.txt", 200),
            ("traps/1", "/", 200),
            ("traps/1", "/items/I-0001", 200),
            ("traps/1", "/items/I%2D0001;jsessionid=7", 200),
            ("traps/1", "/items/I%2d0002?ref=home", 200),
            ("traps/1", "/items/I-0003", 403),
            ("traps/1", "/items/I-0031", 404),
            ("traps/1", "/hp/0badc0de", 200),
            ("traps/1", "/calendar/2026-01", 200),
            ("limits/1", "/items/I-0004", 200),
        ],
        &[
            ("traps/1", "duplicate"),
            ("traps/1", "honeypot"),
            ("traps/1", "trap"),
            ("traps/1", "trap"),
            ("traps/1", "wrong_base"),
            ("limits/1", "honeypot"),
        ],
    );

    assert_eq!(
        broken(&seen, &TRAPS).unwrap(),
        Broken {
            task: "traps/1".to_owned(),
            requests: 9,
            retry_after_ignored: 0,
            over_in_flight_cap: 0,
            honeypot: 1,
            flagged: true,
            wanted_fetched: 2,
            wanted: 30,
            duplicate: 1,
            trap: 2,
            stopped: None,
        }
    );
}

/// `limits/1` flags an address on its third request inside a pause, not
/// before, and `limits/3` never flags, however many requests went over its
/// cap.
#[test]
fn limits_flags_on_the_third_ignored_retry_after_and_never_on_the_cap() {
    let ignored = |times: usize| {
        let findings = vec![("limits/1", "retry_after_ignored"); times];
        broken(&observations(&[], &findings), &LIMITS_RATE)
            .unwrap()
            .flagged
    };
    let capped = broken(
        &observations(
            &[("limits/3", "/items/L-0024", 200)],
            &[("limits/3", "over_in_flight_cap"); 5],
        ),
        &LIMITS_IN_FLIGHT,
    )
    .unwrap();

    assert_eq!(
        (
            ignored(2),
            ignored(3),
            capped.flagged,
            capped.over_in_flight_cap,
            capped.wanted_fetched
        ),
        (false, true, false, 5, 1)
    );
}

/// Counts read from part of the record would be wrong: observations with
/// no findings, or with findings or requests the Proving ground left
/// unrecorded, or a finding of a kind unknown here, fail the trial.
#[test]
fn observations_that_cannot_be_read_whole_fail() {
    let whole = observations(&[("traps/1", "/", 200)], &[("traps/1", "trap")]);
    let without = |name: &str| {
        let mut seen = whole.clone();
        seen.as_object_mut().unwrap().remove(name);
        seen
    };
    let with = |name: &str, value: Value| {
        let mut seen = whole.clone();
        seen[name] = value;
        seen
    };
    let failures = [
        without("findings"),
        with("unrecorded_findings", json!(1)),
        with("unrecorded_requests", json!(3)),
        with(
            "findings",
            json!([{ "task": "traps/1", "kind": "something_new" }]),
        ),
    ]
    .map(|seen| broken(&seen, &TRAPS).is_err());

    assert_eq!(
        (broken(&whole, &TRAPS).is_ok(), failures),
        (true, [true; 4])
    );
}
