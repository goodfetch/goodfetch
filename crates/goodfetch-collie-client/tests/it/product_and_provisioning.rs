//! What a Collie engine lacks and a browser build it cannot fetch (ADRs
//! 0087, 0090, 0095, 0098 and 0115), through the client crate: the
//! corpus's script of the `product-and-provisioning-errors` client
//! conformance case, which the TypeScript SDK plays too, with the Scripted
//! engine as the Daemon. It fails on a message it does not expect and on a
//! step left unused, so its clean exit says the crate sent exactly what the
//! script holds. This is Rust's `daemon` mode play.

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

/// Foxhound's install command on the engine's channel, npm.
const INSTALL: &str = "npm install @goodfetch/foxhound";

/// The command each provisioning error's fix names.
const BROWSER_INSTALL: &str = "`goodfetch-collie browser install`";

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
}

/// The error `answer` holds; a success fails the test.
fn failure<T: std::fmt::Debug>(answer: Result<T, Error>) -> Error {
    answer.expect_err("the call failed")
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
    let seeds = json!({"seeds": ["https://shop.test/"]});
    let operation = failure(connection.call::<Value>("crawl.start", &seeds).await);
    let capped = json!({"settings": {"crawl": {"max_requests": 10}}});
    let setting = failure(connection.call::<Value>("run.start", &capped).await);
    let run = connection.start().await.unwrap();
    let not_installed = failure(run.new_page().await);
    let offline = failure(run.new_page().await);
    let download_failed = failure(run.new_page().await);
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Refused {
        operation,
        setting,
        not_installed,
        offline,
        download_failed,
    }
}

/// `error`'s code, group and whether it is retryable.
fn kind(error: &Error) -> (ErrorCode, ErrorGroup, bool) {
    (error.code(), error.group(), error.retryable())
}

/// Whether `error`'s fix names the Product's `browser install`.
fn fix_names_browser_install(error: &Error) -> bool {
    error.fix().is_some_and(|fix| fix.contains(BROWSER_INSTALL))
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
        kind(operation),
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

    assert_eq!(kind(setting), kind(operation));
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
        kind(error),
        (
            ErrorCode::BROWSER_NOT_INSTALLED,
            ErrorGroup::Unavailable,
            false
        )
    );
    assert!(fix_names_browser_install(error), "{error:?}");
    let details = error.details::<BrowserNotInstalledDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details.missing).unwrap(),
        json!({"kind": "pinned", "artifact": build(), "pinned_by": pinned_by(), "cache": CACHE})
    );
}

/// A download `egress.offline` refuses fails with
/// `artifact_unavailable_offline`, a refusal naming the setting.
fn assert_offline(error: &Error) {
    assert_eq!(
        kind(error),
        (
            ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
            ErrorGroup::Refused,
            false
        )
    );
    assert!(fix_names_browser_install(error), "{error:?}");
    let details = error
        .details::<ArtifactUnavailableOfflineDetails>()
        .unwrap();
    let mut wire = serde_json::to_value(details).unwrap();
    wire.as_object_mut().unwrap().remove("fix");
    assert_eq!(
        wire,
        json!({
            "artifact": build(),
            "pinned_by": pinned_by(),
            "cache": CACHE,
            "limit": {"key": "egress.offline", "value": true, "layer": "user_configuration"},
        })
    );
}

/// A failed download fails with `artifact_download_failed`, retryable,
/// naming its host and failure.
fn assert_download_failed(error: &Error) {
    assert_eq!(
        kind(error),
        (
            ErrorCode::ARTIFACT_DOWNLOAD_FAILED,
            ErrorGroup::Unavailable,
            true
        )
    );
    assert!(fix_names_browser_install(error), "{error:?}");
    let details = error.details::<ArtifactDownloadFailedDetails>().unwrap();
    let mut wire = serde_json::to_value(details).unwrap();
    wire.as_object_mut().unwrap().remove("fix");
    assert_eq!(
        wire,
        json!({
            "artifact": build(),
            "pinned_by": pinned_by(),
            "host": "storage.googleapis.com",
            "failure": "http_status",
            "status": 404,
        })
    );
}

/// The `product-and-provisioning-errors` client conformance case, played
/// by the Scripted engine's Daemon as a Collie engine: Foxhound's
/// operation and setting fail with `not_in_product`, naming both Products
/// and Foxhound's install command; a page whose build is missing fails with
/// the provisioning code, naming the build, the Product version that pins
/// it and its `browser install`.
/// Covers: story:11-10, story:11-13, story:11-189, story:11-294
#[test]
fn foxhounds_operation_and_setting_and_each_missing_build_fail_with_their_code_and_fix() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon(&env, "run/product-and-provisioning-errors.json");
    let refused = goodfetch_test_support::real_time_runtime().block_on(refused_calls(endpoint));

    assert_not_in_product(&refused.operation, &refused.setting);
    assert_not_installed(&refused.not_installed);
    assert_offline(&refused.offline);
    assert_download_failed(&refused.download_failed);
    assert!(
        process.child().wait().unwrap().success(),
        "the Scripted engine played its script whole"
    );
}
