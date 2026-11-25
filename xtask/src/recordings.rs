//! The Model recordings check (ADR 0065), in the Quick suite: every
//! committed recording, a JSON file in a folder named `recordings/`, keeps
//! only the headers of the allowlist, carries no key in a query parameter,
//! leaves no request or organization ID as the provider wrote it, and holds
//! no string shaped like a provider key; and every cell of the matrix names
//! a committed recording or the reason it waits.
//!
//! The allowlist and the matrix are `test-support/fixtures/model-recordings.toml`,
//! whose types below fix the matrix's cells: a cell the file leaves out or
//! adds fails its parse. A waiting cell is reported as a note.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md), and xtask is a repository tool on no Shared path, whose regex compiles ADR 0140 does not count"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;
use std::sync::LazyLock;

use anyhow::{Context as _, Result};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::check::{self, Finding};
use crate::tree;

/// The allowlist and the matrix, from the repository's root.
const MATRIX: &str = "test-support/fixtures/model-recordings.toml";

/// The folder name that makes a JSON file a recording.
const FOLDER: &str = "recordings";

/// What the recorder writes in place of a request or an organization ID.
const PLACEHOLDERS: [&str; 2] = ["<request-id>", "<organization-id>"];

/// `xtask recordings`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a header outside the allowlist, a key query parameter, an
    /// unreplaced request or organization ID, a string shaped like a
    /// provider key, and a cell of the matrix whose recording is missing
    /// (Quick suite).
    Check,
}

pub(crate) fn run(command: &Command, root: &Path) -> Result<ExitCode> {
    match command {
        Command::Check => {
            let checked = check(root)?;
            check::note(&checked.waits);
            Ok(check::report(&checked.findings))
        }
    }
}

struct Checked {
    findings: Vec<Finding>,
    waits: Vec<Finding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    headers: Allowlist,
    #[serde(rename = "openai-compatible")]
    openai_compatible: OpenAiCompatible,
    #[serde(rename = "anthropic-messages")]
    anthropic_messages: DirectOnly,
    gemini: DirectOnly,
    decision: Decision,
}

/// The headers a recording keeps, in lowercase.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    request: Vec<String>,
    response: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAiCompatible {
    direct: DirectCells,
    #[serde(rename = "vercel-ai-gateway")]
    vercel_ai_gateway: GatewayCells,
    openrouter: GatewayCells,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectOnly {
    direct: DirectCells,
}

/// Each hosted Decision provider of ADR 0009.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    jev: DecisionCells,
    openai: DecisionCells,
}

/// A format's cases on its direct provider.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct DirectCells {
    text: DirectCell,
    tool_call_and_result: DirectCell,
    parallel_tool_calls: DirectCell,
    structured_output: DirectCell,
    text_stream: DirectCell,
    tool_call_stream: DirectCell,
    parallel_tool_calls_stream: DirectCell,
    structured_output_stream: DirectCell,
    usage: DirectCell,
    prompt_caching: DirectCell,
    search_grounding: DirectCell,
    image_input: DirectCell,
    errors: DirectCell,
}

/// The OpenAI-compatible format's cases through a gateway of ADR 0012.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct GatewayCells {
    text: Cell,
    tool_call: Cell,
    structured_output: Cell,
    error: Cell,
}

/// A hosted Decision provider's cases.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct DecisionCells {
    choice: Cell,
    abstention: Cell,
    low_confidence_choice: Cell,
    errors: Cell,
}

