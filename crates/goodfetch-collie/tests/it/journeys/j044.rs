//! J-044, "Run the same SDK program on an Attached engine, then an Exposed
//! Daemon by two variables", on the `sdk-rust` surface (ADRs 0049, 0063,
//! 0067, 0078, 0083, 0092, 0095 and 0098; CL-06): the same Rust SDK program
//! runs with `engine` left at `auto` first on the linked engine and then on
//! an Exposed Daemon by setting `GOODFETCH_URL` and `GOODFETCH_TOKEN`, with
//! no code change. TypeScript's `tests/journeys/J-044.test.ts` and Python's
//! `tests/journeys/test_j044.py` are its twins.
#![expect(
    clippy::disallowed_methods,
    reason = "the test prepares the Exposed Daemon's home, starts `daemon run`, and reads its stdout log"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_collie::{Collie, ConnectOptions, EngineInfo, EngineKind, RunId};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::home::{DaemonFile, USER_CONFIGURATION_FILE, per_major};
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{
    Canaries, CapturedLog, JourneyRun, Output, Process, SecretKind, TestEnv, capturing,
    real_time_runtime,
};
use serde_json::{Value, json};
use tracing::Level;
use tracing::subscriber::with_default;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// How long the test waits for the Exposed Daemon to report its endpoint.
const LOG_DEADLINE: Duration = Duration::from_secs(60);

/// The Run ID the shared SDK program uses on both engines.
const RUN_ID: &str = "j044-same-program";

/// The setting the shared SDK program asks `explain_settings` for, which is
/// unset in both homes so both engines return its built-in default.
const EXPLAINED_KEY: &str = "timeout.navigation";

/// The bytes stored and read back by the shared SDK program.
const STORED_BYTES: &[u8] = b"j044-same-program-bytes";

/// The deterministic result of the shared SDK program, compared across engines.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProgramResult {
    run_id: String,
    lifecycle: String,
    absent: Option<Value>,
    stored: Option<Value>,
    stored_bytes: Option<(Vec<u8>, String)>,
    deleted: [bool; 2],
    explained: Value,
}

/// The Exposed Daemon started with `daemon run` in a separate Goodfetch home.
struct ExposedDaemon {
    process: Process,
    lines: mpsc::Receiver<String>,
    logged: Vec<String>,
}

impl ExposedDaemon {
    fn start(
        env: &TestEnv,
        engine: &Path,
        version: &ProductVersion,
        token: &str,
    ) -> (Self, String) {
        let daemon_home = env.folder().join("daemon-home");
        let major_dir = per_major(&daemon_home, version);
        std::fs::create_dir_all(&major_dir).unwrap();
        let config_path = major_dir.join(USER_CONFIGURATION_FILE);
        std::fs::write(
            &config_path,
            "version = \"0.1\"\n\n[settings]\ndaemon.address = \"127.0.0.1:0\"\n",
        )
        .unwrap();
        let token_path = major_dir.join(DaemonFile::Token.name());
        std::fs::write(&token_path, token).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let mut command = env.process_command(engine);
        command
            .args(["daemon", "run"])
            .env("GOODFETCH_HOME", &daemon_home)
            .stderr(Stdio::null());
        let process = env.spawn(&mut command).unwrap();
        let stdout = process.child().stdout.take().unwrap();
        let (sent, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if sent.send(line).is_err() {
                    return;
                }
            }
        });

        let mut daemon = Self {
            process,
            lines,
            logged: Vec::new(),
        };
        let endpoint = daemon.wait_for_endpoint();
        (daemon, format!("ws://{endpoint}"))
    }

    fn wait_for_endpoint(&mut self) -> String {
        loop {
            let line = self
                .lines
                .recv_timeout(LOG_DEADLINE)
                .unwrap_or_else(|_| panic!("Exposed Daemon never served: {:#?}", self.logged));
            self.logged.push(line.clone());
            if let Ok(record) = serde_json::from_str::<Value>(&line)
                && record["fields"]["message"] == json!("the foreground Daemon serves")
                && let Some(endpoint) = record["fields"]["endpoint"].as_str()
            {
                return endpoint.to_owned();
            }
        }
    }

    fn stop_cleanly(&mut self, remote_env: BTreeMap<OsString, OsString>) -> bool {
        real_time_runtime().block_on(async {
            let collie = Collie::connect(ConnectOptions::default().environment(remote_env))
                .await
                .unwrap();
            let _ = collie.call("daemon.stop").params(json!({})).await.unwrap();
            let _ = collie.close().await;
        });
        let status = self.process.child().wait().unwrap();
        while let Ok(line) = self.lines.recv_timeout(LOG_DEADLINE) {
            self.logged.push(line);
        }
        status.success()
    }
}

