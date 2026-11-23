//! Collie's testing kit (issue 3398, ADR 0105): `TestEngine` starts a real
//! engine, linked in the test's process, in a temporary Goodfetch home it
//! removes on close, with a cleared environment and `egress.offline` on;
//! a host `network.resolve` maps to loopback alone is reached live and any
//! other is refused, and browsers come from the caller's Browser cache,
//! which the engine reads and never writes: the kit turns
//! `browser_cache.read_only` on (issue 3458). With a Daemon running in the
//! caller's home, the code under test runs unchanged on the Daemon's
//! connection and on the test engine's, which links its own engine. These
//! tests are the kit's sample under `cargo test`'s own harness, with no
//! plugin (issue 3399). TypeScript's `test-engine.test.ts` and Python's
//! `test_test_engine.py` are its twins.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use goodfetch_collie::settings::{Settings, network};
use goodfetch_collie::testing::{TestEngine, TestEngineOptions};
use goodfetch_collie::{Collie, ConnectOptions, EngineChoice, EngineKind, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{
    Daemon, Product, ProvingGround, TestEnv, browser_cache, case_logs, chrome_for_testing_files,
    empty_browser_cache, real_time_runtime,
};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 3398;

/// A host the test's `network.resolve` maps to an address that is not
/// loopback, TEST-NET-1's, which no Run reaches.
const PUBLIC_HOST: &str = "public.test";

/// The caller's environment map of `env`, its Goodfetch home and the
/// variables it sets, with `GOODFETCH_BROWSERS_PATH` naming `cache`.
fn caller(env: &TestEnv, cache: &Path) -> BTreeMap<OsString, OsString> {
    let mut caller = env.env_os();
    caller.insert("GOODFETCH_BROWSERS_PATH".into(), cache.into());
    caller
}

/// Every entry of the Browser cache at `cache`, its kind, length and
/// modification time, symbolic links unfollowed, by its path under
/// `cache`: the cache's own records, and not the build's files, where no
/// engine writes and the browser it launches keeps its own folders.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Browser cache's folder before and after the engine runs, outside any async worker"
)]
fn tree(cache: &Path) -> BTreeMap<PathBuf, (bool, u64, SystemTime)> {
    let build_files = chrome_for_testing_files(cache);
    let mut entries = BTreeMap::new();
    let mut pending = vec![cache.to_owned()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path == build_files {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.push(path.clone());
            }
            entries.insert(
                path.strip_prefix(cache).unwrap().to_owned(),
                (
                    metadata.is_dir(),
                    metadata.len(),
                    metadata.modified().unwrap(),
                ),
            );
        }
    }
    entries
}

/// The code under test: it starts a Run on `collie` and answers how it
/// ended.
async fn finished(collie: &Collie) -> Lifecycle {
    let run = collie.start().await.unwrap();
    run.finish().await.unwrap().lifecycle
}

/// The Testing-kit case, linked: the code under test takes the Product's
/// own connection, `&Collie`, and the engine's home is its own.
/// Covers: story:11-156, story:11-157, story:11-168, story:11-171
#[test]
fn a_test_runs_its_code_on_the_test_engine_s_connection_and_close_removes_its_home() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "test-engine-runs-the-code-under-test",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let (lifecycle, home, existed) = real_time_runtime().block_on(async {
        let engine = TestEngine::start(TestEngineOptions::new().env(env.env_os()))
            .await
            .unwrap();
        let connection: &Collie = engine.connection();
        let home = connection.engine().home.clone().unwrap();
        let existed = home.is_dir();

        let lifecycle = finished(connection).await;
        engine.close().await.unwrap();
        (lifecycle, home, existed)
    });

    assert_eq!(lifecycle, Lifecycle::Finished);
    assert_ne!(home, env.home());
    assert_eq!(
        (existed, home.exists()),
        (true, false),
        "{}",
        home.display()
    );
}

