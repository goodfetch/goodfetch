//! The oracle (spec #152): judges a submitted result for a task against the
//! task's expected result, and answers `verified`, `incomplete`,
//! `false_success` or `failed` with what was missing, extra or wrong, the task
//! it judged and its own time. Effects are judged from the Proving ground's
//! own record of what it carried out, never from the tool's report: an
//! Effect carried out twice is an extra one. So are a Crawl's findings (a page
//! fetched twice, a trap followed, a honeypot hit, a `Retry-After` ignored):
//! a task with one is never verified. The tool's status is recorded, never what
//! makes a result verified: a correct `needs-review` result is verified
//! (ADR 0013), and only a result the tool calls a success can be a false
//! success. A downloaded file is judged by its name and exact bytes. Where the
//! only right result is a refusal, the tool's report of a failure is part of
//! it: nothing carried out and a success reported is a false success, unless
//! the Records it reports are complete, which only outwaiting the task's
//! incident gives: data delivered late, a failure. Where the refusal names an
//! error, the tool's must be it. The Proving ground's own observations of the
//! task's requests judge what the tool fetched: a path it must not request,
//! and the Web Bot Auth signatures it must or must not send.
//! What else the Proving ground's own record refutes, such as a
//! fingerprint that changed between requests or an escape a page's script
//! reported, is a finding, and a result with a finding is never verified.

use crate::interface::ToolStatus;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::clock::Stopwatch;
use crate::digest;
use crate::html;
use crate::markdown;
use crate::observation::Seen;
use crate::robots;
use crate::task::{self, Effect, Expected, ExpectedError, Naming, Record};

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
    /// The files the tool downloaded.
    #[serde(default)]
    files: Vec<SubmittedFile>,
    /// The Access dimensions the result was run under, which the answer
    /// repeats so results are reported per each.
    #[serde(default)]
    access: Option<AccessDimensions>,
    /// The error a failing tool reports.
    #[serde(default)]
    error: Option<SubmittedError>,
}

/// A failing tool's error: its Error code and, for a navigation that
/// failed, the browser's network error.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SubmittedError {
    code: String,
    #[serde(default)]
    network_error: Option<String>,
    /// The rule an egress refusal names, such as `private_address`.
    #[serde(default)]
    rule: Option<String>,
    /// The engine setting a `response_limit_exceeded` names, such as
    /// `network.max_decompression_ratio`.
    #[serde(default)]
    setting: Option<String>,
}

/// A file a tool returns: its name and its bytes in lowercase or uppercase
/// hex.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmittedFile {
    name: String,
    hex: String,
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
    /// The names of expected files the result lacks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing_files: Vec<String>,
    /// Submitted files whose bytes are not the expected ones.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    wrong_files: Vec<WrongFile>,
    /// The names of submitted files with no expected file, or one already
    /// submitted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extra_files: Vec<String>,
    /// The names of submitted files saved under a name the page suggested
    /// that is not one safe file name, such as a path.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unsafe_files: Vec<String>,
    /// Complete Records reported as a success where the task's incident made
    /// a failure the only right result: data delivered late, not a false
    /// success.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    late: bool,
    /// A failure whose error is not the one the refusal names.
    #[serde(skip_serializing_if = "Option::is_none")]
    wrong_error: Option<WrongError>,
    /// The targets of the task's requests to a path it must not request.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    forbidden_requests: Vec<String>,
    /// The task's requests whose Web Bot Auth signature is not as the task
    /// requires.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    signature_faults: Vec<SignatureFault>,
    /// What the Proving ground's own record refutes beside the Records,
    /// Effects and files: what it saw the tool do wrong on the task's host,
    /// and the checks of what it observed that failed.
    findings: Vec<Refutation>,
    quality: Quality,
    /// The Access dimensions the submission named.
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<AccessDimensions>,
    oracle_time_microseconds: u64,
}

/// The error a refusal names, and the one the tool reported.
#[derive(Serialize)]
struct WrongError {
    expected: ExpectedError,
    submitted: Option<SubmittedError>,
}

