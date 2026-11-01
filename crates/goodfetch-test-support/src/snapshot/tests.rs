use serde::Deserialize;

use super::{SnapshotValues, redact, snapshot_settings};
use crate::env::TestEnv;

/// The shared cases every language's `redact` passes.
const CASES: &str = include_str!("../../../../test-support/fixtures/snapshot-filter-cases.json");

#[derive(Deserialize)]
struct Cases {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    description: String,
    output: String,
    redacted: String,
}

/// One run's values, as an output would print them.
struct Run<'a> {
    id: &'a str,
    schedule: &'a str,
    instant: &'a str,
    duration: &'a str,
    port: u16,
    product_version: &'a str,
    commit: &'a str,
}

fn output(env: &TestEnv, run: &Run<'_>) -> String {
    format!(
        "run {} of Schedule {} started {}, took {}\nwrote {}/home/report.json\nengine goodfetch-collie {} ({})\nproving ground at http://127.0.0.1:{}/pages",
        run.id,
        run.schedule,
        run.instant,
        run.duration,
        env.folder().display(),
        run.product_version,
        run.commit,
        run.port,
    )
}

const FIRST: Run<'static> = Run {
    id: "01JH8Z5Q3W4X5Y6Z7A8B9C0D1E",
    schedule: "01JH8Z5Q3W4X5Y6Z7A8B9C0D1F",
    instant: "2026-01-05T09:00:00Z",
    duration: "1.5s",
    port: 54_321,
    product_version: "0.3.1",
    commit: "4f1c2e9a7b3d5f6e8a9b0c1d2e3f4a5b6c7d8e9f",
};

const SECOND: Run<'static> = Run {
    id: "01K0AB1C2D3E4F5G6H7J8K9M0N",
    schedule: "01K0AB1C2D3E4F5G6H7J8K9M0P",
    instant: "2026-03-29T01:59:59.123+01:00",
    duration: "250ms",
    port: 8080,
    product_version: "0.4.0",
    commit: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
};

fn values(env: &TestEnv, run: &Run<'_>) -> SnapshotValues {
    SnapshotValues::of(env).with_executable(run.product_version, run.commit)
}

#[test]
fn two_runs_of_one_output_snapshot_alike_under_the_shared_settings() {
    let first = TestEnv::new();
    let second = TestEnv::new();

    let first_output = output(&first, &FIRST);
    let second_output = output(&second, &SECOND);

    snapshot_settings(&values(&first, &FIRST)).bind(|| {
        insta::assert_snapshot!(first_output, @r"
        run [ULID] of Schedule [ULID] started [INSTANT], took [DURATION]
        wrote [FOLDER]/home/report.json
        engine goodfetch-collie [PRODUCT_VERSION] ([COMMIT])
        proving ground at http://127.0.0.1:[PORT]/pages
        ");
    });
    snapshot_settings(&values(&second, &SECOND)).bind(|| {
        insta::assert_snapshot!(second_output, @r"
        run [ULID] of Schedule [ULID] started [INSTANT], took [DURATION]
        wrote [FOLDER]/home/report.json
        engine goodfetch-collie [PRODUCT_VERSION] ([COMMIT])
        proving ground at http://127.0.0.1:[PORT]/pages
        ");
    });
}

#[test]
fn a_value_outside_the_shared_rules_stays_in_the_snapshot() {
    let env = TestEnv::new();

    let output = "proxy https://example.com:8443/ answered token 7c1f in state succeeded";

    snapshot_settings(&SnapshotValues::of(&env)).bind(|| {
        insta::assert_snapshot!(output, @"proxy https://example.com:8443/ answered token 7c1f in state succeeded");
    });
}

#[test]
fn each_shared_case_redacts_as_its_file_says() {
    let cases: Cases = serde_json::from_str(CASES).unwrap();

    let failed: Vec<String> = cases
        .cases
        .iter()
        .filter_map(|case| {
            let redacted = redact(&case.output, &SnapshotValues::default());
            (redacted != case.redacted).then(|| format!("{}: {redacted:?}", case.description))
        })
        .collect();

    assert_eq!(failed, Vec::<String>::new());
}

/// A Run ID as the engine writes it, in lowercase (ADR 0059), redacts
/// under insta's settings too.
#[test]
fn a_lowercase_run_id_redacts_under_the_shared_settings() {
    let env = TestEnv::new();

    snapshot_settings(&SnapshotValues::of(&env)).bind(|| {
        insta::assert_snapshot!("run 01ke6p4ym0kwx1ravx9sfp18dj finished", @"run [ULID] finished");
    });
}

/// The test's folder redacts as its path, its canonical path and both
/// written inside a JSON string, the forms an output prints it in: here a
/// folder reached through a link, whose name holds a quote and a
/// backslash, which JSON escapes.
#[test]
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test makes and canonicalizes its own folders outside any async worker"
)]
fn the_tests_folder_redacts_in_each_form_an_output_prints_it() {
    let env = TestEnv::new();
    let real = env.folder().join(r#"real "q" \b"#);
    std::fs::create_dir(&real).unwrap();
    let link = env.folder().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let folder = link.display().to_string();
    let canonical = std::fs::canonicalize(&link).unwrap().display().to_string();
    let escaped = |path: String| serde_json::to_string(&path).unwrap();
    let output = format!(
        "{folder}/a {canonical}/b {{\"path\":{}}} {{\"path\":{}}}",
        escaped(format!("{folder}/c")),
        escaped(format!("{canonical}/d")),
    );
    let values = SnapshotValues {
        folder: Some(folder),
        ..SnapshotValues::default()
    };

    let redacted = redact(&output, &values);

    assert_eq!(
        redacted,
        r#"[FOLDER]/a [FOLDER]/b {"path":"[FOLDER]/c"} {"path":"[FOLDER]/d"}"#
    );
}

/// A path the test names, such as an executable's, redacts as its path,
/// its canonical path and both written inside a JSON string: here an
/// executable reached through a linked build folder, which an executable
/// names by its canonical path when the OS resolves the link for it.
#[test]
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test makes and canonicalizes its own folders outside any async worker"
)]
fn a_named_path_redacts_in_each_form_an_output_prints_it() {
    let env = TestEnv::new();
    let real = env.folder().join("volume");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("goodfetch-collie"), "").unwrap();
    let target = env.folder().join("target");
    std::os::unix::fs::symlink(&real, &target).unwrap();
    let executable = target.join("goodfetch-collie").display().to_string();
    let canonical = std::fs::canonicalize(&executable)
        .unwrap()
        .display()
        .to_string();
    let output = format!(
        "Run `{executable} daemon restart`. Run `{canonical} daemon restart`. {{\"path\":{}}}",
        serde_json::to_string(&canonical).unwrap(),
    );

    let redacted = redact(
        &output,
        &SnapshotValues::of(&env).with_path(&executable, "[COLLIE]"),
    );

    assert_eq!(
        redacted,
        r#"Run `[COLLIE] daemon restart`. Run `[COLLIE] daemon restart`. {"path":"[COLLIE]"}"#
    );
}
