//! Errors and outcomes (ADR 0079) through the client crate: the corpus's
//! script of the `errors-and-outcomes` client conformance case, which the
//! TypeScript and Python SDKs play too, with the Scripted engine as the
//! Daemon. A navigation to a missing page answers its 404 response, an
//! outcome, where each failure is an `Error` carrying its code, group,
//! retryable flag, operation, call and Run IDs, the fields that apply and
//! its typed details, a code no catalog has keeping its name and its
//! group. The program fills a field with a Secret's value, the canary
//! (ADR 0067): no rendering of any error, its `Display` or `Debug` form,
//! nor anything the program's subscriber recorded, holds it. The Scripted
//! engine fails on a message it does not expect and on a step left unused,
//! so its clean exit says the crate sent exactly what the script holds.
//! This is Rust's `daemon` mode play.

use goodfetch_collie_client::logging::engine_log_level;
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{
    DaemonEndpoint, DeadlineExceededDetails, EnginePanickedDetails, HelloParams,
};
use goodfetch_protocol::page::TargetNotUniqueDetails;
use goodfetch_protocol::secrets::{SecretInputRefusedDetails, SecretUnavailableDetails};
use goodfetch_test_support::{Canaries, Canary, Output, SecretKind, TestEnv, capturing};
use serde_json::{Value, json};
use tracing::Level;
use tracing::subscriber::with_default;

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};
use crate::start::failure;

/// The Run the script starts.
const RUN_ID: &str = "01ke6p4ym0j8x3v5wq2rtbn7hc";

/// The Secret's value the program fills `#card` with, which no error may
/// render.
const CANARY: &str = "typed-password-canary-3f9b2d71c8e0";

/// What the script's calls answered.
struct Played {
    /// The navigation to a missing page: its status and URL.
    missing: (u16, String),
    /// Each call's failure, in the script's order: an unbound placeholder,
    /// a refused input, a locator matching two elements, an outlived
    /// timeout, a Secret with no value, a code no catalog has and an
    /// engine panic.
    errors: [Error; 7],
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
    let missing = page
        .goto("https://shop.test/missing")
        .await
        .unwrap()
        .expect("a 404 is a response");
    let errors = [
        failure(page.locator("#user").fill("%shop_user%").await),
        failure(page.locator("#password").fill("%shop_password%").await),
        failure(page.locator("#buy").click().await),
        failure(page.locator("#card").fill(CANARY).await),
        failure(page.locator("#otp").fill("%shop_otp%").await),
        failure(page.locator("#next").click().await),
        failure(page.locator("#done").click().await),
    ];
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Played {
        missing: (missing.status(), missing.url().to_owned()),
        errors,
    }
}

/// `error`'s code, group and whether it is retryable.
fn code_group_retryable(error: &Error) -> (ErrorCode, ErrorGroup, bool) {
    (error.code(), error.group(), error.retryable())
}

/// The timeout the script's page failures outlived, set by the call's
/// argument.
fn action_timeout() -> Value {
    json!({"key": "timeout.action", "value": 5000, "layer": "run", "origin": "argument"})
}

/// `error`'s limit, as JSON.
fn limit(error: &Error) -> Value {
    serde_json::to_value(error.limit()).unwrap()
}

/// An unbound placeholder is a program error, and a refused input a
/// refused one naming the Secret's reference.
fn assert_program_and_refused(unbound: &Error, input_refused: &Error) {
    assert_eq!(
        code_group_retryable(unbound),
        (ErrorCode::SECRET_UNBOUND, ErrorGroup::Program, false)
    );
    assert_eq!(
        (unbound.target(), unbound.url(), unbound.fix()),
        (
            Some("#user"),
            Some("https://shop.test/login"),
            Some(
                "Bind `shop_user` to a Secret reference in the Run's `secrets`, or use a placeholder it binds"
            )
        )
    );

    assert_eq!(
        code_group_retryable(input_refused),
        (ErrorCode::SECRET_INPUT_REFUSED, ErrorGroup::Refused, false)
    );
    assert_eq!(input_refused.target(), Some("#password"));
    let details = input_refused
        .details::<SecretInputRefusedDetails>()
        .unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "placeholder": "shop_password",
            "reference": "env:SHOP_PASSWORD",
            "domain": "shop.test",
            "reason": "invalid_certificate_exception",
        })
    );
}

/// A locator matching two elements is a page error with its limit and call
/// log, and an outlived timeout a retryable deadline saying what it
/// observed and which limit.
fn assert_page_and_deadline(not_unique: &Error, deadline: &Error) {
    assert_eq!(
        code_group_retryable(not_unique),
        (ErrorCode::TARGET_NOT_UNIQUE, ErrorGroup::Page, false)
    );
    assert_eq!(
        (not_unique.target(), not_unique.url(), limit(not_unique)),
        (
            Some("#buy"),
            Some("https://shop.test/cart"),
            action_timeout()
        )
    );
    assert_eq!(
        not_unique.call_log(),
        ["waiting for #buy", "#buy matched 2 elements"]
    );
    let details = not_unique.details::<TargetNotUniqueDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "target": "#buy",
            "matches": [
                {
                    "element": "<button id=\"buy\">Buy now</button>",
                    "locator": "getByRole('button', { name: 'Buy now' })",
                },
                {
                    "element": "<button id=\"buy\">Buy later</button>",
                    "locator": "getByRole('button', { name: 'Buy later' })",
                },
            ],
            "limit": action_timeout(),
            "elapsed": 5000,
        })
    );

    assert_eq!(
        code_group_retryable(deadline),
        (
            ErrorCode::DEADLINE_EXCEEDED,
            ErrorGroup::DeadlineExceeded,
            true
        )
    );
    assert_eq!(
        (deadline.target(), deadline.fix()),
        (
            Some("#card"),
            Some("Wait for #card to be enabled first, or raise the call's timeout")
        )
    );
    let details = deadline.details::<DeadlineExceededDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "limit": action_timeout(),
            "elapsed": 5000,
            "waited_for": "target_editable",
            "observed": "the field stayed disabled",
        })
    );
    assert_eq!(
        (deadline.observed(), limit(deadline)),
        (Some("the field stayed disabled"), action_timeout())
    );
}