/// The Testing-kit case beside a Daemon: the Daemon of the caller's home
/// answers `connect()`, and the test engine links its own engine in its
/// own home, never reaching that Daemon.
/// Covers: story:11-157, story:11-168
#[test]
fn the_code_under_test_runs_unchanged_on_a_daemon_and_on_the_test_engine() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "test-engine-runs-the-code-under-test",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let daemon = Daemon::start(&env, Product::Collie).unwrap();

    let (on_the_daemon, on_the_test_engine, daemon_kind, reached) =
        real_time_runtime().block_on(async {
            let collie = Collie::connect(
                ConnectOptions::default()
                    .environment(env.env_os())
                    .engine(EngineChoice::Daemon),
            )
            .await
            .unwrap();
            let on_the_daemon = finished(&collie).await;
            let daemon_kind = collie.engine().kind;
            collie.close().await.unwrap();
            let engine = TestEngine::start(TestEngineOptions::new().env(env.env_os()))
                .await
                .unwrap();
            let on_the_test_engine = finished(engine.connection()).await;
            let reached = engine.connection().engine().clone();
            engine.close().await.unwrap();
            (on_the_daemon, on_the_test_engine, daemon_kind, reached)
        });
    daemon.stop(&env).unwrap();

    assert_eq!(
        (on_the_daemon, on_the_test_engine),
        (Lifecycle::Finished, Lifecycle::Finished)
    );
    assert_eq!(
        (daemon_kind, reached.kind),
        (EngineKind::Daemon, EngineKind::Linked)
    );
    assert_ne!(reached.home.as_deref(), Some(env.home()));
}

/// The caller's Browser cache stays writable here: the kit's
/// `browser_cache.read_only` alone keeps the engine from recording the
/// build's use in it (ADR 0098).
/// Covers: story:11-168, story:11-169
#[test]
fn a_loopback_resolve_entry_is_reached_live_any_other_host_is_refused_and_the_cache_is_unchanged() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut resolve: BTreeMap<String, Vec<String>> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), vec![address.clone()]))
        .collect();
    resolve.insert(PUBLIC_HOST.to_owned(), vec!["192.0.2.7".to_owned()]);
    let reached = proving_ground.start_url("statuses/1").unwrap();
    let refused = proving_ground.url(PUBLIC_HOST, "/");
    let before = tree(&cache);

    let (status, refusal) = real_time_runtime().block_on(async {
        let options = TestEngineOptions::new()
            .env(caller(&env, &cache))
            .settings(Settings::new().set(network::RESOLVE, resolve));
        let engine = TestEngine::start(options).await.unwrap();
        let run = engine.connection().start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let status = page
            .goto(&reached)
            .await
            .unwrap()
            .map(|response| response.status());
        let refusal = page.goto(&refused).await.map(|_| ()).unwrap_err().code();
        page.close().await.unwrap();
        run.finish().await.unwrap();
        engine.close().await.unwrap();
        (status, refusal)
    });

    assert_eq!((status, refusal), (Some(200), ErrorCode::EGRESS_REFUSED));
    assert_eq!(tree(&cache), before);
}

/// `artifact_unavailable_offline`, its fix naming `browser install`, and
/// the caller's empty Browser cache left empty.
/// Covers: story:11-169
#[test]
fn with_no_browser_installed_the_first_page_fails_offline_naming_browser_install() {
    let (env, cache) = empty_browser_cache(TestEnv::new()).unwrap();

    let failure = real_time_runtime().block_on(async {
        let engine = TestEngine::start(TestEngineOptions::new().env(env.env_os()))
            .await
            .unwrap();
        let run = engine.connection().start().await.unwrap();
        let failure = run.new_page().await.err();
        run.finish().await.unwrap();
        engine.close().await.unwrap();
        failure.expect("the first page fails")
    });

    assert_eq!(
        (failure.code(), failure.fix()),
        (
            ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
            Some(
                "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set"
            )
        )
    );
    assert_eq!(tree(&cache), BTreeMap::new());
}