/// A direct provider's cell, which may be recorded through the gateway.
#[derive(Deserialize)]
#[serde(untagged)]
enum DirectCell {
    Recorded(DirectRecorded),
    Waits(Waiting),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectRecorded {
    recording: String,
    /// The gateway the recording went through, which counts for the direct
    /// provider (ADR 0076).
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "read so the file's `via` is checked against the one gateway that counts"
    )]
    via: Option<Via>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Via {
    VercelAiGateway,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Cell {
    Recorded(Recorded),
    Waits(Waiting),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recorded {
    recording: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Waiting {
    waits: Reason,
}

/// Why a cell has no recording yet.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
enum Reason {
    Recorder,
    Revenue,
    GeminiNative,
}

impl Reason {
    fn describe(self) -> &'static str {
        match self {
            Self::Recorder => "the recorder job (#1073), through the Vercel AI Gateway's free tier",
            Self::Revenue => "revenue (ADR 0076)",
            Self::GeminiNative => {
                "a recording path for Gemini's native format, which the gateway does not serve (ADR 0076)"
            }
        }
    }
}

/// A cell as the check reads it: its recording, or why it waits.
enum Entry<'a> {
    Recorded(&'a str),
    Waits(Reason),
}

impl DirectCell {
    fn entry(&self) -> Entry<'_> {
        match self {
            Self::Recorded(recorded) => Entry::Recorded(&recorded.recording),
            Self::Waits(waiting) => Entry::Waits(waiting.waits),
        }
    }
}

impl Cell {
    fn entry(&self) -> Entry<'_> {
        match self {
            Self::Recorded(recorded) => Entry::Recorded(&recorded.recording),
            Self::Waits(waiting) => Entry::Waits(waiting.waits),
        }
    }
}

impl DirectCells {
    fn entries(&self) -> [(&'static str, Entry<'_>); 13] {
        [
            ("text", self.text.entry()),
            ("tool-call-and-result", self.tool_call_and_result.entry()),
            ("parallel-tool-calls", self.parallel_tool_calls.entry()),
            ("structured-output", self.structured_output.entry()),
            ("text-stream", self.text_stream.entry()),
            ("tool-call-stream", self.tool_call_stream.entry()),
            (
                "parallel-tool-calls-stream",
                self.parallel_tool_calls_stream.entry(),
            ),
            (
                "structured-output-stream",
                self.structured_output_stream.entry(),
            ),
            ("usage", self.usage.entry()),
            ("prompt-caching", self.prompt_caching.entry()),
            ("search-grounding", self.search_grounding.entry()),
            ("image-input", self.image_input.entry()),
            ("errors", self.errors.entry()),
        ]
    }
}

impl GatewayCells {
    fn entries(&self) -> [(&'static str, Entry<'_>); 4] {
        [
            ("text", self.text.entry()),
            ("tool-call", self.tool_call.entry()),
            ("structured-output", self.structured_output.entry()),
            ("error", self.error.entry()),
        ]
    }
}

impl DecisionCells {
    fn entries(&self) -> [(&'static str, Entry<'_>); 4] {
        [
            ("choice", self.choice.entry()),
            ("abstention", self.abstention.entry()),
            ("low-confidence-choice", self.low_confidence_choice.entry()),
            ("errors", self.errors.entry()),
        ]
    }
}

impl File {
    /// Every cell, by its table and case.
    fn cells(&self) -> Vec<(&'static str, &'static str, Entry<'_>)> {
        let openai = &self.openai_compatible;
        let tables: [(&'static str, Vec<(&'static str, Entry<'_>)>); 7] = [
            ("openai-compatible.direct", openai.direct.entries().into()),
            (
                "openai-compatible.vercel-ai-gateway",
                openai.vercel_ai_gateway.entries().into(),
            ),
            (
                "openai-compatible.openrouter",
                openai.openrouter.entries().into(),
            ),
            (
                "anthropic-messages.direct",
                self.anthropic_messages.direct.entries().into(),
            ),
            ("gemini.direct", self.gemini.direct.entries().into()),
            ("decision.jev", self.decision.jev.entries().into()),
            ("decision.openai", self.decision.openai.entries().into()),
        ];
        tables
            .into_iter()
            .flat_map(|(table, entries)| {
                entries
                    .into_iter()
                    .map(move |(case, entry)| (table, case, entry))
            })
            .collect()
    }
}

/// A request or organization ID as a provider writes it: `req_…`
/// (Anthropic's request IDs) or `org-…` (the OpenAI-compatible format's
/// organization IDs).
static PROVIDER_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:req_[A-Za-z0-9]{20,}|org-[A-Za-z0-9]{20,})\b")
        .expect("invariant: a valid pattern")
});