/// A request whose signature is not as its task requires.
#[derive(Serialize)]
struct SignatureFault {
    target: String,
    fault: Fault,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Fault {
    /// Not signed, where every request must be.
    Unsigned,
    /// Signed, but its signature was refused against the agent's directory.
    Refused,
    /// Signed, where no request may be.
    Signed,
}

/// A finding of the oracle's answer: a Crawl's misstep the family that
/// served the request recorded (named by its `kind`), or a check of what
/// the Proving ground observed that failed (named by its `check`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Refutation {
    /// A Crawl's misstep: a page fetched twice, a trap followed, a honeypot
    /// hit, a `Retry-After` ignored, a request past the in-flight cap.
    Crawl(task::Finding),
    /// A check of the Proving ground's own record that failed.
    Check(Finding),
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
        self.findings
            .extend(findings.into_iter().map(Refutation::Check));
        self
    }
}

/// Extraction quality (pain 14), scored field by field against the ground
/// truth: of each field's expected values, one per expected Record, how
/// many the result holds. A missing Record's fields are missing values, and
/// an extra Record scores nothing.
#[derive(Default, Serialize)]
struct Quality {
    fields: BTreeMap<&'static str, Score>,
    expected: u64,
    correct: u64,
}

#[derive(Default, Serialize)]
struct Score {
    expected: u64,
    correct: u64,
}

