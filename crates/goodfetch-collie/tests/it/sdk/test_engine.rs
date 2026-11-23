//! Collie's testing kit (issue 3398, ADR 0105): `TestEngine` starts a real
//! engine, linked in the test's process, in a temporary Goodfetch home it
//! removes on close.

use goodfetch_collie::testing::{TestEngine, TestEngineOptions};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{TestEnv, real_time_runtime};

#[test]
fn a_test_runs_its_code_on_the_test_engine_s_connection_and_close_removes_its_home() {
    let env = TestEnv::new();
    let (report, home, existed) = real_time_runtime().block_on(async {
        let engine = TestEngine::start(TestEngineOptions::new().env(env.env_os()))
            .await
            .unwrap();
        let home = engine.connection().engine().home.clone().unwrap();
        let existed = home.is_dir();

        let report = engine
            .connection()
            .start()
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        engine.close().await.unwrap();
        (report, home, existed)
    });

    assert_eq!(report.lifecycle, Lifecycle::Finished);
    assert_ne!(home, env.home());
    assert_eq!(
        (existed, home.exists()),
        (true, false),
        "{}",
        home.display()
    );
}
