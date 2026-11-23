//! Complete error details (ADRs 0092 and 0114) through the client crate:
//! the corpus's script of the `complete-error-details` client conformance
//! case, which the TypeScript and Python SDKs play too, with the Scripted
//! engine as the Daemon. A fill whose one-time code never came fails with
//! `secret_unavailable`, whose typed details say `no_code`, and a failure
//! with `page_navigated_during_call` reads both documents through
//! `PageNavigatedDuringCallDetails`. The Scripted engine fails on a message
//! it does not expect and on a step left unused, so its clean exit says the
//! crate sent exactly what the script holds. This is Rust's `daemon` mode
//! play.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::page::ai_work::PageNavigatedDuringCallDetails;
use goodfetch_protocol::secrets::{SecretFailure, SecretUnavailableDetails};
use goodfetch_test_support::{TestEnv, case_logs};

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};
use crate::start::{code_group_retryable, failure};

/// What the script's calls answered.
struct Played {
    /// The fill of the one-time code no provider made.
    no_code: Error,
    /// The click whose document a navigation it did not start replaced.
    navigated: Error,
}

/// Plays the script's calls on a Connection to the Daemon at `endpoint`.
async fn played(endpoint: DaemonEndpoint, hello: HelloParams) -> Played {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        hello,
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    let no_code = failure(page.locator("#otp").fill("%shop_totp%").await);
    let navigated = failure(page.locator("#book").click().await);
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Played { no_code, navigated }
}

/// The `complete-error-details` client conformance case, played by the
/// Scripted engine's Daemon: `no_code` and both documents of a navigation
/// the call did not start, each read through its code's typed details.
///
/// Covers: error:secret_unavailable, error:page_navigated_during_call
#[test]
fn each_failure_reads_every_detail_its_code_declares() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "complete-error-details",
        RustMode::Daemon,
    )
    .unwrap();
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let hello = daemon_hello();
    let (process, endpoint) =
        scripted_daemon_hearing(&env, "sdk-rust/run/complete-error-details.json", &hello);
    let played = runtime.block_on(played(endpoint, hello));
    let status = process.child().wait().unwrap();

    let no_code = &played.no_code;
    assert_eq!(
        code_group_retryable(no_code),
        (
            ErrorCode::SECRET_UNAVAILABLE,
            ErrorGroup::Unavailable,
            false
        )
    );
    assert_eq!(
        no_code.details::<SecretUnavailableDetails>(),
        Some(SecretUnavailableDetails {
            placeholder: "shop_totp".to_owned(),
            reference: "bw:shop/totp".to_owned(),
            provider: "bw".to_owned(),
            failure: SecretFailure::NoCode,
            fix: "Check the TOTP seed of the vault's shop item".to_owned(),
        }),
        "a provider that made no one-time code reads as `no_code`, naming the placeholder, the reference and the provider"
    );
    let navigated = &played.navigated;
    assert_eq!(
        code_group_retryable(navigated),
        (
            ErrorCode::PAGE_NAVIGATED_DURING_CALL,
            ErrorGroup::Page,
            true
        )
    );
    assert_eq!(
        navigated.details::<PageNavigatedDuringCallDetails>(),
        Some(PageNavigatedDuringCallDetails {
            url: "https://travel.test/books".to_owned(),
            navigated_to: "https://travel.test/books/1".to_owned(),
        }),
        "the details name the document the call read and the one that replaced it"
    );
    assert_eq!(
        navigated.fix(),
        Some("Await the Collie call before your driver acts on the page.")
    );
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
