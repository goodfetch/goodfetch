//! The exchanges the protocol suite's case IDs name, each judged against
//! what `goodfetch-protocol` says the protocol is: its JSON-RPC failures,
//! its Error codes and its operations' types.

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::connection::{CALL_CANCELLED, CANCEL, HelloResult};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo};
use goodfetch_protocol::events::EVENTS_SUBSCRIBE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::result_document::CaseCheck;
use serde_json::{Value, json};

use super::session::{Gone, Session};

/// A method no Product carries, which every target must refuse.
const UNKNOWN_METHOD: &str = "conformance.unknown_method";

/// One case the protocol suite can declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Hello,
    EngineInfo,
    UnknownMethod,
    InvalidParams,
    CancelInFlight,
    CancelAnswered,
}

/// The checks a case made so far.
#[derive(Default)]
pub(crate) struct Checks(pub(crate) Vec<CaseCheck>);

impl Checks {
    fn check(&mut self, name: impl Into<String>, passed: bool) {
        self.0.push(CaseCheck {
            name: name.into(),
            passed,
        });
    }
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn of(id: &str) -> Option<Self> {
        Some(match id {
            "hello-is-answered-with-the-engines-product" => Self::Hello,
            "engine-info-names-the-engine-hello-named" => Self::EngineInfo,
            "an-unknown-method-is-method-not-found" => Self::UnknownMethod,
            "params-the-operation-does-not-take-are-invalid-params" => Self::InvalidParams,
            "a-call-in-flight-cancelled-ends-with-call-cancelled" => Self::CancelInFlight,
            "cancelling-an-answered-call-leaves-the-connection-answering" => Self::CancelAnswered,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection, whose `hello` the engine
    /// answered with `hello`.
    pub(crate) fn run(
        self,
        session: &mut Session,
        hello: &HelloResult,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Hello => {
                checks.check(
                    format!(
                        "the engine speaks the protocol's major {}",
                        PROTOCOL_VERSION.major
                    ),
                    hello.protocol_version.major == PROTOCOL_VERSION.major,
                );
                Ok(())
            }
            Self::EngineInfo => engine_info(session, hello, checks),
            Self::UnknownMethod => {
                let id = session.request(UNKNOWN_METHOD, &json!({}))?;
                let answer = session.answer(id)?.message;
                refused_as(&answer, ProtocolFailure::MethodNotFound, checks);
                Ok(())
            }
            Self::InvalidParams => {
                let id = session.request(ENGINE_INFO.name, &json!({ "unknown": true }))?;
                let answer = session.answer(id)?.message;
                refused_as(&answer, ProtocolFailure::InvalidParams, checks);
                Ok(())
            }
            Self::CancelInFlight => cancel_in_flight(session, checks),
            Self::CancelAnswered => cancel_answered(session, checks),
        }
    }
}

fn engine_info(
    session: &mut Session,
    hello: &HelloResult,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let id = session.request(ENGINE_INFO.name, &json!({}))?;
    let answer = session.answer(id)?.message;
    let info = answer
        .get("result")
        .and_then(|result| serde_json::from_value::<EngineInfo>(result.clone()).ok());
    checks.check(
        "`engine.info` answers with what its result type holds",
        info.is_some(),
    );
    if let Some(info) = info {
        checks.check(
            "it names the Product `hello` named",
            info.product == hello.product,
        );
        checks.check(
            "it names the Product version `hello` named",
            info.product_version == hello.product_version,
        );
        checks.check(
            "it names the protocol version `hello` named",
            info.protocol_version == hello.protocol_version,
        );
        checks.check(
            "its Product major is its Product version's",
            info.product_major == info.product_version.major(),
        );
    }
    Ok(())
}

/// Checks that `answer` is the refusal JSON-RPC predefines for `failure`,
/// its `data.code` naming it.
fn refused_as(answer: &Value, failure: ProtocolFailure, checks: &mut Checks) {
    let error = &answer["error"];
    checks.check(
        format!(
            "the call is refused with JSON-RPC's code {}",
            failure.code()
        ),
        error["code"] == json!(failure.code()),
    );
    checks.check(
        format!("its `data.code` is `{}`", failure.name()),
        error["data"]["code"] == json!(failure.name()),
    );
}

/// A subscription stays in flight until its Client cancels it (ADR 0080),
/// then ends with `call_cancelled`, stopped by the Client (ADR 0113).
fn cancel_in_flight(session: &mut Session, checks: &mut Checks) -> Result<(), Gone> {
    let id = session.request(EVENTS_SUBSCRIBE.name, &json!({}))?;
    session.notify(CANCEL, &json!({ "id": id }))?;
    let error = &session.answer(id)?.message["error"];
    checks.check(
        format!(
            "the call ends with the `{}` group's code {}",
            CALL_CANCELLED.group.as_str(),
            CALL_CANCELLED.group.json_rpc_class()
        ),
        error["code"] == json!(CALL_CANCELLED.group.json_rpc_class()),
    );
    checks.check(
        format!("its `data.code` is `{}`", CALL_CANCELLED.name),
        error["data"]["code"] == json!(CALL_CANCELLED.name),
    );
    checks.check(
        "it says no retry succeeds",
        error["data"]["retryable"] == json!(CALL_CANCELLED.retryable),
    );
    checks.check(
        "it says the Client stopped it",
        error["data"]["by"] == json!("client"),
    );
    Ok(())
}

/// A `$/cancel` of a call already answered gets no answer of its own, and
/// the next call is answered.
fn cancel_answered(session: &mut Session, checks: &mut Checks) -> Result<(), Gone> {
    let answered = session.request(ENGINE_INFO.name, &json!({}))?;
    session.answer(answered)?;
    session.notify(CANCEL, &json!({ "id": answered }))?;
    let next = session.request(ENGINE_INFO.name, &json!({}))?;
    let answer = session.answer(next)?;
    checks.check(
        "the next call is answered with a result",
        answer.message.get("result").is_some(),
    );
    checks.check(
        "the cancel gets no answer of its own",
        answer
            .others
            .iter()
            .all(|message| message.get("id") != Some(&json!(answered))),
    );
    Ok(())
}