/// The shared SDK program: leaves `engine` at `auto`, connects with `environment`,
/// runs Collie operations, finishes the Run, and closes the Connection.
async fn run_program(
    environment: BTreeMap<OsString, OsString>,
    seed_url: &str,
) -> (EngineInfo, ProgramResult, String) {
    let options = ConnectOptions::default().environment(environment);
    let options_debug = format!("{options:?}");
    let collie = Collie::connect(options).await.unwrap();
    let engine = collie.engine().clone();
    let collie_debug = format!("{collie:?} {engine:?}");

    let run = collie
        .start()
        .run_id(&RunId::parse(RUN_ID).unwrap())
        .await
        .unwrap();
    let store = run.key_value_store();
    let absent: Option<Value> = store.get("item").await.unwrap();
    let payload = json!({
        "seed_url": seed_url,
        "count": 2,
        "nested": [1, null],
    });
    store.set("item", &payload).await.unwrap();
    let stored: Option<Value> = store.get("item").await.unwrap();
    store
        .set_bytes("note", STORED_BYTES, "text/plain")
        .await
        .unwrap();
    let stored_bytes = store
        .get_bytes("note")
        .await
        .unwrap()
        .map(|value| (value.content, value.content_type));
    let deleted = [
        store.delete("item").await.unwrap(),
        store.delete("item").await.unwrap(),
    ];
    let explained = run.explain_settings().keys([EXPLAINED_KEY]).await.unwrap();
    let explained = serde_json::to_value(&explained.settings).unwrap();
    let report = run.finish().await.unwrap();
    collie.close().await.unwrap();

    let result = ProgramResult {
        run_id: report.run_id.as_str().to_owned(),
        lifecycle: format!("{:?}", report.lifecycle),
        absent,
        stored,
        stored_bytes,
        deleted,
        explained,
    };
    (engine, result, format!("{options_debug} {collie_debug}"))
}

fn environment_with(env: &TestEnv, extra: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
    let mut map = env.env_os();
    for (name, value) in extra {
        map.insert(OsString::from(name), OsString::from(value));
    }
    map
}

fn captured_text(log: &CapturedLog) -> String {
    String::from_utf8(log.written()).unwrap()
}

