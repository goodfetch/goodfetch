use std::path::Path;
use std::time::SystemTime;

use serde_json::json;

use crate::exercise::{AnsweredCalls, COMMAND_RECORD};
use crate::locales::LocaleTableError;
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

/// On Linux an in-memory setup's folder, under the usual name, lies on a
/// memory file system, so a store test's synced writes and new files wait
/// for no disk however busy it is (fix-queue 123).
#[cfg(target_os = "linux")]
#[test]
fn an_in_memory_setup_keeps_its_folder_on_a_memory_file_system() {
    let env = TestEnv::in_memory();

    let name = env.folder().file_name().unwrap().to_str().unwrap();

    assert_eq!(
        (file_system_of(env.folder()), name.contains(" é ")),
        ("tmpfs".to_owned(), true),
        "{name}"
    );
}

/// The type of the file system `path` lies on: that of the longest mount
/// point above it in `/proc/self/mounts`.
#[cfg(target_os = "linux")]
fn file_system_of(path: &Path) -> String {
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap();
    mounts
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let point = fields.nth(1)?.replace("\\040", " ");
            Some((std::path::PathBuf::from(point), fields.next()?.to_owned()))
        })
        .filter(|(point, _)| path.starts_with(point))
        .max_by_key(|(point, _)| point.as_os_str().len())
        .map(|(_, kind)| kind)
        .unwrap()
}

