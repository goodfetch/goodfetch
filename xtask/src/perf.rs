//! `xtask perf`: each change's Performance budget (ADR 0071, as amended by
//! ADRs 0087, 0094, 0133 and 0134). Only counted figures gate a change:
//! instructions and heap metrics per benchmark, measured by gungraun under
//! Callgrind and DHAT, and the brotli bytes of each size-limit entry. The
//! base and the head are measured in one job on one runner, so the runner's
//! CPU cancels out; `compare` then holds each figure to its threshold over
//! the base's and to its ceiling, and writes one table. Until M1's
//! optimization pass a figure past either is a note that fails nothing
//! (`budget::GATING`, ADR 0134 as amended on 2026-10-22); a broken
//! measurement still fails. Each figure is also held to a pinned reference
//! build, which fails whatever `GATING` says (`reference`, ADR 0140).

mod admission;
mod budget;
mod bundle;
mod ceilings;
mod compare;
mod drift;
mod figure;
mod figures;
mod gungraun;
mod measure;
mod pull_request;
mod reference;
mod table;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::{github, output};

use budget::Gating;
use compare::{Row, Verdict};

// The figures `xtask rss check` saves for the drift check, in the same file.
/// Each absolute ceiling's benchmark or entry and its number, which the
/// register check compares with the Quality register (ADR 0140).
pub(crate) fn ceiling_limits() -> impl Iterator<Item = (&'static str, u64)> {
    budget::ceilings().map(budget::Ceiling::limit)
}

pub(crate) use budget::with_valgrind_tolerance;
pub(crate) use figure::{Figure, Measured};
pub(crate) use figures::{Figures, Recorded};
pub(crate) use gungraun::read as read_gungraun_summaries;
pub(crate) use measure::gungraun_output;
pub(crate) use reference::PIN_FILE as REFERENCE_FILE;

/// The file that holds the absolute ceilings, relative to the repository.
pub(crate) const BUDGET_FILE: &str = "xtask/src/perf/budget.rs";

/// The crates whose benchmarks an absolute ceiling reads, so a change to the
/// ceilings selects their bench targets (`xtask affected`).
pub(crate) fn ceilinged_crates() -> std::collections::BTreeSet<&'static str> {
    budget::ceilings()
        .flat_map(budget::Ceiling::subjects)
        .filter_map(|subject| subject.split_once('/').map(|(package, _)| package))
        .filter(|package| *package != "size-limit")
        .collect()
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Before the deny: moves the base's kept worktree (target/perf-base-tree)
    /// to the base, checking it out the first time, and builds the base's and
    /// the head's (the workspace's) selected bench targets.
    Build {
        /// The base: the merge base locally, the base branch's head in the
        /// pull-request tier, the group's base in the merge queue.
        #[arg(long)]
        base: String,
        /// `<crate>/<bench target>`s, separated by spaces, as `xtask
        /// affected` prints them in `bench-targets`.
        #[arg(long, default_value = "")]
        bench_targets: String,
    },
    /// Under the deny: measures the base's bench targets at once, then the
    /// head's, as `build` planned, then compares them like `compare`, saving
    /// the head's figures and every report into target/junit/.
    Run {
        /// The pull request's acceptance, as `pull-request` saved it.
        #[arg(long)]
        pull_request: Option<PathBuf>,
        /// The job's summary page (`GITHUB_STEP_SUMMARY`).
        #[arg(long)]
        summary: Option<PathBuf>,
    },
    /// Holds the measured figures to their thresholds over the base's and to
    /// their ceilings, and writes the table.
    Compare(Reports),
    /// Saves what a pull request's "Performance change" section and its
    /// acceptance comments accept, for a later `compare` under the deny.
    #[command(group(clap::ArgGroup::new("pull").required(true).args(["number", "commit"])))]
    PullRequest {
        /// The pull request's number.
        #[arg(long)]
        number: Option<u64>,
        /// A commit of `main`: the pull request that merged it.
        #[arg(long)]
        commit: Option<String>,
        #[command(flatten)]
        github: github::Arguments,
        #[arg(long)]
        save: PathBuf,
    },
    /// Compares `main`'s latest figures in the results store with the last
    /// release's, and opens or updates one `needs-triage` issue per figure
    /// that drifted.
    Drift {
        /// The results store's checkout.
        #[arg(long)]
        store: PathBuf,
        #[command(flatten)]
        github: github::Arguments,
    },
}

