//! `xtask compat jobs` (ADR 0066): the Release suite's skew jobs, generated
//! from what each release shipped, the checks each older minor takes and the
//! Journey variants excluded from one.
//!
//! The sample list has three minors of major 1, the latest patch of each
//! being 1.0.1, 1.1.0 and 1.2.3, after a release of an earlier line. Its
//! checks leave 30 minutes of each job to checks: Collie's fill 24 of one
//! job, so Foxhound's start a second.

use std::fmt::Write as _;

use rstest::rstest;
use serde_json::{Value, json};

use crate::workspace::{Outcome, Workspace, finding};

const JOBS: &str = "ci/release/skew-jobs.json";

const CHECKS: &str = r#"
setup_minutes = 15

[[check]]
id = "collie-cli"
product = "collie"
minutes = 8
journeys = { surfaces = ["cli"] }

[[check]]
id = "collie-sdks"
product = "collie"
minutes = 10
journeys = { surfaces = ["sdk-typescript", "sdk-python"] }

[[check]]
id = "collie-sdks-stdio"
product = "collie"
minutes = 6
journeys = { surfaces = ["sdk-typescript", "sdk-python"], stdio = true }

[[check]]
id = "foxhound-cli"
product = "foxhound"
minutes = 12
journeys = { surfaces = ["cli", "foxhound-cli"] }

[[check]]
id = "foxhound-sdks"
product = "foxhound"
minutes = 10
journeys = { surfaces = ["sdk-python"] }

[[check]]
id = "rolling-upgrade"
product = "foxhound"
minutes = 10
awaiting = 436
"#;

const RUN: [&str; 5] = [
    "collie-cli",
    "collie-sdks",
    "collie-sdks-stdio",
    "foxhound-cli",
    "foxhound-sdks",
];

const THREE_MINORS: [&str; 6] = ["0.9.0", "1.0.0", "1.0.1", "1.1.0", "1.2.0", "1.2.3"];

/// `compat/releases.toml` listing `versions`; a skew job reads nothing else
/// of an entry.
fn releases(versions: &[&str]) -> String {
    let mut text = String::new();
    for version in versions {
        writeln!(
            text,
            "[[release]]\nversion = \"{version}\"\nformats = {{}}\n"
        )
        .unwrap();
        for table in [
            "collie.helm_chart",
            "collie.image",
            "foxhound.helm_chart",
            "foxhound.image",
            "collie_client.npm",
            "collie_client.pypi",
            "collie_client.crates_io",
        ] {
            let digest = "0".repeat(64);
            writeln!(
                text,
                "[release.{table}]\nname = \"{table}\"\nurl = \"https://example.test/{table}\"\nsha256 = \"{digest}\"\nattestation = {{ url = \"https://example.test/{table}.jsonl\", sha256 = \"{digest}\" }}\n"
            )
            .unwrap();
        }
    }
    text
}

/// A workspace of `versions` released, the checks above and no exclusion,
/// whose executables carry `current`.
fn workspace(versions: &[&str], current: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("compat/releases.toml", releases(versions).as_bytes());
    workspace.write("compat/skew-checks.toml", CHECKS.as_bytes());
    workspace.write(
        "crates/goodfetch-collie/Cargo.toml",
        format!("[package]\nname = \"goodfetch-collie\"\nversion = \"{current}\"\n").as_bytes(),
    );
    workspace
}

fn exclude(workspace: &Workspace, minor: &str, variant: &str, reason: &str) {
    workspace.write(
        "compat/skew-exclusions.toml",
        format!(
            "[[exclusion]]\nminor = \"{minor}\"\nvariant = \"{variant}\"\nreason = \"{reason}\"\n"
        )
        .as_bytes(),
    );
}

fn run(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["compat", "jobs"])
}

/// The jobs `xtask compat jobs` writes.
fn generated(workspace: &Workspace) -> Value {
    let outcome = run(workspace);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    serde_json::from_str(&workspace.read_text(JOBS)).unwrap()
}

fn jobs(generated: &Value) -> Vec<Value> {
    generated["jobs"].as_array().unwrap().clone()
}

fn names(jobs: &[Value]) -> Vec<&str> {
    jobs.iter()
        .map(|job| job["name"].as_str().unwrap())
        .collect()
}

fn checks(job: &Value) -> Vec<&str> {
    job["steps"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|step| step["checks"].as_array().unwrap())
        .map(|check| check.as_str().unwrap())
        .collect()
}

