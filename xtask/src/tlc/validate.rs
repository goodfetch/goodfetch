//! Trace validation (ADR 0060), in etcd's style: TLC checks a model's trace
//! spec, its `trace.cfg` in the model's folder, on one NDJSON trace at a
//! time, which the `TRACE` environment variable names, with the template of
//! [`models::TEMPLATE_FOLDER`] as its library. The trace validates when TLC
//! breaks the spec's `TraceUnmatched` invariant, a state having matched
//! every line; it fails when the search ends with no such state. TLC's
//! output is kept under `target/tlc/`, and a failure names its file.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process and blocking-fs bans target async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use super::models::{self, Model};
use super::run::Outcome;
use super::tools;
use crate::check::Finding;
use crate::output;

/// The trace spec's configuration, in the model's folder.
const CONFIG: &str = "trace.cfg";

/// What TLC prints when a state matched every line.
const MATCHED: &str = "Error: Invariant TraceUnmatched is violated.";

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The models whose committed traces to validate, by their folder's
    /// name under `test-support/models/`; a model with no trace spec is
    /// skipped.
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    models: Vec<String>,
    /// Every model that has a trace spec.
    #[arg(long)]
    all: bool,
    /// One trace file to validate in place of the model's committed ones,
    /// from the repository root: for the job's self-test, a trace altered
    /// by hand.
    #[arg(long, conflicts_with = "all")]
    trace: Option<String>,
    /// Whether each trace must match a behaviour of the model or not.
    #[arg(long, value_enum, default_value_t = Verdict::Matched)]
    expect: Verdict,
}

/// A trace's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Verdict {
    /// A behaviour of the model matches every line.
    Matched,
    /// No behaviour of the model matches every line.
    Unmatched,
}

impl Verdict {
    const fn kind(self) -> &'static str {
        match self {
            Self::Matched => "trace-matched",
            Self::Unmatched => "trace-unmatched",
        }
    }
}

pub(super) fn run(arguments: &Arguments, root: &Path) -> Result<Vec<Finding>> {
    let root = &std::path::absolute(root).context("finding the workspace's folder")?;
    let every = models::read(root)?;
    let chosen: Vec<&Model> = if arguments.all {
        every
            .iter()
            .filter(|model| model.trace_spec.is_some())
            .collect()
    } else {
        arguments
            .models
            .iter()
            .map(|name| {
                every
                    .iter()
                    .find(|model| &model.name == name)
                    .with_context(|| format!("no model `{name}` in {}", models::MODELS_FOLDER))
            })
            .collect::<Result<_>>()?
    };
    if arguments.trace.is_some() && chosen.len() != 1 {
        bail!("`--trace` validates one trace against one model");
    }
    let class_path =
        std::env::join_paths(tools::kept(root)?).context("joining the tools' class path")?;
    let mut findings = Vec::new();
    for model in chosen {
        let Some(spec) = &model.trace_spec else {
            output::line(&format!(
                "{}: no trace spec, no trace to validate",
                model.name
            ));
            continue;
        };
        let traces = arguments
            .trace
            .as_ref()
            .map_or_else(|| model.trace_files.clone(), |trace| vec![trace.clone()]);
        for trace in &traces {
            let (verdict, log) = validate(model, spec, trace, &class_path, root)?;
            if verdict == Ok(arguments.expect) {
                output::line(&format!(
                    "{}: {trace} {}, as expected",
                    model.name,
                    arguments.expect.kind()
                ));
                continue;
            }
            let got = verdict.map_or_else(Outcome::kind, Verdict::kind);
            findings.push(Finding::new(
                got,
                model.name.clone(),
                format!(
                    "{trace} is {got}, not the expected {}; TLC's output is in {}",
                    arguments.expect.kind(),
                    log.display()
                ),
            ));
        }
    }
    Ok(findings)
}

/// TLC's verdict on one trace, or the outcome it stopped on instead, with
/// the file that keeps its output.
fn validate(
    model: &Model,
    spec: &str,
    trace: &str,
    class_path: &OsStr,
    root: &Path,
) -> Result<(Result<Verdict, Outcome>, std::path::PathBuf)> {
    let folder = root.join(&model.folder);
    let trace_path = root.join(trace);
    if !trace_path.is_file() {
        bail!("the trace {trace} does not exist");
    }
    let name = Path::new(trace)
        .file_stem()
        .with_context(|| format!("{trace} names no file"))?;
    let state_folder = root.join("target/tlc").join(&model.name).join("trace");
    let mut library = std::ffi::OsString::from("-DTLA-Library=");
    library.push(root.join(models::TEMPLATE_FOLDER));
    let ran = Command::new("java")
        .arg("-XX:+UseParallelGC")
        .arg(library)
        .arg("-cp")
        .arg(class_path)
        .args([
            "tlc2.TLC",
            "-workers",
            "auto",
            "-noGenerateSpecTE",
            "-cleanup",
            "-config",
            CONFIG,
        ])
        .arg("-metadir")
        .arg(state_folder.join(name))
        .arg(spec)
        .env("TRACE", &trace_path)
        .current_dir(&folder)
        .output()
        .context("running Java, which TLC needs (Java 11 or later)")?;
    std::fs::create_dir_all(&state_folder)
        .with_context(|| format!("creating {}", state_folder.display()))?;
    let log = state_folder.join(name).with_extension("out");
    std::fs::write(&log, [ran.stdout.as_slice(), &ran.stderr].concat())
        .with_context(|| format!("writing {}", log.display()))?;
    let outcome = ran
        .status
        .code()
        .map_or(Outcome::Failed, Outcome::from_exit_status);
    let verdict = match outcome {
        Outcome::InvariantViolated
            if String::from_utf8_lossy(&ran.stdout)
                .lines()
                .any(|line| line == MATCHED) =>
        {
            Ok(Verdict::Matched)
        }
        Outcome::Passed => Ok(Verdict::Unmatched),
        other => Err(other),
    };
    Ok((verdict, log))
}
