//! The writer: a case log another runner wrote and the declared case list
//! become a `conformance` result document whose outcomes the constructor
//! derives (ADRs 0063 and 0069).

use goodfetch_conformance::{
    CaseList, CaseLogError, DeclaredCapability, DeclaredCase, WriteError, read_case_log,
    write_document,
};
use goodfetch_protocol::journey::FeatureId;
use goodfetch_protocol::result_document::{
    Blocking, CaseOutcome, ConformanceDocument, ConformanceHeader, Outcome, ProvingGroundCopy,
    ResultDocument, Suite, Target, TargetIdentity,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::{platform, schema_violations};

fn feature(id: &str) -> FeatureId {
    FeatureId::try_from(id.to_owned()).unwrap()
}

fn case(id: &str, covers: &[&str]) -> DeclaredCase {
    DeclaredCase {
        id: id.to_owned(),
        covers: covers.iter().map(|id| feature(id)).collect(),
    }
}

/// Two capabilities, `cancel` with two cases and `credits` with one.
fn case_list() -> CaseList {
    CaseList {
        suite: Suite {
            name: "clients".to_owned(),
            version: "1".to_owned(),
        },
        capabilities: vec![
            DeclaredCapability {
                name: "cancel".to_owned(),
                cases: vec![
                    case("cancel-before-start", &["operation:runs.cancel"]),
                    case("cancel-mid-run", &["operation:runs.cancel", "story:11-4"]),
                ],
            },
            DeclaredCapability {
                name: "credits".to_owned(),
                cases: vec![case("credits-released-on-cancel", &["story:11-9"])],
            },
        ],
    }
}

fn header() -> ConformanceHeader {
    ConformanceHeader {
        suite: case_list().suite,
        target: Target {
            name: "goodfetch-collie".to_owned(),
            identity: TargetIdentity::Version("0.0.0".to_owned()),
            product: None,
        },
        platform: platform(),
        runtime: "node-24".to_owned(),
        proving_ground: ProvingGroundCopy {
            version: "0.0.0+0000000000000000000000000000000000000000".to_owned(),
            seed: 169,
        },
        online: false,
        gate: false,
    }
}

/// One entry as a runner writes it, every check holding unless `failed`
/// names one.
fn entry(case: &str, capability: &str, failed: Option<&str>) -> Value {
    json!({
        "case": case,
        "capability": capability,
        "platform": { "os": "linux", "arch": "x86_64" },
        "runtime": "node-24",
        "checks": [
            { "name": "the call returns", "passed": true },
            { "name": "the run ends cancelled", "passed": failed != Some("the run ends cancelled") },
        ],
        "evidence": {
            "messages": [{ "method": "runs.cancel" }],
        },
    })
}

/// A JSON Lines case log of the given entries.
fn log(entries: &[Value]) -> String {
    entries
        .iter()
        .map(|entry| entry.to_string() + "\n")
        .collect()
}

fn outcomes(document: &ConformanceDocument) -> Vec<Outcome> {
    document
        .capabilities()
        .iter()
        .map(goodfetch_protocol::result_document::CapabilityResult::outcome)
        .collect()
}

/// Covers: story:2-85
#[test]
fn a_log_with_every_declared_case_passing_proves_each_capability() {
    let log = read_case_log(&log(&[
        entry("cancel-before-start", "cancel", None),
        entry("cancel-mid-run", "cancel", None),
        entry("credits-released-on-cancel", "credits", None),
    ]))
    .unwrap();

    let document = write_document(header(), &case_list(), &log).unwrap();

    assert_eq!(outcomes(&document), [Outcome::Proved, Outcome::Proved]);
}

/// Covers: story:2-56, story:2-85
#[test]
fn a_declared_case_absent_from_the_log_leaves_its_capability_unproved() {
    let log = read_case_log(&log(&[
        entry("cancel-before-start", "cancel", None),
        entry("credits-released-on-cancel", "credits", None),
    ]))
    .unwrap();

    let document = write_document(header(), &case_list(), &log).unwrap();

    assert_eq!(outcomes(&document), [Outcome::Unproved, Outcome::Proved]);
    let unreached = &document.capabilities()[0].cases()[1];
    assert_eq!(
        (unreached.facts().id.as_str(), unreached.outcome()),
        ("cancel-mid-run", CaseOutcome::NotReached)
    );
}

/// Covers: story:2-57, story:2-85
#[test]
fn a_failed_check_fails_its_capability_with_the_failing_case_and_its_evidence() {
    let log = read_case_log(&log(&[
        entry("cancel-before-start", "cancel", None),
        entry("cancel-mid-run", "cancel", Some("the run ends cancelled")),
        entry("credits-released-on-cancel", "credits", None),
    ]))
    .unwrap();

    let document = write_document(header(), &case_list(), &log).unwrap();

    let capability = &document.capabilities()[0];
    let failing: Vec<(&str, CaseOutcome)> = capability
        .cases()
        .iter()
        .map(|case| (case.facts().id.as_str(), case.outcome()))
        .collect();
    assert_eq!(capability.outcome(), Outcome::Failed);
    assert_eq!(
        failing,
        [
            ("cancel-before-start", CaseOutcome::Passed),
            ("cancel-mid-run", CaseOutcome::Failed),
        ]
    );
    assert_eq!(
        serde_json::to_value(&capability.cases()[1].facts().entries[0].evidence).unwrap(),
        json!({ "messages": [{ "method": "runs.cancel" }] })
    );
}

/// Covers: story:2-57, story:2-85
#[test]
fn the_document_names_its_suite_target_platform_runtime_and_proving_ground_and_carries_each_cases_covers()
 {
    let log = read_case_log(&log(&[entry(
        "credits-released-on-cancel",
        "credits",
        None,
    )]))
    .unwrap();
    let list = CaseList {
        capabilities: vec![case_list().capabilities[1].clone()],
        ..case_list()
    };

    let document = ResultDocument::Conformance(write_document(header(), &list, &log).unwrap());

    assert_eq!(
        serde_json::to_value(&document).unwrap(),
        json!({
            "kind": "conformance",
            "suite": { "name": "clients", "version": "1" },
            "target": { "name": "goodfetch-collie", "version": "0.0.0" },
            "platform": { "os": "linux", "arch": "x86_64" },
            "runtime": "node-24",
            "proving_ground": {
                "version": "0.0.0+0000000000000000000000000000000000000000",
                "seed": 169,
            },
            "online": false,
            "capabilities": [{
                "name": "credits",
                "outcome": "proved",
                "cases": [{
                    "id": "credits-released-on-cancel",
                    "covers": ["story:11-9"],
                    "outcome": "passed",
                    "entries": [{
                        "checks": [
                            { "name": "the call returns", "passed": true },
                            { "name": "the run ends cancelled", "passed": true },
                        ],
                        "evidence": { "messages": [{ "method": "runs.cancel" }] },
                    }],
                }],
            }],
        })
    );
    assert_eq!(schema_violations(&document), Vec::<String>::new());
}

/// An entry whose oracle answer carries a vendor's `verdict` of `outcome`,
/// for `reason` if any, the answer's check failing as an online task's
/// does when its vendor did not pass the tool.
fn judged_by_vendor(case: &str, vendor: &str, outcome: &str, reason: Option<&str>) -> Value {
    let mut verdict = json!({ "vendor": vendor, "outcome": outcome });
    if let Some(reason) = reason {
        verdict["reason"] = json!(reason);
    }
    let failed = (outcome != "passed").then_some("the run ends cancelled");
    let mut judged = entry(case, "cancel", failed);
    judged["evidence"]["oracle_answers"] = json!([{
        "task": "challenges/1",
        "outcome": if outcome == "passed" { "verified" } else { "failed" },
        "verdict": verdict,
    }]);
    judged
}

/// Covers: story:2-38, story:2-87
#[test]
fn a_vendors_inconclusive_verdict_makes_its_case_inconclusive_and_names_the_vendor() {
    let log = read_case_log(&log(&[
        judged_by_vendor(
            "cancel-before-start",
            "turnstile",
            "inconclusive",
            Some("out_of_quota"),
        ),
        judged_by_vendor("cancel-mid-run", "botid", "failed", Some("bot")),
    ]))
    .unwrap();

    let document = write_document(header(), &case_list(), &log).unwrap();

    let cases = document.capabilities()[0].cases();
    assert_eq!(
        cases
            .iter()
            .map(|case| (case.outcome(), case.facts().entries[0].unreachable.clone()))
            .collect::<Vec<_>>(),
        [
            (
                CaseOutcome::Inconclusive,
                vec!["turnstile (out_of_quota)".to_owned()]
            ),
            (CaseOutcome::Failed, Vec::new()),
        ]
    );
}

/// The document of a run on the online copy, in gate mode or not, whose
/// `cancel-before-start` case ran with blocking on, judged by a vendor as
/// `on` says, and with blocking off, as `off` says.
fn online_document(gate: bool, on: &str, off: &str) -> ConformanceDocument {
    let entries: Vec<Value> = [("on", on), ("off", off)]
        .into_iter()
        .map(|(blocking, outcome)| {
            let reason = (outcome == "inconclusive").then_some("unreachable");
            let mut judged = judged_by_vendor("cancel-before-start", "turnstile", outcome, reason);
            judged["blocking"] = json!(blocking);
            judged
        })
        .collect();
    let log = read_case_log(&log(&entries)).unwrap();
    let header = ConformanceHeader {
        online: true,
        gate,
        ..header()
    };
    write_document(header, &case_list(), &log).unwrap()
}

/// In gate mode, a case whose vendor was unreachable with blocking on,
/// whatever it did with blocking off, is inconclusive: it does not hold the
/// daily channel back and never counts as a pass. Nor does one the vendor
/// failed with blocking on and could not judge with blocking off.
///
/// Covers: story:2-73
#[rstest]
#[case::both_ways("inconclusive", "inconclusive")]
#[case::with_blocking_on_alone("inconclusive", "passed")]
#[case::with_blocking_on_and_failing_off("inconclusive", "failed")]
#[case::with_blocking_off_alone("failed", "inconclusive")]
fn a_gate_case_whose_vendor_was_unreachable_neither_passes_nor_blocks(
    #[case] on: &str,
    #[case] off: &str,
) {
    let document = online_document(true, on, off);

    let case = &document.capabilities()[0].cases()[0];
    assert_eq!(case.outcome(), CaseOutcome::Inconclusive);
    let recorded: Vec<(Option<Blocking>, bool)> = case
        .facts()
        .entries
        .iter()
        .map(|entry| (entry.blocking, entry.unreachable.is_empty()))
        .collect();
    assert_eq!(
        recorded,
        [
            (Some(Blocking::On), on != "inconclusive"),
            (Some(Blocking::Off), off != "inconclusive"),
        ]
    );
    assert_eq!(document.capabilities()[0].outcome(), Outcome::Unproved);
    assert!(!document.blocks());
}

/// The same online failure holds the job that asked for gate mode back, and
/// only reports everywhere else.
///
/// Covers: story:2-74
#[rstest]
#[case::in_gate_mode(true)]
#[case::outside_gate_mode(false)]
fn an_online_failure_blocks_in_gate_mode_alone(#[case] gate: bool) {
    let document = online_document(gate, "failed", "passed");

    assert_eq!(document.capabilities()[0].outcome(), Outcome::Failed);
    assert_eq!(document.blocks(), gate);
}

/// Covers: story:2-85
#[test]
fn an_entry_that_sets_an_outcome_is_refused() {
    let mut setting_one = entry("cancel-before-start", "cancel", None);
    setting_one["outcome"] = json!("passed");

    let refused = read_case_log(&log(&[
        entry("cancel-mid-run", "cancel", None),
        setting_one,
    ]));

    assert!(
        matches!(refused, Err(CaseLogError::Entry { line: 2, .. })),
        "{refused:?}"
    );
}

/// Covers: story:2-85
#[test]
fn an_entry_for_an_undeclared_case_is_refused() {
    let log = read_case_log(&log(&[entry("cancel-twice", "cancel", None)])).unwrap();

    let refused = write_document(header(), &case_list(), &log);

    assert!(
        matches!(&refused, Err(WriteError::UndeclaredCase { case }) if case == "cancel-twice"),
        "{refused:?}"
    );
}

/// Covers: story:2-85
#[test]
fn an_entry_under_another_capability_than_its_cases_is_refused() {
    let log = read_case_log(&log(&[entry("cancel-before-start", "credits", None)])).unwrap();

    let refused = write_document(header(), &case_list(), &log);

    assert!(
        matches!(&refused, Err(WriteError::OtherCapability { case, .. }) if case == "cancel-before-start"),
        "{refused:?}"
    );
}

/// Covers: story:2-85
#[test]
fn an_entry_from_another_runtime_than_the_documents_is_refused() {
    let mut on_bun = entry("cancel-before-start", "cancel", None);
    on_bun["runtime"] = json!("bun-1");
    let log = read_case_log(&log(&[on_bun])).unwrap();

    let refused = write_document(header(), &case_list(), &log);

    assert!(
        matches!(&refused, Err(WriteError::OtherHost { case }) if case == "cancel-before-start"),
        "{refused:?}"
    );
}

/// Covers: story:2-85
#[test]
fn a_header_naming_another_suite_than_the_case_lists_is_refused() {
    let header = ConformanceHeader {
        suite: Suite {
            name: "clients".to_owned(),
            version: "2".to_owned(),
        },
        ..header()
    };

    let refused = write_document(header, &case_list(), &[]);

    assert!(
        matches!(refused, Err(WriteError::OtherSuite)),
        "{refused:?}"
    );
}
