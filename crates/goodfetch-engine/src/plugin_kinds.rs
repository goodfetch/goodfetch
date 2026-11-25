//! The Plugin kinds' one call path (ADRs 0033, 0104, 0127 and 0129): each
//! kind has one trait of primitives, which every form of implementation
//! fills (the engine's own, a process Plugin through
//! [`plugin_host`](crate::plugin_host), and a program's SDK handler), and a
//! driver above them that does what they all share. The driver validates
//! the input, waits for the answer under the call's deadline, checks the
//! answer's shape and reads its failure into one typed [`Failure`]; an
//! implementation answers for its narrow primitive alone.
//!
//! The engine classes every call itself, from the kind's method and what
//! it carries, never from the implementation's word: [`Primitive`] is the
//! table of each primitive's class.
//!
//! A failure an implementation answered is checked like any decoded input
//! (ADR 0127): a code the catalog knows is kept with its message and
//! details; an unknown code, or an answer that cannot be read or whose
//! shape the driver refuses, becomes the kind's `<kind>_handler_failed`.
//! Its text is logged at `warn` and kept out of the error.
//!
//! The driver accounts for every call it makes, whatever its form and
//! however it ends (ADR 0033), through the [`Accounts`] of the Run it is
//! for: one `plugin_calls` charged before the call goes, its `plugin_time`
//! once it ended, both counted under the implementation's name, and one
//! line of the Run trace by implementation and method with the class the
//! engine gave the call ([`PluginCall`]).
//!
//! [`extractor`] holds the Extractor's primitives and driver, and
//! [`run_plugins`] the process Plugins one Run starts.

use std::time::Duration;

use futures_util::future::BoxFuture;
use goodfetch_protocol::catalog::{OperationClass, PluginKind};
use goodfetch_protocol::entries;
use goodfetch_protocol::plugin::extractor::{EXTRACT_CONTENT, EXTRACTOR};
use goodfetch_protocol::plugin::{PLUGIN_CRASHED, PLUGIN_QUARANTINED};
use serde::Serialize;

use crate::call_error::CallError;
use crate::plugin_host::instance::CallFailure;

pub mod extractor;
pub mod run_plugins;

/// The wire name of a crawl handler's failure, `handler_failed` (ADR 0093),
/// which the decoder gives a failure that names no code. The protocol's
/// `foxhound` module, which defines it, is Foxhound's alone (ADR 0099).
const CRAWL_HANDLER_FAILED: &str = "handler_failed";

/// What a kind's driver accounts each call to: the Run the call is for,
/// whose Usage the call is charged to and whose Run trace records it.
pub trait Accounts: Send + Sync {
    /// Charges one `plugin_calls` for `call` before it goes.
    ///
    /// # Errors
    ///
    /// The error the call fails with unmade: `limit_reached` when a Limit
    /// holding the Run has nothing left.
    fn call_starts<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>>;

    /// Charges the `plugin_time` of `call`, which ended, and records it in
    /// the Run trace.
    ///
    /// # Errors
    ///
    /// The error the call fails with although it was made: `limit_reached`
    /// when a Limit holding the Run has nothing left of the time.
    fn call_ended<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>>;
}

/// One call into an implementation, as its driver accounts it and the Run
/// trace's `trace/plugin-calls.jsonl` holds it: by implementation and
/// method, with the class the engine gave it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginCall<'call> {
    /// The implementation's name, as the setting that chose it names it,
    /// under which Usage counts it.
    pub plugin: &'call str,
    /// The kind's name.
    pub kind: &'static str,
    /// The primitive's method.
    pub method: &'static str,
    /// The class the engine gave the call, from its method ([`Primitive`]).
    #[serde(serialize_with = "class_name")]
    pub class: OperationClass,
    /// How long it took, in whole milliseconds, rounded up: zero before it
    /// went.
    pub time: u64,
    /// The Error code it failed with, when it failed.
    pub failed: Option<&'static str>,
}

impl<'call> PluginCall<'call> {
    /// The call of `primitive` into the implementation `plugin`, before it
    /// goes.
    pub fn of(plugin: &'call str, primitive: Primitive) -> Self {
        Self {
            plugin,
            kind: primitive.kind().name,
            method: primitive.method(),
            class: primitive.class(),
            time: 0,
            failed: None,
        }
    }

    /// The call, once it ended after `took`, with `failed` the Error code
    /// it failed with, if it failed.
    #[must_use]
    pub fn ended(self, took: Duration, failed: Option<&'static str>) -> Self {
        Self {
            time: u64::try_from(took.as_micros().div_ceil(1_000)).unwrap_or(u64::MAX),
            failed,
            ..self
        }
    }
}

