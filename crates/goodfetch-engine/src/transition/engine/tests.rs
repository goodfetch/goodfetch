use proptest::prelude::*;
use rstest::rstest;

use super::{
    ConnectionEffect, ConnectionEvent, ConnectionState, EngineEvent, EngineState, connection,
    engine,
};
use crate::transition::Refusal;

type ConnectionOutcome = Result<(ConnectionState, ConnectionEffect), Refusal>;

const INTERNAL: Result<(ConnectionState, ConnectionEffect), Refusal> = Err(Refusal::Internal);
const INVALID_REQUEST: ConnectionOutcome = Err(Refusal::Program("invalid_request"));

/// Every pair of the Connection's table, its outcome copied from the
/// table in the module's documentation (ADR 0128).
///
/// Covers: story:3-152
#[rstest]
#[case::opening_hello(ConnectionState::Opening, ConnectionEvent::Hello, Ok((ConnectionState::Opening, ConnectionEffect::Handshake)))]
#[case::opening_request(ConnectionState::Opening, ConnectionEvent::Request, Ok((ConnectionState::Closed, ConnectionEffect::Refuse)))]
#[case::opening_notification(ConnectionState::Opening, ConnectionEvent::Notification, Ok((ConnectionState::Closed, ConnectionEffect::Refuse)))]
#[case::opening_reply(ConnectionState::Opening, ConnectionEvent::Reply, Ok((ConnectionState::Closed, ConnectionEffect::Refuse)))]
#[case::opening_unreadable(ConnectionState::Opening, ConnectionEvent::Unreadable, Ok((ConnectionState::Closed, ConnectionEffect::Refuse)))]
#[case::opening_accepted(ConnectionState::Opening, ConnectionEvent::Accepted, Ok((ConnectionState::Open, ConnectionEffect::Nothing)))]
#[case::opening_rejected(ConnectionState::Opening, ConnectionEvent::Rejected, Ok((ConnectionState::Closed, ConnectionEffect::Nothing)))]
#[case::opening_client_lost(ConnectionState::Opening, ConnectionEvent::ClientLost, Ok((ConnectionState::Closing, ConnectionEffect::CancelCalls)))]
#[case::opening_calls_ended(ConnectionState::Opening, ConnectionEvent::CallsEnded, INTERNAL)]
#[case::open_hello(ConnectionState::Open, ConnectionEvent::Hello, INVALID_REQUEST)]
#[case::open_request(ConnectionState::Open, ConnectionEvent::Request, Ok((ConnectionState::Open, ConnectionEffect::Serve)))]
#[case::open_notification(ConnectionState::Open, ConnectionEvent::Notification, Ok((ConnectionState::Open, ConnectionEffect::Serve)))]
#[case::open_reply(ConnectionState::Open, ConnectionEvent::Reply, Ok((ConnectionState::Open, ConnectionEffect::Serve)))]
#[case::open_unreadable(ConnectionState::Open, ConnectionEvent::Unreadable, Ok((ConnectionState::Open, ConnectionEffect::Answer)))]
#[case::open_accepted(ConnectionState::Open, ConnectionEvent::Accepted, INTERNAL)]
#[case::open_rejected(ConnectionState::Open, ConnectionEvent::Rejected, INTERNAL)]
#[case::open_client_lost(ConnectionState::Open, ConnectionEvent::ClientLost, Ok((ConnectionState::Closing, ConnectionEffect::CancelCalls)))]
#[case::open_calls_ended(ConnectionState::Open, ConnectionEvent::CallsEnded, INTERNAL)]
#[case::closing_hello(ConnectionState::Closing, ConnectionEvent::Hello, INTERNAL)]
#[case::closing_request(ConnectionState::Closing, ConnectionEvent::Request, INTERNAL)]
#[case::closing_notification(ConnectionState::Closing, ConnectionEvent::Notification, INTERNAL)]
#[case::closing_reply(ConnectionState::Closing, ConnectionEvent::Reply, INTERNAL)]
#[case::closing_unreadable(ConnectionState::Closing, ConnectionEvent::Unreadable, INTERNAL)]
#[case::closing_accepted(ConnectionState::Closing, ConnectionEvent::Accepted, INTERNAL)]
#[case::closing_rejected(ConnectionState::Closing, ConnectionEvent::Rejected, INTERNAL)]
#[case::closing_client_lost(ConnectionState::Closing, ConnectionEvent::ClientLost, Ok((ConnectionState::Closing, ConnectionEffect::Nothing)))]
#[case::closing_calls_ended(ConnectionState::Closing, ConnectionEvent::CallsEnded, Ok((ConnectionState::Closed, ConnectionEffect::Nothing)))]
#[case::closed_hello(ConnectionState::Closed, ConnectionEvent::Hello, INTERNAL)]
#[case::closed_request(ConnectionState::Closed, ConnectionEvent::Request, INTERNAL)]
#[case::closed_notification(ConnectionState::Closed, ConnectionEvent::Notification, INTERNAL)]
#[case::closed_reply(ConnectionState::Closed, ConnectionEvent::Reply, INTERNAL)]
#[case::closed_unreadable(ConnectionState::Closed, ConnectionEvent::Unreadable, INTERNAL)]
#[case::closed_accepted(ConnectionState::Closed, ConnectionEvent::Accepted, INTERNAL)]
#[case::closed_rejected(ConnectionState::Closed, ConnectionEvent::Rejected, INTERNAL)]
#[case::closed_client_lost(ConnectionState::Closed, ConnectionEvent::ClientLost, Ok((ConnectionState::Closed, ConnectionEffect::Nothing)))]
#[case::closed_calls_ended(ConnectionState::Closed, ConnectionEvent::CallsEnded, INTERNAL)]
fn each_pair_of_the_connections_table_gives_its_outcome(
    #[case] state: ConnectionState,
    #[case] event: ConnectionEvent,
    #[case] expected: ConnectionOutcome,
) {
    assert_eq!(
        connection(state, event).map_err(|invalid| invalid.refusal),
        expected
    );
}