/// A Secret with no value is an unavailable error naming its provider, a
/// code no catalog has keeps its name and its group, and an engine panic
/// is an internal error.
fn assert_unavailable_model_and_internal(
    unavailable: &Error,
    from_outside: &Error,
    panicked: &Error,
) {
    assert_eq!(
        code_group_retryable(unavailable),
        (
            ErrorCode::SECRET_UNAVAILABLE,
            ErrorGroup::Unavailable,
            false
        )
    );
    let fix = "Set SHOP_OTP in the engine's environment";
    assert_eq!(unavailable.fix(), Some(fix));
    let details = unavailable.details::<SecretUnavailableDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "placeholder": "shop_otp",
            "reference": "env:SHOP_OTP",
            "provider": "env",
            "failure": "missing",
            "fix": fix,
        })
    );

    assert_eq!(
        (
            from_outside.code().as_str(),
            from_outside.group(),
            from_outside.retryable(),
            from_outside.target()
        ),
        (
            "model_output_invalid",
            ErrorGroup::Model,
            true,
            Some("#next")
        ),
        "a code no catalog has keeps its name and its group"
    );

    assert_eq!(
        code_group_retryable(panicked),
        (ErrorCode::ENGINE_PANICKED, ErrorGroup::Internal, false)
    );
    let details = panicked.details::<EnginePanickedDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "ended": "call",
            "id": "panic-1",
            "crash_report": "/srv/goodfetch/crashes/panic-1.json",
        })
    );
}

/// No error's `Display` or `Debug` form, nor anything the program's
/// subscriber recorded, `logged`, holds the canary.
fn assert_no_secret_rendered(env: &TestEnv, errors: &[Error], logged: &[u8]) {
    let mut canaries = Canaries::new(env.seed());
    canaries.add(Canary::known(
        "card-typed-password",
        SecretKind::TypedPassword,
        CANARY,
    ));
    let rendered: Vec<(String, String)> = errors
        .iter()
        .enumerate()
        .flat_map(|(index, error)| {
            [
                (format!("display {index}"), format!("{error}")),
                (format!("debug {index}"), format!("{error:?}")),
                (format!("pretty debug {index}"), format!("{error:#?}")),
            ]
        })
        .collect();
    let outputs: Vec<Output<'_>> = rendered
        .iter()
        .map(|(name, text)| Output::Bytes {
            name,
            bytes: text.as_bytes(),
        })
        .chain([Output::Bytes {
            name: "log",
            bytes: logged,
        }])
        .collect();
    assert_eq!(canaries.search(&outputs).unwrap(), []);
}

/// The `errors-and-outcomes` client conformance case, played by the
/// Scripted engine's Daemon: the 404 is returned, and each failure carries
/// its code, group, fields and typed details, rendering no Secret.
/// Covers: story:11-28, story:11-30, story:11-187
#[test]
fn an_outcome_is_returned_and_each_failure_carries_its_code_rendering_no_secret() {
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (subscriber, recorded) = capturing(&[("goodfetch", Level::DEBUG)]);

    let (process, played) = with_default(subscriber, || {
        let hello = HelloParams {
            log_level: Some(engine_log_level()),
            ..daemon_hello()
        };
        let (process, endpoint) =
            scripted_daemon_hearing(&env, "sdk-rust/run/errors-and-outcomes.json", &hello);
        let played = runtime.block_on(played(endpoint, hello));
        (process, played)
    });
    let status = process.child().wait().unwrap();

    assert_eq!(
        played.missing,
        (404, "https://shop.test/missing".to_owned()),
        "a 404 is the navigation's outcome, returned, not an error"
    );
    assert_eq!(
        played
            .errors
            .iter()
            .map(|error| (error.operation(), error.call_id(), error.run_id()))
            .collect::<Vec<_>>(),
        [
            ("locator.fill", "call-1"),
            ("locator.fill", "call-2"),
            ("locator.click", "call-3"),
            ("locator.fill", "call-4"),
            ("locator.fill", "call-5"),
            ("locator.click", "call-6"),
            ("locator.click", "call-7"),
        ]
        .map(|(operation, call_id)| (Some(operation), Some(call_id), Some(RUN_ID)))
    );
    let [
        unbound,
        input_refused,
        not_unique,
        deadline,
        unavailable,
        from_outside,
        panicked,
    ] = &played.errors;
    assert_program_and_refused(unbound, input_refused);
    assert_page_and_deadline(not_unique, deadline);
    assert_unavailable_model_and_internal(unavailable, from_outside, panicked);
    assert_no_secret_rendered(&env, &played.errors, &recorded.written());
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
