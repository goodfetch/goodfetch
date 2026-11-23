//! TLC on the fetched tools, one model and configuration at a time: Java runs
//! `tlc2.TLC` in the model's folder, with its states under `target/tlc/`, and
//! TLC's exit status names the outcome.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use super::models::{self, Model};
use super::tools;
use crate::check::Finding;
use crate::output;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The models to check, by their folder's name under
    /// `test-support/models/`.
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    models: Vec<String>,
    /// Every model.
    #[arg(long)]
    all: bool,
    /// The configuration, `<config>.cfg` in each model's folder: `small` in
    /// the Pre-merge and Release suites, `large` in the nightly job.
    #[arg(long, default_value = "small")]
    config: String,
    /// The outcome each check must reach: `passed`, or for the job's
    /// self-test the violation a broken variant must show.
    #[arg(long, value_enum, default_value_t = Outcome::Passed)]
    expect: Outcome,
}

/// TLC's outcome, from its exit status (`tlc2.output.EC.ExitStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Outcome {
    Passed,
    /// An `ASSUME` is false.
    AssumptionViolated,
    Deadlock,
    /// An invariant or safety property is false in a reachable state.
    InvariantViolated,
    LivenessViolated,
    /// An `Assert` failed.
    AssertionFailed,
    /// TLC stopped on an error of the spec, the configuration or itself.
    Failed,
}

impl Outcome {
    pub(super) const fn from_exit_status(status: i32) -> Self {
        match status {
            0 => Self::Passed,
            10 => Self::AssumptionViolated,
            11 => Self::Deadlock,
            12 => Self::InvariantViolated,
            13 => Self::LivenessViolated,
            14 => Self::AssertionFailed,
            _ => Self::Failed,
        }
    }

    pub(super) const fn kind(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::AssumptionViolated => "assumption-violated",
            Self::Deadlock => "deadlock",
            Self::InvariantViolated => "invariant-violated",
            Self::LivenessViolated => "liveness-violated",
            Self::AssertionFailed => "assertion-failed",
            Self::Failed => "tlc-failed",
        }
    }
}

pub(super) fn run(arguments: &Arguments, root: &Path) -> Result<Vec<Finding>> {
    // TLC runs in each model's folder, so its paths start at the root.
    let root = &std::path::absolute(root).context("finding the workspace's folder")?;
    let every = models::read(root)?;
    let chosen: Vec<&Model> = if arguments.all {
        every.iter().collect()
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
    let class_path =
        std::env::join_paths(tools::kept(root)?).context("joining the tools' class path")?;
    let mut findings = Vec::new();
    for model in chosen {
        let outcome = check(model, &arguments.config, &class_path, root)?;
        let configuration = format!("{}.cfg", arguments.config);
        if outcome == arguments.expect {
            output::line(&format!(
                "{}: {} with {configuration}, as expected",
                model.name,
                outcome.kind()
            ));
        } else {
            findings.push(Finding::new(
                outcome.kind(),
                model.name.clone(),
                format!(
                    "TLC's outcome with {configuration} is {}, not the expected {}",
                    outcome.kind(),
                    arguments.expect.kind()
                ),
            ));
        }
    }
    Ok(findings)
}

fn check(
    model: &Model,
    config: &str,
    class_path: &std::ffi::OsStr,
    root: &Path,
) -> Result<Outcome> {
    let folder = root.join(&model.folder);
    let configuration = folder.join(format!("{config}.cfg"));
    if !configuration.is_file() {
        bail!("{} has no {config}.cfg", model.folder);
    }
    let state_folder = root.join("target/tlc").join(&model.name).join(config);
    let status = Command::new("java")
        .arg("-XX:+UseParallelGC")
        .arg("-cp")
        .arg(class_path)
        .args([
            "tlc2.TLC",
            "-workers",
            "auto",
            "-noGenerateSpecTE",
            "-cleanup",
        ])
        .arg("-config")
        .arg(&configuration)
        .arg("-metadir")
        .arg(&state_folder)
        .arg(&model.spec)
        .current_dir(&folder)
        .status()
        .context("running Java, which TLC needs (Java 11 or later)")?;
    Ok(status
        .code()
        .map_or(Outcome::Failed, Outcome::from_exit_status))
}
