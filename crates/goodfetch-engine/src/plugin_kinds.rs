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
//! [`extractor`] holds the Extractor's primitives and driver, and
//! [`run_plugins`] the process Plugins one Run starts.

use goodfetch_protocol::catalog::{OperationClass, PluginKind};
use goodfetch_protocol::entries;
use goodfetch_protocol::foxhound::crawl::HANDLER_FAILED;
use goodfetch_protocol::plugin::extractor::{EXTRACT_CONTENT, EXTRACTOR};
use goodfetch_protocol::plugin::{PLUGIN_CRASHED, PLUGIN_QUARANTINED};

use crate::call_error::CallError;
use crate::plugin_host::instance::CallFailure;

pub mod extractor;
pub mod run_plugins;

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
                    .filter(|definition| definition.name != HANDLER_FAILED.name)
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
            | Self::Ended(error) => error,
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
            | Self::Ended(error) => error,
        }
    }
}