impl Quality {
    fn count(&mut self, field: &'static str, correct: bool) {
        let score = self.fields.entry(field).or_default();
        score.expected += 1;
        self.expected += 1;
        if correct {
            score.correct += 1;
            self.correct += 1;
        }
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
    /// The first character that differs, when the submitted value is text.
    #[serde(skip_serializing_if = "Option::is_none")]
    difference: Option<Difference>,
}

/// The first character at which a submitted text differs from the expected
/// one, both held in UTF-8 in the normalization form served (ADR 0073), so
/// a text read in another encoding or normalized names the character that
/// changed.
#[derive(Serialize)]
struct Difference {
    /// The character's index in the expected text, counted in Unicode
    /// scalar values.
    at: usize,
    /// The expected character there as `U+XXXX`; `null` past the expected
    /// text's end.
    expected: Option<String>,
    /// The submitted character there; `null` past the submitted text's end.
    submitted: Option<String>,
}

impl Difference {
    /// Where `submitted` first differs from `expected`.
    fn between(expected: &str, submitted: &str) -> Self {
        let mut expected = expected.chars();
        let mut submitted = submitted.chars();
        let mut at = 0;
        loop {
            match (expected.next(), submitted.next()) {
                (Some(left), Some(right)) if left == right => at += 1,
                (left, right) => {
                    let name = |character: char| format!("U+{:04X}", u32::from(character));
                    return Self {
                        at,
                        expected: left.map(name),
                        submitted: right.map(name),
                    };
                }
            }
        }
    }
}

/// A submitted file whose bytes are not the expected ones.
#[derive(Serialize)]
struct WrongFile {
    name: String,
    expected_sha256: String,
    /// `null` when its `hex` is not hex.
    submitted_sha256: Option<String>,
}

/// What the submitted Records hold against the expected ones.
struct Records {
    missing: Vec<String>,
    extra: Vec<Value>,
    wrong: Vec<Wrong>,
    quality: Quality,
    /// Whether any submitted Record is an expected one.
    any_correct: bool,
}

/// What the Effects carried out hold against the expected ones.
struct Effects {
    missing: Vec<Effect>,
    extra: Vec<Effect>,
    /// Whether any Effect carried out is an expected one.
    any_expected: bool,
}

/// What the submitted files hold against the expected ones.
struct Files {
    missing: Vec<String>,
    wrong: Vec<WrongFile>,
    extra: Vec<String>,
    /// Submitted files saved under an unsafe name a page suggested.
    unsafe_names: Vec<String>,
    /// Whether any submitted file is an expected one with its bytes.
    any_correct: bool,
}

/// Judges `submission` against `expected`, and the Effects carried out for
/// its task, `effects`, against the expected ones; any of the task's
/// `findings` refutes it. A task whose expected result is a refusal is
/// verified only when nothing was carried out and the tool reported a
/// failure (the one named, if any), and the task's `requests`, as observed,
/// are judged against its forbidden paths and signatures.
pub(crate) fn judge(
    expected: &Expected,
    submission: Submission,
    effects: &[Effect],
    requests: &[Seen],
    findings: Vec<task::Finding>,
) -> Answer {
    let stopwatch = Stopwatch::start();
    let records = judge_records(expected, submission.records);
    let effects = judge_effects(expected, effects);
    let files = judge_files(expected, &submission.files);
    let forbidden_requests: Vec<String> = requests
        .iter()
        .filter(|request| {
            request.target != "/robots.txt"
                && (expected
                    .forbidden_paths
                    .iter()
                    .any(|path| request.target.starts_with(path))
                    || expected.robots_txt.is_some_and(|robots| {
                        !robots::allows(
                            robots,
                            request.fingerprint.user_agent.as_deref(),
                            &request.target,
                        )
                    }))
        })
        .map(|request| request.target.clone())
        .collect();
    let signature_faults = judge_signatures(expected.signed, requests);
    let wrong_error = expected.error.as_ref().and_then(|named| {
        let matches = submission.error.as_ref().is_some_and(|error| {
            error.code == named.code
                && named
                    .network_error
                    .is_none_or(|network| error.network_error.as_deref() == Some(network))
                && named
                    .rule
                    .is_none_or(|rule| error.rule.as_deref() == Some(rule))
                && named
                    .setting
                    .is_none_or(|setting| error.setting.as_deref() == Some(setting))
        });
        (!matches && submission.status == ToolStatus::Failure).then(|| WrongError {
            expected: named.clone(),
            submitted: submission.error.clone(),
        })
    });
    let as_fetched = forbidden_requests.is_empty() && signature_faults.is_empty();
    let correct = records.missing.is_empty()
        && effects.missing.is_empty()
        && files.missing.is_empty()
        && findings.is_empty()
        && nothing_wrong(&records, &effects, &files)
        && as_fetched;
    // A refusal's expected Records are those only outwaiting its incident
    // gives: their absence is no fault, their whole presence is late.
    let refused_cleanly = effects.missing.is_empty()
        && files.missing.is_empty()
        && findings.is_empty()
        && nothing_wrong(&records, &effects, &files)
        && as_fetched
        && wrong_error.is_none();
    let late = expected.refusal
        && submission.status == ToolStatus::Success
        && !expected.records.is_empty()
        && records.missing.is_empty()
        && nothing_wrong(&records, &effects, &files);
    let outcome = if expected.refusal {
        match submission.status {
            ToolStatus::Failure if refused_cleanly => Outcome::Verified,
            ToolStatus::Success if !late => Outcome::FalseSuccess,
            ToolStatus::Success | ToolStatus::Failure | ToolStatus::NeedsReview => Outcome::Failed,
        }
    } else if correct {
        Outcome::Verified
    } else if submission.status == ToolStatus::Success {
        Outcome::FalseSuccess
    } else if findings.is_empty()
        && nothing_wrong(&records, &effects, &files)
        && as_fetched
        && (records.any_correct || effects.any_expected || files.any_correct)
    {
        Outcome::Incomplete
    } else {
        Outcome::Failed
    };
    Answer {
        task: submission.task,
        outcome,
        tool_status: submission.status,
        missing: records.missing,
        extra: records.extra,
        wrong: records.wrong,
        missing_effects: effects.missing,
        extra_effects: effects.extra,
        missing_files: files.missing,
        wrong_files: files.wrong,
        extra_files: files.extra,
        unsafe_files: files.unsafe_names,
        late,
        wrong_error,
        forbidden_requests,
        signature_faults,
        findings: findings.into_iter().map(Refutation::Crawl).collect(),
        quality: records.quality,
        access: submission.access,
        oracle_time_microseconds: stopwatch.microseconds(),
    }
}

/// The requests whose signature is not as `signed` requires: each one
/// signed and verified, or none signed.
fn judge_signatures(signed: Option<bool>, requests: &[Seen]) -> Vec<SignatureFault> {
    let Some(signed) = signed else {
        return Vec::new();
    };
    requests
        .iter()
        .filter_map(|request| {
            let fault = match (signed, request.signature) {
                (true, None) => Fault::Unsigned,
                (true, Some(false)) => Fault::Refused,
                (false, Some(_)) => Fault::Signed,
                (true, Some(true)) | (false, None) => return None,
            };
            Some(SignatureFault {
                target: request.target.clone(),
                fault,
            })
        })
        .collect()
}

/// Whether nothing submitted or carried out is extra or wrong.
fn nothing_wrong(records: &Records, effects: &Effects, files: &Files) -> bool {
    records.extra.is_empty()
        && records.wrong.is_empty()
        && effects.extra.is_empty()
        && files.wrong.is_empty()
        && files.extra.is_empty()
        && files.unsafe_names.is_empty()
}

/// Whether `name` is one file name that is safe on every OS (ADR 0067): no
/// path separator, drive or stream colon, control character or character
/// Windows forbids; not `.` or `..`; no trailing dot or space; not a name
/// Windows reserves, with or without an extension; at most 255 bytes.
fn is_safe_file_name(name: &str) -> bool {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let numbered = ["COM", "LPT"].iter().any(|prefix| {
        stem.len() == 4
            && stem
                .get(..3)
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            && stem.as_bytes()[3].is_ascii_digit()
    });
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|character| character.is_control() || "/\\:*?\"<>|".contains(character))
        && !numbered
        && !RESERVED
            .iter()
            .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Judges the submitted Records: each expected one once, a tolerated one at
/// most once, each with its expected fields.
fn judge_records(expected_task: &Expected, submitted: Vec<Value>) -> Records {
    let expected = expected_task;
    let by_id: BTreeMap<&str, &Record> = expected
        .records
        .iter()
        .chain(&expected.tolerated)
        .map(|record| (record["id"].as_str(), record))
        .collect();
    let mut seen = BTreeSet::new();
    let mut extra = Vec::new();
    let mut wrong = Vec::new();
    let mut quality = Quality::default();
    for record in submitted {
        let matched = record
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| by_id.get_key_value(id))
            .filter(|(id, _)| seen.insert(**id));
        let Some((id, expected)) = matched else {
            extra.push(record);
            continue;
        };
        quality.count("id", true);
        for (field, value) in expected.iter().filter(|(field, _)| **field != "id") {
            let submitted = record.get(*field);
            let format = expected_task
                .markdown_site
                .and_then(|site| Format::of(field).map(|format| (site, format)));
            let correct = match format {
                Some((site, format)) => submitted
                    .and_then(Value::as_str)
                    .is_some_and(|text| format.same(site, id, value, text)),
                None => same_value(value, submitted),
            };
            quality.count(field, correct);
            if !correct {
                wrong.push(Wrong {
                    id: (*id).to_owned(),
                    field,
                    expected: value.clone(),
                    submitted: submitted.cloned().unwrap_or(Value::Null),
                    difference: submitted
                        .and_then(Value::as_str)
                        .filter(|_| format.is_none())
                        .map(|text| Difference::between(value, text)),
                });
            }
        }
    }
    let missing: Vec<String> = expected
        .records
        .iter()
        .map(|record| record["id"].as_str())
        .filter(|id| !seen.contains(id))
        .map(str::to_owned)
        .collect();
    for record in missing.iter().map(|id| by_id[id.as_str()]) {
        for field in record.keys() {
            quality.count(field, false);
        }
    }
    Records {
        missing,
        extra,
        wrong,
        quality,
        any_correct: !seen.is_empty(),
    }
}

