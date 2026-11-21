//! `blocked` as a Foxhound program reads it (ADRs 0079 and 0095): the
//! Scripted engine, as the Daemon of the home, answers a navigation with
//! `blocked` and the cause `decoy`, which Collie's client keeps as unknown;
//! the program reads it through `Error::details` as Foxhound's known
//! `Decoy`.

use goodfetch_collie_client::logging::engine_log_level;
use goodfetch_foxhound::{ConnectOptions, EngineChoice, ErrorCode, Foxhound};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{Form, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::foxhound::block::{BlockCause, BlockedDetails};
use goodfetch_test_support::{
    SCRIPTED_DAEMON_TOKEN, TestEnv, Tool, engine_script, real_time_runtime, reported_address, tools,
};
use serde_json::Value;

/// The script the Scripted engine plays as the Daemon of the home.
const SCRIPT: &str = "sdk-rust/run/blocked-decoy.json";

/// This SDK's `hello` to the Daemon of the home.
fn hello() -> HelloParams {
    HelloParams {
        needs: Some(Product::Foxhound),
        log_level: Some(engine_log_level()),
        token: Some(SCRIPTED_DAEMON_TOKEN.to_owned()),
        form: Some(Form::Rust),
        ..HelloParams::new(
            Surface::Rust,
            ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap(),
        )
    }
}

/// Covers: story:11-36, block-cause:decoy
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
fn a_blocked_answer_with_the_decoy_cause_reads_it_as_foxhounds_known_cause() {
    let env = TestEnv::new();
    let mut script: Value =
        serde_json::from_slice(&std::fs::read(engine_script(SCRIPT)).unwrap()).unwrap();
    script["steps"][0]["expect"]["params"] = serde_json::to_value(hello()).unwrap();
    let path = env.folder().join("script.json");
    std::fs::write(&path, serde_json::to_vec(&script).unwrap()).unwrap();
    let mut command = env.process_command(tools().unwrap().path(Tool::ScriptedEngine).unwrap());
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", "socket"])
        .stderr(std::process::Stdio::null());
    let daemon = env.spawn(&mut command).unwrap();
    reported_address(&mut daemon.child()).unwrap();
    let options = ConnectOptions::default()
        .environment(env.env_os())
        .engine(EngineChoice::Daemon);

    let error = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options).await.unwrap();
        let page = foxhound.start().await.unwrap().new_page().await.unwrap();
        let error = page.goto("https://shop.test/trap").await.unwrap_err();
        foxhound.close().await.unwrap();
        error
    });

    assert!(
        daemon.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
    assert_eq!(
        (
            error.code(),
            error.details::<BlockedDetails>().unwrap().cause
        ),
        (ErrorCode::BLOCKED, BlockCause::Decoy)
    );
}
