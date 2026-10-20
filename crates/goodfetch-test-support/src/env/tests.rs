use std::path::Path;
use std::time::SystemTime;

use crate::variables::Variables;

use super::TestEnv;

fn test_env(name: &str) -> TestEnv {
    TestEnv::create(name, &Variables::default()).unwrap()
}

#[test]
fn two_values_never_share_a_folder_or_a_home() {
    let first = test_env("env::tests::shared");

    let second = test_env("env::tests::shared");

    assert_ne!(first.folder(), second.folder());
    assert_ne!(first.home(), second.home());
}

#[test]
fn folder_name_holds_a_space_and_a_non_ascii_letter() {
    let env = test_env("env::tests::folder_name");

    let name = env.folder().file_name().unwrap().to_str().unwrap();

    assert!(name.starts_with("env-tests-folder_name é "), "{name}");
}

#[test]
fn cleared_map_holds_only_the_listed_variables() {
    let variables = Variables::from_iter([
        ("GOODFETCH_TEST_SEED", "7"),
        ("SystemRoot", "C:\\Windows"),
        ("ComSpec", "C:\\Windows\\system32\\cmd.exe"),
        ("PATHEXT", ".COM;.EXE"),
    ]);

    let env = TestEnv::create("env::tests::cleared_map", &variables).unwrap();

    let mut expected = vec![
        "BREAKPAD_DUMP_LOCATION",
        "GOODFETCH_HOME",
        "GOODFETCH_LOG",
        "GOODFETCH_LOG_OUTPUT",
        "HOME",
        "LANG",
        "TZ",
        "USERPROFILE",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
        "XDG_STATE_HOME",
    ];
    if cfg!(windows) {
        expected.extend(["ComSpec", "PATHEXT", "SystemRoot"]);
        expected.sort_unstable();
    }
    assert_eq!(env.env().keys().collect::<Vec<_>>(), expected);
}

#[test]
fn every_folder_of_the_map_lies_inside_the_test_folder() {
    let env = test_env("env::tests::inside");

    let outside: Vec<_> = [
        "BREAKPAD_DUMP_LOCATION",
        "GOODFETCH_HOME",
        "HOME",
        "USERPROFILE",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
        "XDG_STATE_HOME",
    ]
    .into_iter()
    .filter(|name| {
        let path = Path::new(&env.env()[*name]);
        !(path.starts_with(env.folder()) && path.is_dir())
    })
    .collect();

    assert_eq!(outside, Vec::<&str>::new());
}

#[test]
fn goodfetch_home_is_the_home_folder() {
    let env = test_env("env::tests::home");

    let home = Path::new(&env.env()["GOODFETCH_HOME"]);

    assert_eq!(home, env.folder().join("home"));
    assert_eq!(env.home(), home);
}

#[test]
fn crash_dump_folder_lies_in_the_goodfetch_home_and_the_map_names_it() {
    let env = test_env("env::tests::crash_dumps");

    let crash_dumps = Path::new(&env.env()["BREAKPAD_DUMP_LOCATION"]);

    assert_eq!(crash_dumps, env.folder().join("home").join("crash-dumps"));
    assert_eq!(env.crash_dump_folder(), crash_dumps);
    assert!(crash_dumps.is_dir());
}

#[test]
fn crash_dump_folder_is_removed_when_the_value_drops() {
    let env = test_env("env::tests::crash_dumps_removed");
    let crash_dumps = env.crash_dump_folder().to_owned();

    drop(env);

    assert!(!crash_dumps.exists());
}

#[test]
fn time_zone_and_locale_default_to_utc_and_c_utf8() {
    let env = test_env("env::tests::defaults");

    let pair = (env.env()["TZ"].as_str(), env.env()["LANG"].as_str());

    assert_eq!(pair, ("UTC", "C.UTF-8"));
}

#[test]
fn with_locale_replaces_lang() {
    let env = test_env("env::tests::locale");

    let env = env.with_locale("tr-TR");

    assert_eq!(env.env()["LANG"], "tr_TR.UTF-8");
}

#[test]
fn with_time_zone_replaces_tz() {
    let env = test_env("env::tests::time_zone");

    let env = env.with_time_zone("Asia/Kathmandu");

    assert_eq!(env.env()["TZ"], "Asia/Kathmandu");
}