mod process {
    use super::{EngineEvent, EngineState, Refusal, engine, rstest};

    const INTERNAL: Result<EngineState, Refusal> = Err(Refusal::Internal);
    const NOT_A_DAEMON: Result<EngineState, Refusal> = Err(Refusal::Program("not_a_daemon"));

    /// Every pair of the engine process's table, its outcome copied from
    /// the table in the module's documentation (ADR 0128).
    ///
    /// Covers: story:3-152
    #[rstest]
    #[case::attached_client_lost(
        EngineState::Attached,
        EngineEvent::ClientLost,
        Ok(EngineState::Attached)
    )]
    #[case::attached_stdio_ended(
        EngineState::Attached,
        EngineEvent::StdioEnded,
        Ok(EngineState::Stopping)
    )]
    #[case::attached_parent_gone(
        EngineState::Attached,
        EngineEvent::ParentGone,
        Ok(EngineState::Stopping)
    )]
    #[case::attached_idle(EngineState::Attached, EngineEvent::Idle, INTERNAL)]
    #[case::attached_stop_asked(EngineState::Attached, EngineEvent::StopAsked, NOT_A_DAEMON)]
    #[case::attached_hand_over(EngineState::Attached, EngineEvent::HandOver, INTERNAL)]
    #[case::attached_drain(EngineState::Attached, EngineEvent::Drain, INTERNAL)]
    #[case::attached_drained(EngineState::Attached, EngineEvent::Drained, INTERNAL)]
    #[case::attached_listener_failed(EngineState::Attached, EngineEvent::ListenerFailed, INTERNAL)]
    #[case::serving_client_lost(
        EngineState::Serving,
        EngineEvent::ClientLost,
        Ok(EngineState::Serving)
    )]
    #[case::serving_stdio_ended(EngineState::Serving, EngineEvent::StdioEnded, INTERNAL)]
    #[case::serving_parent_gone(EngineState::Serving, EngineEvent::ParentGone, INTERNAL)]
    #[case::serving_idle(EngineState::Serving, EngineEvent::Idle, Ok(EngineState::Stopping))]
    #[case::serving_stop_asked(
        EngineState::Serving,
        EngineEvent::StopAsked,
        Ok(EngineState::Stopping)
    )]
    #[case::serving_hand_over(
        EngineState::Serving,
        EngineEvent::HandOver,
        Ok(EngineState::HandingOver)
    )]
    #[case::serving_drain(EngineState::Serving, EngineEvent::Drain, Ok(EngineState::Draining))]
    #[case::serving_drained(EngineState::Serving, EngineEvent::Drained, INTERNAL)]
    #[case::serving_listener_failed(
        EngineState::Serving,
        EngineEvent::ListenerFailed,
        Ok(EngineState::Draining)
    )]
    #[case::handing_over_client_lost(
        EngineState::HandingOver,
        EngineEvent::ClientLost,
        Ok(EngineState::HandingOver)
    )]
    #[case::handing_over_stdio_ended(EngineState::HandingOver, EngineEvent::StdioEnded, INTERNAL)]
    #[case::handing_over_parent_gone(EngineState::HandingOver, EngineEvent::ParentGone, INTERNAL)]
    #[case::handing_over_idle(
        EngineState::HandingOver,
        EngineEvent::Idle,
        Ok(EngineState::HandingOver)
    )]
    #[case::handing_over_stop_asked(
        EngineState::HandingOver,
        EngineEvent::StopAsked,
        Ok(EngineState::HandingOver)
    )]
    #[case::handing_over_hand_over(
        EngineState::HandingOver,
        EngineEvent::HandOver,
        Ok(EngineState::HandingOver)
    )]
    #[case::handing_over_drain(
        EngineState::HandingOver,
        EngineEvent::Drain,
        Ok(EngineState::HandingOver)
    )]
    #[case::handing_over_drained(EngineState::HandingOver, EngineEvent::Drained, INTERNAL)]
    #[case::handing_over_listener_failed(
        EngineState::HandingOver,
        EngineEvent::ListenerFailed,
        Ok(EngineState::HandingOver)
    )]
    #[case::draining_client_lost(
        EngineState::Draining,
        EngineEvent::ClientLost,
        Ok(EngineState::Draining)
    )]
    #[case::draining_stdio_ended(EngineState::Draining, EngineEvent::StdioEnded, INTERNAL)]
    #[case::draining_parent_gone(EngineState::Draining, EngineEvent::ParentGone, INTERNAL)]
    #[case::draining_idle(EngineState::Draining, EngineEvent::Idle, Ok(EngineState::Draining))]
    #[case::draining_stop_asked(
        EngineState::Draining,
        EngineEvent::StopAsked,
        Ok(EngineState::Draining)
    )]
    #[case::draining_hand_over(
        EngineState::Draining,
        EngineEvent::HandOver,
        Ok(EngineState::Draining)
    )]
    #[case::draining_drain(EngineState::Draining, EngineEvent::Drain, Ok(EngineState::Draining))]
    #[case::draining_drained(
        EngineState::Draining,
        EngineEvent::Drained,
        Ok(EngineState::Stopping)
    )]
    #[case::draining_listener_failed(
        EngineState::Draining,
        EngineEvent::ListenerFailed,
        Ok(EngineState::Draining)
    )]
    #[case::stopping_client_lost(
        EngineState::Stopping,
        EngineEvent::ClientLost,
        Ok(EngineState::Stopping)
    )]
    #[case::stopping_stdio_ended(
        EngineState::Stopping,
        EngineEvent::StdioEnded,
        Ok(EngineState::Stopping)
    )]
    #[case::stopping_parent_gone(
        EngineState::Stopping,
        EngineEvent::ParentGone,
        Ok(EngineState::Stopping)
    )]
    #[case::stopping_idle(EngineState::Stopping, EngineEvent::Idle, Ok(EngineState::Stopping))]
    #[case::stopping_stop_asked(
        EngineState::Stopping,
        EngineEvent::StopAsked,
        Ok(EngineState::Stopping)
    )]
    #[case::stopping_hand_over(
        EngineState::Stopping,
        EngineEvent::HandOver,
        Ok(EngineState::Stopping)
    )]
    #[case::stopping_drain(EngineState::Stopping, EngineEvent::Drain, Ok(EngineState::Stopping))]
    #[case::stopping_drained(
        EngineState::Stopping,
        EngineEvent::Drained,
        Ok(EngineState::Stopping)
    )]
    #[case::stopping_listener_failed(
        EngineState::Stopping,
        EngineEvent::ListenerFailed,
        Ok(EngineState::Stopping)
    )]
    fn each_pair_of_the_engines_table_gives_its_outcome(
        #[case] state: EngineState,
        #[case] event: EngineEvent,
        #[case] expected: Result<EngineState, Refusal>,
    ) {
        assert_eq!(
            engine(state, event).map_err(|invalid| invalid.refusal),
            expected
        );
    }
}

