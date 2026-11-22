//! Errors and outcomes (ADR 0079), the `errors-and-outcomes` client
//! conformance case in `linked` mode: Rust's linked engine is the real one,
//! in the program's process, whose composition here is the engine's own
//! contribution and a test contribution answering the page calls as the
//! corpus's `run/errors-and-outcomes.json` does, for the engine has no page
//! that fails in every group on demand. A navigation to a missing page
//! answers its 404 response, an outcome, where each failure, crossing the
//! linked engine's protocol server and the client crate's reading of it, is
//! an `Error` carrying its code, group, retryable flag, operation, call and
//! Run IDs, the fields that apply and its typed details, a code no catalog
//! has keeping its name and its group. The program fills a field with a
//! Secret's value, the canary (ADR 0067): no rendering of any error, its
//! `Display` or `Debug` form, nor anything the program's and the linked
//! engine's tracing recorded, holds it. The case's Daemon part is
//! `goodfetch-collie-client`'s `errors_and_outcomes`, whose checks these
//! repeat; TypeScript's `errors-and-outcomes.test.ts` and Python's
//! `test_errors_and_outcomes.py` are its twins.

use std::sync::{Arc, OnceLock};

use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_collie::{EngineChoice, Error, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_engine::{CallContext, CallError, Composition, Contribution, Handler};
use goodfetch_protocol::catalog::{ErrorCodeDefinition, ErrorGroup, PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    DEADLINE_EXCEEDED, DeadlineExceededDetails, ENGINE_PANICKED, EnginePanickedDetails,
};
use goodfetch_protocol::page::{
    LOCATOR_CLICK, LOCATOR_FILL, PAGE_GOTO, RUN_NEW_PAGE, TARGET_NOT_UNIQUE, TargetNotUniqueDetails,
};
use goodfetch_protocol::secrets::{
    SECRET_INPUT_REFUSED, SECRET_UNAVAILABLE, SECRET_UNBOUND, SecretInputRefusedDetails,
    SecretUnavailableDetails, SecretUnboundDetails,
};
use goodfetch_test_support::{
    Canaries, Canary, CapturedLog, Output, SecretKind, TestEnv, capturing, case_logs,
    real_time_runtime,
};
use serde_json::{Value, json};
use tracing::Level;

use super::options;

/// The Secret's value the program fills `#card` with, which no error may
/// render.
const CANARY: &str = "typed-password-canary-3f9b2d71c8e0";

/// A model code no catalog has, which the engine answers as a model
/// provider's contribution would.
static MODEL_OUTPUT_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "model_output_invalid",
    description: "The model's answer did not fit the output schema.",
    group: ErrorGroup::Model,
    retryable: true,
    exit_code: None,
    arrived_in: PROTOCOL_VERSION,
};

/// The timeout the page failures outlived, set by the call's argument.
fn action_timeout() -> Value {
    json!({"key": "timeout.action", "value": 5000, "layer": "run", "origin": "argument"})
}

/// A failure the script answers: its code, its message and its details.
type Failing = (&'static ErrorCodeDefinition, &'static str, Value);

/// What the locator call on `selector` fails with in the Run `run_id`,
/// the script's failure for it.
fn failing(selector: &str, run_id: &str) -> CallError {
    let (definition, message, mut details) = secret_failing(selector)
        .or_else(|| other_failing(selector))
        .unwrap_or_else(|| panic!("no call of the case targets {selector}"));
    details["run_id"] = Value::from(run_id);
    CallError::new(definition, message, &details)
}

