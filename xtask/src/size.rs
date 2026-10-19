//! `xtask size`: each Product's Size budget (ADR 0057, as amended by ADRs
//! 0087 and 0094). It measures a set of built files per Product, target and
//! profile, named by a TOML manifest, and writes one table of every measured
//! size in three modes:
//!
//! - **estimate**, the pull request's Linux x64 report as a comment, which
//!   never fails;
//! - **increase**, the merge queue's check of each file against `main`'s
//!   report of the same Product and profile, and of each crate tarball
//!   against its ceiling; a tripped file passes only with the pull request's
//!   "Size increase" section and acceptance comment on its head;
//! - **ceilings**, the release's check of release-profile files against
//!   each Product's own ceilings, whatever the pull request says.

mod budget;
mod files;
mod pull_request;
mod report;
mod table;

use pull_request::PullRequest;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::{github, output};

use budget::{Bytes, Limit};
use files::Measured;
use table::{Row, Verdict};

/// `main`'s report, and the reference whose tree it must have been saved for.
struct Main<'a> {
    report: &'a Path,
    reference: &'a str,
}

enum Mode<'a> {
    Estimate {
        main: Option<Main<'a>>,
    },
    Increase {
        main: Main<'a>,
        /// The pull request whose acceptance a tripped file needs; with none,
        /// every tripped file fails.
        pull_request: Option<PullRequest<'a>>,
    },
    Ceilings,
}

/// `xtask size`'s modes.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// The pull request's report, compared with `main`'s; never fails.
    Estimate {
        #[command(flatten)]
        files: FileArguments,
        /// `main`'s report, when there is one to compare with.
        #[arg(long)]
        main_report: Option<PathBuf>,
        #[arg(long, default_value = "origin/main")]
        main: String,
    },
    /// Fails on a file grown past its increase threshold over `main`'s
    /// report, unless the pull request accepts it, and on a crate tarball over
    /// its ceiling.
    Increase {
        #[command(flatten)]
        files: FileArguments,
        /// `main`'s report, saved for the tree of `--main`.
        #[arg(long)]
        main_report: PathBuf,
        /// The reference to `main`'s head, whose tree the report must have.
        #[arg(long, default_value = "origin/main")]
        main: String,
        /// The pull request whose "Size increase" section and acceptance
        /// comment a tripped file needs.
        #[arg(long)]
        pull_request: Option<u64>,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Fails on a release-profile file over its Product's ceiling.
    Ceilings {
        #[command(flatten)]
        files: FileArguments,
    },
}

/// The built files every mode measures.
#[derive(clap::Args)]
pub(crate) struct FileArguments {
    /// The TOML manifest naming each built file's Product, target, kind and
    /// path.
    #[arg(long)]
    files: PathBuf,
    /// Saves the measured sizes as a report keyed by the tree of `HEAD`,
    /// which a later run reads as `main`'s (ADR 0061).
    #[arg(long)]
    save: Option<PathBuf>,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    let (mode, files) = match command {
        Command::Estimate {
            files,
            main_report,
            main,
        } => {
            let main = main_report.as_deref().map(|report| Main {
                report,
                reference: main,
            });
            (Mode::Estimate { main }, files)
        }
        Command::Increase {
            files,
            main_report,
            main,
            pull_request,
            github,
        } => {
            let main = Main {
                report: main_report,
                reference: main,
            };
            let pull_request = pull_request.map(|number| PullRequest {
                api: github.api(environment.token()),
                repository: &github.repository,
                number,
            });
            (Mode::Increase { main, pull_request }, files)
        }
        Command::Ceilings { files } => (Mode::Ceilings, files),
    };
    measure(&mode, root, &files.files, files.save.as_deref())
}

