//! Complete error details (ADRs 0035, 0079, 0092, 0101, 0106, 0114, 0115 and
//! 0118), the `complete-error-details` client conformance case in `linked`
//! mode: Rust's linked engine is the real one, in the program's process,
//! whose composition here is the engine's own contribution and a test
//! contribution answering the page calls with each of the case's eight
//! failures, matching `goodfetch-collie-client`'s `complete_error_details`
//! Daemon play, TypeScript's `complete-error-details.test.ts` and Python's
//! `test_complete_error_details.py`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::future::ready;
use std::time::{Duration, SystemTime};

use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_collie::{EngineChoice, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_engine::{CallContext, CallError, Composition, Contribution, Handler};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::browser_cache::{
    ARTIFACT_UNAVAILABLE_OFFLINE, ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails,
    Pinner,
};
use goodfetch_protocol::catalog::{ErrorGroup, Product};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::effect::EFFECT_UNCERTAIN;
use goodfetch_protocol::model::{
    MODEL_UNAVAILABLE, ModelUnavailableDetails, ModelUnavailableReason,
};
use goodfetch_protocol::page::ai_work::{
    PAGE_NAVIGATED_DURING_CALL, PageNavigatedDuringCallDetails,
};
use goodfetch_protocol::page::{
    BLOCKED, BlockedDetails, LOCATOR_CLICK, LOCATOR_FILL, PAGE_GOTO, RUN_NEW_PAGE, RedirectLimit,
    TOO_MANY_REDIRECTS, TooManyRedirectsDetails,
};
use goodfetch_protocol::run::UncertainEffect;
use goodfetch_protocol::secrets::{SECRET_UNAVAILABLE, SecretFailure, SecretUnavailableDetails};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde_json::{Value, json};

use super::options;

/// 2026-11-21T08:00:00Z: 498,680 hours after the Unix epoch.
const SITE_ASKS_FOR: Duration = Duration::from_hours(498_680);

/// What the click on `selector` fails with.
fn click_failing(selector: &str) -> CallError {
    match selector {
        "#book" => CallError::new(
            &PAGE_NAVIGATED_DURING_CALL,
            "a navigation the call did not start replaced https://travel.test/books with https://travel.test/books/1",
            &json!({
                "operation": "locator.click",
                "call_id": "call-2",
                "target": "#book",
                "url": "https://travel.test/books",
                "navigated_to": "https://travel.test/books/1",
                "fix": "Await the Collie call before your driver acts on the page.",
            }),
        ),
        "#summarize" => CallError::new(
            &MODEL_UNAVAILABLE,
            "the Model provider rate-limited calls and asked to wait 1500 ms",
            &json!({
                "operation": "locator.click",
                "call_id": "call-5",
                "target": "#summarize",
                "url": "https://shop.test/item",
                "reason": "rate_limited",
                "retry_after": 1500,
            }),
        ),
        "#pay" => CallError::new(
            &EFFECT_UNCERTAIN,
            "the Effect charge-order-42 was left uncertain when the browser closed",
            &json!({
                "operation": "locator.click",
                "call_id": "call-6",
                "target": "#pay",
                "url": "https://shop.test/checkout",
                "entry": "effect-1",
                "key": "charge-order-42",
                "request": null,
                "reason": "browser closed before confirmation",
                "first_command": "locator.click",
                "since": "2026-11-21T08:00:00Z",
            }),
        ),
        _ => panic!("no click of the case targets {selector}"),
    }
}

/// What the navigation to `url` fails with.
fn goto_failing(url: &str) -> CallError {
    match url {
        "https://shop.test/deals" => CallError::new(
            &BLOCKED,
            "the site refused https://shop.test/deals with the Block cause rate_limit (HTTP 429)",
            &json!({
                "operation": "page.goto",
                "call_id": "call-3",
                "url": "https://shop.test/deals",
                "cause": "rate_limit",
                "status": 429,
                "retry_after": "2026-11-21T10:00:00+02:00",
            }),
        ),
        "https://shop.test/abroad" => CallError::new(
            &BLOCKED,
            "the site refused https://shop.test/abroad with the Block cause geo_block (HTTP 451)",
            &json!({
                "operation": "page.goto",
                "call_id": "call-4",
                "url": "https://shop.test/abroad",
                "cause": "geo_block",
                "status": 451,
                "retry_after": null,
            }),
        ),
        "https://shop.test/offline" => CallError::new(
            &ARTIFACT_UNAVAILABLE_OFFLINE,
            "the offline setting refused downloading chrome-for-testing",
            &json!({
                "operation": "page.goto",
                "call_id": "call-7",
                "artifact": {
                    "kind": "browser",
                    "name": "chrome-for-testing",
                    "build": "141.0.7390.54-linux64",
                },
                "pinned_by": [
                    { "kind": "product", "name": "collie", "version": "0.0.0" },
                    { "kind": "plugin", "name": "demo-plugin", "version": "1.2.0" },
                ],
                "cache": "/var/cache/goodfetch/browsers",
                "limit": { "key": "offline", "value": true, "layer": "run" },
                "fix": "Run `collie browser install` where the cache is writable.",
            }),
        ),
        "https://shop.test/bounce" => CallError::new(
            &TOO_MANY_REDIRECTS,
            "navigation to https://shop.test/bounce exceeded the browser's 20-request redirect limit",
            &json!({
                "operation": "page.goto",
                "call_id": "call-8",
                "url": "https://shop.test/bounce",
                "hops": ["https://shop.test/bounce/1", "https://shop.test/bounce/2"],
                "limit": { "kind": "browser", "count": 20 },
            }),
        ),
        _ => panic!("no navigation of the case targets {url}"),
    }
}

/// The engine's own contribution and the case's page calls answered as the
/// script answers them.
fn answering_as_the_script() -> Composition {
    Composition::default()
        .with(goodfetch_engine::contribution())
        .with(
            Contribution::new("test_complete_error_details")
                .with_handler(Handler::new(&RUN_NEW_PAGE, |_: Value, _: CallContext| {
                    ready(Ok::<_, CallError>(
                        json!({"page_id": "page-1", "session_id": "session-1"}),
                    ))
                }))
                .with_handler(Handler::new(&LOCATOR_FILL, |_: Value, _: CallContext| {
                    ready(Err::<Value, _>(CallError::new(
                        &SECRET_UNAVAILABLE,
                        "the Secret reference bw:shop/totp gave no one-time code: Bitwarden made none from the item's seed",
                        &json!({
                            "operation": "locator.fill",
                            "call_id": "call-1",
                            "target": "#otp",
                            "url": "https://shop.test/login",
                            "placeholder": "shop_totp",
                            "reference": "bw:shop/totp",
                            "provider": "bw",
                            "failure": "no_code",
                            "fix": "Check the TOTP seed of the vault's shop item",
                        }),
                    )))
                }))
                .with_handler(Handler::new(
                    &LOCATOR_CLICK,
                    |params: Value, _: CallContext| {
                        let selector = params["locator"]["selector"].as_str().unwrap_or_default();
                        ready(Err::<Value, _>(click_failing(selector)))
                    },
                ))
                .with_handler(Handler::new(&PAGE_GOTO, |params: Value, _: CallContext| {
                    let url = params["url"].as_str().unwrap_or_default();
                    ready(Err::<Value, _>(goto_failing(url)))
                })),
        )
}

/// What the case's calls answered.
struct Played {
    no_code: Error,
    navigated: Error,
    rate_limited: Error,
    unknown_cause: Error,
    model_unavailable: Error,
    effect_uncertain: Error,
    artifact_offline: Error,
    too_many_redirects: Error,
}

fn failure<T: std::fmt::Debug>(answer: Result<T, Error>) -> Error {
    let Err(error) = answer else {
        panic!("{answer:?}");
    };
    error
}

fn code_group_retryable(error: &Error) -> (ErrorCode, ErrorGroup, bool) {
    (error.code(), error.group(), error.retryable())
}

/// Plays the case's calls on the linked engine of `env`'s home.
async fn played(env: &TestEnv) -> Played {
    let engine = Composed {
        product: Product::Collie,
        composition: answering_as_the_script,
    };
    let collie = ProductConnection::connect(engine, options(env).engine(EngineChoice::Linked))
        .await
        .unwrap();
    let run = collie.connection().start().await.unwrap();
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
    collie.close().await.unwrap();
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

fn check_secret_navigated_and_blocked(played: &Played) {
    assert_eq!(
        code_group_retryable(&played.no_code),
        (
            ErrorCode::SECRET_UNAVAILABLE,
            ErrorGroup::Unavailable,
            false
        )
    );
    assert_eq!(
        played.no_code.details::<SecretUnavailableDetails>(),
        Some(SecretUnavailableDetails {
            placeholder: "shop_totp".to_owned(),
            reference: "bw:shop/totp".to_owned(),
            provider: "bw".to_owned(),
            failure: SecretFailure::NoCode,
            fix: "Check the TOTP seed of the vault's shop item".to_owned(),
        })
    );
    assert_eq!(
        code_group_retryable(&played.navigated),
        (
            ErrorCode::PAGE_NAVIGATED_DURING_CALL,
            ErrorGroup::Page,
            true
        )
    );
    assert_eq!(
        played.navigated.details::<PageNavigatedDuringCallDetails>(),
        Some(PageNavigatedDuringCallDetails {
            url: "https://travel.test/books".to_owned(),
            navigated_to: "https://travel.test/books/1".to_owned(),
        })
    );
    assert_eq!(
        played.navigated.fix(),
        Some("Await the Collie call before your driver acts on the page.")
    );
    assert_eq!(
        code_group_retryable(&played.rate_limited),
        (ErrorCode::BLOCKED, ErrorGroup::Page, false)
    );
    assert_eq!(
        played.rate_limited.details::<BlockedDetails>(),
        Some(BlockedDetails {
            cause: BlockCause::RateLimit,
            url: "https://shop.test/deals".to_owned(),
            status: Some(429),
            retry_after: Some(SystemTime::UNIX_EPOCH + SITE_ASKS_FOR),
        })
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
        ("geo_block", "https://shop.test/abroad", Some(451), None)
    );
}

fn check_model_effect_artifact_and_redirects(played: &Played) {
    assert_eq!(
        code_group_retryable(&played.model_unavailable),
        (ErrorCode::MODEL_UNAVAILABLE, ErrorGroup::Unavailable, true)
    );
    assert_eq!(
        played
            .model_unavailable
            .details::<ModelUnavailableDetails>(),
        Some(ModelUnavailableDetails {
            reason: ModelUnavailableReason::RateLimited,
            retry_after: Some(Duration::from_millis(1500)),
        })
    );
    assert_eq!(
        code_group_retryable(&played.effect_uncertain),
        (ErrorCode::EFFECT_UNCERTAIN, ErrorGroup::Page, false)
    );
    assert_eq!(
        played.effect_uncertain.details::<UncertainEffect>(),
        Some(UncertainEffect {
            entry: "effect-1".to_owned(),
            key: Some("charge-order-42".to_owned()),
            instruction: None,
            request: None,
            reason: "browser closed before confirmation".to_owned(),
            first_command: Some("locator.click".to_owned()),
            since: SystemTime::UNIX_EPOCH + SITE_ASKS_FOR,
        })
    );
    assert_eq!(
        code_group_retryable(&played.artifact_offline),
        (
            ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
            ErrorGroup::Refused,
            false
        )
    );
    assert_eq!(
        played
            .artifact_offline
            .details::<ArtifactUnavailableOfflineDetails>(),
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
                value: Value::Bool(true),
                layer: SettingsLayer::Run,
                origin: None,
            },
            fix: "Run `collie browser install` where the cache is writable.".to_owned(),
        })
    );
    assert_eq!(
        code_group_retryable(&played.too_many_redirects),
        (ErrorCode::TOO_MANY_REDIRECTS, ErrorGroup::Page, false)
    );
    assert_eq!(
        played
            .too_many_redirects
            .details::<TooManyRedirectsDetails>(),
        Some(TooManyRedirectsDetails {
            url: "https://shop.test/bounce".to_owned(),
            hops: vec![
                "https://shop.test/bounce/1".to_owned(),
                "https://shop.test/bounce/2".to_owned(),
            ],
            limit: RedirectLimit::Browser { count: 20 },
        })
    );
}

/// The `complete-error-details` client conformance case, linked: every
/// failure reads every detail its code declares through the linked engine.
///
/// Covers: story:11-38, story:11-39, story:11-191, error:secret_unavailable, error:page_navigated_during_call, error:blocked, error:model_unavailable, error:effect_uncertain, error:artifact_unavailable_offline, error:too_many_redirects, block-cause:rate_limit
#[test]
fn each_failure_reads_every_detail_its_code_declares_on_the_linked_engine() {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "complete-error-details",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();

    let played = real_time_runtime().block_on(played(&env));

    check_secret_navigated_and_blocked(&played);
    check_model_effect_artifact_and_redirects(&played);
}