#[test]
fn folder_is_removed_when_the_value_drops() {
    let env = test_env("env::tests::removed");
    let folder = env.folder().to_owned();

    drop(env);

    assert!(!folder.exists());
}

#[test]
fn seed_and_suite_come_from_the_variables() {
    let variables = Variables::from_iter([
        ("GOODFETCH_TEST_SEED", "17"),
        ("GOODFETCH_TEST_SUITE", "release"),
    ]);

    let env = TestEnv::create("env::tests::seed", &variables).unwrap();

    assert_eq!((env.seed(), env.suite()), (17, crate::Suite::Release));
}

#[test]
fn start_time_is_monday_the_fifth_of_january_2026_at_nine_utc() {
    let env = test_env("env::tests::start_time");

    let start = env.start_time();

    let expected: jiff::Timestamp = "2026-01-05T09:00:00Z".parse().unwrap();
    assert_eq!(start, SystemTime::from(expected));
}

#[test]
fn unique_names_join_the_prefix_the_seed_and_a_counter() {
    let variables = Variables::from_iter([("GOODFETCH_TEST_SEED", "7")]);
    let env = TestEnv::create("env::tests::unique", &variables).unwrap();

    let names = [env.unique_name("keychain"), env.unique_name("keychain")];

    assert_eq!(names, ["keychain-7-1", "keychain-7-2"]);
}

#[test]
fn an_unreadable_seed_fails_the_creation() {
    let variables = Variables::from_iter([("GOODFETCH_TEST_SEED", "seventeen")]);

    let created = TestEnv::create("env::tests::bad_seed", &variables);

    assert!(matches!(
        created,
        Err(super::TestEnvError::Seed(crate::SeedError::NotANumber))
    ));
}

#[test]
fn map_turns_on_the_engines_json_log_with_the_dispatch_records() {
    let env = test_env("env::tests::log");

    let log = (
        env.env()["GOODFETCH_LOG_OUTPUT"].as_str(),
        env.env()["GOODFETCH_LOG"].as_str(),
    );

    assert_eq!(log, ("file", "info,goodfetch::rpc::dispatch=trace"));
}

/// The variables of a test nextest runs under `just test`, whose exercise
/// records go to `records`.
fn recording(records: &Path) -> Variables {
    Variables::from_iter([
        ("GOODFETCH_TEST_RECORDS", records.to_str().unwrap()),
        ("NEXTEST_BINARY_ID", "sample::it"),
        ("NEXTEST_TEST_NAME", "store::keeps_a_value"),
    ])
}

/// Every line of every file in `records`.
fn record_lines(records: &Path) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(records)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|file| {
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn dropping_the_value_copies_the_engines_logs_of_its_home_into_the_tests_exercise_record() {
    let records = tempfile::tempdir().unwrap();
    let env = TestEnv::create("store::keeps_a_value", &recording(records.path())).unwrap();
    let logs = env.home().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join("goodfetch-collie-7.jsonl"),
        "{\"target\":\"goodfetch::rpc::dispatch\",\"fields\":{\"method\":\"hello\"}}\n",
    )
    .unwrap();

    drop(env);

    assert_eq!(
        record_lines(records.path()),
        [
            r#"{"test":"sample::it store::keeps_a_value"}"#,
            r#"{"target":"goodfetch::rpc::dispatch","fields":{"method":"hello"}}"#,
        ]
    );
}

#[test]
fn a_test_whose_engine_never_logged_still_leaves_its_exercise_record() {
    let records = tempfile::tempdir().unwrap();
    let env = TestEnv::create("store::keeps_a_value", &recording(records.path())).unwrap();

    drop(env);

    assert_eq!(
        record_lines(records.path()),
        [r#"{"test":"sample::it store::keeps_a_value"}"#]
    );
}

#[test]
fn outside_nextest_a_test_leaves_no_exercise_record() {
    let records = tempfile::tempdir().unwrap();
    let variables =
        Variables::from_iter([("GOODFETCH_TEST_RECORDS", records.path().to_str().unwrap())]);
    let env = TestEnv::create("store::keeps_a_value", &variables).unwrap();

    drop(env);

    assert_eq!(record_lines(records.path()), Vec::<String>::new());
}
