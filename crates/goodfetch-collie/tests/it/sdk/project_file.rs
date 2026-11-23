//! The project file `connect()` reads (ADR 0083; story 11-75). By default
//! it is found upward from the working directory, as the CLI finds it.
//! `project` names another and `no_project()` reads none. `engine()`
//! reports the file, which is logged at info. Its values reach the engine
//! at `project_configuration` on `config.explain` and `run.start`, and its
//! `timeout.connect` bounds `hello`. A remote Daemon, the Scripted engine,
//! refuses a value the project file has no authority for there, and the
//! connection raises that `setting_locked`. Each test runs in its own
//! process under nextest, so it may move its working directory.

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_collie::{
    Collie, ConnectOptions, EngineChoice, ErrorCode, ExplainedSetting, RemoteDaemon,
};
use goodfetch_protocol::checkpoint::FormatVersionUnsupportedDetails;
use goodfetch_protocol::settings::{SettingInvalidDetails, SettingLockedDetails, SettingsLayer};
use goodfetch_test_support::{TestEnv, real_time_runtime};
use rstest::rstest;
use serde_json::{Value, json};
use tracing::Level;
use tracing::subscriber::with_default;

use super::engine_choice::{hello, played_whole, remote, subprotocol, text};
use super::options;

/// The remote Daemon's token.
const TOKEN: &str = "endpoint-token-canary-5f0e9b27c4d1";

/// A project file's text that sets `timeout.navigation` to `navigation`
/// milliseconds.
fn navigation(navigation: u64) -> String {
    format!("version = \"0.1\"\n\n[settings.timeout]\nnavigation = {navigation}\n")
}

/// Writes `text` as the file `name` of `folder`, creating the folder, and
/// returns its path.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes its project file before any async worker runs"
)]
fn write(folder: &Path, name: &str, text: &str) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Moves the test's working directory to `folder`, created first, and
/// returns it as the OS names it.
#[expect(
    clippy::disallowed_methods,
    reason = "the test creates the folder it works in before any async worker runs"
)]
fn work_in(folder: &Path) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    std::env::set_current_dir(folder).unwrap();
    std::env::current_dir().unwrap()
}

/// `timeout.navigation` as the connection explains it, and as a Run it
/// starts explains it: its value and its layer each time.
async fn explained_navigation(collie: &Collie) -> (Value, Value) {
    let as_pair = |setting: &ExplainedSetting| json!([setting.value, setting.layer]);
    let connection = collie
        .explain_settings()
        .keys(["timeout.navigation"])
        .await
        .unwrap();
    let run = collie.start().await.unwrap();
    let own = run
        .explain_settings()
        .keys(["timeout.navigation"])
        .await
        .unwrap();
    run.finish().await.unwrap();
    (as_pair(&connection.settings[0]), as_pair(&own.settings[0]))
}

/// Covers: story:11-75, operation:config.explain, operation:run.start
#[test]
fn connect_reads_the_project_file_found_upward_from_the_working_directory() {
    let env = TestEnv::new();
    write(
        &env.folder().join("project"),
        "goodfetch.toml",
        &navigation(45_000),
    );
    let deeper = work_in(&env.folder().join("project").join("src").join("deeper"));
    // The working directory as the OS gives it, with no `\\?\` prefix on
    // Windows, which canonicalizing adds.
    let project = deeper
        .ancestors()
        .nth(2)
        .expect("two folders above")
        .join("goodfetch.toml");
    let (subscriber, log) = goodfetch_test_support::capturing(&[("goodfetch", Level::INFO)]);

    let (engine, explained) = with_default(subscriber, || {
        real_time_runtime().block_on(async {
            let collie = Collie::connect(options(&env)).await.unwrap();
            let explained = explained_navigation(&collie).await;
            collie.close().await.unwrap();
            (collie.engine().clone(), explained)
        })
    });

    let at_project = json!([45_000, "project_configuration"]);
    assert_eq!(
        (engine.project_file, explained),
        (Some(project.clone()), (at_project.clone(), at_project))
    );
    let written = text(&log);
    let reached = written
        .lines()
        .find(|line| line.contains("INFO") && line.contains("reached its engine"))
        .unwrap_or_default();
    assert!(
        reached.contains(&format!("project_file={:?}", project.display().to_string())),
        "{written}"
    );
}

/// Which project file a test's connection asks for.
#[derive(Clone, Copy, Debug)]
enum Asked {
    /// The other file, by its absolute path.
    Absolute,
    /// The other file, by its path from the working directory.
    Relative,
    /// None.
    None,
}

