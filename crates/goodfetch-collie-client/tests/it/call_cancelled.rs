//! `call_cancelled` read by type (ADRs 0099 and 0113): a call another
//! thread, person, Client or the browser stops fails with its details,
//! who stopped it, what closed under it and whom the stopping Client acts
//! for, and a value a newer engine names reaches the program as sent
//! (ADR 0079). Over a peer the test plays on the in-memory network.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie_client::transport::Endpoint;
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CallCancelledDetails, CancelledBy, ClosedValue,
};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use crate::connection::{hello, reply, said, scripted};

/// The answer to the request `line` with `call_cancelled` carrying
/// `details`.
fn cancelled(line: &str, details: &Value) -> Vec<u8> {
    let request: Value = serde_json::from_str(line).unwrap();
    let mut data = json!({ "code": CALL_CANCELLED.name, "retryable": false });
    data.as_object_mut()
        .unwrap()
        .extend(details.as_object().unwrap().clone());
    let mut answer = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "error": {
            "code": CALL_CANCELLED.group.json_rpc_class(),
            "message": "the call was stopped under it",
            "data": data,
        },
    }))
    .unwrap();
    answer.push(b'\n');
    answer
}

/// The error of an `engine.info` call the played engine answers with
/// `call_cancelled` carrying `details`.
fn stopped_with(details: &Value) -> Error {
    let (network, mut lines, mut writer) = scripted();
    let said = said();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said)).await.unwrap();
            while let Some(line) = lines.next_line().await.unwrap() {
                writer.write_all(&cancelled(&line, details)).await.unwrap();
            }
            writer.shutdown().await.unwrap();
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let called: Result<EngineInfo, Error> = connection
                .call(ENGINE_INFO.name, &EngineInfoParams::default())
                .await;
            connection.close().await.unwrap();
            called.unwrap_err()
        };
        let (error, ()) = tokio::join!(client, peer);
        error
    })
}

/// A call a person's Client stopped by stopping the Run under it fails
/// with `call_cancelled`, unavailable and not retryable, whose details
/// name who, what closed and for whom.
///
/// Covers: story:11-48, error:call_cancelled
#[test]
fn a_call_stopped_under_it_reads_who_stopped_it_what_closed_and_for_whom() {
    let error = stopped_with(&json!({ "by": "person", "closed": "run", "person": "ada" }));

    let details = error.details::<CallCancelledDetails>().unwrap();
    assert_eq!(
        (
            error.code(),
            error.retryable(),
            details.by,
            details.closed,
            details.person.as_deref(),
        ),
        (
            ErrorCode::CALL_CANCELLED,
            false,
            CancelledBy::Person,
            Some(ClosedValue::Run),
            Some("ada"),
        )
    );
}

/// Who stopped a call and what closed under it, as a newer engine names
/// them, reach the program unchanged, and no `closed` reads as none.
///
/// Covers: story:11-48
#[rstest]
#[case::newer_names(json!({ "by": "a_newer_stopper", "closed": "a_newer_value" }), ("a_newer_stopper", Some("a_newer_value")))]
#[case::nothing_closed(json!({ "by": "client" }), ("client", None))]
fn who_and_what_reach_the_program_as_the_engine_named_them(
    #[case] sent: Value,
    #[case] read: (&str, Option<&str>),
) {
    let error = stopped_with(&sent);

    let details = error.details::<CallCancelledDetails>().unwrap();
    assert_eq!(
        (
            details.by.as_str(),
            details.closed.as_ref().map(ClosedValue::as_str)
        ),
        read
    );
}
