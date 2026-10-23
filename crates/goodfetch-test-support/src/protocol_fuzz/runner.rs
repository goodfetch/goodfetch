//! `ProtocolFuzz`: runs sequences of generated calls over the caller's
//! transport, as `proptest-state-machine` sequences on proptest's
//! configuration, checks each answer and the state the caller reads after
//! it, and reports the sequence proptest shrank a failure to.

use std::cell::RefCell;
use std::fmt;
use std::ops::RangeInclusive;
use std::sync::atomic::Ordering;

use proptest::strategy::Just;
use proptest::test_runner::{Config, TestCaseError, TestError, TestRunner};
use proptest_state_machine::strategy::Sequential;
use serde_json::Value;

use super::calls::{FuzzedCall, calls};
use super::schema::{Answer, Contract, ProtocolSchema};

/// A connection to the engine under test that sends one call and returns
/// its reply.
pub trait ProtocolTransport {
    /// Sends `request`, a JSON-RPC call, and returns the reply carrying its
    /// id, past any notification sent meanwhile.
    ///
    /// # Errors
    ///
    /// When the connection fails or closes before the reply.
    fn call(&mut self, request: &Value) -> Result<Value, TransportFailure>;
}

/// The transport failed or closed before a call's reply.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the transport failed: {message}")]
pub struct TransportFailure {
    message: String,
}

impl TransportFailure {
    /// The transport failed, as `message` says.
    pub fn new(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// How a call broke the property.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Broken<S> {
    /// An invalid call was answered with a result.
    Accepted {
        /// The reply.
        reply: Value,
    },
    /// A call was answered with no typed error and no result of its own.
    Untyped {
        /// The reply.
        reply: Value,
    },
    /// A call was answered with an error that reports a bug in Goodfetch.
    Internal {
        /// The reply.
        reply: Value,
    },
    /// A valid call's result does not meet its operation's result schema.
    ResultBreaksSchema {
        /// The reply.
        reply: Value,
    },
    /// An invalid call changed the state the caller reads.
    StateChanged {
        /// The state before the call.
        before: S,
        /// The state after it.
        after: S,
    },
    /// The transport failed before the call's reply.
    Transport {
        /// How.
        failure: TransportFailure,
    },
}

/// Why the property failed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ProtocolFuzzError<S> {
    /// A sequence broke the property: the sequence proptest shrank it to,
    /// whose last call broke it.
    Broken {
        /// The calls, in the order they were sent.
        calls: Vec<FuzzedCall>,
        /// How the last one broke the property.
        broken: Broken<S>,
    },
    /// The sequence proptest shrank a failure to passed when sent again on
    /// a new connection, so the engine under test does not answer the same
    /// calls the same way.
    NotReproduced {
        /// The calls.
        calls: Vec<FuzzedCall>,
        /// The failure proptest saw.
        reason: String,
    },
    /// Proptest gave up, such as when the generator rejected too many
    /// values.
    Abandoned {
        /// Proptest's reason.
        reason: String,
    },
}

/// Protocol fuzzing of one schema: invalid calls of every operation, and
/// valid calls of the operations the caller names.
#[derive(Clone, Debug)]
pub struct ProtocolFuzz {
    schema: ProtocolSchema,
    valid: Vec<String>,
}

impl ProtocolFuzz {
    /// Fuzzing from `schema`, with invalid calls only.
    pub fn new(schema: ProtocolSchema) -> Self {
        Self {
            schema,
            valid: Vec::new(),
        }
    }

    /// Also sends valid calls of the operations `methods` names, each of
    /// which must answer at once.
    #[must_use]
    pub fn with_valid_calls(mut self, methods: &[&str]) -> Self {
        self.valid = methods.iter().map(|&method| method.to_owned()).collect();
        self
    }