/// `project` names a file away from the working directory, read in place
/// of the one found there and reported by its absolute path, also when it
/// is named from the working directory, and `no_project()` reads none.
/// Covers: story:11-75, operation:config.explain, operation:run.start
#[rstest]
#[case::named(Asked::Absolute, json!([60_000, "project_configuration"]))]
#[case::named_from_the_working_directory(
    Asked::Relative,
    json!([60_000, "project_configuration"])
)]
#[case::none(Asked::None, json!([30_000, "default"]))]
fn project_names_another_file_and_no_project_reads_none(
    #[case] asked: Asked,
    #[case] expected: Value,
) {
    let env = TestEnv::new();
    write(
        &env.folder().join("work"),
        "goodfetch.toml",
        &navigation(45_000),
    );
    let elsewhere = write(
        &env.folder().join("elsewhere"),
        "other.toml",
        &navigation(60_000),
    );
    let work = work_in(&env.folder().join("work"));
    let relative = Path::new("..").join("elsewhere").join("other.toml");
    // Made absolute against the working directory as the OS makes a path
    // absolute: Unix keeps the `..`, Windows resolves it.
    let from_the_working_directory = if cfg!(windows) {
        work.parent()
            .expect("the working directory's parent")
            .join("elsewhere")
            .join("other.toml")
    } else {
        work.join(&relative)
    };
    let (options, reported) = match asked {
        Asked::Absolute => (options(&env).project(&elsewhere), Some(elsewhere)),
        Asked::Relative => (
            options(&env).project(&relative),
            Some(from_the_working_directory),
        ),
        Asked::None => (options(&env).no_project(), None),
    };

    let (engine, explained) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options).await.unwrap();
        let explained = explained_navigation(&collie).await;
        collie.close().await.unwrap();
        (collie.engine().clone(), explained)
    });

    assert_eq!(
        (engine.project_file, explained),
        (reported, (expected.clone(), expected))
    );
}

/// A file `project` names that is not there, or that does not read as a
/// project file, fails the connect with `setting_invalid` at
/// `project_configuration`, before any engine is reached.
/// Covers: story:11-75, error:setting_invalid
#[rstest]
#[case::missing(None)]
#[case::no_version(Some("[settings.timeout]\nnavigation = 45000\n"))]
#[case::no_toml(Some("version = \"0.1\"\n[settings\n"))]
fn a_named_project_file_that_does_not_read_fails_the_connect(#[case] text: Option<&str>) {
    let env = TestEnv::new();
    let folder = env.folder().join("work");
    let path = match text {
        Some(text) => write(&folder, "goodfetch.toml", text),
        None => folder.join("goodfetch.toml"),
    };

    let refused =
        real_time_runtime().block_on(async { Collie::connect(options(&env).project(&path)).await });

    let error = refused.unwrap_err();
    let details = error.details::<SettingInvalidDetails>();
    assert_eq!(
        (
            error.code(),
            details.as_ref().and_then(|details| details.layer),
            details.and_then(|details| details.fix).is_some()
        ),
        (
            ErrorCode::SETTING_INVALID,
            Some(SettingsLayer::ProjectConfiguration),
            true
        ),
        "{error:?}"
    );
}

/// A project file of a format version this engine cannot read fails the
/// connect with `format_version_unsupported`, before any engine is reached.
/// Covers: story:11-75, error:format_version_unsupported
#[test]
fn a_project_file_of_a_newer_format_fails_the_connect() {
    let env = TestEnv::new();
    let path = write(
        &env.folder().join("work"),
        "goodfetch.toml",
        "version = \"9.0\"\n\n[settings.timeout]\nnavigation = 45000\n",
    );

    let refused =
        real_time_runtime().block_on(async { Collie::connect(options(&env).project(&path)).await });

    let error = refused.unwrap_err();
    assert_eq!(
        (
            error.code(),
            error
                .details::<FormatVersionUnsupportedDetails>()
                .map(|details| details.found)
        ),
        (
            ErrorCode::FORMAT_VERSION_UNSUPPORTED,
            Some("9.0".to_owned())
        ),
        "{error:?}"
    );
}

