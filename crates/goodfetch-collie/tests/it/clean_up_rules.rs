//! The version of the Clean-up rules each Run holds to (spec #577, ticket
//! #714; ADR 0047), through Collie's executable: chosen at the Run's start
//! from the embedded part, the daily channel or the Filter lists' own URLs,
//! its digest recorded in run state and kept by a resume, named in the Run
//! trace's Provenance with its date and the release that applied it, and
//! warned of in the Run report when it is older than 14 days or not the one
//! asked for. The daily channel's registry and the lists' authors are
//! local HTTPS stand-ins that `network.resolve` maps their hosts to, their
//! certificate authority trusted through `tls.trusted_cas` in user
//! configuration: no test reaches the internet.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::{Path, PathBuf};

use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::run::call;

/// The digest of the embedded part, as the clean-up crate's build gives it.
const EMBEDDED: &str = goodfetch_clean_up_rules::rules::EMBEDDED_DIGEST;

/// The folder of the Run `run_id` in the Goodfetch home `home`.
fn run_folder(home: &Path, run_id: &str) -> PathBuf {
    home.join("0.0").join("runs").join(run_id)
}

/// The Clean-up rules digest the run state of `run_id` records.
fn recorded_digest(home: &Path, run_id: &str) -> Option<String> {
    Connection::open(run_folder(home, run_id).join("run.sqlite"))
        .unwrap()
        .query_row("SELECT clean_up_digest FROM run", [], |row| row.get(0))
        .unwrap()
}

/// The Provenance entries of the Run `run_id` that name its Clean-up rules.
fn rules_provenance(home: &Path, run_id: &str) -> Vec<Value> {
    std::fs::read_to_string(run_folder(home, run_id).join("trace/provenance.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == json!("clean_up_rules"))
        .collect()
}

/// Starts and finishes the Run `run_id` on a new Attached engine with
/// `settings`, and answers its Run report.
fn start_and_finish(env: &TestEnv, run_id: &str, settings: &Value) -> Value {
    let mut engine = Attached::start(env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let started = call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let report = call(&mut engine, 2, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();
    report["result"].clone()
}

/// The codes of the warnings of the Run report `report`.
fn warning_codes(report: &Value) -> Vec<String> {
    report["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|warning| warning["code"].as_str().map(str::to_owned))
        .collect()
}

/// The date of the embedded part, as its manifest gives it.
fn embedded_date() -> Value {
    serde_json::from_str::<Value>(goodfetch_clean_up_rules::rules::EMBEDDED_MANIFEST).unwrap()
        ["published_at"]
        .clone()
}

/// Covers: story:5-97, setting:clean_up.update
#[test]
fn a_run_holds_to_the_embedded_part_by_default_named_with_its_date_and_release() {
    let env = TestEnv::new();

    let report = start_and_finish(&env, "embedded", &json!({}));

    assert_eq!(
        (
            recorded_digest(env.home(), "embedded"),
            rules_provenance(env.home(), "embedded"),
            warning_codes(&report).contains(&"clean_up_rules_fallback".to_owned()),
        ),
        (
            Some(EMBEDDED.to_owned()),
            vec![json!({
                "action": "clean_up_rules",
                "digest": EMBEDDED,
                "source": "embedded",
                "published_at": embedded_date(),
                "applied_by": { "product": "collie", "product_version": "0.0.0" },
                "update": "release",
                "resumed": false,
            })],
            false,
        )
    );
}

/// The embedded part's digest is the Browser cache's digest of its files,
/// so a copy of them that a release publishes is checked as any download
/// is (ADR 0024).
/// Covers: story:5-97
#[test]
fn the_embedded_parts_digest_is_the_browser_caches_digest_of_its_files() {
    let rules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../goodfetch-clean-up-rules/rules");
    let copy = tempfile::tempdir().unwrap();
    for name in [
        "manifest.json",
        "easylist.txt",
        "easyprivacy.txt",
        "fanboy-annoyance.txt",
        "brave-unbreak.txt",
    ] {
        std::fs::copy(rules.join(name), copy.path().join(name)).unwrap();
    }

    assert_eq!(
        goodfetch_engine::browser_cache::content_digest(copy.path())
            .unwrap()
            .to_string(),
        EMBEDDED
    );
}
