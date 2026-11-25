//! Both Products' executables of one commit in one Goodfetch home (spec
//! #213's Seam 1, TESTING.md's "Two Products on one machine"): one Daemon
//! per major runs its pick, Foxhound once it counts as installed; a Collie
//! Daemon that is idle hands over to it when a Foxhound Client meets it; a
//! busy one fails the Client with a `fix` whose `daemon restart` makes
//! Foxhound the pick; and Clients of the two Products alternating restart
//! the Daemon at most once (ADR 0098); a Collie program on the Daemon
//! Foxhound started shares its Runs with both; and a Foxhound program a
//! busy Collie Daemon refuses links its own engine at `auto` and fails at
//! `daemon`. Collie's executable comes from the test tool manifest.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_collie::{Collie, ConnectOptions, EngineChoice, EngineKind, RunId};
use goodfetch_foxhound::{ErrorCode, Foxhound};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{DaemonClient, Hello};
use goodfetch_test_support::{
    RuntimeFile, SnapshotValues, TestEnv, Tool, capturing, cli, engine_script, real_time_runtime,
    reported_address, snapshot_settings, tools,
};
use serde_json::{Value, json};
use tracing::Level;
use tracing::subscriber::with_default;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn collie() -> PathBuf {
    tools().unwrap().path(Tool::Collie).unwrap().to_owned()
}

/// A user configuration whose `daemon.idle_timeout` bounds a Daemon a test
/// forgets.
fn short_idle(env: &TestEnv) {
    env.user_configuration("daemon.idle_timeout = 60000\n");
}

/// Stops the test home's Daemon when the test ends.
struct StopsTheDaemon<'env>(&'env TestEnv);

impl Drop for StopsTheDaemon<'_> {
    fn drop(&mut self) {
        let _ = cli(self.0, FOXHOUND).args(["daemon", "stop"]).run();
    }
}

fn json_of(executable: &Path, env: &TestEnv, arguments: &[&str]) -> Value {
    let output = cli(env, executable).args(arguments).run().unwrap();
    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    serde_json::from_str(&output.stdout).unwrap()
}

/// The running Daemon's Product and PID, as `daemon status` says.
fn running(env: &TestEnv) -> (Value, Value) {
    let status = json_of(FOXHOUND.as_ref(), env, &["daemon", "status", "--json"]);
    (status["product"].clone(), status["pid"].clone())
}

fn explain(executable: &Path, env: &TestEnv) -> goodfetch_test_support::CliOutput {
    cli(env, executable)
        .args(["config", "explain", "daemon", "--daemon", "--json"])
        .run()
        .unwrap()
}

/// A test's own Client of the Daemon, which keeps it busy while it is
/// connected.
struct Busy(#[expect(dead_code, reason = "held only to keep the Connection open")] DaemonClient);

impl Busy {
    fn on(env: &TestEnv) -> Self {
        let found = RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap();
        Self(DaemonClient::opened(
            &found.endpoint,
            &Hello::needing(Product::Collie)
                .surface(Surface::SdkTypescript)
                .token(&found.token),
        ))
    }
}

/// Covers: story:3-5, story:3-137, story:3-138
#[test]
fn a_foxhound_client_meeting_an_idle_collie_daemon_gets_a_foxhound_daemon() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    json_of(&collie(), &env, &["daemon", "start", "--json"]);
    let before = running(&env);

    let explained = explain(FOXHOUND.as_ref(), &env);
    let after = running(&env);

    assert_eq!(explained.exit, Some(0), "{}", explained.stderr);
    assert_eq!(
        (before.0, after.0, before.1 == after.1),
        (json!("collie"), json!("foxhound"), false)
    );
}

/// A busy Collie Daemon refuses a Foxhound Client under `--daemon` with
/// `not_in_product`, which exits `11`, as a refused protocol version does,
/// since the same `daemon restart` ends both (ticket #1183).
///
/// Covers: story:3-5, story:3-137, story:3-138, error:not_in_product, command:daemon.restart
#[test]
fn a_foxhound_installed_but_never_run_becomes_the_pick_when_the_fixs_daemon_restart_runs() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    json_of(&collie(), &env, &["daemon", "start", "--json"]);
    let busy = Busy::on(&env);

    let refused = explain(FOXHOUND.as_ref(), &env);
    let restarted = json_of(FOXHOUND.as_ref(), &env, &["daemon", "restart", "--json"]);
    drop(busy);

    assert_eq!(
        (refused.exit, restarted["product"].clone()),
        (Some(11), json!("foxhound")),
        "{}",
        refused.stderr
    );
    let values = SnapshotValues::of(&env).with_path(FOXHOUND, "[FOXHOUND]");
    snapshot_settings(&values).bind(|| {
        insta::assert_snapshot!("busy_collie_daemon_refuses_foxhound", refused.stdout.trim());
    });
}

