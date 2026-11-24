//! What a Collie engine lacks and a browser build it cannot fetch (ADRs
//! 0087, 0090, 0095, 0098 and 0115), the `product-and-provisioning-errors`
//! client conformance case in `linked` mode: Rust's linked engine is the
//! real one, installed from crates.io, so it plays the case's Collie part,
//! Foxhound's operation and setting refused with `not_in_product` naming
//! Foxhound's install command on crates.io, and its provisioning part on an
//! empty Browser cache (#3100): the first page downloads the pinned build
//! from the Proving ground's `browser-builds.test`, never the internet, and
//! `egress.offline` or a host that answers `404` fails it with
//! `artifact_unavailable_offline` or `artifact_download_failed`, while
//! `connect()` succeeds. A read-only cache's `browser_not_installed` is the
//! `browser-cache-not-writable` case's, on Unix, whose folder modes make
//! one. The client
//! crate's `product_and_provisioning` plays the case on the Scripted engine
//! as a Daemon. TypeScript's `product-and-provisioning-errors.test.ts` and
//! Python's `test_product_and_provisioning_errors.py` are its twins.

use std::path::Path;

use goodfetch_collie::settings::{Settings, egress};
use goodfetch_collie::{Collie, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
#[cfg(unix)]
use goodfetch_protocol::browser_cache::BrowserNotInstalledDetails;
use goodfetch_protocol::browser_cache::{
    ArtifactDownloadFailedDetails, ArtifactUnavailableOfflineDetails,
};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::NotInProductDetails;
#[cfg(unix)]
use goodfetch_test_support::ReadOnlyBrowserCache;
use goodfetch_test_support::{
    BROWSER_BUILDS_HOST, CHROME_FOR_TESTING_BUILD, ProvingGround, TestEnv, browser_builds,
    case_logs, empty_browser_cache, real_time_runtime,
};
use serde_json::{Value, json};

use super::options;

/// The Proving ground's own seed for the provisioning tests.
const SEED: u64 = 3100;

/// The linked engine's Product version, which pins the build.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A test's empty Browser cache, its root, and the Proving ground its
/// engines reach, whose `browser-builds.test` they download a missing
/// build from: serving the pinned build when `serving`, else answering
/// `404`.
fn first_use(serving: bool) -> (TestEnv, std::path::PathBuf, ProvingGround) {
    let (env, cache) = empty_browser_cache(TestEnv::new()).unwrap();
    let proving_ground = if serving {
        ProvingGround::start_serving_builds(&env, SEED, &browser_builds(&env).unwrap())
    } else {
        ProvingGround::start(&env, SEED)
    }
    .unwrap();
    let reached = proving_ground.description();
    env.user_configuration(&format!("{}{}", reached.reaching(), reached.mirroring()));
    (env, cache, proving_ground)
}

/// What the linked engine's first page fails with, on a Run started with
/// `settings`, once `connect()` and the Run's start succeeded.
fn first_page_failure(env: &TestEnv, settings: Settings) -> Error {
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(env)).await.unwrap();
        let run = collie.start().settings(settings).await.unwrap();
        let failed = run.new_page().await.err();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        failed.expect("the first page fails")
    })
}

/// The pinned build, as each provisioning error names it.
fn build() -> Value {
    json!({"kind": "browser", "name": "chrome-for-testing", "build": CHROME_FOR_TESTING_BUILD})
}

/// What pins it: the Product version alone, in a list that also names the
/// Plugins using a Plugin runtime (ADR 0115).
fn pinned_by() -> Value {
    json!([{"kind": "product", "name": "collie", "version": VERSION}])
}

/// The build and what pins it, as each provisioning error's message names
/// them.
fn described() -> String {
    format!(
        "the browser build chrome-for-testing {CHROME_FOR_TESTING_BUILD}, pinned by Collie {VERSION}"
    )
}