/// The cleared map's variables, in order: those of every OS, and on Windows
/// those a process needs to start.
#[cfg(not(windows))]
const CLEARED_MAP: [&str; 11] = [
    "BREAKPAD_DUMP_LOCATION",
    "GOODFETCH_HOME",
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
#[cfg(windows)]
const CLEARED_MAP: [&str; 14] = [
    "BREAKPAD_DUMP_LOCATION",
    "ComSpec",
    "GOODFETCH_HOME",
    "HOME",
    "LANG",
    "PATHEXT",
    "SystemRoot",
    "TZ",
    "USERPROFILE",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "XDG_STATE_HOME",
];

#[test]
fn cleared_map_holds_only_the_listed_variables() {
    let variables = Variables::from_iter([
        ("GOODFETCH_TEST_SEED", "7"),
        ("SystemRoot", "C:\\Windows"),
        ("ComSpec", "C:\\Windows\\system32\\cmd.exe"),
        ("PATHEXT", ".COM;.EXE"),
    ]);

    let env = TestEnv::create("env::tests::cleared_map", &variables).unwrap();

    assert_eq!(
        env.env().keys().map(String::as_str).collect::<Vec<_>>(),
        CLEARED_MAP
    );
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

    let env = env.with_locale("tr-TR").unwrap();

    assert_eq!(env.env()["LANG"], "tr_TR.UTF-8");
}

#[test]
fn with_locale_refuses_a_tag_outside_the_shared_table() {
    let env = test_env("env::tests::unknown_locale");

    let refused = env.with_locale("tr_TR.UTF-8");

    assert!(
        matches!(&refused, Err(LocaleTableError::LocaleUnknown { tag }) if tag == "tr_TR.UTF-8"),
        "{:?}",
        refused.map(|_| ())
    );
}

#[test]
fn with_time_zone_refuses_a_zone_outside_the_shared_table() {
    let env = test_env("env::tests::unknown_time_zone");

    let refused = env.with_time_zone("Europe/Berlin");

    assert!(
        matches!(&refused, Err(LocaleTableError::TimeZoneUnknown { zone }) if zone == "Europe/Berlin"),
        "{:?}",
        refused.map(|_| ())
    );
}

#[test]
fn with_time_zone_replaces_tz() {
    let env = test_env("env::tests::time_zone");

    let env = env.with_time_zone("Asia/Kathmandu").unwrap();

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

/// The test home's user configuration, as written.
fn user_configuration(env: &TestEnv) -> String {
    std::fs::read_to_string(
        env.home()
            .join(crate::runtime_file::major_folder(env!("CARGO_PKG_VERSION")))
            .join("configuration.toml"),
    )
    .unwrap()
}

#[test]
fn user_configuration_turns_on_the_engines_json_log_with_the_dispatch_and_outcome_records() {
    let env = test_env("env::tests::log");

    assert_eq!(
        user_configuration(&env),
        "version = \"0.1\"\n\n[settings]\ngovernor.host_memory = false\nlog.output = \"file\"\nlog.file.max_files = 10000\n\
         log.level = \"info,goodfetch::rpc::dispatch=trace,goodfetch::run::outcome=trace\"\n"
    );
}

/// Where the tests run with no GUI session, every test's user
/// configuration sets the headless shell first, and a test's own setting of
/// it wins (#3224).
#[test]
fn with_no_gui_session_the_user_configuration_launches_the_headless_shell() {
    let variables: Variables = [(crate::NO_GUI_SESSION_VARIABLE, "1")]
        .into_iter()
        .collect();
    let env = TestEnv::create("env::tests::no_gui_session", &variables).unwrap();
    let created = user_configuration(&env);

    env.user_configuration("browser.headless_shell = false\n");

    assert_eq!(
        (created, user_configuration(&env)),
        (
            "version = \"0.1\"\n\n[settings]\nbrowser.headless_shell = true\ngovernor.host_memory = false\n\
             log.output = \"file\"\nlog.file.max_files = 10000\n\
             log.level = \"info,goodfetch::rpc::dispatch=trace,goodfetch::run::outcome=trace\"\n"
                .to_owned(),
            "version = \"0.1\"\n\n[settings]\ngovernor.host_memory = false\nlog.output = \"file\"\n\
             log.file.max_files = 10000\n\
             log.level = \"info,goodfetch::rpc::dispatch=trace,goodfetch::run::outcome=trace\"\n\
             browser.headless_shell = false\n"
                .to_owned(),
        )
    );
}

#[test]
fn a_log_level_keeps_the_settings_the_test_wrote_and_its_own_settings_win() {
    let env = test_env("env::tests::log_level");
    env.user_configuration("daemon.idle_timeout = 500\nlog.output = \"none\"\n");

    env.log_level("trace");

    assert_eq!(
        user_configuration(&env),
        "version = \"0.1\"\n\n[settings]\ngovernor.host_memory = false\nlog.file.max_files = 10000\ndaemon.idle_timeout = 500\n\
         log.output = \"none\"\nlog.level = \"trace\"\n"
    );
}

/// A setting whose key starts with a default's key, or whose value names
/// one, leaves that default in place: only a setting of the same key
/// replaces it.
#[test]
fn a_setting_that_only_spells_a_defaults_key_keeps_the_default() {
    let env = test_env("env::tests::key_prefix");

    env.user_configuration("log.output_format = \"json\"\nproxy.note = \"log.level\"\n");

    assert_eq!(
        user_configuration(&env),
        "version = \"0.1\"\n\n[settings]\ngovernor.host_memory = false\nlog.output = \"file\"\nlog.file.max_files = 10000\n\
         log.level = \"info,goodfetch::rpc::dispatch=trace,goodfetch::run::outcome=trace\"\n\
         log.output_format = \"json\"\nproxy.note = \"log.level\"\n"
    );
}

/// An engine reading the user configuration while the test sets a log
/// level, as each engine a test starts beside another does, reads the file
/// before or after, never cut: a read begun before the write ends on the
/// text it began with, and a read begun after it reads the new text whole.
/// The read straddles the write by itself, so the test needs no race and
/// no count of synced writes, whose time the disk's other writers set.
#[test]
fn a_log_level_set_while_an_engine_reads_the_user_configuration_never_cuts_it() {
    let env = test_env("env::tests::log_level_read");
    let before = user_configuration(&env);
    let mut reading = std::fs::File::open(
        env.home()
            .join(crate::runtime_file::major_folder(env!("CARGO_PKG_VERSION")))
            .join("configuration.toml"),
    )
    .unwrap();
    let mut begun = [0_u8; 10];
    std::io::Read::read_exact(&mut reading, &mut begun).unwrap();

    env.log_level("trace");

    let mut read = String::from_utf8(begun.to_vec()).unwrap();
    std::io::Read::read_to_string(&mut reading, &mut read).unwrap();
    assert_eq!(read, before);
    assert_eq!(
        user_configuration(&env),
        "version = \"0.1\"\n\n[settings]\ngovernor.host_memory = false\nlog.output = \"file\"\nlog.file.max_files = 10000\n\
         log.level = \"trace\"\n"
    );
}

/// Under `just coverage`, every executable a test starts writes its
/// coverage profile where cargo-llvm-cov reads it (ADR 0072).
#[test]
fn map_passes_the_coverage_profile_file_on_when_one_is_set() {
    let profile = "/work/target/llvm-cov-target/goodfetch-%p-%14m.profraw";
    let variables = Variables::from_iter([("LLVM_PROFILE_FILE", profile)]);

    let env = TestEnv::create("env::tests::coverage", &variables).unwrap();

    assert_eq!(
        env.env().get("LLVM_PROFILE_FILE").map(String::as_str),
        Some(profile)
    );
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

/// The calls a Client read answered reach the exercise record from the
/// answer record, after the command record and before the engines' logs:
/// each request's method, and its Error code when it failed; a reply to the
/// engine's own request, a notification and an answer to no request sent
/// are no call (#1779).
#[test]
fn dropping_the_value_copies_the_calls_its_clients_read_answered_into_the_tests_exercise_record() {
    let records = tempfile::tempdir().unwrap();
    let env = TestEnv::create("store::keeps_a_value", &recording(records.path())).unwrap();
    std::fs::write(
        env.folder().join(COMMAND_RECORD),
        "{\"target\":\"goodfetch::test::command\",\"fields\":{\"command\":\"attached-engine\",\"exit_code\":null}}\n",
    )
    .unwrap();
    let logs = env.home().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join("goodfetch-foxhound-7.jsonl"),
        "{\"target\":\"goodfetch::rpc::dispatch\",\"fields\":{\"method\":\"hello\"}}\n",
    )
    .unwrap();
    let mut answered = AnsweredCalls::default();
    for (sent, read) in [
        (
            json!({ "jsonrpc": "2.0", "id": "export", "method": "run.export", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": "export", "result": {} }),
        ),
        (
            json!({ "jsonrpc": "2.0", "id": 2, "method": "run.report", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32001, "message": "no", "data": { "code": "run_not_found" } } }),
        ),
        (
            json!({ "jsonrpc": "2.0", "id": 9, "result": {} }),
            json!({ "jsonrpc": "2.0", "id": 9, "method": "crawl_request.handle", "params": {} }),
        ),
        (
            json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 2 } }),
            json!({ "jsonrpc": "2.0", "id": "never", "result": {} }),
        ),
    ] {
        answered.sent(&sent);
        answered.read(&read);
    }
    answered.record_in(env.folder()).unwrap();
    let seen = env.exercise_lines();

    drop(env);

    let dispatched = |fields: serde_json::Value| json!({ "target": "goodfetch::rpc::dispatch", "fields": fields });
    let expected = vec![
        json!({ "test": "sample::it store::keeps_a_value" }),
        json!({ "target": "goodfetch::test::command", "fields": { "command": "attached-engine", "exit_code": null } }),
        dispatched(
            json!({ "message": "request answered, as its Client read it", "method": "run.export" }),
        ),
        dispatched(
            json!({ "message": "request failed, as its Client read it", "method": "run.report", "error_code": "run_not_found" }),
        ),
        dispatched(json!({ "method": "hello" })),
    ];
    let lines: Vec<serde_json::Value> = record_lines(records.path())
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!((lines, seen), (expected.clone(), expected[1..].to_vec()));
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

#[cfg(target_os = "linux")]
#[test]
fn a_lazyfs_tests_folder_is_made_in_the_folder_goodfetch_lazyfs_folder_names() {
    let parent = test_env("env::tests::lazyfs_parent");
    let variables =
        Variables::from_iter([("GOODFETCH_LAZYFS_FOLDER", parent.folder().to_str().unwrap())]);

    let env = TestEnv::create_for_lazyfs("env::tests::lazyfs_named", &variables).unwrap();

    assert_eq!(env.folder().parent(), Some(parent.folder()));
}

/// `/tmp` whatever `TMPDIR` names: the folders where the operating system
/// lets a user mount a FUSE file system include it (Ubuntu's AppArmor
/// profile of `fusermount3`), and a `TMPDIR` such as `/g` lies outside them.
#[cfg(target_os = "linux")]
#[test]
fn a_lazyfs_tests_folder_is_made_in_tmp_when_no_folder_is_named() {
    let env =
        TestEnv::create_for_lazyfs("env::tests::lazyfs_default", &Variables::default()).unwrap();

    assert_eq!(env.folder().parent(), Some(Path::new("/tmp")));
}