/// A field that holds a request or organization ID, its quotes escaped or
/// not (a stream's chunk holds JSON as a string), and its value.
static ID_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\\?"(?:request_id|requestId|request-id|organization_id|organizationId|org_id)\\?"\s*:\s*\\?"([^"\\]*)"#,
    )
    .expect("invariant: a valid pattern")
});

/// gitleaks 8.30.1's rules for the providers' keys, verbatim: its
/// `openai-api-key`, `anthropic-api-key`, `anthropic-admin-api-key` and
/// `gcp-api-key` (which Gemini's keys are). gitleaks has no rule for the
/// keys of ADR 0012's gateways.
static PROVIDER_KEYS: LazyLock<[(&str, Regex); 4]> = LazyLock::new(|| {
    [
        (
            "openai-api-key",
            r#"\b(sk-(?:proj|svcacct|admin)-(?:[A-Za-z0-9_-]{74}|[A-Za-z0-9_-]{58})T3BlbkFJ(?:[A-Za-z0-9_-]{74}|[A-Za-z0-9_-]{58})\b|sk-[a-zA-Z0-9]{20}T3BlbkFJ[a-zA-Z0-9]{20})(?:[\x60'"\s;]|\\[nr]|$)"#,
        ),
        (
            "anthropic-api-key",
            r#"\b(sk-ant-api03-[a-zA-Z0-9_\-]{93}AA)(?:[\x60'"\s;]|\\[nr]|$)"#,
        ),
        (
            "anthropic-admin-api-key",
            r#"\b(sk-ant-admin01-[a-zA-Z0-9_\-]{93}AA)(?:[\x60'"\s;]|\\[nr]|$)"#,
        ),
        (
            "gcp-api-key",
            r#"\b(AIza[\w-]{35})(?:[\x60'"\s;]|\\[nr]|$)"#,
        ),
    ]
    .map(|(rule, pattern)| {
        (
            rule,
            Regex::new(pattern).expect("invariant: a valid pattern"),
        )
    })
});

fn check(root: &Path) -> Result<Checked> {
    let path = root.join(MATRIX);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: File = match toml::from_str(&text) {
        Ok(file) => file,
        Err(error) => {
            return Ok(Checked {
                findings: vec![Finding::new(
                    "matrix",
                    MATRIX,
                    format!("not ADR 0065's allowlist and matrix, every cell given once: {error}"),
                )],
                waits: Vec::new(),
            });
        }
    };
    let recordings: Vec<String> = tree::every_file(root)?
        .into_iter()
        .filter(|file| recording(file))
        .collect();
    let mut findings = Vec::new();
    for relative in &recordings {
        let text = std::fs::read_to_string(root.join(relative))
            .with_context(|| format!("reading {relative}"))?;
        findings.extend(recording_findings(relative, &text, &file.headers));
    }
    let committed: BTreeSet<&str> = recordings.iter().map(String::as_str).collect();
    let mut waiting: BTreeMap<(&str, Reason), Vec<&str>> = BTreeMap::new();
    for (table, case, entry) in file.cells() {
        match entry {
            Entry::Recorded(recording) if committed.contains(recording) => {}
            Entry::Recorded(recording) => findings.push(Finding::new(
                "missing-cell",
                format!("{table}.{case}"),
                format!(
                    "names `{recording}`, which is no committed recording (a JSON file in a `{FOLDER}/` folder)"
                ),
            )),
            Entry::Waits(reason) => waiting.entry((table, reason)).or_default().push(case),
        }
    }
    let waits = waiting
        .into_iter()
        .map(|((table, reason), cases)| {
            Finding::new(
                "waits",
                table,
                format!(
                    "{} cell(s) wait on {}: {}",
                    cases.len(),
                    reason.describe(),
                    cases.join(", ")
                ),
            )
        })
        .collect();
    Ok(Checked { findings, waits })
}