/// Measures the files the manifest at `files` names, saves their report to
/// `save` when given, and writes the table.
fn measure(mode: &Mode<'_>, root: &Path, files: &Path, save: Option<&Path>) -> Result<ExitCode> {
    let (profile, measured) = files::measure(root, files)?;
    if let Some(save) = save {
        report::save(root, save, &profile, &measured)?;
    }
    let mut rows: Vec<Row> = match mode {
        Mode::Ceilings => {
            if !profile.is_release() {
                bail!(
                    "ceilings mode measures release-profile files, not `{}` ones",
                    profile.0
                );
            }
            measured.into_iter().map(ceiling_row).collect()
        }
        Mode::Estimate { main } => {
            let main = match main {
                Some(main) => report::read_main(root, main.report, main.reference, &profile)?,
                None => BTreeMap::new(),
            };
            measured
                .into_iter()
                .map(|file| increase_row(file, &main))
                .collect()
        }
        Mode::Increase { main, pull_request } => {
            let main = report::read_main(root, main.report, main.reference, &profile)?;
            let mut rows: Vec<Row> = measured
                .into_iter()
                .map(|file| increase_row(file, &main))
                .collect();
            if let Some(pull_request) = pull_request {
                accept(&mut rows, pull_request)?;
            }
            rows
        }
    };
    rows.sort_by(|left, right| {
        (left.file.product, &left.file.target, &left.file.name).cmp(&(
            right.file.product,
            &right.file.target,
            &right.file.name,
        ))
    });
    if let Mode::Estimate { main } = mode {
        output::line(&format!(
            "### Size estimate\n\n{}\n{}",
            table::render(&rows),
            if main.is_some() {
                "Compared with `main`'s report; the merge queue's increase check decides."
            } else {
                "No report of `main` to compare with; the merge queue's increase check decides."
            }
        ));
        return Ok(ExitCode::SUCCESS);
    }
    output::line(&table::render(&rows));
    let findings: Vec<Finding> = rows
        .iter()
        .filter(|row| row.verdict == Verdict::Over)
        .map(finding)
        .collect();
    Ok(check::report(&findings))
}

fn ceiling_row(file: Measured) -> Row {
    let limit = budget::ceiling(file.product, file.kind);
    let verdict = match limit {
        Limit::Ceiling(ceiling) if file.bytes > ceiling => Verdict::Over,
        _ => Verdict::Within,
    };
    Row {
        file,
        main: None,
        limit,
        verdict,
    }
}

fn increase_row(file: Measured, main: &BTreeMap<String, Bytes>) -> Row {
    let limit = budget::increase(file.kind);
    let main = main.get(&file.id()).copied();
    let verdict = match (limit, main) {
        (Limit::Ceiling(ceiling), _) if file.bytes > ceiling => Verdict::Over,
        (Limit::Increase(threshold), Some(main))
            if file.bytes.0 > main.0.saturating_add(threshold.0) =>
        {
            Verdict::Over
        }
        (Limit::Increase(_), None) => Verdict::New,
        _ => Verdict::Within,
    };
    Row {
        file,
        main,
        limit,
        verdict,
    }
}

/// Marks each file past its increase threshold that the pull request
/// accepts; GitHub is read only when a file tripped.
fn accept(rows: &mut [Row], pull_request: &PullRequest<'_>) -> Result<()> {
    let tripped =
        |row: &Row| row.verdict == Verdict::Over && matches!(row.limit, Limit::Increase(_));
    if !rows.iter().any(tripped) {
        return Ok(());
    }
    let acceptance = pull_request::read(pull_request)?;
    for row in rows.iter_mut().filter(|row| tripped(row)) {
        let change = row
            .change()
            .and_then(|change| u64::try_from(change).ok())
            .map(Bytes);
        if change.is_some_and(|change| pull_request::accepts(&acceptance, &row.file.id(), change)) {
            row.verdict = Verdict::Accepted;
        }
    }
    Ok(())
}

fn finding(row: &Row) -> Finding {
    match row.limit {
        Limit::Ceiling(_) => Finding::new(
            "over-ceiling",
            row.file.id(),
            format!(
                "{} is over {}'s ceiling of {}",
                row.file.bytes.megabytes(),
                row.file.product,
                row.limit
            ),
        ),
        Limit::Increase(_) | Limit::None => Finding::new(
            "size-increase",
            row.file.id(),
            format!(
                "grew by {} over main's figure, past its threshold of {}; list it with its delta and reason in the pull request's \"Size increase\" section",
                row.change().map_or_else(String::new, table::change),
                row.limit
            ),
        ),
    }
}