fn engine_version(env: &TestEnv, engine: &Path) -> ProductVersion {
    let output = env
        .process_command(engine)
        .arg("--version")
        .output()
        .expect("run engine --version");
    assert!(
        output.status.success(),
        "{} --version failed: {}",
        engine.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("engine --version is UTF-8");
    stdout
        .split_whitespace()
        .find_map(|token| ProductVersion::parse(token).ok())
        .unwrap_or_else(|| panic!("no SemVer ProductVersion in `{stdout}`"))
}

/// Covers: story:14-62, story:14-63, row:CL-06
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn sdk_rust() {
    let mut run = JourneyRun::start(
        "J-044",
        "sdk-rust",
        Path::new(COLLIE),
        ResultDocument::journey,
    )
    .expect("J-044's sdk-rust variant starts");
    run.surface_package("goodfetch-collie", env!("CARGO_PKG_VERSION"));
    let version = engine_version(run.env(), run.engine());
    let seed_url = run
        .proving_ground()
        .expect("J-044 has a Proving ground")
        .start_url("catalog/2")
        .unwrap();

    let mut canaries = Canaries::new(run.env().seed());
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();

    let mut daemon: Option<ExposedDaemon> = None;
    let mut daemon_url = String::new();
    let mut local_outcome: Option<(EngineInfo, ProgramResult)> = None;
    let mut remote_outcome: Option<(EngineInfo, ProgramResult, String, String)> = None;
    let mut daemon_logs = String::new();
    let mut daemon_stopped = false;

    run.step(
        "Start an Exposed Daemon in a separate Goodfetch home on loopback port 0 with a planted Secret canary as its persistent token",
        |run| {
            let (started, url) = ExposedDaemon::start(run.env(), run.engine(), &version, &token);
            daemon = Some(started);
            daemon_url = url;
        },
    );
    run.step(
        "Run the SDK program with `engine` left at `auto` and neither `GOODFETCH_URL` nor `GOODFETCH_TOKEN` set, reading seeded Proving ground input, storing and reading values in the Run's Key-value store, explaining settings, and finishing the Run",
        |run| {
            let (engine, result, _) =
                real_time_runtime().block_on(run_program(run.env().env_os(), &seed_url));
            local_outcome = Some((engine, result));
        },
    );
    run.step(
        "Run the exact same SDK program with `GOODFETCH_URL` pointing at the Exposed Daemon and `GOODFETCH_TOKEN` carrying the Secret canary, with no code change",
        |run| {
            let (subscriber, sdk_log) = capturing(&[("goodfetch", Level::TRACE)]);
            let remote_env = environment_with(
                run.env(),
                &[("GOODFETCH_URL", &daemon_url), ("GOODFETCH_TOKEN", &token)],
            );
            let (engine, result, shown) = with_default(subscriber, || {
                real_time_runtime().block_on(run_program(remote_env, &seed_url))
            });
            remote_outcome = Some((engine, result, shown, captured_text(&sdk_log)));
        },
    );
    run.step(
        "Stop the Exposed Daemon cleanly and collect its logs",
        |run| {
            if let Some(mut exposed) = daemon.take() {
                let remote_env = environment_with(
                    run.env(),
                    &[("GOODFETCH_URL", &daemon_url), ("GOODFETCH_TOKEN", &token)],
                );
                daemon_stopped = exposed.stop_cleanly(remote_env);
                daemon_logs = exposed.logged.join("\n");
            }
        },
    );

    run.check(
        "The program returned identical results on the local engine and on the Exposed Daemon",
        |_| {
            let (local_engine, local_result) = local_outcome.as_ref().expect("local run completed");
            let (_, remote_result, _, _) = remote_outcome.as_ref().expect("remote run completed");
            assert_eq!(local_engine.kind, EngineKind::Linked);
            assert_eq!(remote_result, local_result);
            assert!(daemon_stopped, "Exposed Daemon exited cleanly");
        },
    );
    run.check(
        "On the Exposed Daemon, `EngineInfo` reported `kind` as `remote`, the `url` reached, Product `collie`, the executable's Product version, and protocol version `0.1`, with no home, engine path, or fallback",
        |_| {
            let (remote_engine, _, _, _) = remote_outcome.as_ref().expect("remote run completed");
            assert_eq!(
                *remote_engine,
                EngineInfo {
                    kind: EngineKind::Remote,
                    product: Product::Collie,
                    version: version.clone(),
                    protocol_version: PROTOCOL_VERSION,
                    project_file: None,
                    home: None,
                    engine_path: None,
                    url: Some(daemon_url.clone()),
                    fallback: None,
                }
            );
        },
    );
    run.check(
        "The Secret canary in `GOODFETCH_TOKEN` appeared nowhere in `EngineInfo`, SDK logs, Exposed Daemon logs, or program outputs",
        |_| {
            let (_, remote_result, shown, sdk_logs) =
                remote_outcome.as_ref().expect("remote run completed");
            let program_output = format!("{remote_result:?}");
            let hits = canaries
                .search(&[
                    Output::Bytes {
                        name: "engine_and_options_debug",
                        bytes: shown.as_bytes(),
                    },
                    Output::Bytes {
                        name: "sdk_logs",
                        bytes: sdk_logs.as_bytes(),
                    },
                    Output::Bytes {
                        name: "daemon_logs",
                        bytes: daemon_logs.as_bytes(),
                    },
                    Output::Bytes {
                        name: "program_output",
                        bytes: program_output.as_bytes(),
                    },
                ])
                .unwrap();
            assert!(hits.is_empty(), "canary leaked: {hits:?}");
        },
    );
    run.finish()
        .expect("J-044's document is written")
        .assert_proved();
}