/// `class`, as its name.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `serialize_with` passes the field by reference"
)]
fn class_name<S: serde::Serializer>(
    class: &OperationClass,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(class.as_str())
}

/// A primitive of a Plugin kind, one method the engine calls an
/// implementation with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    /// The Extractor's `extract.content`.
    ExtractContent,
}

impl Primitive {
    /// Every primitive of Collie's kinds.
    pub const ALL: [Self; 1] = [Self::ExtractContent];

    /// Its kind.
    pub fn kind(self) -> &'static PluginKind {
        match self {
            Self::ExtractContent => &EXTRACTOR,
        }
    }

    /// Its method's name on the wire, with its kind's.
    pub fn method(self) -> &'static str {
        match self {
            Self::ExtractContent => EXTRACT_CONTENT,
        }
    }

    /// The class the engine gives each of its calls: reading a page's main
    /// content changes nothing.
    pub fn class(self) -> OperationClass {
        match self {
            Self::ExtractContent => OperationClass::Read,
        }
    }
}

/// Why a call into an implementation failed, as its kind's driver reads
/// it (ADR 0129). Each variant holds the error the call fails with.
#[derive(Clone, Debug)]
pub enum Failure {
    /// The process Plugin crashed while it held the call: `plugin_crashed`,
    /// which a call under a lease retries.
    Crashed(CallError),
    /// The Plugin is quarantined for the rest of its run:
    /// `plugin_quarantined`.
    Quarantined(CallError),
    /// The call's deadline passed before its answer came, and the call was
    /// abandoned: `deadline_exceeded`, naming its setting (ADR 0080).
    DeadlineExceeded(CallError),
    /// The implementation answered an Error code the catalog knows, kept.
    Answered(CallError),
    /// An answer the driver cannot take: one that cannot be read, a shape
    /// it refuses or an unknown code, the kind's `<kind>_handler_failed`.
    Malformed(CallError),
    /// The instance carried no call out: it was refused at `hello`, its
    /// host cannot confine it, or it was stopped with its Run or its task.
    Ended(CallError),
    /// The Run's Usage refused the call ([`Accounts`]): `limit_reached`,
    /// before it went or for the time it took.
    Refused(CallError),
}

impl Failure {
    /// The failure of a call whose implementation ended it with `failure`,
    /// `handler_failed` the kind's `<kind>_handler_failed` for it. An
    /// unknown code is logged at `warn` with its text.
    ///
    /// A failure that names no code reaches the driver as `handler_failed`,
    /// which the decoder gives it: that code is a program's crawl handler's
    /// (ADR 0093), never an implementation's, so it is read as no code.
    pub fn of(failure: CallFailure, handler_failed: impl FnOnce() -> CallError) -> Self {
        match failure {
            CallFailure::Answered(answer) => {
                if let Some(definition) = entries::error_code(&answer.data.code)
                    .filter(|definition| definition.name != CRAWL_HANDLER_FAILED)
                {
                    Self::Answered(CallError::new(
                        definition,
                        answer.message,
                        &answer.data.details,
                    ))
                } else {
                    tracing::warn!(
                        code = answer.data.code,
                        message = answer.message,
                        "a Plugin answered with an Error code the catalog does not know"
                    );
                    Self::Malformed(handler_failed())
                }
            }
            CallFailure::Malformed(malformed) => {
                tracing::warn!(%malformed, "a Plugin's answer cannot be read");
                Self::Malformed(handler_failed())
            }
            CallFailure::Unavailable(error) => {
                if error.code() == PLUGIN_CRASHED.name {
                    Self::Crashed(error)
                } else if error.code() == PLUGIN_QUARANTINED.name {
                    Self::Quarantined(error)
                } else {
                    Self::Ended(error)
                }
            }
        }
    }

    /// The error the call fails with.
    pub fn error(&self) -> &CallError {
        match self {
            Self::Crashed(error)
            | Self::Quarantined(error)
            | Self::DeadlineExceeded(error)
            | Self::Answered(error)
            | Self::Malformed(error)
            | Self::Ended(error)
            | Self::Refused(error) => error,
        }
    }

    /// The error the call fails with, taken.
    pub fn into_error(self) -> CallError {
        match self {
            Self::Crashed(error)
            | Self::Quarantined(error)
            | Self::DeadlineExceeded(error)
            | Self::Answered(error)
            | Self::Malformed(error)
            | Self::Ended(error)
            | Self::Refused(error) => error,
        }
    }
}
