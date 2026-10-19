//! The oracle (spec #152): judges a submitted result for a task against the
//! task's expected result, and answers `verified`, `incomplete`,
//! `false_success` or `failed` with what was missing, extra or wrong, the task
//! it judged and its own time. Effects are judged from the Proving ground's
//! own record of what it carried out, never from the tool's report: an
//! Effect carried out twice is an extra one. The tool's status is recorded, never what
//! makes a result verified: a correct `needs-review` result is verified
//! (ADR 0013), and only a result the tool calls a success can be a false
//! success. What else the Proving ground's own record refutes, such as a
//! fingerprint that changed between requests or an escape a page's script
//! reported, is a finding, and a result with a finding is never verified.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::clock::Stopwatch;
use crate::task::{Effect, Expected, Record};

/// A tool's result for one task, as submitted to `POST /oracle`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submission {
    /// The task's ID.
    pub(crate) task: String,
    status: ToolStatus,
    /// The Records the tool returned, each with its fields by name.
    #[serde(default)]
    records: Vec<Value>,
    /// The Access dimensions the result was run under, which the answer
    /// repeats so results are reported per each.
    #[serde(default)]
    access: Option<AccessDimensions>,
}

/// The dimensions an Access result is reported per (ADR 0035): the
/// Fingerprint method, the browser engine, the Access mode and the build,
/// whose age the Proving ground reckons; and, for a control run of stock
/// Chrome, which control it is.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccessDimensions {
    fingerprint_method: FingerprintMethod,
    engine: BrowserEngine,
    access_mode: AccessMode,
    /// The major version of the browser build that ran.
    pub(crate) browser_major: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    control: Option<Control>,
}

/// How a Session's browser comes to present its Identity profile (ADR 0035).
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum FingerprintMethod {
    None,
    JsInjection,
    Basic,
    Engine,
}

/// The engine of the browser that ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserEngine {
    Chromium,
    Firefox,
    Webkit,
}

/// How a Session presents itself to sites.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AccessMode {
    Standard,
    Identified,
    Stealth,
}

/// A control run every Access result is set against.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Control {
    StockHeadless,
    StockHeadful,
}

impl AccessDimensions {
    /// The Chrome major of a Chromium build, from which its age is reckoned;
    /// `None` for another engine.
    pub(crate) fn chromium_major(&self) -> Option<u32> {
        (self.engine == BrowserEngine::Chromium).then_some(self.browser_major)
    }
}

impl Submission {
    /// The `id` of each submitted Record that has one, in order.
    pub(crate) fn record_ids(&self) -> Vec<String> {
        self.records
            .iter()
            .filter_map(|record| record.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }
}

/// What the tool said of its own result.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ToolStatus {
    Success,
    /// Done, with its success left to a person's review (ADR 0013).
    NeedsReview,
    Failure,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// Correct and complete.
    Verified,
    /// Correct as far as it goes, with Records missing, from a tool that did
    /// not claim success.
    Incomplete,
    /// Claimed a success the expected result refutes.
    FalseSuccess,
    /// Wrong or extra data, or nothing correct, from a tool that did not
    /// claim success.
    Failed,
}

/// The oracle's answer.
#[derive(Serialize)]
pub(crate) struct Answer {
    task: String,
    outcome: Outcome,
    tool_status: ToolStatus,
    /// The IDs of expected Records the result lacks.
    missing: Vec<String>,
    /// Submitted Records with no expected Record, or one already submitted.
    extra: Vec<Value>,
    wrong: Vec<Wrong>,
    /// Expected Effects the Proving ground did not carry out.
    missing_effects: Vec<Effect>,
    /// Effects it carried out beyond the expected ones: another, or one
    /// again.
    extra_effects: Vec<Effect>,
    /// What the Proving ground's own record refutes beside the Records and
    /// Effects.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    findings: Vec<Finding>,
    /// The Access dimensions the submission named.
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<AccessDimensions>,
    oracle_time_microseconds: u64,
}

/// Something the Proving ground's own record refutes: a fingerprint that
/// changed between a task's requests, a probe's signal, an escape or a call
/// to the Daemon a page's script reported, a Record of a page no request
/// cleared.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Finding {
    /// What was checked, such as `ja4` or `escape`.
    pub(crate) check: String,
    /// The page or request target it was found on.
    pub(crate) at: String,
    /// What the check expected, when it compares with a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expected: Option<Value>,
    pub(crate) observed: Value,
}

