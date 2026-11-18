//! Declared Effects (ADR 0106): a program wraps a block that changes the
//! world outside, such as placing an order, in `run.effect(key, body)`,
//! and the Run's Effect journal keeps what the block returned under its
//! key, so a re-run or a resume answers from the journal instead of doing
//! it again. The SDKs carry a declaration in two calls only they make:
//! `effect.open` writes the Effect's entry before its body starts and
//! answers `perform` or the journal's answer, and `effect.close` records
//! how the body ended, judged by whether the engine sent anything for it.
//! A body that sent something and did not return a value leaves its
//! Effect uncertain unless it is repeatable: never run again, and raised
//! as `effect_uncertain` until a person reconciles it (ADR 0029).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, SharedType, SharedTypeValue, Stability, ValueKind, schema_of,
};
use crate::run::{PointInTime, RunId, UncertainEffect};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most Unicode code points an Effect key holds.
pub const KEY_MAX_CODE_POINTS: usize = 255;

/// What `effect.open` takes: the Run, the Effect key and whether its body
/// is safe to run again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectOpenParams {
    /// The Run whose Effect journal keeps the Effect.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Effect key, chosen by the program: 1 to 255 Unicode code
    /// points. Its namespace is the crawl request in a handler and the Run
    /// elsewhere.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Whether the body is safe to run again: a body that sent something
    /// and did not return then runs again at its next declaration instead
    /// of leaving its Effect uncertain. `false` by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub repeatable: bool,
    /// The crawl attempt whose handler's `ctx.run` declares the Effect
    /// (ADR 0093), whose crawl request is the key's namespace. Absent, the
    /// key's namespace is the Run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: Option<EffectAttempt>,
}

/// The crawl attempt an Effect is declared through: a handler's `ctx.run`,
/// which is bound to its attempt (ADR 0093).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectAttempt {
    /// The ID of the crawl request whose attempt declares the Effect.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
}

/// What `effect.open` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum EffectOpened {
    /// The Effect journal holds no answer: run the body, then
    /// `effect.close` the entry.
    Perform {
        /// The Effect journal entry, which `effect.close` names.
        #[schemars(extend("arrived_in" = "0.1"))]
        entry: String,
        /// An opaque key, the same on every attempt and resume of this
        /// Effect on any machine and different in every Run and install,
        /// for the body to send as a site's `Idempotency-Key`.
        #[schemars(extend("arrived_in" = "0.1"))]
        idempotency_key: String,
    },
    /// An earlier attempt or start performed the Effect: the body does not
    /// run, and the output it returned then is the answer.
    Journal {
        /// What the body returned, or absent when it returned nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        output: Option<Value>,
        /// When it was performed.
        #[schemars(extend("arrived_in" = "0.1"))]
        performed_at: PointInTime,
    },
    /// A person answered "it happened" (ADR 0029): the body does not run,
    /// and no output is known.
    Reconciled {
        /// Who answered.
        #[schemars(extend("arrived_in" = "0.1"))]
        reconciled_by: String,
        /// When they answered.
        #[schemars(extend("arrived_in" = "0.1"))]
        reconciled_at: PointInTime,
    },
}

/// What `effect.close` takes: the entry `effect.open` answered and how its
/// body ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectCloseParams {
    /// The Run whose Effect journal keeps the Effect.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The entry `effect.open` answered with `perform`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entry: String,
    /// How the body ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub outcome: EffectOutcome,
}

/// How an Effect's body ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "ended", rename_all = "snake_case")]
pub enum EffectOutcome {
    /// It returned a value, or nothing.
    Returned {
        /// What it returned: JSON up to `effect.output.max_size` bytes, or
        /// absent when it returned nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        output: Option<Value>,
    },
    /// It raised an error, which the SDK propagates, or wraps in
    /// `effect_uncertain` when the Effect is left uncertain.
    Threw,
    /// It was cut short: a cancellation or its attempt's end, whose error
    /// the SDK propagates unchanged (ADRs 0080 and 0093).
    CutShort,
}

