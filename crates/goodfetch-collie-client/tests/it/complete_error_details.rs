//! Complete error details (ADRs 0035, 0079, 0092, 0101, 0106, 0114, 0115 and
//! 0118) through the client crate: the corpus's script of the
//! `complete-error-details` client conformance case, which the TypeScript and
//! Python SDKs play too, with the Scripted engine as the Daemon. A fill whose
//! one-time code never came fails with `secret_unavailable`, whose typed
//! details say `no_code`, and a failure with `page_navigated_during_call`
//! reads both documents through `PageNavigatedDuringCallDetails`. A navigation
//! a site rate-limits fails with `blocked`, whose `BlockedDetails` read the
//! Block cause `rate_limit` and `retry_after` as a `SystemTime`; a cause no
//! catalog names reads as `BlockCause::Unknown`, kept as received. A Model
//! provider rate-limiting fails with `model_unavailable` with `retry_after`
//! as a `Duration`; `effect_uncertain` reads `UncertainEffect` with `request`
//! as `None` on a Collie Run; `artifact_unavailable_offline` reads `pinned_by`
//! as a list of product and plugin pinners; `too_many_redirects` reads `hops`
//! and `limit`. The Scripted engine fails on a message it does not expect and
//! on a step left unused, so its clean exit says the crate sent exactly what
//! the script holds. This is Rust's `daemon` mode play.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::browser_cache::{
    ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails, Pinner,
};
use goodfetch_protocol::catalog::{ErrorGroup, Product};
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams, ProductVersion};
use goodfetch_protocol::model::{ModelUnavailableDetails, ModelUnavailableReason};
use goodfetch_protocol::page::ai_work::PageNavigatedDuringCallDetails;
use goodfetch_protocol::page::{BlockedDetails, RedirectLimit, TooManyRedirectsDetails};
use goodfetch_protocol::run::UncertainEffect;
use goodfetch_protocol::secrets::{SecretFailure, SecretUnavailableDetails};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use goodfetch_test_support::{TestEnv, case_logs};

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};
use crate::start::{code_group_retryable, failure};

/// What the script's calls answered.
struct Played {
    /// The fill of the one-time code no provider made.
    no_code: Error,
    /// The click whose document a navigation it did not start replaced.
    navigated: Error,
    /// The navigation the site rate-limited.
    rate_limited: Error,
    /// The navigation the site refused with a cause no catalog names.
    unknown_cause: Error,
    /// The call whose Model provider rate-limited it with a wait duration.
    model_unavailable: Error,
    /// The call whose Effect was left uncertain on a Collie Run.
    effect_uncertain: Error,
    /// The call refused by `offline` for a pinned browser artifact.
    artifact_offline: Error,
    /// The navigation that exceeded the browser's redirect limit.
    too_many_redirects: Error,
}

/// 2026-11-21T08:00:00Z, the time the rate-limited site asks for, which
/// its `Retry-After` sent as 10:00 at UTC+2: 498,680 hours after the Unix
/// epoch.
const SITE_ASKS_FOR: Duration = Duration::from_hours(498_680);

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
    let rate_limited = failure(page.goto("https://shop.test/deals").await);
    let unknown_cause = failure(page.goto("https://shop.test/abroad").await);
    let model_unavailable = failure(page.locator("#summarize").click().await);
    let effect_uncertain = failure(page.locator("#pay").click().await);
    let artifact_offline = failure(page.goto("https://shop.test/offline").await);
    let too_many_redirects = failure(page.goto("https://shop.test/bounce").await);
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Played {
        no_code,
        navigated,
        rate_limited,
        unknown_cause,
        model_unavailable,
        effect_uncertain,
        artifact_offline,
        too_many_redirects,
    }
}

/// Checks `secret_unavailable`, `page_navigated_during_call`, and both
/// `blocked` failures.
fn check_secret_navigated_and_blocked(played: &Played) {
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
    let rate_limited = &played.rate_limited;
    assert_eq!(
        code_group_retryable(rate_limited),
        (ErrorCode::BLOCKED, ErrorGroup::Page, false)
    );
    assert_eq!(
        rate_limited.details::<BlockedDetails>(),
        Some(BlockedDetails {
            cause: BlockCause::RateLimit,
            url: "https://shop.test/deals".to_owned(),
            status: Some(429),
            retry_after: Some(SystemTime::UNIX_EPOCH + SITE_ASKS_FOR),
        }),
        "the cause reads by its wire value, and the site's time as a point in time"
    );
    let unknown = played
        .unknown_cause
        .details::<BlockedDetails>()
        .expect("a blocked answer reads as BlockedDetails");
    let BlockCause::Unknown(cause) = &unknown.cause else {
        panic!("{:?} is a cause Collie knows", unknown.cause);
    };
    assert_eq!(
        (
            cause.as_str(),
            unknown.url.as_str(),
            unknown.status,
            unknown.retry_after
        ),
        ("geo_block", "https://shop.test/abroad", Some(451), None),
        "a cause no catalog names is kept as received"
    );
}