fn connection_event() -> impl Strategy<Value = ConnectionEvent> {
    prop::sample::select(vec![
        ConnectionEvent::Hello,
        ConnectionEvent::Request,
        ConnectionEvent::Notification,
        ConnectionEvent::Reply,
        ConnectionEvent::Unreadable,
        ConnectionEvent::Accepted,
        ConnectionEvent::Rejected,
        ConnectionEvent::ClientLost,
        ConnectionEvent::CallsEnded,
    ])
}

fn engine_event() -> impl Strategy<Value = EngineEvent> {
    prop::sample::select(vec![
        EngineEvent::ClientLost,
        EngineEvent::StdioEnded,
        EngineEvent::ParentGone,
        EngineEvent::Idle,
        EngineEvent::StopAsked,
        EngineEvent::HandOver,
        EngineEvent::Drain,
        EngineEvent::Drained,
        EngineEvent::ListenerFailed,
    ])
}

/// Whether `code` is an Error code a Client can be answered with: one the
/// catalog defines, or one of the RPC layer's own.
fn is_error_code(code: &str) -> bool {
    goodfetch_protocol::entries::MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .any(|defined| defined.name == code)
        || code == goodfetch_protocol::jsonrpc::ProtocolFailure::InvalidRequest.name()
}

proptest! {
    /// Whatever reaches it, a closed Connection never opens again, a
    /// Connection serves a message only once open, and a program's refusal
    /// answers with an Error code.
    ///
    /// Covers: story:3-152
    #[test]
    fn a_connection_serves_only_once_open_and_never_opens_after_it_closes(
        events in proptest::collection::vec(connection_event(), 1..40),
    ) {
        let mut state = ConnectionState::Opening;
        let mut closed = false;
        for event in events {
            match connection(state, event) {
                Ok((next, effect)) => {
                    if effect == ConnectionEffect::Serve {
                        prop_assert_eq!(state, ConnectionState::Open);
                    }
                    if closed {
                        prop_assert_ne!(next, ConnectionState::Open);
                    }
                    closed |= next == ConnectionState::Closed;
                    state = next;
                }
                Err(refused) => {
                    prop_assert_eq!((refused.state, refused.event), (state, event));
                    match refused.refusal {
                        Refusal::Program(code) => prop_assert!(is_error_code(code), "{code}"),
                        Refusal::Internal => {}
                    }
                }
            }
        }
    }

    /// Whatever reaches it, a stopping engine stays stopping, and an
    /// Attached engine never takes a Daemon's states.
    ///
    /// Covers: story:3-152
    #[test]
    fn a_stopping_engine_stays_stopping_and_an_attached_one_never_acts_as_a_daemon(
        attached in any::<bool>(),
        events in proptest::collection::vec(engine_event(), 1..40),
    ) {
        let mut state = if attached { EngineState::Attached } else { EngineState::Serving };
        for event in events {
            match engine(state, event) {
                Ok(next) => {
                    if state == EngineState::Stopping {
                        prop_assert_eq!(next, EngineState::Stopping);
                    }
                    if attached {
                        prop_assert!(
                            next == EngineState::Attached || next == EngineState::Stopping,
                            "an Attached engine went {next:?}"
                        );
                    }
                    state = next;
                }
                Err(refused) => {
                    prop_assert_eq!((refused.state, refused.event), (state, event));
                    match refused.refusal {
                        Refusal::Program(code) => prop_assert!(is_error_code(code), "{code}"),
                        Refusal::Internal => {}
                    }
                }
            }
        }
    }
}