/// What `effect.close` answers: how the engine judged the body by what it
/// sent for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "judged", rename_all = "snake_case")]
pub enum EffectClosed {
    /// The body returned a value the Effect journal keeps: it never runs
    /// again under its key, and the SDK answers `performed`.
    Done {
        /// When it was performed.
        #[schemars(extend("arrived_in" = "0.1"))]
        performed_at: PointInTime,
    },
    /// The body did not return, and nothing was sent or the Effect is
    /// repeatable: the entry is dropped, the SDK propagates the body's
    /// error, and the next declaration runs the body.
    Dropped {
        /// Whether something was sent for it.
        #[schemars(extend("arrived_in" = "0.1"))]
        sent: bool,
    },
    /// The body did not return a value the Effect journal keeps after
    /// something was sent, and the Effect is not repeatable: it never runs
    /// again until a person reconciles it, and every later declaration of
    /// its key raises `effect_uncertain`. The SDK raises `effect_uncertain`
    /// with the body's error as its cause, or propagates a cut-short
    /// body's error unchanged.
    Uncertain {
        /// The Effect left uncertain, as the Run report lists it.
        #[schemars(extend("arrived_in" = "0.1"))]
        effect: UncertainEffect,
        /// The Error code the body's value met, `effect_output_invalid`,
        /// when it returned one the Effect journal does not keep.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        cause: Option<String>,
    },
}

/// Why an Effect is uncertain: its `reason` in `effect_uncertain`'s
/// details and in the Run report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UncertainReason {
    /// Its body raised, or returned a value the Effect journal does not
    /// keep, after something was sent.
    FailedAfterSend,
    /// Its body was cut short after something was sent: a cancellation,
    /// its attempt's end, a crash, a lost lease or a dropped connection.
    Interrupted,
    /// The Effect journal may lack its entry, after a resume from a
    /// Checkpoint the Run may have gone on after.
    JournalIncomplete,
}

impl UncertainReason {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FailedAfterSend => "failed_after_send",
            Self::Interrupted => "interrupted",
            Self::JournalIncomplete => "journal_incomplete",
        }
    }

    /// The reason `name` names.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "failed_after_send" => Some(Self::FailedAfterSend),
            "interrupted" => Some(Self::Interrupted),
            "journal_incomplete" => Some(Self::JournalIncomplete),
            _ => None,
        }
    }
}