/// Whether the file at `relative` is a recording: a JSON file in a folder
/// named `recordings/`, at any depth.
fn recording(relative: &str) -> bool {
    let path = Path::new(relative);
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        && path
            .parent()
            .is_some_and(|parent| parent.iter().any(|part| part == FOLDER))
}

fn recording_findings(relative: &str, text: &str, allowlist: &Allowlist) -> Vec<Finding> {
    let mut findings = Vec::new();
    let finding = |kind, message: String| Finding::new(kind, relative, message);
    let exchanges = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(mut fields)) => match fields.remove("exchanges") {
            Some(Value::Array(exchanges)) => exchanges,
            _ => {
                return vec![finding(
                    "recording",
                    "has no `exchanges` array (goodfetch-test-support's `ModelRecording`)".into(),
                )];
            }
        },
        Ok(_) => {
            return vec![finding(
                "recording",
                "is no object of `exchanges` (goodfetch-test-support's `ModelRecording`)".into(),
            )];
        }
        Err(error) => return vec![finding("recording", format!("is no JSON: {error}"))],
    };
    for (index, exchange) in exchanges.iter().enumerate() {
        let number = index + 1;
        for (side, allowed) in [
            ("request", &allowlist.request),
            ("response", &allowlist.response),
        ] {
            let Some(headers) = exchange[side]["headers"].as_object() else {
                continue;
            };
            for name in headers.keys() {
                if !allowed.iter().any(|kept| kept.eq_ignore_ascii_case(name)) {
                    findings.push(finding(
                        "header",
                        format!(
                            "exchange {number}'s {side} keeps the header `{name}`, outside the allowlist of {MATRIX}"
                        ),
                    ));
                }
            }
        }
        if let Some(path) = exchange["request"]["path"].as_str() {
            for name in key_parameters(path) {
                findings.push(finding(
                    "key-parameter",
                    format!("exchange {number}'s request path has the key parameter `{name}`"),
                ));
            }
        }
    }
    for found in PROVIDER_ID.find_iter(text) {
        findings.push(finding(
            "unreplaced-id",
            format!(
                "holds `{}` as the provider wrote it; the recorder writes `<request-id>` or `<organization-id>`",
                found.as_str()
            ),
        ));
    }
    for captures in ID_FIELD.captures_iter(text) {
        let value = captures.get(1).map_or("", |value| value.as_str());
        if !PLACEHOLDERS.contains(&value) && !PROVIDER_ID.is_match(value) {
            findings.push(finding(
                "unreplaced-id",
                format!(
                    "holds `{}` with a value other than `<request-id>` or `<organization-id>`",
                    captures.get(0).map_or("", |field| field.as_str())
                ),
            ));
        }
    }
    for (rule, pattern) in PROVIDER_KEYS.iter() {
        if pattern.is_match(text) {
            findings.push(finding(
                "key-shaped",
                format!("holds a string gitleaks' `{rule}` rule reads as a provider key"),
            ));
        }
    }
    findings
}

/// The query parameters of `path` whose name says they carry a key: `key`,
/// `api_key`, `access_token`, `client_secret`, `x-goog-api-key` and the like.
fn key_parameters(path: &str) -> Vec<&str> {
    let Some((_, query)) = path.split_once('?') else {
        return Vec::new();
    };
    query
        .split('&')
        .map(|pair| pair.split_once('=').map_or(pair, |(name, _)| name))
        .filter(|name| {
            let name: String = name
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|character| character.to_ascii_lowercase())
                .collect();
            ["key", "token", "secret", "password", "signature"]
                .iter()
                .any(|word| name.ends_with(word))
                || ["auth", "authorization", "sig"].contains(&name.as_str())
        })
        .collect()
}