/// Covers: story:3-137
#[test]
fn clients_of_the_two_products_alternating_restart_the_daemon_at_most_once() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);

    let pids: Vec<Value> = [collie(), FOXHOUND.into(), collie(), FOXHOUND.into()]
        .iter()
        .map(|executable| {
            let explained = explain(executable, &env);
            assert_eq!(explained.exit, Some(0), "{}", explained.stderr);
            running(&env).1
        })
        .collect();

    let changes = pids.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert_eq!((changes, running(&env).0), (1, json!("foxhound")));
}

/// The Run IDs `run list --daemon` answers through `executable`'s Client.
fn listed_on_the_daemon(executable: &Path, env: &TestEnv) -> Vec<Value> {
    let listed = json_of(executable, env, &["run", "list", "--daemon", "--json"]);
    listed["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["run_id"].clone())
        .collect()
}

/// A Collie program reaches the Daemon Foxhound's executable started, with
/// nothing in it changed for Foxhound, and the Run it starts there is in
/// the one Run index both Products' executables list (ADR 0098).
///
/// Covers: story:11-289
#[test]
fn a_collie_program_runs_on_the_daemon_foxhound_started_and_both_products_list_its_run() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    json_of(FOXHOUND.as_ref(), &env, &["daemon", "start", "--json"]);
    let shared = RunId::parse("shared").unwrap();

    let engine = real_time_runtime().block_on(async {
        let options = ConnectOptions::default()
            .environment(env.env_os())
            .engine(EngineChoice::Daemon);
        let collie = Collie::connect(options).await.unwrap();
        let engine = collie.engine().clone();
        collie.start().run_id(&shared).await.unwrap();
        collie.close().await.unwrap();
        engine
    });

    assert_eq!(
        (engine.kind, engine.product),
        (EngineKind::Daemon, Product::Foxhound)
    );
    assert_eq!(
        (
            listed_on_the_daemon(&collie(), &env),
            listed_on_the_daemon(FOXHOUND.as_ref(), &env),
            running(&env).0
        ),
        (
            vec![json!("shared")],
            vec![json!("shared")],
            json!("foxhound")
        )
    );
}

/// A Collie Client a Daemon refuses by version is told to install Foxhound
/// too, since a Foxhound of the major counts as installed and its own
/// executable would still not be the pick (ADR 0098).
///
/// Covers: story:3-5
#[test]
fn a_refused_collie_client_is_told_to_install_foxhound_while_a_foxhound_is_the_pick() {
    let env = TestEnv::new();
    let recorded = cli(&env, FOXHOUND)
        .args(["config", "explain", "daemon"])
        .run()
        .unwrap();
    assert_eq!(recorded.exit, Some(0), "{}", recorded.stderr);
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(engine_script("daemon/cli-hello-refused.json"))
        .args(["--mode", "socket"])
        .stderr(Stdio::null());
    let daemon = env.spawn(&mut command).unwrap();
    reported_address(&mut daemon.child()).unwrap();

    let refused = cli(&env, collie())
        .args(["config", "explain", "daemon", "--daemon"])
        .run()
        .unwrap();
    daemon.child().wait().unwrap();

    assert_eq!(refused.exit, Some(11), "{}", refused.stderr);
    let values = SnapshotValues::of(&env).with_path(&collie().display().to_string(), "[COLLIE]");
    snapshot_settings(&values).bind(|| {
        insta::assert_snapshot!(
            "refused_collie_told_to_install_foxhound",
            refused.stderr.trim()
        );
    });
}

