//! What a Collie engine lacks and a browser build it cannot fetch (ADRs
//! 0087, 0090, 0095, 0098 and 0115), through the client crate: the
//! corpus's script of the `product-and-provisioning-errors` client
//! conformance case for a Daemon installed from crates.io, with the
//! Scripted engine as the Daemon. It fails on a message it does not expect
//! and on a step left unused, so its clean exit says the crate sent exactly
//! what the script holds. This is Rust's `daemon` mode play; the SDK crate's
//! `sdk::product_and_provisioning` is its `linked` one.

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_protocol::browser_cache::{
    ArtifactDownloadFailedDetails, ArtifactUnavailableOfflineDetails, BrowserNotInstalledDetails,
};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{DaemonEndpoint, NotInProductDetails};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::scripted_daemon::{daemon_hello, scripted_daemon};
use crate::start::{code_group_retryable, failure};

/// Foxhound's install command on the Daemon's channel, crates.io.
const INSTALL: &str = "cargo install goodfetch-foxhound --locked --features executable";

/// The Browser cache the engine reads.
const CACHE: &str = "/srv/goodfetch/browsers";

/// What each of the script's refused calls failed with.
struct Refused {
    /// Foxhound's operation `crawl.start`.
    operation: Error,
    /// A Run's start setting Foxhound's `crawl.max_requests`.
    setting: Error,
    /// The first page, whose build the engine cannot write to its cache.
    not_installed: Error,
    /// The second, whose download `egress.offline` refuses.
    offline: Error,
    /// The third, whose download fails with a 404.
    download_failed: Error,
    /// The fourth, whose download's connection breaks.
    connection_broke: Error,
    /// The fifth, whose downloaded bytes have another digest.
    wrong_digest: Error,
}

/// Plays the script's calls on a Connection to the Daemon at `endpoint`.
async fn refused_calls(endpoint: DaemonEndpoint) -> Refused {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        daemon_hello(),
    )
    .await
    .unwrap();
    let seeds = json!({"seeds": [{"url": "https://shop.test/", "label": "page"}]});
    let operation = failure(connection.raw_call("crawl.start").params(seeds).await);
    let capped = json!({"settings": {"crawl": {"max_requests": 10}}});
    let setting = failure(connection.raw_call("run.start").params(capped).await);
    let run = connection.start().await.unwrap();
    let not_installed = failure(run.new_page().await);
    let offline = failure(run.new_page().await);
    let download_failed = failure(run.new_page().await);
    let connection_broke = failure(run.new_page().await);
    let wrong_digest = failure(run.new_page().await);
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Refused {
        operation,
        setting,
        not_installed,
        offline,
        download_failed,
        connection_broke,
        wrong_digest,
    }
}

/// The build every refused page needs.
fn build() -> Value {
    json!({"kind": "browser", "name": "chrome-for-testing", "build": "141.0.7390.54-linux64"})
}

/// The Product version that pins it.
fn pinned_by() -> Value {
    json!({"product": "collie", "product_version": "0.0.0"})
}

/// Foxhound's operation and setting fail with `not_in_product`, naming
/// both Products and Foxhound's install command.
fn assert_not_in_product(operation: &Error, setting: &Error) {
    assert_eq!(
        code_group_retryable(operation),
        (ErrorCode::NOT_IN_PRODUCT, ErrorGroup::Unavailable, false)
    );
    let fix = format!(
        "Run it with Foxhound, which contains everything Collie does (`{INSTALL}`), or connect to a Foxhound engine."
    );
    assert_eq!(operation.fix(), Some(fix.as_str()));
    let details = operation.details::<NotInProductDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "product": "collie",
            "product_version": "0.0.0",
            "needs": "foxhound",
            "requested": "crawl.start",
            "install": INSTALL,
            "fix": fix,
        })
    );

    assert_eq!(
        code_group_retryable(setting),
        code_group_retryable(operation)
    );
    let details = setting.details::<NotInProductDetails>().unwrap();
    assert_eq!(
        (details.requested.as_deref(), details.install.as_deref()),
        (Some("crawl.max_requests"), Some(INSTALL))
    );
}