impl Answer {
    /// The Access dimensions the submission named, if any.
    pub(crate) fn access(&self) -> Option<&AccessDimensions> {
        self.access.as_ref()
    }

    /// The answer with `findings` added: a result they refute is no longer
    /// verified nor incomplete, but a false success when the tool claimed a
    /// success and failed otherwise.
    #[must_use]
    pub(crate) fn refuted_by(mut self, findings: Vec<Finding>) -> Self {
        if !findings.is_empty() && matches!(self.outcome, Outcome::Verified | Outcome::Incomplete) {
            self.outcome = if self.tool_status == ToolStatus::Success {
                Outcome::FalseSuccess
            } else {
                Outcome::Failed
            };
        }
        self.findings.extend(findings);
        self
    }
}

/// A field of a Record whose submitted value is not the expected one.
#[derive(Serialize)]
struct Wrong {
    id: String,
    field: &'static str,
    expected: String,
    /// `null` when the field is absent.
    submitted: Value,
}

/// Judges `submission` against `expected`, and the Effects carried out for
/// its task, `effects`, against the expected ones.
pub(crate) fn judge(expected: &Expected, submission: Submission, effects: &[Effect]) -> Answer {
    let stopwatch = Stopwatch::start();
    let by_id: BTreeMap<&str, &Record> = expected
        .records
        .iter()
        .map(|record| (record["id"].as_str(), record))
        .collect();
    let mut seen = BTreeSet::new();
    let mut extra = Vec::new();
    let mut wrong = Vec::new();
    for record in submission.records {
        let matched = record
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| by_id.get_key_value(id))
            .filter(|(id, _)| seen.insert(**id));
        let Some((id, expected)) = matched else {
            extra.push(record);
            continue;
        };
        for (field, value) in expected.iter().filter(|(field, _)| **field != "id") {
            let submitted = record.get(*field);
            if !same_value(value, submitted) {
                wrong.push(Wrong {
                    id: (*id).to_owned(),
                    field,
                    expected: value.clone(),
                    submitted: submitted.cloned().unwrap_or(Value::Null),
                });
            }
        }
    }
    let missing: Vec<String> = by_id
        .keys()
        .filter(|id| !seen.contains(*id))
        .map(|id| (*id).to_owned())
        .collect();
    let mut missing_effects = expected.effects.clone();
    let mut extra_effects = Vec::new();
    for effect in effects {
        match missing_effects
            .iter()
            .position(|expected| expected == effect)
        {
            Some(at) => {
                missing_effects.remove(at);
            }
            None => extra_effects.push(effect.clone()),
        }
    }
    let carried_out = effects.len() > extra_effects.len();
    let outcome = if missing.is_empty()
        && extra.is_empty()
        && wrong.is_empty()
        && missing_effects.is_empty()
        && extra_effects.is_empty()
    {
        Outcome::Verified
    } else if submission.status == ToolStatus::Success {
        Outcome::FalseSuccess
    } else if extra.is_empty()
        && wrong.is_empty()
        && extra_effects.is_empty()
        && (!seen.is_empty() || carried_out)
    {
        Outcome::Incomplete
    } else {
        Outcome::Failed
    };
    Answer {
        task: submission.task,
        outcome,
        tool_status: submission.status,
        missing,
        extra,
        wrong,
        missing_effects,
        extra_effects,
        findings: Vec::new(),
        access: submission.access,
        oracle_time_microseconds: stopwatch.microseconds(),
    }
}

/// Whether a submitted value is the expected one: the same text, or a number
/// of the same decimal value (`12.3` for `"12.30"`).
fn same_value(expected: &str, submitted: Option<&Value>) -> bool {
    match submitted {
        Some(Value::String(text)) => text == expected,
        Some(Value::Number(number)) => {
            decimal(expected).is_some_and(|value| decimal(&number.to_string()) == Some(value))
        }
        _ => false,
    }
}

/// A plain decimal's whole and fractional digits, without leading or
/// trailing zeros; `None` for anything else.
fn decimal(text: &str) -> Option<(&str, &str)> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    (!whole.is_empty() && digits(whole) && digits(fraction)).then(|| {
        (
            whole.trim_start_matches('0'),
            fraction.trim_end_matches('0'),
        )
    })
}
