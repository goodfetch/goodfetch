//! The caller as a Secret provider (ADRs 0067, 0092 and 0104; issue 1281):
//! the `caller-secret-resolution` client conformance case played against the
//! Scripted engine as a Daemon. Also verifies that `secret_requests()` fails
//! with `setting_invalid` when called without `declare_resolve_secret()` or
//! when a second stream is opened while one is active.

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::secrets::{SecretFailure, SecretUnavailableDetails};
use goodfetch_test_support::{TestEnv, case_logs};

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};
use crate::start::{code_group_retryable, failure};

const CANARY: &str = "canary-caller-secret-value-not-real";

struct Played {
    undeclared_error: Error,
    second_reader_error: Error,
    seen: Vec<(String, String, Option<String>)>,
    failed: Error,
}

async fn played(endpoint: DaemonEndpoint, hello: HelloParams) -> Played {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        hello,
    )
    .await
    .unwrap();

    // Reading secret_requests() without declaration fails setting_invalid.
    let undeclared_error = connection.secret_requests().unwrap_err();

    connection.declare_resolve_secret();
    let mut requests = connection.secret_requests().unwrap();
    // A second stream while one is active fails setting_invalid.
    let second_reader_error = connection.secret_requests().unwrap_err();

    let resolver = async move {
        let mut seen = Vec::new();
        while let Some(item) = requests.next().await {
            let req = item.unwrap();
            let debug_text = format!("{req:?}");
            assert!(!debug_text.contains(CANARY));
            seen.push((
                req.name().to_owned(),
                req.run_id().as_str().to_owned(),
                req.domain().map(str::to_owned),
            ));
            if req.name() == "shop_pw" {
                req.answer(CANARY).unwrap();
            } else {
                req.fail(format!("vault read failed for {CANARY}")).unwrap();
            }
        }
        seen
    };

    let caller = async {
        let run = connection.start().await.unwrap();
        let page = run.new_page().await.unwrap();
        page.locator("#pw").fill("%shop_pw%").await.unwrap();
        let failed = failure(page.locator("#otp").fill("%shop_otp%").await);
        run.finish().await.unwrap();
        connection.close().await.unwrap();
        failed
    };

    let (seen, failed) = tokio::join!(resolver, caller);

    Played {
        undeclared_error,
        second_reader_error,
        seen,
        failed,
    }
}

/// The `caller-secret-resolution` client conformance case, played by the
/// Scripted engine's Daemon: `secret_requests()` requires declaration, allows
/// only one reader (`setting_invalid` otherwise), answers `caller:` references
/// at the moment of use, and redacts failed errors so no canary leaks.
///
/// Covers: story:11-98
#[test]
fn answers_caller_references_and_enforces_single_reader() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "caller-secret-resolution",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let hello = daemon_hello();
    let (process, endpoint) =
        scripted_daemon_hearing(&env, "sdk-rust/run/caller-secret-resolution.json", &hello);
    let played = runtime.block_on(played(endpoint, hello));
    let status = process.child().wait().unwrap();

    assert_eq!(played.undeclared_error.code(), ErrorCode::SETTING_INVALID);
    assert_eq!(
        played.second_reader_error.code(),
        ErrorCode::SETTING_INVALID
    );
    assert_eq!(
        played.seen,
        vec![
            (
                "shop_pw".to_owned(),
                "01ke6p4ym0j8x3v5wq2rtbn7hc".to_owned(),
                Some("shop.test".to_owned()),
            ),
            (
                "shop_otp".to_owned(),
                "01ke6p4ym0j8x3v5wq2rtbn7hc".to_owned(),
                Some("shop.test".to_owned()),
            ),
        ]
    );
    assert_eq!(
        code_group_retryable(&played.failed),
        (
            ErrorCode::SECRET_UNAVAILABLE,
            ErrorGroup::Unavailable,
            false,
        )
    );
    assert_eq!(
        played.failed.details::<SecretUnavailableDetails>(),
        Some(SecretUnavailableDetails {
            placeholder: "shop_otp".to_owned(),
            reference: "caller:shop_otp".to_owned(),
            provider: "caller".to_owned(),
            failure: SecretFailure::ResolverFailed,
            fix: "The program's resolveSecret failed for `caller:shop_otp`; make it answer that name.".to_owned(),
        })
    );
    assert!(!format!("{}", played.failed).contains(CANARY));
    assert!(!format!("{:?}", played.failed).contains(CANARY));
    assert!(
        status.success(),
        "the Scripted engine played every step, so no canary reached the JSON-RPC error"
    );
}
