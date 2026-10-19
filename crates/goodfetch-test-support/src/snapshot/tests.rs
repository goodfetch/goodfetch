use super::{SnapshotValues, snapshot_settings};
use crate::env::TestEnv;

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