/// A build absent from a cache the engine cannot write fails with
/// `browser_not_installed`, not retryable, naming the build and what pins
/// it.
fn assert_not_installed(error: &Error) {
    assert_eq!(
        code_group_retryable(error),
        (
            ErrorCode::BROWSER_NOT_INSTALLED,
            ErrorGroup::Unavailable,
            false
        )
    );
    let fix = "Run `goodfetch-collie browser install` where the Browser cache is writable, such as in the image's build, or point GOODFETCH_BROWSERS_PATH at a cache that holds it";
    assert_eq!(error.fix(), Some(fix));
    let details = error.details::<BrowserNotInstalledDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "missing": {"kind": "pinned", "artifact": build(), "pinned_by": pinned_by(), "cache": CACHE},
            "fix": fix,
        })
    );
}

/// A download `egress.offline` refuses fails with
/// `artifact_unavailable_offline`, a refusal naming the setting.
fn assert_offline(error: &Error) {
    assert_eq!(
        code_group_retryable(error),
        (
            ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
            ErrorGroup::Refused,
            false
        )
    );
    let fix = "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set";
    assert_eq!(error.fix(), Some(fix));
    let details = error
        .details::<ArtifactUnavailableOfflineDetails>()
        .unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "artifact": build(),
            "pinned_by": pinned_by(),
            "cache": CACHE,
            "limit": {"key": "egress.offline", "value": true, "layer": "user_configuration"},
            "fix": fix,
        })
    );
}

/// What a failed download's error says to do, whatever failed.
const DOWNLOAD_FIX: &str = "Retry, or run `goodfetch-collie browser install` beforehand, or point GOODFETCH_BROWSERS_PATH at a prepared Browser cache";

/// A failed download fails with `artifact_download_failed`, retryable,
/// naming its host and `failure`, whose own fields `failure` holds.
fn assert_download_failed(error: &Error, failure: &Value) {
    assert_eq!(
        code_group_retryable(error),
        (
            ErrorCode::ARTIFACT_DOWNLOAD_FAILED,
            ErrorGroup::Unavailable,
            true
        )
    );
    assert_eq!(error.fix(), Some(DOWNLOAD_FIX));
    let details = error.details::<ArtifactDownloadFailedDetails>().unwrap();
    let mut expected = json!({
        "artifact": build(),
        "pinned_by": pinned_by(),
        "host": "storage.googleapis.com",
        "fix": DOWNLOAD_FIX,
    });
    expected
        .as_object_mut()
        .unwrap()
        .extend(failure.as_object().unwrap().clone());
    assert_eq!(serde_json::to_value(details).unwrap(), expected);
}

/// The `product-and-provisioning-errors` client conformance case, played
/// by the Scripted engine's Daemon as a Collie engine: Foxhound's
/// operation and setting fail with `not_in_product`, naming both Products
/// and Foxhound's install command; a page whose build is missing fails with
/// the provisioning code, naming the build, the Product version that pins
/// it and its `browser install`, a failed download its `failure`:
/// `http_status`, `network` or `digest_mismatch`.
/// Covers: story:11-10, story:11-13, story:11-189, story:11-294
#[test]
fn foxhounds_operation_and_setting_and_each_missing_build_fail_with_their_code_and_fix() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(
        &env,
        "sdk-rust/run/product-and-provisioning-errors-from-cargo.json",
    );
    let refused = goodfetch_test_support::real_time_runtime().block_on(refused_calls(endpoint));

    assert_not_in_product(&refused.operation, &refused.setting);
    assert_not_installed(&refused.not_installed);
    assert_offline(&refused.offline);
    assert_download_failed(
        &refused.download_failed,
        &json!({"failure": "http_status", "status": 404}),
    );
    assert_download_failed(&refused.connection_broke, &json!({"failure": "network"}));
    assert_download_failed(
        &refused.wrong_digest,
        &json!({"failure": "digest_mismatch"}),
    );
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}