/// A remote Daemon whose port takes the connection and never answers the
/// WebSocket handshake fails the connect once the project file's
/// `timeout.connect` passes, its `limit` naming the project file's layer;
/// the connection's own `timeout` wins over it.
/// Covers: story:11-75, setting:timeout.connect
#[rstest]
#[case::the_project_files(
    "[settings.timeout]\nconnect = 200\n",
    None,
    json!({ "key": "timeout.connect", "value": 200, "layer": "project_configuration" })
)]
#[case::the_project_files_written_whole(
    "[settings]\n\"timeout.connect\" = 200\n",
    None,
    json!({ "key": "timeout.connect", "value": 200, "layer": "project_configuration" })
)]
#[case::the_connections_own(
    "[settings.timeout]\nconnect = 200\n",
    Some(Duration::from_millis(300)),
    json!({ "key": "timeout.connect", "value": 300, "layer": "run", "origin": "argument" })
)]
fn a_project_files_connect_timeout_bounds_hello(
    #[case] settings: &str,
    #[case] own: Option<Duration>,
    #[case] limit: Value,
) {
    let env = TestEnv::new();
    #[expect(
        clippy::disallowed_types,
        reason = "the test holds a loopback port outside the engine, which takes connections and never answers (ADR 0060)"
    )]
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = silent.local_addr().unwrap();
    let project = write(
        &env.folder().join("work"),
        "goodfetch.toml",
        &format!("version = \"0.1\"\n\n{settings}"),
    );
    let mut options = ConnectOptions::default()
        .environment(env.env_os())
        .project(&project)
        .engine(EngineChoice::Remote(RemoteDaemon::new(
            format!("ws://{address}"),
            TOKEN,
        )));
    if let Some(own) = own {
        options = options.timeout(own);
    }

    let refused = real_time_runtime().block_on(async { Collie::connect(options).await });

    let error = refused.unwrap_err();
    assert_eq!(
        (error.code(), serde_json::to_value(error.limit()).unwrap()),
        (ErrorCode::DEADLINE_EXCEEDED, limit),
        "{error:?}"
    );
}

/// A project file's `timeout.connect` that is no whole number of 1 ms or
/// more fails the connect with `setting_invalid` at
/// `project_configuration`, before any engine is reached.
/// Covers: story:11-75, setting:timeout.connect, error:setting_invalid
#[rstest]
#[case::zero("0")]
#[case::text("\"30s\"")]
fn a_project_files_connect_timeout_that_is_no_duration_fails_the_connect(#[case] value: &str) {
    let env = TestEnv::new();
    let project = write(
        &env.folder().join("work"),
        "goodfetch.toml",
        &format!("version = \"0.1\"\n\n[settings.timeout]\nconnect = {value}\n"),
    );

    let refused = real_time_runtime()
        .block_on(async { Collie::connect(options(&env).project(&project)).await });

    let error = refused.unwrap_err();
    let details = error.details::<SettingInvalidDetails>();
    assert_eq!(
        (
            error.code(),
            details
                .as_ref()
                .map(|details| (details.key.as_str(), details.layer)),
        ),
        (
            ErrorCode::SETTING_INVALID,
            Some(("timeout.connect", Some(SettingsLayer::ProjectConfiguration))),
        ),
        "{error:?}"
    );
}

/// Against the Scripted engine as a remote Daemon, a Run whose start
/// carries a project value that widens `tls.trusted_cas` fails with the
/// Daemon's `setting_locked` at `project_configuration`.
/// Covers: story:11-75
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
fn a_remote_daemon_refuses_a_project_value_that_widens_trusted_cas() {
    let env = TestEnv::new();
    let project = write(
        &env.folder().join("work"),
        "goodfetch.toml",
        "version = \"0.1\"\n\n[settings.tls.trusted_cas]\n\"intranet.example\" = [\"/etc/ssl/intranet-ca.pem\"]\n",
    );
    let mut script: Value = serde_json::from_slice(
        &std::fs::read(goodfetch_test_support::engine_script(
            "connect/project-value-locked-by-a-remote-daemon.json",
        ))
        .unwrap(),
    )
    .unwrap();
    script["steps"][2]["expect"]["params"]["project"]["path"] =
        json!(project.display().to_string());
    let played = write(env.folder(), "project-script.json", &script.to_string());
    let (process, line) = remote(
        &env,
        played.to_str().unwrap(),
        &hello(TOKEN),
        &subprotocol(),
    );
    let url = format!("ws://{}", line["address"].as_str().unwrap());

    let refused = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            options(&env)
                .project(&project)
                .engine(EngineChoice::Remote(RemoteDaemon::new(url, TOKEN))),
        )
        .await
        .unwrap();
        let refused = collie.start().await.map(|_run| ());
        collie.close().await.unwrap();
        refused
    });

    played_whole(&process);
    let error = refused.unwrap_err();
    assert_eq!(
        (
            error.code(),
            error
                .details::<SettingLockedDetails>()
                .map(|details| (details.key, details.layer)),
        ),
        (
            ErrorCode::SETTING_LOCKED,
            Some((
                "tls.trusted_cas".to_owned(),
                SettingsLayer::ProjectConfiguration
            )),
        ),
        "{error:?}"
    );
}
