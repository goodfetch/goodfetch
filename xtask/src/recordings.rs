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
use std::path::{Path, PathBuf};
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
    /// Applies ADR 0065's allowlist and ID redaction at write time to a raw
    /// exchange JSON file, refusing to write if a provider-key shape remains.
    Scrub {
        /// Raw recorded exchange JSON file.
        #[arg(long = "in")]
        input: PathBuf,
        /// Destination path for the scrubbed `ModelRecording` JSON file.
        #[arg(long = "out")]
        output: PathBuf,
    },
    /// Records Proving ground prompt exchanges through the Vercel AI Gateway,
    /// applies the allowlist and ID redaction at write time, updates
    /// `test-support/fixtures/model-recordings.toml` cells with
    /// `via = "vercel-ai-gateway"`, and verifies with `recordings check`.
    Record {
        /// Comma-separated Vercel AI Gateway free-tier model IDs.
        #[arg(long, default_value = "openai/gpt-5.4-nano")]
        models: String,
        /// Optional comma-separated cell filter (`openai-compatible.direct.text`, …).
        #[arg(long, default_value = "")]
        cells: String,
        /// Gateway base URL (defaults to Vercel AI Gateway).
        #[arg(long, default_value = "https://ai-gateway.vercel.sh")]
        gateway_url: String,
        /// Directory under the repository root where scrubbed recordings are written.
        #[arg(long, default_value = "crates/goodfetch-engine/tests/recordings")]
        out_dir: PathBuf,
    },
}

pub(crate) fn run(command: &Command, root: &Path) -> Result<ExitCode> {
    match command {
        Command::Check => {
            let checked = check(root)?;
            check::note(&checked.waits);
            Ok(check::report(&checked.findings))
        }
        Command::Scrub { input, output } => scrub(root, input, output),
        Command::Record {
            models,
            cells,
            gateway_url,
            out_dir,
        } => record(root, models, cells, gateway_url, out_dir),
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

/// Whether `name` is a query parameter whose name says it carries a key.
fn is_key_parameter(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    ["key", "token", "secret", "password", "signature"]
        .iter()
        .any(|word| normalized.ends_with(word))
        || ["auth", "authorization", "sig"].contains(&normalized.as_str())
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
        .filter(|name| is_key_parameter(name))
        .collect()
}

fn scrub(root: &Path, input: &Path, output: &Path) -> Result<ExitCode> {
    let matrix_path = root.join(MATRIX);
    let matrix_text = std::fs::read_to_string(&matrix_path)
        .with_context(|| format!("reading {}", matrix_path.display()))?;
    let file: File = toml::from_str(&matrix_text)
        .with_context(|| format!("parsing {}", matrix_path.display()))?;

    let in_path = if input.is_absolute() {
        input.to_path_buf()
    } else {
        root.join(input)
    };
    let raw_text = std::fs::read_to_string(&in_path)
        .with_context(|| format!("reading {}", in_path.display()))?;
    let mut value: Value = serde_json::from_str(&raw_text)
        .with_context(|| format!("parsing {}", in_path.display()))?;

    let Some(exchanges) = value.get_mut("exchanges").and_then(Value::as_array_mut) else {
        return Ok(check::report(&[Finding::new(
            "recording",
            input.display().to_string(),
            "has no `exchanges` array (goodfetch-test-support's `ModelRecording`)",
        )]));
    };

    for exchange in exchanges {
        scrub_exchange(exchange, &file.headers);
    }

    let rendered = format!("{}\n", serde_json::to_string_pretty(&value)?);
    let subject = output
        .strip_prefix(root)
        .unwrap_or(output)
        .display()
        .to_string();
    let findings = recording_findings(&subject, &rendered, &file.headers);
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }

    let out_path = if output.is_absolute() {
        output.to_path_buf()
    } else {
        root.join(output)
    };
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&out_path, rendered)
        .with_context(|| format!("writing {}", out_path.display()))?;
    Ok(ExitCode::SUCCESS)
}

