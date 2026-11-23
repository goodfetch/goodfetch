//! A conformance run's Live check result in the results store (ADR 0064):
//! each document becomes one test case of a JUnit file, which the ingest
//! stores under the `live` tier as failed, inconclusive or passed. GitHub's
//! interface is a local stand-in, and the store a local folder.
//! Covers: story:2-87

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::result_document::{
    Blocking, CapabilityFacts, CaseCheck, CaseEntry, CaseFacts, ConformanceDocument,
    ConformanceHeader, Evidence, Platform, ProvingGroundCopy, ResultDocument, Suite, Target,
    TargetIdentity,
};
use serde_json::Value;

use crate::github::GitHub;
use crate::store::{STORE, read_store_file};
use crate::workspace::Workspace;

const ARTIFACT: &str = "junit-conformance-live-linux-x64-1";

/// A gate run's entry of one case, with `blocking`, whose one check held
/// when `passed`, and which could not reach `unreachable`.
fn entry(blocking: Blocking, passed: bool, unreachable: &[&str]) -> CaseEntry {
    CaseEntry {
        mode: Some("attached".to_owned()),
        blocking: Some(blocking),
        unreachable: unreachable.iter().map(|&name| name.to_owned()).collect(),
        checks: vec![CaseCheck {
            name: "the page loads".to_owned(),
            passed,
        }],
        evidence: Evidence::default(),
    }
}

/// The daily channel's gate run of the `page` suite on `target`, against
/// the online Proving ground: one capability with the case `steady`, which
/// passes both ways, and the case `vendor`, with the entries `vendor`.
fn gate_run(target: &str, vendor: Vec<CaseEntry>) -> Vec<u8> {
    let header = ConformanceHeader {
        suite: Suite {
            name: "page".to_owned(),
            version: "1".to_owned(),
        },
        target: Target {
            name: target.to_owned(),
            identity: TargetIdentity::Version("0.0.0".to_owned()),
            product: Some(Product::Collie),
        },
        platform: Platform {
            os: "linux".to_owned(),
            arch: "x64".to_owned(),
        },
        runtime: "rust".to_owned(),
        proving_ground: ProvingGroundCopy {
            version: "1.0.0".to_owned(),
            seed: 7,
        },
        online: true,
        gate: true,
    };
    let case = |id: &str, entries| CaseFacts {
        id: id.to_owned(),
        covers: Vec::new(),
        entries,
    };
    let document = ConformanceDocument::new(
        header,
        vec![CapabilityFacts {
            name: "pages".to_owned(),
            cases: vec![
                case(
                    "steady",
                    vec![
                        entry(Blocking::On, true, &[]),
                        entry(Blocking::Off, true, &[]),
                    ],
                ),
                case("vendor", vendor),
            ],
        }],
    );
    serde_json::to_vec(&ResultDocument::Conformance(document)).unwrap()
}

/// Three targets' gate runs: blocking alone breaks a page on the first; a
/// vendor's site was down while the second ran with blocking on; every
/// page of the third passed.
fn write_gate_runs(workspace: &Workspace) {
    workspace.write(
        "documents/blocked.json",
        &gate_run(
            "blocked",
            vec![
                entry(Blocking::On, false, &[]),
                entry(Blocking::Off, true, &[]),
            ],
        ),
    );
    workspace.write(
        "documents/outage.json",
        &gate_run(
            "outage",
            vec![
                entry(Blocking::On, false, &["challenge vendor"]),
                entry(Blocking::Off, true, &[]),
            ],
        ),
    );
    workspace.write(
        "documents/steady.json",
        &gate_run(
            "steady",
            vec![
                entry(Blocking::On, true, &[]),
                entry(Blocking::Off, true, &[]),
            ],
        ),
    );
}