/// `error`'s code, group, whether it is retryable, its message and `fix`.
fn summary(error: &Error) -> (ErrorCode, ErrorGroup, bool, &str, Option<&str>) {
    (
        error.code(),
        error.group(),
        error.retryable(),
        error.message(),
        error.fix(),
    )
}

/// The `cache` field of an error's details: the cache's root as the engine
/// names it.
fn cache_name(cache: &Path) -> String {
    cache.display().to_string()
}

/// Foxhound's install command on the linked engine's channel, crates.io.
const INSTALL: &str = "cargo install goodfetch-foxhound --locked --features executable";

/// The error `answer` holds, which must be Foxhound's `not_in_product`
/// for `requested`, naming its install command on crates.io whole.
fn assert_not_in_product(answer: Result<Value, Error>, requested: &str) {
    let error = match answer {
        Ok(value) => panic!("`{requested}` answered {value}"),
        Err(error) => error,
    };
    assert_eq!(
        (error.code(), error.group(), error.retryable()),
        (ErrorCode::NOT_IN_PRODUCT, ErrorGroup::Unavailable, false),
        "{error:?}"
    );
    let fix = format!(
        "Run it with Foxhound, which contains everything Collie does (`{INSTALL}`), or connect to a Foxhound engine."
    );
    assert_eq!(error.fix(), Some(fix.as_str()));
    assert_eq!(
        serde_json::to_value(error.details::<NotInProductDetails>().unwrap()).unwrap(),
        json!({
            "product": "collie",
            "product_version": env!("CARGO_PKG_VERSION"),
            "needs": "foxhound",
            "requested": requested,
            "install": INSTALL,
            "fix": fix,
        })
    );
}

/// Foxhound's operation and a Run's start setting Foxhound's setting, on
/// the linked engine, each fail with `not_in_product` naming Foxhound and
/// its install command on crates.io.
/// Covers: story:11-10, story:11-189
#[test]
fn foxhounds_operation_and_setting_fail_on_the_linked_engine_naming_foxhounds_cargo_install() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "product-and-provisioning-errors",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();

    let (operation, setting) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let operation = collie
            .call("crawl.start")
            .params(json!({"seeds": [{"url": "https://shop.test/", "label": "page"}]}))
            .await;
        let setting = collie
            .call("run.start")
            .params(json!({"settings": {"crawl": {"max_requests": 10}}}))
            .await;
        collie.close().await.unwrap();
        (operation, setting)
    });

    assert_not_in_product(operation, "crawl.start");
    assert_not_in_product(setting, "crawl.max_requests");
}

/// The first page on an empty Browser cache the engine may write
/// downloads the pinned build from the Proving ground's
/// `browser-builds.test`, which `browser.mirror` names, into the cache
/// `GOODFETCH_BROWSERS_PATH` names, and opens.
/// Covers: story:11-13, setting:browser.mirror
#[test]
fn first_use_on_an_empty_cache_downloads_the_pinned_build_from_the_mirror_and_opens_the_page() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "product-and-provisioning-errors",
        RustMode::Linked,
    )
    .unwrap();
    let (env, cache, proving_ground) = first_use(true);
    let start = proving_ground.start_url("statuses/1").unwrap();

    let status = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();
        let navigated = page.goto(&start).await.unwrap();
        page.close().await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        navigated.map(|response| response.status())
    });

    let entry = cache
        .join("browser")
        .join("chrome-for-testing")
        .join(CHROME_FOR_TESTING_BUILD)
        .join("entry.json");
    assert_eq!((status, entry.is_file()), (Some(200), true));
}