fn scrub_exchange(exchange: &mut Value, allowlist: &Allowlist) {
    for (side, allowed) in [
        ("request", &allowlist.request),
        ("response", &allowlist.response),
    ] {
        if let Some(headers) = exchange
            .get_mut(side)
            .and_then(|s| s.get_mut("headers"))
            .and_then(Value::as_object_mut)
        {
            let kept: serde_json::Map<String, Value> = headers
                .iter()
                .filter(|(name, _)| allowed.iter().any(|k| k.eq_ignore_ascii_case(name)))
                .map(|(name, val)| (name.to_ascii_lowercase(), val.clone()))
                .collect();
            *headers = kept;
        }
    }
    if let Some(path_val) = exchange
        .get("request")
        .and_then(|r| r.get("path"))
        .and_then(Value::as_str)
        .map(strip_key_parameters)
    {
        exchange["request"]["path"] = Value::String(path_val);
    }
    scrub_ids_in_value(exchange);
}

fn strip_key_parameters(path: &str) -> String {
    let Some((base, query)) = path.split_once('?') else {
        return path.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(n, _)| n);
            !name.is_empty() && !is_key_parameter(name)
        })
        .collect();
    if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

fn scrub_ids_in_value(value: &mut Value) {
    match value {
        Value::String(text) => {
            *text = scrub_ids_in_str(text);
        }
        Value::Array(items) => {
            for item in items {
                scrub_ids_in_value(item);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if let Value::String(text) = item {
                    if matches!(key.as_str(), "request_id" | "requestId" | "request-id") {
                        "<request-id>".clone_into(text);
                        continue;
                    }
                    if matches!(
                        key.as_str(),
                        "organization_id" | "organizationId" | "org_id"
                    ) {
                        "<organization-id>".clone_into(text);
                        continue;
                    }
                }
                scrub_ids_in_value(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn scrub_ids_in_str(input: &str) -> String {
    let replaced_tokens = PROVIDER_ID.replace_all(input, |caps: &regex::Captures<'_>| {
        if caps.get(0).is_some_and(|m| m.as_str().starts_with("req_")) {
            "<request-id>"
        } else {
            "<organization-id>"
        }
    });
    ID_FIELD
        .replace_all(&replaced_tokens, |caps: &regex::Captures<'_>| {
            let whole = caps.get(0).map_or("", |m| m.as_str());
            let val = caps.get(1).map_or("", |m| m.as_str());
            if PLACEHOLDERS.contains(&val) {
                return whole.to_owned();
            }
            let placeholder = if whole.contains("org") {
                "<organization-id>"
            } else {
                "<request-id>"
            };
            whole.replacen(val, placeholder, 1)
        })
        .into_owned()
}

struct RecordSpec {
    table: &'static str,
    case: &'static str,
    filename: &'static str,
    path: &'static str,
    via_gateway: bool,
    anthropic_version: bool,
}

const RECORD_SPECS: [RecordSpec; 3] = [
    RecordSpec {
        table: "openai-compatible.direct",
        case: "text",
        filename: "openai-compatible-text.json",
        path: "/v1/chat/completions",
        via_gateway: true,
        anthropic_version: false,
    },
    RecordSpec {
        table: "openai-compatible.vercel-ai-gateway",
        case: "text",
        filename: "vercel-ai-gateway-text.json",
        path: "/v1/chat/completions",
        via_gateway: false,
        anthropic_version: false,
    },
    RecordSpec {
        table: "anthropic-messages.direct",
        case: "text",
        filename: "anthropic-messages-text.json",
        path: "/v1/messages",
        via_gateway: true,
        anthropic_version: true,
    },
];

fn record(
    root: &Path,
    models: &str,
    cells_filter: &str,
    gateway_url: &str,
    out_dir: &Path,
) -> Result<ExitCode> {
    let api_key = std::env::var("AI_GATEWAY_API_KEY").unwrap_or_default();
    if api_key.trim().is_empty() {
        return Ok(check::report(&[Finding::new(
            "recorder",
            MATRIX,
            "AI_GATEWAY_API_KEY is required to record exchanges",
        )]));
    }
    let model = models
        .split(',')
        .map(str::trim)
        .find(|m| !m.is_empty())
        .unwrap_or("openai/gpt-5.4-nano");
    let filter: BTreeSet<&str> = cells_filter
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();

    let matrix_path = root.join(MATRIX);
    let matrix_text = std::fs::read_to_string(&matrix_path)
        .with_context(|| format!("reading {}", matrix_path.display()))?;
    let file: File = toml::from_str(&matrix_text)
        .with_context(|| format!("parsing {}", matrix_path.display()))?;
    let mut matrix_doc: toml_edit::DocumentMut = matrix_text
        .parse()
        .with_context(|| format!("parsing {} for edit", matrix_path.display()))?;

    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();

    for spec in &RECORD_SPECS {
        let cell_key = format!("{}.{}", spec.table, spec.case);
        if !filter.is_empty() && !filter.contains(cell_key.as_str()) {
            continue;
        }
        let mut exchange = capture_exchange(&agent, gateway_url, &api_key, model, spec)?;
        scrub_exchange(&mut exchange, &file.headers);
        let doc = serde_json::json!({ "exchanges": [exchange] });
        let rendered = format!("{}\n", serde_json::to_string_pretty(&doc)?);
        let rel_out = out_dir.join(spec.filename);
        let rel_str = rel_out.to_string_lossy().replace('\\', "/");
        let findings = recording_findings(&rel_str, &rendered, &file.headers);
        if !findings.is_empty() {
            return Ok(check::report(&findings));
        }
        let abs_out = root.join(&rel_out);
        if let Some(parent) = abs_out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&abs_out, rendered)
            .with_context(|| format!("writing {}", abs_out.display()))?;
        update_matrix_cell(
            &mut matrix_doc,
            spec.table,
            spec.case,
            &rel_str,
            spec.via_gateway,
        )?;
    }

    std::fs::write(&matrix_path, matrix_doc.to_string())
        .with_context(|| format!("writing {}", matrix_path.display()))?;

    let checked = check(root)?;
    check::note(&checked.waits);
    Ok(check::report(&checked.findings))
}

fn capture_exchange(
    agent: &ureq::Agent,
    gateway_url: &str,
    api_key: &str,
    model: &str,
    spec: &RecordSpec,
) -> Result<Value> {
    let url = format!("{}{}", gateway_url.trim_end_matches('/'), spec.path);
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 64,
        "messages": [{ "role": "user", "content": "Proving ground smoke: reply with ok." }]
    });
    let mut req_headers = serde_json::Map::new();
    req_headers.insert(
        "content-type".to_owned(),
        Value::String("application/json".to_owned()),
    );
    req_headers.insert(
        "authorization".to_owned(),
        Value::String(format!("Bearer {api_key}")),
    );
    let mut req = agent
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", &format!("Bearer {api_key}"));
    if spec.anthropic_version {
        req = req.header("anthropic-version", "2023-06-01");
        req_headers.insert(
            "anthropic-version".to_owned(),
            Value::String("2023-06-01".to_owned()),
        );
    }
    let mut resp = req
        .send_json(&body)
        .with_context(|| format!("sending recording request to {url}"))?;
    let status = resp.status().as_u16();
    let mut resp_headers = serde_json::Map::new();
    for (name, val) in resp.headers() {
        if let Ok(s) = val.to_str() {
            resp_headers.insert(name.as_str().to_owned(), Value::String(s.to_owned()));
        }
    }
    let resp_text = resp
        .body_mut()
        .read_to_string()
        .with_context(|| format!("reading response body from {url}"))?;
    let resp_body = serde_json::from_str::<Value>(&resp_text).unwrap_or(Value::String(resp_text));

    Ok(serde_json::json!({
        "request": {
            "method": "POST",
            "path": spec.path,
            "headers": Value::Object(req_headers),
            "body": body,
        },
        "response": {
            "status": status,
            "headers": Value::Object(resp_headers),
            "body": resp_body,
        }
    }))
}

fn update_matrix_cell(
    doc: &mut toml_edit::DocumentMut,
    table: &str,
    case: &str,
    recording_path: &str,
    via_gateway: bool,
) -> Result<()> {
    let mut parts = table.split('.');
    let first = parts.next().context("matrix table missing first segment")?;
    let second = parts
        .next()
        .context("matrix table missing second segment")?;
    let mut inline = toml_edit::InlineTable::new();
    inline.insert("recording", recording_path.into());
    if via_gateway {
        inline.insert("via", "vercel-ai-gateway".into());
    }
    doc[first][second][case] = toml_edit::Item::Value(toml_edit::Value::InlineTable(inline));
    Ok(())
}