/// The release each job's Clients come from, its checks in its steps' order.
fn plan(jobs: &[Value]) -> Vec<(&str, Vec<&str>)> {
    jobs.iter()
        .map(|job| (job["version"].as_str().unwrap_or("current"), checks(job)))
        .collect()
}

#[test]
fn each_minors_latest_patch_gets_both_products_checks_in_as_few_jobs_as_fit() {
    let workspace = workspace(&THREE_MINORS, "0.0.0");

    let generated = generated(&workspace);

    let jobs = jobs(&generated);
    let [collie, foxhound] = [
        vec!["collie-cli", "collie-sdks", "collie-sdks-stdio"],
        vec!["foxhound-cli", "foxhound-sdks"],
    ];
    assert_eq!(
        plan(&jobs),
        [
            ("1.0.1", collie.clone()),
            ("1.0.1", foxhound.clone()),
            ("1.1.0", collie.clone()),
            ("1.1.0", foxhound.clone()),
            ("1.2.3", collie),
            ("1.2.3", foxhound),
        ]
    );
    assert_eq!(
        names(&jobs),
        ["1.0-1", "1.0-2", "1.1-1", "1.1-2", "1.2-1", "1.2-2"]
    );
    for job in &jobs {
        assert!(job["minutes"].as_u64().unwrap() <= 45, "{job}");
    }
}

#[test]
fn a_minor_whose_checks_fit_one_job_takes_one() {
    let workspace = workspace(&["1.0.0"], "0.0.0");
    workspace.edit(
        "compat/skew-checks.toml",
        "setup_minutes = 15",
        "setup_minutes = 0",
    );
    workspace.edit("compat/skew-checks.toml", "minutes = 12", "minutes = 11");

    let generated = generated(&workspace);

    assert_eq!(plan(&jobs(&generated)), [("1.0.0", RUN.to_vec())]);
    assert_eq!(names(&jobs(&generated)), ["1.0"]);
    assert_eq!(jobs(&generated)[0]["minutes"], json!(45));
}

#[test]
fn each_step_runs_its_tags_journeys_on_its_products_executable() {
    let workspace = workspace(&["1.0.1"], "0.0.0");

    let generated = generated(&workspace);

    let jobs = jobs(&generated);
    assert_eq!(jobs[0]["tag"], "v1.0.1");
    assert_eq!(
        jobs[0]["steps"],
        json!([
            {
                "checks": ["collie-cli", "collie-sdks"],
                "executable": "goodfetch-collie",
                "stdio": false,
                "journeys": [
                    "--tag", "v1.0.1", "--product", "collie",
                    "--surface", "cli", "--surface", "sdk-typescript", "--surface", "sdk-python",
                ],
            },
            {
                "checks": ["collie-sdks-stdio"],
                "executable": "goodfetch-collie",
                "stdio": true,
                "journeys": [
                    "--tag", "v1.0.1", "--product", "collie",
                    "--surface", "sdk-typescript", "--surface", "sdk-python",
                ],
            },
        ])
    );
    assert_eq!(
        jobs[1]["steps"],
        json!([
            {
                "checks": ["foxhound-cli", "foxhound-sdks"],
                "executable": "goodfetch-foxhound",
                "stdio": false,
                "journeys": [
                    "--tag", "v1.0.1", "--product", "foxhound",
                    "--surface", "cli", "--surface", "foxhound-cli", "--surface", "sdk-python",
                ],
            },
        ])
    );
}

#[test]
fn an_exclusion_removes_its_variant_from_its_minor_alone() {
    let workspace = workspace(&THREE_MINORS, "0.0.0");
    let before = generated(&workspace);
    exclude(
        &workspace,
        "1.1",
        "J-003@foxhound-cli",
        "asserts a log line",
    );

    let after = generated(&workspace);

    let mut expected = before;
    let foxhound = &mut expected["jobs"][3]["steps"][0]["journeys"];
    let arguments = foxhound.as_array_mut().unwrap();
    arguments.extend([json!("--exclude"), json!("J-003@foxhound-cli")]);
    assert_eq!(after, expected);
}

/// The budget issue itself is `xtask budgets`' (see `budgets`), on the job's
/// measured time.
#[test]
fn checks_that_cannot_fit_are_noted_and_still_run() {
    let workspace = workspace(&["1.0.0", "1.1.0"], "0.0.0");
    workspace.edit("compat/skew-checks.toml", "minutes = 12", "minutes = 31");

    let outcome = run(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.notes(),
        [
            finding("over-budget", "1.0-2"),
            finding("over-budget", "1.1-2"),
        ]
    );
    let generated: Value = serde_json::from_str(&workspace.read_text(JOBS)).unwrap();
    let minor = |version: &str| -> Vec<String> {
        let mut checks: Vec<String> = jobs(&generated)
            .iter()
            .filter(|job| job["version"] == version)
            .flat_map(checks)
            .map(str::to_owned)
            .collect();
        checks.sort();
        checks
    };
    assert_eq!(minor("1.0.0"), RUN);
    assert_eq!(minor("1.1.0"), RUN);
}