/// Judges the Effects carried out: each expected one once, nothing else.
fn judge_effects(expected: &Expected, effects: &[Effect]) -> Effects {
    let mut missing = expected.effects.clone();
    let mut extra = Vec::new();
    for effect in effects {
        match missing.iter().position(|expected| expected == effect) {
            Some(at) => {
                missing.remove(at);
            }
            None => extra.push(effect.clone()),
        }
    }
    Effects {
        any_expected: effects.len() > extra.len(),
        missing,
        extra,
    }
}

/// Judges the submitted files: each expected one once, by name, with its
/// exact bytes. A file whose page suggests a hostile name is the first
/// submitted file no other expected name claims, saved under any safe name.
fn judge_files(expected: &Expected, submitted: &[SubmittedFile]) -> Files {
    let mut seen = BTreeSet::new();
    let mut wrong = Vec::new();
    let mut extra = Vec::new();
    let mut unsafe_names = Vec::new();
    let mut any_correct = false;
    for file in submitted {
        let by_name = expected
            .files
            .iter()
            .find(|expected| expected.naming == Naming::Exact && expected.name == file.name);
        let Some(expected) = by_name
            .or_else(|| {
                expected.files.iter().find(|expected| {
                    expected.naming == Naming::Safe && !seen.contains(expected.name.as_str())
                })
            })
            .filter(|expected| seen.insert(expected.name.as_str()))
        else {
            extra.push(file.name.clone());
            continue;
        };
        if expected.naming == Naming::Safe && !is_safe_file_name(&file.name) {
            unsafe_names.push(file.name.clone());
        }
        let submitted_sha256 = from_hex(&file.hex).map(|bytes| digest::sha256(&bytes));
        if submitted_sha256.as_deref() == Some(expected.sha256.as_str()) {
            any_correct = true;
        } else {
            wrong.push(WrongFile {
                name: file.name.clone(),
                expected_sha256: expected.sha256.clone(),
                submitted_sha256,
            });
        }
    }
    let missing = expected
        .files
        .iter()
        .filter(|file| !seen.contains(file.name.as_str()))
        .map(|file| file.name.clone())
        .collect();
    Files {
        missing,
        wrong,
        extra,
        unsafe_names,
        any_correct,
    }
}