    /// Runs `config`'s cases, each a sequence of calls on the transport
    /// `connect` opens for it, with the state `state` reads before the
    /// first call and after each one.
    ///
    /// # Errors
    ///
    /// When a sequence breaks the property, with the sequence proptest
    /// shrank it to.
    pub fn run<T, S>(
        &self,
        config: Config,
        connect: impl FnMut() -> T,
        state: impl FnMut(&mut T) -> S,
    ) -> Result<(), ProtocolFuzzError<S>>
    where
        T: ProtocolTransport,
        S: Clone + fmt::Debug + PartialEq,
    {
        let calls = calls(&self.schema.contract, &self.valid);
        let sequences = Sequential::new(
            CALLS_PER_SEQUENCE.into(),
            || Just(()),
            |(), _: &FuzzedCall| true,
            move |()| calls.clone(),
            |(), _: &FuzzedCall| (),
        );
        let mut sequence = Sequence {
            contract: &self.schema.contract,
            connect,
            state,
        };
        let cell = RefCell::new(&mut sequence);
        let outcome = TestRunner::new(config).run(&sequences, |((), calls, seen)| {
            // Proptest-state-machine counts the calls sent, so shrinking
            // first drops those never sent.
            let mut count = || {
                if let Some(seen) = &seen {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            };
            match cell.borrow_mut().send(&calls, &mut count) {
                Ok(()) => Ok(()),
                Err((index, broken)) => Err(TestCaseError::fail(format!(
                    "call {index} broke the property: {broken:?}"
                ))),
            }
        });
        match outcome {
            Ok(()) => Ok(()),
            Err(TestError::Abort(reason)) => Err(ProtocolFuzzError::Abandoned {
                reason: reason.to_string(),
            }),
            Err(TestError::Fail(reason, ((), mut calls, _))) => {
                match sequence.send(&calls, &mut || {}) {
                    Ok(()) => Err(ProtocolFuzzError::NotReproduced {
                        calls,
                        reason: reason.to_string(),
                    }),
                    Err((index, broken)) => {
                        calls.truncate(index + 1);
                        Err(ProtocolFuzzError::Broken { calls, broken })
                    }
                }
            }
        }
    }
}

/// How many calls one sequence sends.
const CALLS_PER_SEQUENCE: RangeInclusive<usize> = 1..=12;

/// What sends one sequence: the contract its answers are checked against,
/// the caller's connection and its state read.
struct Sequence<'a, C, R> {
    contract: &'a Contract,
    connect: C,
    state: R,
}

impl<C, R> Sequence<'_, C, R> {
    /// Sends `calls` on a new connection, calling `count` before each, and
    /// stops at the first that breaks the property, with its index.
    fn send<T, S>(
        &mut self,
        calls: &[FuzzedCall],
        count: &mut dyn FnMut(),
    ) -> Result<(), (usize, Broken<S>)>
    where
        C: FnMut() -> T,
        R: FnMut(&mut T) -> S,
        T: ProtocolTransport,
        S: PartialEq,
    {
        let mut transport = (self.connect)();
        let mut model = (self.state)(&mut transport);
        for (index, call) in calls.iter().enumerate() {
            count();
            let id = u64::try_from(index + 1).expect("invariant: a sequence is short");
            let reply = transport
                .call(&call.request(id))
                .map_err(|failure| (index, Broken::Transport { failure }))?;
            let answer = self.contract.answer(id, call.method(), &reply);
            let after = (self.state)(&mut transport);
            let broken = match (call.invalidity(), answer) {
                (_, Answer::Untyped) => Broken::Untyped { reply },
                (_, Answer::Internal) => Broken::Internal { reply },
                (Some(_), Answer::Result { .. }) => Broken::Accepted { reply },
                (
                    None,
                    Answer::Result {
                        meets_schema: false,
                    },
                ) => Broken::ResultBreaksSchema { reply },
                (Some(_), Answer::Refused) if after != model => Broken::StateChanged {
                    before: model,
                    after,
                },
                (Some(_), Answer::Refused) => continue,
                (None, Answer::Refused | Answer::Result { meets_schema: true }) => {
                    model = after;
                    continue;
                }
            };
            return Err((index, broken));
        }
        Ok(())
    }
}