/// The script's failure of a Secret's placeholder or value on `selector`.
fn secret_failing(selector: &str) -> Option<Failing> {
    Some(match selector {
        "#user" => (
            &SECRET_UNBOUND,
            "no scope binds the Secret placeholder `%shop_user%`",
            json!({
                "operation": "locator.fill",
                "call_id": "call-1",
                "target": "#user",
                "url": "https://shop.test/login",
                "placeholder": "shop_user",
                "bound": ["shop_password", "shop_otp"],
                "fix": "Bind `shop_user` to a Secret reference in the Run's `secrets`, or use a placeholder it binds",
            }),
        ),
        "#password" => (
            &SECRET_INPUT_REFUSED,
            "the Secret `shop_password` was not typed: shop.test was reached through an invalid-certificate exception",
            json!({
                "operation": "locator.fill",
                "call_id": "call-2",
                "target": "#password",
                "url": "https://shop.test/login",
                "placeholder": "shop_password",
                "reference": "env:SHOP_PASSWORD",
                "domain": "shop.test",
                "reason": "invalid_certificate_exception",
                "fix": "Fix shop.test's certificate, or lift its invalid-certificate exception, before typing a Secret there",
            }),
        ),
        "#otp" => (
            &SECRET_UNAVAILABLE,
            "the Secret reference env:SHOP_OTP has no value: the environment variable SHOP_OTP is not set",
            json!({
                "operation": "locator.fill",
                "call_id": "call-5",
                "target": "#otp",
                "url": "https://shop.test/login",
                "placeholder": "shop_otp",
                "reference": "env:SHOP_OTP",
                "provider": "env",
                "failure": "missing",
                "fix": "Set SHOP_OTP in the engine's environment",
            }),
        ),
        _ => return None,
    })
}

/// The script's other failures on `selector`: of the page, a deadline, the
/// model and the engine.
fn other_failing(selector: &str) -> Option<Failing> {
    Some(match selector {
        "#buy" => (
            &TARGET_NOT_UNIQUE,
            "the locator #buy matched 2 elements, where the click needs one",
            json!({
                "operation": "locator.click",
                "call_id": "call-3",
                "target": "#buy",
                "url": "https://shop.test/cart",
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
                "call_log": ["waiting for #buy", "#buy matched 2 elements"],
                "fix": "Narrow the locator to one element, such as with one of the locators `matches` names",
            }),
        ),
        "#card" => (
            &DEADLINE_EXCEEDED,
            "the fill of #card did not finish within `timeout.action`'s 5000 ms: the field stayed disabled",
            json!({
                "operation": "locator.fill",
                "call_id": "call-4",
                "target": "#card",
                "url": "https://shop.test/checkout",
                "observed": "the field stayed disabled",
                "limit": action_timeout(),
                "elapsed": 5000,
                "waited_for": "target_editable",
                "call_log": ["waiting for #card to be editable", "#card is disabled"],
                "fix": "Wait for #card to be enabled first, or raise the call's timeout",
            }),
        ),
        "#next" => (
            &MODEL_OUTPUT_INVALID,
            "the model's answer did not fit the output schema after 2 attempts",
            json!({
                "operation": "locator.click",
                "call_id": "call-6",
                "target": "#next",
                "attempts": 2,
            }),
        ),
        "#done" => (
            &ENGINE_PANICKED,
            "the engine panicked during the call; the call ended, and the engine goes on",
            json!({
                "operation": "locator.click",
                "call_id": "call-7",
                "ended": "call",
                "id": "panic-1",
                "crash_report": "/srv/goodfetch/crashes/panic-1.json",
            }),
        ),
        _ => return None,
    })
}

/// The Run the case's page opened in, which `run.new_page` keeps for the
/// locator calls' failures to name.
type OpenedIn = Arc<OnceLock<String>>;

/// The Run `opened_in` holds.
fn run_of(opened_in: &OpenedIn) -> &str {
    opened_in
        .get()
        .expect("the page opened before any locator call")
}

/// A locator call's handler, failing as [`failing`] says for its selector.
fn locator_call(
    opened_in: &OpenedIn,
) -> impl Fn(Value, CallContext) -> std::future::Ready<Result<Value, CallError>> + Send + Sync + 'static
{
    let opened_in = Arc::clone(opened_in);
    move |params: Value, _: CallContext| {
        let selector = params["locator"]["selector"].as_str().unwrap_or_default();
        std::future::ready(Err(failing(selector, run_of(&opened_in))))
    }
}

