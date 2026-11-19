//! What dispatch takes of a request's params before its handler, over the
//! in-memory network: the order a call's refusals come in, and the IDs and
//! the Trace context its span holds whoever answers it. A `_meta` the engine
//! refuses is refused before the call's guard is asked; a guard's refusal
//! comes before `invalid_params` for params that are not the handler's
//! type; and the span of a call a guard refused, or whose params its
//! handler refused, holds what the params name (ADRs 0077 and 0132).
#![expect(
    clippy::disallowed_types,
    reason = "the guard and the handler, which the engine calls synchronously, note that they ran under a lock no loom model needs (ADR 0060)"
)]

use std::future::ready;
use std::sync::{Arc, Mutex};

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use opentelemetry::trace::{SpanId, SpanKind};
use rstest::rstest;
use serde::Deserialize;
use serde_json::{Value, json};

use super::trace::{PARENT_ID, TRACEPARENT, attribute, exported, the_span, trace_id};
use super::{Client, attached, engine_with, opened, owed};
use crate::call_error::CallError;
use crate::composition::{
    Answered, CallContext, Contribution, Enclose, Guard, GuardAnswer, Handler,
};

/// A call whose params are a type of their own, behind a guard.
const TYPED: Operation = Operation {
    name: "test.typed",
    description: "Answers the count its params hold.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// The params of [`TYPED`], which name neither ID a span records.
#[derive(Deserialize)]
struct Typed {
    count: u32,
}

/// Who ran for a call, in order: its guard, its handler.
type Ran = Arc<Mutex<Vec<&'static str>>>;

/// [`TYPED`] made cancellable, as every call an enclosing call can end is.
const CANCELLABLE: Operation = Operation {
    name: "test.cancellable",
    cancellable: true,
    ..TYPED
};

/// `test.typed`, whose guard refuses params that hold a member `refuse`
/// with `invalid_request`; each notes in `ran` that it ran.
fn typed(ran: &Ran) -> Contribution {
    typed_as(&TYPED, ran)
}

/// [`typed`]'s guard and handler, answering `operation`.
fn typed_as(operation: &'static Operation, ran: &Ran) -> Contribution {
    let (asked, handled) = (Arc::clone(ran), Arc::clone(ran));
    let guard: Guard = Arc::new(move |_, params, _, _| {
        asked.lock().unwrap().push("guard");
        params.json().contains(r#""refuse""#).then(|| {
            Box::pin(ready(Err(CallError::protocol(
                ProtocolFailure::InvalidRequest,
                "the guard refuses",
            )))) as GuardAnswer
        })
    });
    Contribution::new("test_typed")
        .with_handler(Handler::new(
            operation,
            move |params: Typed, _: CallContext| {
                handled.lock().unwrap().push("handler");
                ready(Ok::<Value, CallError>(json!(params.count)))
            },
        ))
        .guarded(&guard)
}

/// Opens an Attached engine with `test.typed`, says `hello`, calls it with
/// `params`, the JSON text of its params as written, and answers the reply
/// with who ran.
fn called(params: &str) -> (Value, Vec<&'static str>) {
    let ran = Ran::default();
    let engine = engine_with(Product::Collie, Some(typed(&ran)));
    let line = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"test.typed","params":{params}}}"#);
    let reply = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        client.send_bytes(format!("{line}\n").as_bytes()).await;
        owed(&mut client).await.expect("the call is answered")
    });
    let ran = ran.lock().unwrap().clone();
    (reply, ran)
}

/// Opens an Attached engine with `operation`, enclosed by a bound that
/// answers `"enclosed"` in place of the handler's answer, calls it, and
/// answers the reply with who ran.
fn called_enclosed(operation: &'static Operation) -> (Value, Vec<&'static str>) {
    let ran = Ran::default();
    let bounded = Arc::clone(&ran);
    let enclose: Enclose = Arc::new(move |_, _, context, answer| {
        bounded.lock().unwrap().push("enclose");
        let context = context.clone();
        Box::pin(async move {
            answer.await?;
            Answered::encode(&context, &json!("enclosed"))
        })
    });
    let engine = engine_with(
        Product::Collie,
        Some(typed_as(operation, &ran).enclosed(&enclose)),
    );
    let line = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"{}","params":{{"count":2}}}}"#,
        operation.name
    );
    let reply = attached(&engine, |mut client: Client| async move {
        opened(&mut client).await;
        client.send_bytes(format!("{line}\n").as_bytes()).await;
        owed(&mut client).await.expect("the call is answered")
    });
    let ran = ran.lock().unwrap().clone();
    (reply, ran)
}

/// An enclosing call bounds only the calls a cancel can end: the bound
/// drops a call's answer when the enclosing end comes, which only a
/// cancellable call allows (ADR 0080).
#[rstest]
#[case::cancellable(&CANCELLABLE, json!("enclosed"), vec!["guard", "handler", "enclose"])]
#[case::not_cancellable(&TYPED, json!(2), vec!["guard", "handler"])]
fn an_enclosing_bound_wraps_only_a_cancellable_calls_answer(
    #[case] operation: &'static Operation,
    #[case] answer: Value,
    #[case] expected: Vec<&'static str>,
) {
    let (reply, ran) = called_enclosed(operation);

    assert_eq!((&reply["result"], ran), (&answer, expected), "{reply}");
}