/// Under `--daemon`, a Daemon of another Product that refuses the command
/// by Product and cannot hand over ends it with exit `11` and a `fix`
/// naming this executable's `daemon restart` by its path, read from the
/// Scripted engine playing a busy Collie Daemon (ticket #1183, ADR 0098).
///
/// Covers: story:11-213, command:run.start, error:not_in_product
#[test]
fn under_daemon_a_daemon_of_another_product_fails_with_exit_11_and_its_daemon_restart() {
    let env = TestEnv::new();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(engine_script("daemon/cli-hello-refused-product.json"))
        .args(["--mode", "socket"])
        .stderr(Stdio::null());
    let daemon = env.spawn(&mut command).unwrap();
    reported_address(&mut daemon.child()).unwrap();

    let refused = cli(&env, FOXHOUND)
        .args(["run", "start", "--run-id", "first", "--daemon"])
        .run()
        .unwrap();
    daemon.child().wait().unwrap();

    assert_eq!(
        (refused.exit, refused.stdout.as_str()),
        (Some(11), ""),
        "{refused}"
    );
    let values = SnapshotValues::of(&env).with_path(FOXHOUND, "[FOXHOUND]");
    snapshot_settings(&values).bind(|| {
        insta::assert_snapshot!("run_start_refused_by_product", refused.stderr.trim());
    });
}

/// What a Foxhound program opens with: the test's environment, which names
/// its Goodfetch home, at `engine`.
fn foxhound_options(env: &TestEnv, engine: EngineChoice) -> goodfetch_foxhound::ConnectOptions {
    goodfetch_foxhound::ConnectOptions::default()
        .environment(env.env_os())
        .engine(engine)
}

/// The `fix` a busy Collie Daemon's refusal ends with for a Rust program
/// linking Foxhound (ADR 0098): the install of Foxhound's executable, then
/// its `daemon restart`.
const FOXHOUND_RESTART: &str = "Install Foxhound's executable at 0.0.0 (`cargo install goodfetch-foxhound --locked --features executable`), then run its `daemon restart` to restart the Daemon on its major's pick.";

/// A Foxhound program on `auto` that a busy Collie Daemon refuses by
/// Product links its own engine, as it would with no Daemon: `engine()`
/// says why in `fallback`, one warning reaches `goodfetch::client`, and
/// the Collie Daemon serves on (ADR 0098).
///
/// Covers: story:11-1006, error:not_in_product
#[test]
fn a_foxhound_program_on_auto_links_its_own_engine_past_a_busy_collie_daemon() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    json_of(&collie(), &env, &["daemon", "start", "--json"]);
    let busy = Busy::on(&env);
    let (subscriber, log) = capturing(&[("goodfetch::client", Level::WARN)]);

    let engine = with_default(subscriber, || {
        real_time_runtime().block_on(async {
            let foxhound = Foxhound::connect(foxhound_options(&env, EngineChoice::Auto))
                .await
                .unwrap();
            let engine = foxhound.engine().clone();
            foxhound.close().await.unwrap();
            engine
        })
    });
    let after = running(&env);
    drop(busy);

    let fallback = engine.fallback.clone().expect("the fallback is reported");
    assert_eq!(
        (engine.kind, engine.product, after.0),
        (EngineKind::Linked, Product::Foxhound, json!("collie"))
    );
    assert!(
        fallback.cause.contains("not_in_product") && fallback.fix.ends_with(FOXHOUND_RESTART),
        "{fallback:?}"
    );
    let written = String::from_utf8(log.written()).unwrap();
    assert!(
        written.lines().count() == 1 && written.contains("refused it with not_in_product"),
        "{written}"
    );
}

/// A Foxhound program on `daemon` that a busy Collie Daemon refuses by
/// Product fails the connect with `not_in_product`, whose `fix` names the
/// install of Foxhound's executable and its `daemon restart`; nothing is
/// warned (ADR 0098).
///
/// Covers: story:11-1006, error:not_in_product
#[test]
fn a_foxhound_program_on_daemon_fails_past_a_busy_collie_daemon_with_its_daemon_restart() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    json_of(&collie(), &env, &["daemon", "start", "--json"]);
    let busy = Busy::on(&env);
    let (subscriber, log) = capturing(&[("goodfetch::client", Level::WARN)]);

    let refused = with_default(subscriber, || {
        real_time_runtime().block_on(async {
            Foxhound::connect(foxhound_options(&env, EngineChoice::Daemon))
                .await
                .err()
        })
    });
    drop(busy);

    let error = refused.expect("the connect fails");
    assert_eq!(error.code(), ErrorCode::NOT_IN_PRODUCT, "{error:?}");
    let fix = error.fix().unwrap_or_default();
    assert!(fix.ends_with(FOXHOUND_RESTART), "{fix}");
    assert_eq!(String::from_utf8(log.written()).unwrap(), "");
}
