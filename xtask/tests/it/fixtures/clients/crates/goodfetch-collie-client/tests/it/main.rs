//! The Rust SDK's tests on a Daemon.

#[tokio::test]
async fn refuses_a_newer_major() {
    let _case = goodfetch_conformance::ClientCaseRun::start(
        None,
        "hello-refused",
        goodfetch_conformance::RustMode::Daemon,
    )
    .unwrap();
}
