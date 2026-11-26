//! A job's Live checks (ADR 0064; TESTING.md, Live checks): `xtask results
//! live-run` runs each check `ci/live-checks.toml` gives the job and writes
//! one JUnit test case per check, named by its ID, which the ingest keeps
//! under the `live` tier. A check's command passes on exit 0, is
//! inconclusive on exit 75 (the outside side was unavailable), and fails,
//! with its job, on any other exit. GitHub's interface is a local stand-in,
//! and the store a local folder.

use crate::store::ingest_artifact;
use crate::workspace::Workspace;

const ARTIFACT: &str = "junit-sinks-live-linux-x64-1";

/// Three checks of the nightly Sinks job, and one of another job.
const CHECKS: &str = r#"[[live-check]]
id = "sinks up"
area = 6
job = "nightly.yml#sinks"
run = "echo stored"

[[live-check]]
id = "sinks down"
area = 6
job = "nightly.yml#sinks"
run = "echo 'account unreachable'; exit 75"

[[live-check]]
id = "sinks broken"
area = 6
job = "nightly.yml#sinks"
run = "echo 'row count 2, expected 3' >&2; exit 3"

[[live-check]]
id = "testnets proof"
area = 9
job = "nightly.yml#testnets"
run = "touch testnets-ran"
"#;

#[test]
fn each_check_of_the_job_ends_passed_failed_or_inconclusive_in_the_store() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("ci/live-checks.toml", CHECKS.as_bytes());

    let ran = workspace.xtask(&[
        "results",
        "live-run",
        "--job",
        "nightly.yml#sinks",
        "--junit",
        &format!("artifacts/{ARTIFACT}/live.xml"),
    ]);

    assert_eq!(ran.code, 1, "{}", ran.stderr);
    assert!(workspace.read("testnets-ran").is_none());
    let mut kept = ingest_artifact(&workspace, ARTIFACT, "sinks");
    kept.sort();
    assert_eq!(
        kept,
        [
            ("sinks broken", "live", "failed"),
            ("sinks down", "live", "inconclusive"),
            ("sinks up", "live", "passed"),
        ]
        .map(|(test, tier, outcome)| (
            test.to_owned(),
            tier.to_owned(),
            outcome.to_owned()
        ))
    );
    let junit = workspace.read_text(&format!("artifacts/{ARTIFACT}/live.xml"));
    assert!(junit.contains("row count 2, expected 3"), "{junit}");
}

/// A shell job whose area has not added its first check yet passes.
#[test]
fn job_with_no_check_yet_passes() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("ci/live-checks.toml", CHECKS.as_bytes());

    let ran = workspace.xtask(&[
        "results",
        "live-run",
        "--job",
        "nightly.yml#models",
        "--junit",
        "live.xml",
    ]);

    assert_eq!(ran.code, 0, "{}", ran.stderr);
}