/// The bytes a text of hex digit pairs names; `None` for anything else.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    text.len()
        .is_multiple_of(2)
        .then(|| {
            (0..text.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
                .collect()
        })
        .flatten()
}

/// The field a task with a Markdown site judges as Markdown, and the
/// prefix of the others it does.
const MARKDOWN_FIELD: &str = "markdown";
const MARKDOWN_PREFIX: &str = "markdown_";
/// The prefix of the fields such a task judges as HTML.
const HTML_PREFIX: &str = "html_";
/// The field such a task judges as text.
const TEXT_FIELD: &str = "text";

/// How a task with a Markdown site judges one of its formats' fields.
#[derive(Clone, Copy)]
enum Format {
    Markdown,
    Html,
    Text,
}

impl Format {
    fn of(field: &str) -> Option<Self> {
        if field == MARKDOWN_FIELD || field.starts_with(MARKDOWN_PREFIX) {
            Some(Self::Markdown)
        } else if field.starts_with(HTML_PREFIX) {
            Some(Self::Html)
        } else if field == TEXT_FIELD {
            Some(Self::Text)
        } else {
            None
        }
    }

    /// Whether `submitted` is `expected` in this format, for the page `id`
    /// (its path) on `site`.
    fn same(self, site: &str, id: &str, expected: &str, submitted: &str) -> bool {
        match self {
            Self::Markdown => same_markdown_on(site, id, expected, submitted),
            Self::Html => html::same_html(expected, submitted),
            Self::Text => html::same_text(expected, submitted),
        }
    }
}

/// Whether `submitted` is the Markdown `expected` is, for the page `id` (its
/// path) on `site`.
fn same_markdown_on(site: &str, id: &str, expected: &str, submitted: &str) -> bool {
    Url::parse(&format!("http://{site}"))
        .and_then(|origin| origin.join(id))
        .is_ok_and(|page| markdown::same_markdown(expected, submitted, &page, site))
}

/// Whether a submitted value is the expected one: the same text, a number
/// of the same decimal value (`12.3` for `"12.30"`), a boolean written as
/// `"true"` or `"false"`, or an array or object equal to the JSON the
/// expected value is written as.
fn same_value(expected: &str, submitted: Option<&Value>) -> bool {
    match submitted {
        Some(Value::String(text)) => text == expected,
        Some(Value::Bool(flag)) => expected == if *flag { "true" } else { "false" },
        Some(Value::Number(number)) => {
            decimal(expected).is_some_and(|value| decimal(&number.to_string()) == Some(value))
        }
        Some(value @ (Value::Array(_) | Value::Object(_))) => {
            serde_json::from_str::<Value>(expected).is_ok_and(|expected| &expected == value)
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