#[rstest]
#[case::nothing_released(&[], "0.0.0")]
#[case::previews_have_no_skew(&["0.1.0", "0.1.1", "0.2.0"], "0.0.0")]
#[case::a_new_majors_first_release(&["1.0.0", "1.1.0"], "2.0.0")]
fn with_no_older_minor_the_current_build_runs_against_itself(
    #[case] versions: &[&str],
    #[case] current: &str,
) {
    let workspace = workspace(versions, current);

    let generated = generated(&workspace);

    let jobs = jobs(&generated);
    assert_eq!(names(&jobs), ["current-1", "current-2"]);
    assert_eq!(
        jobs.iter().flat_map(checks).collect::<Vec<_>>(),
        RUN.to_vec()
    );
    for job in &jobs {
        assert_eq!((&job["version"], &job["tag"]), (&Value::Null, &Value::Null));
        for step in job["steps"].as_array().unwrap() {
            assert_eq!(step["journeys"][0], "--product", "{step}");
        }
    }
}

/// On a release branch the list may hold a newer minor, which is no older
/// Client of the build.
#[test]
fn a_release_branch_takes_the_minors_up_to_its_own() {
    let workspace = workspace(&THREE_MINORS, "1.1.1");

    let generated = generated(&workspace);

    let jobs = jobs(&generated);
    let mut versions: Vec<&Value> = jobs.iter().map(|job| &job["version"]).collect();
    versions.dedup();
    assert_eq!(versions, ["1.0.1", "1.1.0"]);
}

#[test]
fn a_check_without_its_harness_is_listed_with_its_ticket() {
    let workspace = workspace(&["1.0.0"], "0.0.0");

    let generated = generated(&workspace);

    assert_eq!(
        generated["awaiting"],
        json!([{ "check": "rolling-upgrade", "issue": 436 }])
    );
}

#[rstest]
#[case::unreleased_minor("9.9", "J-003@cli", "why")]
#[case::malformed_journey("1.1", "J-3@cli", "why")]
#[case::unknown_surface("1.1", "J-003@console", "why")]
#[case::no_surface("1.1", "J-003", "why")]
#[case::empty_reason("1.1", "J-003@cli", " ")]
fn a_bad_exclusion_is_refused(#[case] minor: &str, #[case] variant: &str, #[case] reason: &str) {
    let workspace = workspace(&THREE_MINORS, "0.0.0");
    exclude(&workspace, minor, variant, reason);

    let outcome = run(&workspace);

    assert_ne!(outcome.code, 0);
    assert!(
        outcome.stderr.contains("compat/skew-exclusions.toml"),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(JOBS), None);
}

/// Every minor would have no job, and an empty matrix passes.
#[test]
fn checks_that_all_await_their_tickets_are_refused() {
    let workspace = workspace(&THREE_MINORS, "0.0.0");
    let text = CHECKS
        .replace("awaiting = 436\n", "")
        .replace("product = ", "awaiting = 436\nproduct = ");
    workspace.write("compat/skew-checks.toml", text.as_bytes());

    let outcome = run(&workspace);

    assert_ne!(outcome.code, 0);
    assert!(
        outcome.stderr.contains("compat/skew-checks.toml"),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read(JOBS), None);
}

#[rstest]
#[case::unknown_surface("surfaces = [\"cli\"]", "surfaces = [\"console\"]")]
#[case::no_surface("surfaces = [\"cli\"]", "surfaces = []")]
#[case::duplicate_id("id = \"collie-sdks\"", "id = \"collie-cli\"")]
#[case::neither_run_nor_awaited("awaiting = 436", "")]
#[case::zero_minutes("minutes = 8", "minutes = 0")]
fn a_bad_check_is_refused(#[case] from: &str, #[case] to: &str) {
    let workspace = workspace(&THREE_MINORS, "0.0.0");
    let text = CHECKS.replacen(from, to, 1);
    assert_ne!(text, CHECKS);
    workspace.write("compat/skew-checks.toml", text.as_bytes());

    let outcome = run(&workspace);

    assert_ne!(outcome.code, 0);
    assert!(
        outcome.stderr.contains("compat/skew-checks.toml"),
        "{}",
        outcome.stderr
    );
}