/// The Error code of `reply`, and whether its message names `_meta`.
fn refusal(reply: &Value) -> (Option<&str>, bool) {
    (
        reply["error"]["data"]["code"].as_str(),
        reply["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("`_meta`")),
    )
}

#[test]
fn a_call_whose_params_are_its_handlers_type_asks_its_guard_then_its_handler() {
    let (reply, ran) = called(r#"{"count":2,"_meta":{"traceparent":"00-0af7"}}"#);

    assert_eq!(
        (&reply["result"], ran),
        (&json!(2), vec!["guard", "handler"])
    );
}

/// A `_meta` the engine refuses is refused before the guard is asked and
/// before the handler reads the params, wherever it stands among them and
/// whatever else they hold.
#[rstest]
#[case::alone(r#"{"_meta":{"baggage":"user=a"},"count":2}"#)]
#[case::after_the_handlers_members(r#"{"count":2,"_meta":{"traceparent":1}}"#)]
#[case::with_what_the_guard_refuses(r#"{"refuse":true,"count":2,"_meta":[]}"#)]
#[case::before_a_member_of_another_type(r#"{"_meta":null,"count":"many"}"#)]
#[case::after_a_member_of_another_type(r#"{"count":"many","_meta":"00-0af7"}"#)]
#[case::the_last_of_two(r#"{"_meta":{},"count":2,"_meta":{"baggage":"user=a"}}"#)]
fn a_refused_meta_is_refused_before_the_guard_and_the_handler(#[case] params: &str) {
    let (reply, ran) = called(params);

    assert_eq!(
        (refusal(&reply), ran),
        ((Some("invalid_params"), true), Vec::<&str>::new()),
        "{reply}"
    );
}

/// Only the last `_meta` of the params counts, as for any member.
#[test]
fn a_refused_meta_before_one_the_engine_takes_refuses_nothing() {
    let (reply, ran) = called(r#"{"_meta":{"baggage":"user=a"},"count":2,"_meta":{}}"#);

    assert_eq!(
        (&reply["result"], ran),
        (&json!(2), vec!["guard", "handler"])
    );
}

#[rstest]
#[case::of_the_handlers_type(r#"{"refuse":true,"count":2}"#)]
#[case::of_another_type(r#"{"refuse":true,"count":"many"}"#)]
#[case::no_object(r#"["refuse"]"#)]
fn a_guards_refusal_comes_before_the_handler_reads_the_params(#[case] params: &str) {
    let (reply, ran) = called(params);

    assert_eq!(
        (refusal(&reply), ran),
        ((Some("invalid_request"), false), vec!["guard"]),
        "{reply}"
    );
}

#[rstest]
#[case::a_member_of_another_type(r#"{"count":"many"}"#)]
#[case::a_member_missing(r"{}")]
#[case::an_array(r"[2]")]
fn params_that_are_not_the_handlers_type_are_refused_once_the_guard_let_them_pass(
    #[case] params: &str,
) {
    let (reply, ran) = called(params);

    assert_eq!(
        (refusal(&reply), ran),
        ((Some("invalid_params"), false), vec!["guard"]),
        "{reply}"
    );
}

/// The span of `test.typed` called with `params`, with every span
/// exported: its `run_id` and `session_id`, and whether it is the child of
/// the caller's span.
fn span_of(params: &str) -> ([Option<String>; 2], bool) {
    let (_, spans) = exported(1, || called(params));
    let span = the_span(&spans, &SpanKind::Server, "test.typed");
    (
        ["run_id", "session_id"].map(|key| attribute(span, key)),
        span.span_context.trace_id() == trace_id()
            && span.parent_span_id == SpanId::from_hex(PARENT_ID).unwrap(),
    )
}

/// The params `members`, then the caller's Trace context.
fn traced(members: &str) -> String {
    format!(r#"{{{members},"_meta":{{"traceparent":"{TRACEPARENT}"}}}}"#)
}

/// A request's span holds the Run ID and the Session ID its params name
/// and is the child of the span their `_meta` names, whether its handler
/// answered, its guard refused it or its handler refused its params, and
/// whether or not the handler's type names those members.
#[rstest]
#[case::answered(r#""count":2,"run_id":"run-a","session_id":"session-a""#)]
#[case::refused_by_the_guard(r#""refuse":true,"run_id":"run-a","session_id":"session-a""#)]
#[case::of_another_type_before_the_ids(
    r#""count":"many","run_id":"run-a","session_id":"session-a""#
)]
#[case::of_another_type_after_the_ids(
    r#""run_id":"run-a","session_id":"session-a","count":"many""#
)]
#[case::the_last_of_each(
    r#""run_id":"run-b","session_id":"session-b","count":2,"run_id":"run-a","session_id":"session-a""#
)]
#[case::written_with_an_escape(r#""count":2,"run_id":"run-a","session_id":"session-a""#)]
fn a_requests_span_holds_the_ids_and_the_trace_context_its_params_name(#[case] members: &str) {
    assert_eq!(
        span_of(&traced(members)),
        (
            [Some("run-a".to_owned()), Some("session-a".to_owned())],
            true
        )
    );
}

/// An ID that is no string, or no ID, is not recorded, and the call goes
/// on.
#[rstest]
#[case::a_number(r#""count":2,"run_id":7"#)]
#[case::no_id(r#""count":2,"run_id":"a run""#)]
fn a_requests_span_leaves_out_an_id_its_params_do_not_hold(#[case] members: &str) {
    assert_eq!(span_of(&traced(members)), ([None, None], true));
}
