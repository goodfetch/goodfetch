//! `.github/workflows/release-build.yml` and `ci/release/build-*.sh`: verifies
//! the release build workflow's fixed ordering (Proving ground first, then
//! Proving ground signing + digest fix, then both Products' executables in one
//! job per target with Collie first, then Platform signing slot, then
//! packaging + digests, then local rehearsal on every OS), musl static-PIE/PGO
//! build through `ci/musl/cargo`, and that Chromium is fetched by digest,
//! never built (spec #130, issue 141; ADRs 0024, 0032, 0039, 0074 and 0094).

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::release_assets::TARGETS;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn read_workflow() -> Value {
    let text = std::fs::read_to_string(root().join(".github/workflows/release-build.yml")).unwrap();
    serde_saphyr::from_str(&text).unwrap()
}

fn needs_of(job: &Value) -> BTreeSet<String> {
    match job.get("needs") {
        Some(Value::String(one)) => BTreeSet::from([one.clone()]),
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

#[test]
fn release_build_workflow_enforces_fixed_order_and_signing_places() {
    let workflow = read_workflow();

    // Runs on manual dispatch and on release pull request merges.
    let triggers = workflow["on"].as_object().unwrap();
    assert!(triggers.contains_key("workflow_dispatch"));
    assert!(triggers.contains_key("workflow_call"));
    assert!(triggers.contains_key("push"));

    let jobs = workflow["jobs"].as_object().unwrap();

    // 1. Proving ground built first, then signed + digests fixed before executables.
    assert!(jobs.contains_key("proving-ground"));
    assert!(needs_of(&jobs["sign-proving-ground"]).contains("proving-ground"));
    assert!(needs_of(&jobs["build-targets"]).contains("sign-proving-ground"));

    // 2. Chromium fetched by digest, never built.
    let fetch_steps = jobs["fetch-chromium"]["steps"].as_array().unwrap();
    assert!(
        fetch_steps.iter().any(|step| {
            step.get("run")
                .and_then(Value::as_str)
                .is_some_and(|run| run.contains("release fetch-chromium"))
        }),
        "fetch-chromium must invoke `xtask release fetch-chromium`"
    );

    // 3. Clear place between build and digests for signing jobs (#142).
    let sign_needs = needs_of(&jobs["sign-artifacts"]);
    assert!(sign_needs.contains("build-targets"));
    assert!(sign_needs.contains("fetch-chromium"));

    let pkg_needs = needs_of(&jobs["package-and-digests"]);
    assert!(pkg_needs.contains("sign-artifacts"));
    assert!(pkg_needs.contains("sign-proving-ground"));

    // 4. Rehearsal runs on Linux, Windows, and macOS after packaging + digests.
    for job in ["rehearse-linux", "rehearse-windows", "rehearse-macos"] {
        assert!(
            needs_of(&jobs[job]).contains("package-and-digests"),
            "{job} must depend on package-and-digests"
        );
    }

    // 5. All eight targets appear in both proving-ground and build-targets matrices.
    for matrix_job in ["proving-ground", "build-targets"] {
        let entries = jobs[matrix_job]["strategy"]["matrix"]["include"]
            .as_array()
            .unwrap();
        let targets: BTreeSet<&str> = entries
            .iter()
            .filter_map(|entry| entry["target"].as_str())
            .collect();
        let expected: BTreeSet<&str> = TARGETS.into_iter().collect();
        assert_eq!(targets, expected, "{matrix_job} target matrix mismatch");
    }
}

#[test]
fn build_target_script_builds_collie_before_foxhound_and_uses_musl_pgo_wrapper() {
    let text = std::fs::read_to_string(root().join("ci/release/build-target.sh")).unwrap();
    let collie_pos = text
        .find("-p goodfetch-collie")
        .expect("build-target.sh builds goodfetch-collie");
    let foxhound_pos = text
        .find("-p goodfetch-foxhound")
        .expect("build-target.sh builds goodfetch-foxhound");
    assert!(
        collie_pos < foxhound_pos,
        "Collie must be built before Foxhound in the same target folder (ADR 0094)"
    );
    assert!(
        text.contains("ci/musl/cargo"),
        "Linux musl targets must build through ci/musl/cargo (static-PIE + DT_RELR + PGO)"
    );
    assert!(
        text.contains("static-pie"),
        "Linux musl targets must verify static-PIE with DT_RELR"
    );
    assert!(
        text.contains("goodfetch-sandbox") && text.contains("goodfetch-filter-broker"),
        "shared helpers must be built once per target"
    );
}