#[test]
fn each_gate_run_is_a_live_check_the_store_keeps_as_failed_inconclusive_or_passed() {
    let workspace = Workspace::from_fixture("empty");
    write_gate_runs(&workspace);

    let written = workspace.xtask(&[
        "results",
        "live-check",
        "--document",
        "documents/blocked.json",
        "--document",
        "documents/outage.json",
        "--document",
        "documents/steady.json",
        "--junit",
        &format!("artifacts/{ARTIFACT}/conformance.xml"),
    ]);
    assert_eq!(written.code, 0, "{}", written.stderr);
    workspace.write(
        &format!("artifacts/{ARTIFACT}/context.json"),
        br#"{ "commit": "c1", "tree": "t1", "job": "conformance", "order_seed": null, "entropy_seed": null }"#,
    );
    let github = GitHub::serving(&[
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/1",
            r#"{
              "id": 42, "run_attempt": 1, "path": ".github/workflows/daily.yml",
              "event": "schedule", "head_branch": "main", "head_sha": "c1",
              "head_commit": { "id": "c1", "tree_id": "t1" },
              "run_started_at": "2026-11-23T03:00:00Z", "conclusion": "failure"
            }"#,
        ),
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/attempts/1/jobs?per_page=100&page=1",
            r#"{ "total_count": 1, "jobs": [
              { "name": "conformance", "labels": ["ubuntu-24.04"], "conclusion": "failure",
                "started_at": "2026-11-23T03:00:05Z", "completed_at": "2026-11-23T03:20:05Z" }
            ] }"#,
        ),
        (
            "/repos/goodfetch/goodfetch/actions/runs/42/artifacts?per_page=100&page=1",
            &format!(
                r#"{{ "total_count": 1, "artifacts": [
                  {{ "name": "{ARTIFACT}", "created_at": "2026-11-23T03:20:00Z", "expired": false }}
                ] }}"#
            ),
        ),
    ]);

    let ingested = workspace.xtask(&[
        "results",
        "ingest",
        "--repository",
        "goodfetch/goodfetch",
        "--run-id",
        "42",
        "--attempt",
        "1",
        "--artifacts",
        "artifacts",
        "--store",
        STORE,
        "--api-url",
        github.url(),
    ]);

    assert_eq!(ingested.code, 0, "{}", ingested.stderr);
    let file = read_store_file(&workspace, &format!("{STORE}/runs/2026/11/42-1.tar.gz"));
    let kept: Vec<(&str, &str, &str)> = file
        .test_runs
        .iter()
        .map(|run| {
            let field = |name: &str| run[name].as_str().unwrap();
            (field("test"), field("tier"), field("outcome"))
        })
        .collect();
    assert_eq!(
        kept,
        vec![
            ("conformance page blocked gate", "live", "failed"),
            ("conformance page outage gate", "live", "inconclusive"),
            ("conformance page steady gate", "live", "passed"),
        ]
    );
    let junit = workspace.read_text(&format!("artifacts/{ARTIFACT}/conformance.xml"));
    assert!(junit.contains("failed: vendor"), "{junit}");
    assert!(junit.contains("inconclusive: vendor"), "{junit}");
}

/// A document the reader refuses, its stored outcomes not the ones its
/// entries give, writes no Live check.
#[test]
fn a_document_whose_outcomes_do_not_hold_writes_no_live_check() {
    let workspace = Workspace::from_fixture("empty");
    write_gate_runs(&workspace);
    let mut forged: Value =
        serde_json::from_slice(&workspace.read("documents/blocked.json").unwrap()).unwrap();
    forged["capabilities"][0]["cases"][1]["outcome"] = Value::from("passed");
    workspace.write(
        "documents/forged.json",
        &serde_json::to_vec(&forged).unwrap(),
    );

    let written = workspace.xtask(&[
        "results",
        "live-check",
        "--document",
        "documents/forged.json",
        "--junit",
        "conformance.xml",
    ]);

    assert_ne!(written.code, 0);
    assert!(workspace.read("conformance.xml").is_none());
}