/// Checks `model_unavailable`, `effect_uncertain`,
/// `artifact_unavailable_offline`, and `too_many_redirects`.
fn check_model_effect_artifact_and_redirects(played: &Played) {
    let model_unavailable = &played.model_unavailable;
    assert_eq!(
        code_group_retryable(model_unavailable),
        (ErrorCode::MODEL_UNAVAILABLE, ErrorGroup::Unavailable, true)
    );
    assert_eq!(
        model_unavailable.details::<ModelUnavailableDetails>(),
        Some(ModelUnavailableDetails {
            reason: ModelUnavailableReason::RateLimited,
            retry_after: Some(Duration::from_millis(1500)),
        }),
        "model_unavailable reads retry_after as a Duration"
    );
    let effect_uncertain = &played.effect_uncertain;
    assert_eq!(
        code_group_retryable(effect_uncertain),
        (ErrorCode::EFFECT_UNCERTAIN, ErrorGroup::Page, false)
    );
    assert_eq!(
        effect_uncertain.details::<UncertainEffect>(),
        Some(UncertainEffect {
            entry: "effect-1".to_owned(),
            key: Some("charge-order-42".to_owned()),
            instruction: None,
            request: None,
            reason: "browser closed before confirmation".to_owned(),
            first_command: Some("locator.click".to_owned()),
            since: SystemTime::UNIX_EPOCH + SITE_ASKS_FOR,
        }),
        "effect_uncertain reads UncertainEffect with request None on a Collie Run"
    );
    let artifact_offline = &played.artifact_offline;
    assert_eq!(
        code_group_retryable(artifact_offline),
        (
            ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
            ErrorGroup::Refused,
            false
        )
    );
    assert_eq!(
        artifact_offline.details::<ArtifactUnavailableOfflineDetails>(),
        Some(ArtifactUnavailableOfflineDetails {
            artifact: ArtifactId {
                kind: ArtifactKind::Browser,
                name: "chrome-for-testing".to_owned(),
                build: "141.0.7390.54-linux64".to_owned(),
            },
            pinned_by: vec![
                Pinner::Product {
                    name: Product::Collie,
                    version: ProductVersion::parse("0.0.0").unwrap(),
                },
                Pinner::Plugin {
                    name: "demo-plugin".to_owned(),
                    version: "1.2.0".to_owned(),
                },
            ],
            cache: "/var/cache/goodfetch/browsers".to_owned(),
            limit: SettingLimit {
                key: "offline".to_owned(),
                value: serde_json::Value::Bool(true),
                layer: SettingsLayer::Run,
                origin: None,
            },
            fix: "Run `collie browser install` where the cache is writable.".to_owned(),
        }),
        "artifact_unavailable_offline reads pinned_by as a typed list of pinners"
    );
    let too_many_redirects = &played.too_many_redirects;
    assert_eq!(
        code_group_retryable(too_many_redirects),
        (ErrorCode::TOO_MANY_REDIRECTS, ErrorGroup::Page, false)
    );
    assert_eq!(
        too_many_redirects.details::<TooManyRedirectsDetails>(),
        Some(TooManyRedirectsDetails {
            url: "https://shop.test/bounce".to_owned(),
            hops: vec![
                "https://shop.test/bounce/1".to_owned(),
                "https://shop.test/bounce/2".to_owned(),
            ],
            limit: RedirectLimit::Browser { count: 20 },
        }),
        "too_many_redirects reads hops and the browser limit"
    );
}

/// The `complete-error-details` client conformance case, played by the
/// Scripted engine's Daemon: `no_code`, both documents of a navigation the
/// call did not start, two `blocked` navigations, `model_unavailable` with a
/// wait duration, `effect_uncertain` with `request: None`,
/// `artifact_unavailable_offline` with `pinned_by` as a list, and
/// `too_many_redirects`, each read through its code's typed details.
///
/// Covers: story:11-38, story:11-39, story:11-191, error:secret_unavailable, error:page_navigated_during_call, error:blocked, error:model_unavailable, error:effect_uncertain, error:artifact_unavailable_offline, error:too_many_redirects, block-cause:rate_limit
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

    check_secret_navigated_and_blocked(&played);
    check_model_effect_artifact_and_redirects(&played);
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
