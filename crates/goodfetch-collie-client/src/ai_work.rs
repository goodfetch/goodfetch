//! The result of AI work that can claim success, an Agent's task or a Plan
//! replayed toward a goal (ADRs 0079, 0100 and 0117): how it ended, its
//! Verdict and what its model claimed, three fields read apart, with the
//! answer inside how it ended. `require_verified()` turns a result its
//! checks did not prove into `not_verified`, only when the program asks.

#[cfg(test)]
mod tests;

use goodfetch_protocol::page::ai_work::{NotVerifiedDetails, Reported};
use goodfetch_protocol::run::{AxisReason, StopReason, Verdict};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Error;

/// AI work's result (ADR 0079): `ended` holds the answer, `verdict` says
/// whether the program's checks prove it, and `reported` is the model's
/// claim, a signal and never proof.
#[derive(Clone, Debug, PartialEq)]
#[must_use = "AI work's result carries how it ended and its Verdict; read them or call require_verified()"]
pub struct AiWorkResult<T> {
    /// How the work ended, its answer inside.
    pub ended: Ended<T>,
    /// Whether the program's checks prove the work (ADR 0029), never set
    /// by the model's claim; an unknown Verdict is never `verified`.
    pub verdict: Verdict,
    /// Why the Verdict is not `verified`, empty when it is.
    pub verdict_reasons: Vec<AxisReason>,
    /// What the model claimed, absent when no model claimed anything: the
    /// work stopped before a claim, or was replayed whole (ADR 0100).
    pub reported: Option<Reported>,
}

/// How AI work ended (ADR 0117). It is exhaustive: a third way to end
/// would be a new meaning, not a new value, so a `match` needs no wildcard
/// arm, while [`StopReason`] stays open.
#[derive(Clone, Debug, PartialEq)]
pub enum Ended<T> {
    /// It ran to its end: `output` is a complete `T`.
    Finished {
        /// The answer, checked against its schema.
        output: T,
    },
    /// It stopped early, such as at a Limit or its timeout.
    Stopped {
        /// Why it stopped.
        reason: StopReason,
        /// What it had gathered and Grounded, typed as partial.
        partial: PartialOutput,
    },
}

/// What stopped AI work had gathered (ADR 0100): the latest valid,
/// Grounded value of each top-level field of an object output, or each
/// element of a list output, read by type.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialOutput(Value);

impl PartialOutput {
    /// The partial output the engine sent as `value`.
    #[must_use]
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    /// The top-level field `name` as `F`, none when the work gathered none.
    ///
    /// # Errors
    ///
    /// The field does not read as `F`.
    pub fn field<F: DeserializeOwned>(&self, name: &str) -> Result<Option<F>, serde_json::Error> {
        self.0
            .get(name)
            .map(|value| F::deserialize(value))
            .transpose()
    }

    /// The elements of a list output gathered so far, each as `I`; none
    /// for an object output.
    ///
    /// # Errors
    ///
    /// An element does not read as `I`.
    pub fn items<I: DeserializeOwned>(&self) -> Result<Vec<I>, serde_json::Error> {
        match &self.0 {
            Value::Array(items) => items.iter().map(I::deserialize).collect(),
            _ => Ok(Vec::new()),
        }
    }

    /// The JSON it holds.
    #[must_use]
    pub fn as_json(&self) -> &Value {
        &self.0
    }
}

impl<T> AiWorkResult<T> {
    /// The output, only when the work finished and its Verdict is
    /// `verified` (ADR 0079), as httpx's `raise_for_status()` turns an
    /// outcome into an error only when asked.
    ///
    /// # Errors
    ///
    /// `not_verified`, a page error carrying the Verdict, its reasons and
    /// the model's claim, when the work stopped or its Verdict is anything
    /// but `verified`.
    pub fn require_verified(self) -> Result<T, Error> {
        match (self.ended, &self.verdict) {
            (Ended::Finished { output }, Verdict::Verified) => Ok(output),
            _ => Err(Error::not_verified(&NotVerifiedDetails {
                verdict: self.verdict,
                reasons: self.verdict_reasons,
                reported: self.reported,
            })),
        }
    }
}