/// What `compare` reads, each path relative to the workspace.
#[derive(Args)]
#[command(group(clap::ArgGroup::new("reports").required(true).multiple(true).args(["gungraun", "size_head"])))]
pub(crate) struct Reports {
    /// gungraun's JSON summaries of the head, run with `--baseline=base`
    /// (repeatable).
    #[arg(long)]
    gungraun: Vec<PathBuf>,
    /// gungraun's JSON summaries of the base, run with
    /// `--save-baseline=base` (repeatable): a ceiling whose benchmark the
    /// base measured fails when the head has no figure for it.
    #[arg(long)]
    gungraun_base: Vec<PathBuf>,
    /// size-limit's `--json` report of the base.
    #[arg(long, requires = "size_head")]
    size_base: Option<PathBuf>,
    /// size-limit's `--json` report of the head.
    #[arg(long)]
    size_head: Option<PathBuf>,
    /// The folder of the new benchmarks' admission runs, one gungraun
    /// report (`*.jsonl`) per run of one build.
    #[arg(long)]
    admission: Option<PathBuf>,
    /// The pull request's acceptance, as `pull-request` saved it; with
    /// none, every tripped figure is reported, as `budget::GATING` decides.
    #[arg(long)]
    pull_request: Option<PathBuf>,
    /// gungraun's JSON summaries of the pinned reference build (repeatable);
    /// with neither this nor `--reference-size`, `drift.reference` reports
    /// that it has not run.
    #[arg(long)]
    reference_gungraun: Vec<PathBuf>,
    /// size-limit's `--json` report of the pinned reference build.
    #[arg(long)]
    reference_size: Option<PathBuf>,
    /// The job's summary page (`GITHUB_STEP_SUMMARY`), which gets the
    /// table too.
    #[arg(long)]
    summary: Option<PathBuf>,
    /// Where to save the head's figures for the results store
    /// (`perf-figures.json` in the job's JUnit folder).
    #[arg(long)]
    save: Option<PathBuf>,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let (root, cargo) = (environment.root.as_path(), environment.cargo.as_os_str());
    match command {
        Command::Build {
            base,
            bench_targets,
        } => {
            let targets: Vec<String> = bench_targets
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            measure::build(cargo, root, &base, &targets)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Run {
            pull_request,
            summary,
        } => {
            let measured = measure::run(cargo, root)?;
            let reports = Reports {
                gungraun: vec![measured.head],
                gungraun_base: vec![measured.base],
                size_base: measured.size_base,
                size_head: measured.size_head,
                admission: measured.admission,
                reference_gungraun: measured.reference.into_iter().collect(),
                reference_size: measured.size_reference,
                pull_request,
                summary,
                save: Some(PathBuf::from("target/junit").join(figures::FILE_NAME)),
            };
            compare(root, &reports)
        }
        Command::Compare(reports) => compare(root, &reports),
        Command::PullRequest {
            number,
            commit,
            github,
            save,
        } => {
            // clap requires one of `--number` and `--commit`.
            let source = match (number, commit.as_deref()) {
                (Some(number), _) => pull_request::Source::Number(number),
                (None, Some(commit)) => pull_request::Source::MergedCommit(commit),
                (None, None) => anyhow::bail!("give --number or --commit"),
            };
            let api = github.api(environment.token());
            pull_request::save(api, &github.repository, &source, &root.join(save))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Drift { store, github } => drift::check(
            &github.api(environment.token()),
            &github.repository,
            &root.join(store),
        ),
    }
}

fn compare(root: &Path, reports: &Reports) -> Result<ExitCode> {
    let joined =
        |paths: &[PathBuf]| -> Vec<PathBuf> { paths.iter().map(|path| root.join(path)).collect() };
    let mut measured = gungraun::read(&joined(&reports.gungraun))?;
    // What the base measured, so a ceiling whose benchmark or entry the head
    // lost still fails.
    let mut base_measured = gungraun::benchmarks(&joined(&reports.gungraun_base))?;
    if let Some(head) = &reports.size_head {
        let base = reports.size_base.as_ref().map(|path| root.join(path));
        if let Some(base) = &base {
            base_measured.extend(bundle::entry_subjects(base)?);
        }
        measured.extend(bundle::read(base.as_deref(), &root.join(head))?);
    }
    let mut rows: Vec<Row> = measured.into_iter().map(compare::row).collect();
    rows.sort_by_key(|row| row.measured.id());
    if let Some(path) = &reports.pull_request {
        pull_request::accept(&mut rows, &pull_request::read(&root.join(path))?);
    }
    if let Some(save) = &reports.save {
        figures::save(&root.join(save), &rows)?;
    }
    let admissions = match &reports.admission {
        Some(folder) => admission::read(&rows, &root.join(folder))?,
        None => Vec::new(),
    };
    let ceilings = ceilings::check(root, &rows, &base_measured)?;
    let accepted = reference::accepted(root)?;
    let reference = reference::check(&rows, reference_figures(root, reports)?, &accepted);
    let gating = budget::GATING;
    let mut text = format!(
        "### Performance budget\n\n{}{}\n#### Absolute ceilings\n\n{}\n#### Reference build\n\n{}",
        match gating {
            Gating::Informational => {
                "Until M1's optimization pass, a figure past its threshold or over its ceiling is noted and fails nothing (ADR 0134, as amended on 2026-10-22).\n\n"
            }
            Gating::Enforced => "",
        },
        table::render(&rows),
        ceilings::render(&ceilings),
        reference::render(&reference)
    );
    if !admissions.is_empty() {
        let _ = write!(
            text,
            "\n#### Admission of the new benchmarks\n\n{}",
            admission::render(&admissions)
        );
    }
    let text = text.trim_end();
    output::line(text);
    if let Some(summary) = &reports.summary {
        output::append(&root.join(summary), &format!("{text}\n"))?;
    }
    let mut budget_findings: Vec<Finding> =
        rows.iter().filter_map(|row| finding(row, gating)).collect();
    budget_findings.extend(ceilings::over(&ceilings, gating));
    // A broken measurement fails whatever the gating.
    let mut findings = ceilings::not_measured(&ceilings);
    findings.extend(admission::findings(&admissions));
    // ADR 0140: this row gates from its own pull request, whatever `GATING`.
    findings.extend(reference::findings(&reference));
    match gating {
        Gating::Informational => check::note(&budget_findings),
        Gating::Enforced => findings.extend(budget_findings),
    }
    Ok(check::report(&findings))
}

/// The reference build's figures, as the reports hold them, when any report
/// is given.
fn reference_figures(root: &Path, reports: &Reports) -> Result<Option<Vec<figure::Measured>>> {
    if reports.reference_gungraun.is_empty() && reports.reference_size.is_none() {
        return Ok(None);
    }
    let paths: Vec<PathBuf> = reports
        .reference_gungraun
        .iter()
        .map(|path| root.join(path))
        .collect();
    let mut figures = gungraun::read(&paths)?;
    if let Some(size) = &reports.reference_size {
        figures.extend(bundle::read(None, &root.join(size))?);
    }
    Ok(Some(figures))
}

fn finding(row: &Row, gating: Gating) -> Option<Finding> {
    let id = row.measured.id();
    match row.verdict {
        Verdict::Tripped => Some(Finding::new(
            "performance-change",
            id,
            format!(
                "changed by {} over the base, past its threshold; {}",
                table::change(row).unwrap_or_default(),
                gating.remedy(
                    "list it with its change and reason in the pull request's \"Performance change\" section"
                )
            ),
        )),
        // `ceilings` reports every ceiling, a row's own included.
        Verdict::OverCeiling
        | Verdict::Within
        | Verdict::Improved
        | Verdict::New
        | Verdict::Accepted => None,
    }
}
