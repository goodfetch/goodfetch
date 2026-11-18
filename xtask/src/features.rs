//! `xtask features` (ADRs 0072 and 0075): the Feature inventory's types and
//! generators, which `xtask codegen` runs, the scanner that reads what each
//! test declares in its runner's idiom, the Feature check: in the Quick
//! suite without the runners' listings, in the Pre-merge suite with them,
//! the exercise audit of what each declaring test reached, and the Feature
//! report, which the daily test-health job publishes.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod audit;
mod check;
mod command;
mod contract;
mod covers;
mod entry;
mod files;
mod format;
mod inventory;
mod listed;
mod page;
mod report;
mod row;
mod rstest_names;
mod scan;
mod shared_type;
mod skill;
mod story;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use goodfetch_protocol::journey::Milestone;

use crate::environment::Environment;
use crate::{journeys, output};

pub(crate) use command::{CommandLines, SECTION as COMMANDS};
pub(crate) use files::{INVENTORY, MATRIX, schemas};
pub(crate) use inventory::{FoxhoundNames, Sources};
pub(crate) use report::manual_exceptions;
pub(crate) use scan::tests;

/// The report's files in its folder.
const REPORT_JSON: &str = "features.json";
const REPORT_HTML: &str = "features.html";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints every declaration, one `<feature>\t<level>\t<source>\t<test>`
    /// line each: the source is `nextest`, `vitest`, `playwright`, `pytest`,
    /// `journey` or `conformance`, and the test is named as its runner names
    /// it.
    Scan {
        /// Only the declarations of these packages: crates, npm or Python
        /// packages, by name or folder.
        #[arg(long = "package")]
        packages: Vec<String>,
        #[command(flatten)]
        listings: Listings,
    },
    /// The Feature check: fails on a declaration of an unknown or retired
    /// Feature, a generated Feature with no declaring test, no exception or
    /// not one owning area, a wrong exception, a story's missing, unknown or
    /// extra `not_journey`, and a file that breaks its schema. Without
    /// `--listings`, a generated Feature no Rust test, Journey or conformance
    /// case declares is only noted.
    Check {
        #[command(flatten)]
        listings: Listings,
        /// Only the findings of these packages, by name or folder: those of
        /// the Features their tests declare and of the generated Features
        /// their sources define. Findings on the `features/` files stay.
        #[arg(long = "package")]
        packages: Vec<String>,
    },
    /// The exercise audit: fails a Rust test that declares an `operation:`,
    /// `error:`, `event:`, `command:` or `exit:` Feature its exercise record
    /// never shows it reached, or that left no record. With `--junit`, over
    /// the tests those files show ran (the Pre-merge and Release suites);
    /// without it, over every declaring test, run or not.
    Audit {
        /// The folder of the exercise records `just test` has test support
        /// write, one file per test.
        #[arg(long, default_value = "target/test-records")]
        records: PathBuf,
        /// nextest's JUnit files of the run, whose tests that ran are the
        /// ones audited.
        #[arg(long)]
        junit: Vec<PathBuf>,
    },
    /// Writes `features.json` and `features.html` into `--out`: each active
    /// Feature tested, untested with its cause, excepted with its evidence
    /// kind, or planned, at the milestone's gate, from `main`'s latest
    /// Release run in the results store and the quarantine list; and each
    /// Product's view, by area and by the milestone each Feature is due.
    /// Prints each Product's counts. It fails no gate.
    Report {
        /// The milestone whose gate it judges, `M1` to `M4`.
        #[arg(long, value_parser = parse_milestone)]
        milestone: Milestone,
        /// The results store's checkout; without it, no test has run.
        #[arg(long)]
        store: Option<PathBuf>,
        #[command(flatten)]
        listings: Listings,
        /// The folder to write the report to.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(clap::Args)]
pub(crate) struct Listings {
    /// The folder of the runners' listings `xtask journeys listings` writes,
    /// whose Vitest, Playwright and pytest declarations are read too, and,
    /// for the report, the tests that carry each Journey variant.
    #[arg(long = "listings")]
    folder: Option<PathBuf>,
}

fn parse_milestone(text: &str) -> Result<Milestone, String> {
    report::milestone_of(text).ok_or_else(|| format!("`{text}` is not M1, M2, M3 or M4"))
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    match command {
        Command::Scan { packages, listings } => {
            let (declarations, _) = declarations(cargo, root, &listings)?;
            for declaration in declarations.iter().filter(|declaration| {
                packages.is_empty()
                    || check::selected(&packages, declaration.package.as_deref(), &declaration.path)
            }) {
                output::line(&format!(
                    "{}\t{}\t{}\t{}",
                    declaration.feature, declaration.level, declaration.source, declaration.test
                ));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check { listings, packages } => {
            let (declarations, read) = declarations(cargo, root, &listings)?;
            let mut verdict = check::check(root, &declarations, read)?;
            if !packages.is_empty() {
                verdict = check::of_packages(verdict, &declarations, &packages);
            }
            for note in &verdict.notes {
                output::line(&format!(
                    "note[{}] {}: {}; `just features-tests` also reads the Vitest, Playwright and pytest declarations",
                    note.kind, note.subject, note.message
                ));
            }
            Ok(crate::check::report(&verdict.findings))
        }
        Command::Audit { records, junit } => {
            let declarations = scan::rust(cargo, root)?;
            let ran = if junit.is_empty() {
                None
            } else {
                Some(audit::tests_that_ran(root, &junit)?)
            };
            let findings = audit::audit(root, &declarations, &root.join(records), ran.as_ref())?;
            Ok(crate::check::report(&findings))
        }
        Command::Report {
            milestone,
            store,
            listings,
            out,
        } => {
            let (declarations, _) = declarations(cargo, root, &listings)?;
            let variant_tests = listings
                .folder
                .as_ref()
                .map(|folder| journeys::variant_tests(root, &root.join(folder)))
                .transpose()?;
            let store = store.map(|store| environment.path(&store));
            let built = report::build(&report::Sources {
                root,
                gate: milestone,
                declarations: &declarations,
                variants: &journeys::active_variants(root)?,
                variant_tests: variant_tests.as_ref(),
                store: store.as_deref(),
            })?;
            write_report(&environment.path(&out), &built)?;
            for view in &built.views {
                output::line(&format!(
                    "{}: {} tested, {} untested, {} excepted, {} planned at the {:?} gate",
                    view.product.as_str(),
                    view.summary.tested,
                    view.summary.untested_count(),
                    view.summary.excepted_count(),
                    view.summary.planned,
                    built.milestone,
                ));
            }
            output::line(&built.gate_line());
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Every declaration the scanner reads, the runners' listings' too when
/// they are given, sorted, with which of them the check reads.
fn declarations(
    cargo: &OsStr,
    root: &Path,
    listings: &Listings,
) -> Result<(Vec<scan::Declaration>, check::Declarations)> {
    let mut declarations = scan::rust(cargo, root)?;
    declarations.extend(covers::read(root)?);
    let read = match &listings.folder {
        Some(folder) => {
            declarations.extend(listed::read(root, &root.join(folder))?);
            check::Declarations::All
        }
        None => check::Declarations::WithoutListings,
    };
    declarations.sort();
    declarations.dedup();
    Ok((declarations, read))
}

fn write_report(out: &Path, built: &report::Report) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut json = serde_json::to_string_pretty(built)?;
    json.push('\n');
    for (name, contents) in [(REPORT_JSON, json), (REPORT_HTML, page::render(built))] {
        let path = out.join(name);
        std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// The inventory as `xtask codegen` writes it: pretty JSON with a final
/// newline.
pub(crate) fn inventory_text(sources: &Sources<'_>) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(&inventory::generate(sources)?)?;
    text.push('\n');
    Ok(text.into_bytes())
}