/// The engine's own contribution, and the case's page calls answered as
/// the script answers them.
fn answering_as_the_script() -> Composition {
    let opened_in = OpenedIn::default();
    let opening = Arc::clone(&opened_in);
    Composition::default()
        .with(goodfetch_engine::contribution())
        .with(
            Contribution::new("test_errors_and_outcomes")
                .with_handler(Handler::new(
                    &RUN_NEW_PAGE,
                    move |params: Value, _: CallContext| {
                        let run_id = params["run_id"].as_str().unwrap_or_default();
                        opening
                            .set(run_id.to_owned())
                            .expect("the case opens one page");
                        std::future::ready(Ok::<_, CallError>(
                            json!({"page_id": "page-1", "session_id": "session-1"}),
                        ))
                    },
                ))
                .with_handler(Handler::new(&PAGE_GOTO, |params: Value, _: CallContext| {
                    std::future::ready(Ok::<_, CallError>(json!({
                        "response": {
                            "url": params["url"],
                            "status": 404,
                            "status_text": "Not Found",
                            "headers": {"content-type": "text/html; charset=utf-8"},
                            "request_id": "request-1",
                            "from_service_worker": false,
                            "encoding": "UTF-8",
                        }
                    })))
                }))
                .with_handler(Handler::new(&LOCATOR_FILL, locator_call(&opened_in)))
                .with_handler(Handler::new(&LOCATOR_CLICK, locator_call(&opened_in))),
        )
}

/// What the case's calls answered.
struct Played {
    /// The Run the case started.
    run_id: String,
    /// The navigation to a missing page: its status and URL.
    missing: (u16, String),
    /// Each call's failure, in the script's order: an unbound placeholder,
    /// a refused input, a locator matching two elements, an outlived
    /// timeout, a Secret with no value, a code no catalog has and an
    /// engine panic.
    errors: [Error; 7],
}

/// The error `answer` holds.
fn failure<T: std::fmt::Debug>(answer: Result<T, Error>) -> Error {
    let Err(error) = answer else {
        panic!("{answer:?}");
    };
    error
}

/// `error`'s code, group and whether it is retryable.
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
    let connection = collie.connection();
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
    collie.close().await.unwrap();
    Played {
        run_id: run.id().to_string(),
        missing: (missing.status(), missing.url().to_owned()),
        errors,
    }
}

/// `error`'s limit, as JSON.
fn limit(error: &Error) -> Value {
    serde_json::to_value(error.limit()).unwrap()
}

/// An unbound placeholder is a program error naming the placeholders bound,
/// and a refused input a refused one naming the Secret's reference.
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
    let details = unbound.details::<SecretUnboundDetails>().unwrap();
    assert_eq!(
        serde_json::to_value(details).unwrap(),
        json!({
            "placeholder": "shop_user",
            "bound": ["shop_password", "shop_otp"],
        })
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

/// Installs, as the process's global subscriber, which the linked engine's
/// threads log to as the program's do, one that records every event at
/// every level, and returns what it writes. The test runs in a process of
/// its own, as `just test` runs every test.
fn global_log() -> CapturedLog {
    let (subscriber, log) = capturing(&[("", Level::TRACE)]);
    tracing::subscriber::set_global_default(subscriber).unwrap();
    log
}

/// No error's `Display` or `Debug` form, nor anything the program's and
/// the linked engine's tracing recorded, `logged`, holds the canary: a
/// debug event of the fill's value, from the engine's thread, fails it.
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

/// The `errors-and-outcomes` client conformance case, linked: the 404 is
/// returned, and each failure carries its code, group, fields and typed
/// details, rendering no Secret.
///
/// Covers: story:11-28, story:11-30, story:11-187
#[test]
fn an_outcome_is_returned_and_each_failure_carries_its_code_on_the_linked_engine_rendering_no_secret()
 {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "errors-and-outcomes",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let log = global_log();

    let played = real_time_runtime().block_on(played(&env));

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
        .map(|(operation, call_id)| (
            Some(operation),
            Some(call_id),
            Some(played.run_id.as_str())
        ))
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
    assert_no_secret_rendered(&env, &played.errors, &log.written());
}