/// Declares an Effect: writes its entry, or answers from the journal.
pub const EFFECT_OPEN: Operation = Operation {
    name: "effect.open",
    description: "Declares an Effect (ADR 0106): writes its entry in the Run's Effect journal before its body starts and answers `perform` with the entry and an `idempotency_key`, or the journal's answer, `journal` with the output an earlier attempt or start returned or `reconciled` after a person's \"it happened\", when the body must not run. An uncertain Effect raises `effect_uncertain`; a key declared again in one attempt while its body runs or after its answer raises `effect_key_reused`; across attempts of one namespace, the call waits until a body still running under its key is judged. Called by the SDKs' `run.effect` only.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<EffectOpenParams>,
    result: schema_of::<EffectOpened>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Records how an Effect's body ended.
pub const EFFECT_CLOSE: Operation = Operation {
    name: "effect.close",
    description: "Records how an Effect's body ended (ADR 0106), judged by whether the engine sent anything for it: a body that returned a value the Effect journal keeps is `done` and never runs again; one that did not, with nothing sent or repeatable, is `dropped`, and runs at its next declaration; one that sent something and is not repeatable is `uncertain` until a person reconciles it. A returned value larger than `effect.output.max_size` with nothing sent raises `effect_output_invalid`. Called by the SDKs' `run.effect` only.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<EffectCloseParams>,
    result: schema_of::<EffectClosed>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// An Effect key out of its bounds.
pub const EFFECT_KEY_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "effect_key_invalid",
    description: "An Effect key is empty or longer than 255 Unicode code points (ADR 0106). The details name the key.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An Effect key declared again in one attempt.
pub const EFFECT_KEY_REUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "effect_key_reused",
    description: "An Effect key was declared again in one attempt while its body runs, or after this attempt got its answer (ADR 0106): a loop that left its varying part out of the key would otherwise perform one Effect for many. Put what varies in the key. The details name the key.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A body's value the Effect journal does not keep, with nothing sent.
pub const EFFECT_OUTPUT_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "effect_output_invalid",
    description: "An Effect's body returned a value that is not JSON or is larger than `effect.output.max_size`, when nothing was sent for it (ADR 0106); the entry is dropped. After a send it is the cause of `effect_uncertain`. The details name the key, the value's size and the limit.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An Effect that may or may not have happened (ADR 0091's code).
pub const EFFECT_UNCERTAIN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "effect_uncertain",
    description: "An Effect may or may not have happened (ADRs 0091 and 0106): its body sent something and did not return a value the Effect journal keeps, so the engine never runs it again until a person reconciles it (\"it happened\", \"it did not happen\" or \"leave uncertain\"). The details name the Effect journal `entry`, the `key`, the `crawl_request`, the `reason` (`failed_after_send`, `interrupted` or `journal_incomplete`), the `first_command` sent and `since` when.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

fn sixty_four_kibibytes() -> Value {
    Value::from(65_536)
}

/// `effect.output.max_size`: the most bytes of a body's value the Effect
/// journal keeps.
pub const EFFECT_OUTPUT_MAX_SIZE: Setting = Setting {
    name: "effect.output.max_size",
    description: "The most bytes of JSON an Effect's body may return for the Effect journal to keep (ADR 0106), 64 KiB by default as `crawl.payload.max_size`. A larger value is `effect_output_invalid` when nothing was sent, and leaves the Effect uncertain after a send.",
    kind: ValueKind::Count,
    default: sixty_four_kibibytes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// The Audit log's entry of a person's answer about an uncertain Effect.
pub const EFFECT_RECONCILED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "effect_reconciled",
    description: "A person answered about an uncertain Effect (ADRs 0029 and 0106): \"it happened\", \"it did not happen\" or \"leave uncertain\"; `details` give the Run, the Effect journal `entry`, the `key`, the `crawl_request` and the `answer`.",
    arrived_in: ARRIVED_IN,
};

/// The Effect journal's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[EFFECT_OPEN, EFFECT_CLOSE],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        EFFECT_KEY_INVALID,
        EFFECT_KEY_REUSED,
        EFFECT_OUTPUT_INVALID,
        EFFECT_UNCERTAIN,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[EFFECT_OUTPUT_MAX_SIZE],
    hook_stages: &[],
    shared_type_values: &[EFFECT_RECONCILED],
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{EffectClosed, EffectOpened, EffectOutcome};
    use crate::run::PointInTime;

    /// The answers are tagged by what they are, so an SDK narrows them by
    /// one field (ADR 0106's `source`).
    ///
    /// Covers: operation:effect.open, operation:effect.close
    #[test]
    fn the_answers_are_tagged() {
        let at = PointInTime::from_rfc3339("2026-11-17T00:00:00Z".to_owned());
        assert_eq!(
            serde_json::to_value(EffectOpened::Journal {
                output: Some(json!({"order": 1})),
                performed_at: at.clone(),
            })
            .unwrap(),
            json!({"answer": "journal", "output": {"order": 1}, "performed_at": "2026-11-17T00:00:00Z"})
        );
        assert_eq!(
            serde_json::to_value(EffectClosed::Dropped { sent: false }).unwrap(),
            json!({"judged": "dropped", "sent": false})
        );
        assert_eq!(
            serde_json::from_value::<EffectOutcome>(json!({"ended": "returned"})).unwrap(),
            EffectOutcome::Returned { output: None }
        );
    }
}