/// A first page on an empty cache the engine cannot write fails with
/// `browser_not_installed`, not retryable, naming the build, the Product
/// version that pins it, the cache and its `browser install`.
/// Covers: story:11-294, error:browser_not_installed
#[cfg(unix)]
#[test]
fn first_use_on_a_read_only_cache_fails_with_browser_not_installed() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "browser-cache-not-writable",
        RustMode::Linked,
    )
    .unwrap();
    let (env, cache, _proving_ground) = first_use(false);
    let read_only = ReadOnlyBrowserCache::new(&cache).unwrap();

    let error = first_page_failure(&env, Settings::new());
    drop(read_only);

    let fix = "Run `goodfetch-collie browser install` where the Browser cache is writable, such as in the image's build, or point GOODFETCH_BROWSERS_PATH at a cache that holds it";
    let message = format!(
        "{} is not in the Browser cache at {}, which this engine cannot write",
        described(),
        cache_name(&cache)
    );
    assert_eq!(
        summary(&error),
        (
            ErrorCode::BROWSER_NOT_INSTALLED,
            ErrorGroup::Unavailable,
            false,
            message.as_str(),
            Some(fix)
        )
    );
    assert_eq!(
        serde_json::to_value(error.details::<BrowserNotInstalledDetails>().unwrap()).unwrap(),
        json!({
            "missing": {"kind": "pinned", "artifact": build(), "pinned_by": pinned_by(), "cache": cache_name(&cache)},
            "fix": fix,
        })
    );
}

/// A first page under `egress.offline` fails with
/// `artifact_unavailable_offline`, a refusal whose limit names the
/// setting, before any download.
/// Covers: story:11-294, error:artifact_unavailable_offline, setting:egress.offline
#[test]
fn first_use_under_egress_offline_fails_with_artifact_unavailable_offline() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "product-and-provisioning-errors",
        RustMode::Linked,
    )
    .unwrap();
    let (env, cache, _proving_ground) = first_use(false);

    let error = first_page_failure(&env, Settings::new().set(egress::OFFLINE, true));

    let fix = "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set";
    let message = format!(
        "{} is not in the Browser cache at {}, and `egress.offline` refuses its download",
        described(),
        cache_name(&cache)
    );
    assert_eq!(
        summary(&error),
        (
            ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
            ErrorGroup::Refused,
            false,
            message.as_str(),
            Some(fix)
        )
    );
    assert_eq!(
        serde_json::to_value(
            error
                .details::<ArtifactUnavailableOfflineDetails>()
                .unwrap()
        )
        .unwrap(),
        json!({
            "artifact": build(),
            "pinned_by": pinned_by(),
            "cache": cache_name(&cache),
            "limit": {"key": "egress.offline", "value": true, "layer": "run"},
            "fix": fix,
        })
    );
}

/// A first page whose download host answers `404` fails with
/// `artifact_download_failed`, retryable, naming the host and the status.
/// Covers: story:11-294, error:artifact_download_failed, setting:browser.mirror
#[test]
fn first_use_from_a_host_that_answers_404_fails_with_artifact_download_failed() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "product-and-provisioning-errors",
        RustMode::Linked,
    )
    .unwrap();
    let (env, _cache, _proving_ground) = first_use(false);

    let error = first_page_failure(&env, Settings::new());

    let fix = "Retry, or run `goodfetch-collie browser install` beforehand, or point GOODFETCH_BROWSERS_PATH at a prepared Browser cache";
    let message = format!(
        "downloading {} from {BROWSER_BUILDS_HOST} failed: {BROWSER_BUILDS_HOST} answered 404",
        described()
    );
    assert_eq!(
        summary(&error),
        (
            ErrorCode::ARTIFACT_DOWNLOAD_FAILED,
            ErrorGroup::Unavailable,
            true,
            message.as_str(),
            Some(fix)
        )
    );
    assert_eq!(
        serde_json::to_value(error.details::<ArtifactDownloadFailedDetails>().unwrap()).unwrap(),
        json!({
            "artifact": build(),
            "pinned_by": pinned_by(),
            "host": BROWSER_BUILDS_HOST,
            "failure": "http_status",
            "status": 404,
            "fix": fix,
        })
    );
}
