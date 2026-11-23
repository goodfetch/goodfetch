//! `just quick`'s runs in the gate's records (ADR 0135's amendment of
//! 2026-11-22): the implementer's `just quick` and the pre-push hook's are
//! layers of the shared record, so `just quick` never runs twice on one tree.
//! A run is held, and neither `just quick` nor the hook runs it again, when
//! the latest run recorded for the same key passed within a day and lint
//! would read now what it read then (`lint`). The key covers what the run
//! reads: the tree, the host fingerprint with the run's variables (`host`),
//! the form (`just quick` or `just quick all`) and, for the first, the
//! change's merge base with its base. A red run is recorded too, so a red
//! result after a green one on the same key is the one that counts.
//!
//! The record is `quick-runs` in the gate's records (`ci/gate-records.sh`),
//! which every worktree shares, appended to by each run: one
//! `<host>\t<unit>\t<pass|fail>\t<head>\t<tree>\t<unix time>\t<layer>` line a
//! run, the unit `just-quick:<merge base>` or `just-quick-all`. A run on
//! uncommitted changes records nothing, nor does one whose lint read GitHub
//! with the run's token (zizmor's online audits). The duplicate-run check
//! reads these lines with the gate's own (`duplicates`).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};

use super::plan::{GateRun, Layer, Outcome, past_max_age, short};
use super::{Status, host, lint};
use crate::environment::Environment;
use crate::{git, github, output};

/// The record's file in the gate's records.
const FILE: &str = "quick-runs";

/// The unit of `just quick all`, which reads no base.
const ALL: &str = "just-quick-all";

/// `xtask gate quick`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Exits 0, naming the run it holds, when a green run of `just quick`
    /// with this key is recorded within a day and lint would read now what
    /// it read then; exits 1, saying why, when `just quick` is due.
    Held {
        #[command(flatten)]
        key: KeyArguments,
        /// Where lint's quarantine check reads its quarantined tests' issues.
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Appends a run's result, unless the working tree is no longer the
    /// head's or lint read GitHub.
    Record {
        #[command(flatten)]
        key: KeyArguments,
        /// The layer that ran it.
        #[arg(long, value_enum)]
        by: Layer,
        #[arg(long, value_enum)]
        status: Status,
    },
}

/// What `just quick` reads besides the tree's files.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    /// `just quick`: the change against its merge base with `--base`.
    Changed,
    /// `just quick all`: the whole workspace.
    All,
}

/// A run's key, as the `_quick-held` and `_quick-record` recipes give it.
#[derive(clap::Args)]
pub(crate) struct KeyArguments {
    /// The folder of the gate's records.
    #[arg(long)]
    records: PathBuf,
    /// The commit whose tree the run checks.
    #[arg(long)]
    head: String,
    /// The change's base, as `just quick` reads it (`GOODFETCH_QUICK_BASE`,
    /// else the branch's own base).
    #[arg(long)]
    base: String,
    #[arg(long, value_enum)]
    form: Form,
    /// Now, in seconds since the Unix epoch.
    #[arg(long)]
    now: u64,
    /// The run's variables, as `NAME=value` lines (`ci/gate-variables.sh`):
    /// part of the host fingerprint.
    #[arg(long, default_value = "")]
    environment: String,
    /// Lint's zizmor audits online with a GitHub token of the run's
    /// environment: no run is held or recorded.
    #[arg(long)]
    zizmor_online: bool,
}

/// One run's line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct QuickRun {
    pub(crate) host: String,
    pub(crate) unit: String,
    pub(crate) head: String,
    pub(crate) outcome: Outcome,
}

/// A run's key, read from the repository.
struct Key {
    host: String,
    unit: String,
    tree: String,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Held { key, github } => {
            let records = environment.path(&key.records);
            let runs = read(&records)?;
            let found = self::key(root, environment, &key)?;
            let reads = lint::Reads {
                root,
                now: key.now,
                zizmor_online: key.zizmor_online,
                github: &github,
                token: environment.token(),
            };
            match held(&runs, &found, key.now, || reads.hold()) {
                Ok(run) => {
                    output::line(&format!(
                        "`just quick{}` already passed on tree {}: green by {} on {}, recorded in {}, so it does not run again.",
                        if key.form == Form::All { " all" } else { "" },
                        short(&found.tree),
                        run.outcome.by.name(),
                        short(&run.head),
                        records.join(FILE).display(),
                    ));
                    Ok(ExitCode::SUCCESS)
                }
                Err(why) => {
                    output::line(&format!("`just quick` is due: {why}."));
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Command::Record { key, by, status } => {
            if let Some(why) = unrecorded(root, &key)? {
                output::line(&format!("`just quick` records no run: {why}."));
                return Ok(ExitCode::SUCCESS);
            }
            let found = self::key(root, environment, &key)?;
            let records = environment.path(&key.records);
            std::fs::create_dir_all(&records)
                .with_context(|| format!("creating {}", records.display()))?;
            let run = QuickRun {
                host: found.host,
                unit: found.unit,
                head: git::commit(root, &key.head)?,
                outcome: Outcome {
                    green: status == Status::Pass,
                    origin: String::new(),
                    tree: found.tree,
                    at: key.now,
                    by,
                    earned_on: None,
                },
            };
            output::append(&records.join(FILE), &line(&run))?;
            output::line(&format!(
                "`just quick` recorded its {} run on tree {} by {} in {}.",
                if run.outcome.green { "green" } else { "red" },
                short(&run.outcome.tree),
                by.name(),
                records.join(FILE).display(),
            ));
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Why a run is not recorded: the working tree is no longer the head's, or
/// lint read GitHub; none when it is.
fn unrecorded(root: &Path, key: &KeyArguments) -> Result<Option<&'static str>> {
    if key.zizmor_online {
        return Ok(Some(
            "lint's zizmor audited online with the run's GitHub token, so lint read more than the tree",
        ));
    }
    if git::commit(root, "HEAD")? != git::commit(root, &key.head)?
        || !git::line(root, &["status", "--porcelain"])?.is_empty()
    {
        return Ok(Some(
            "the working tree is no longer the tree of the head it checked",
        ));
    }
    Ok(None)
}

fn key(root: &Path, environment: &Environment, key: &KeyArguments) -> Result<Key> {
    let head = git::commit(root, &key.head)?;
    let unit = match key.form {
        Form::All => ALL.to_owned(),
        Form::Changed => {
            let fork = git::line(root, &["merge-base", &head, &key.base])
                .with_context(|| format!("the merge base of {} with {}", short(&head), key.base))?;
            format!("just-quick:{fork}")
        }
    };
    Ok(Key {
        host: host::host_fingerprint(
            root,
            &environment.cargo,
            &host::step_variables(&key.environment),
        ),
        unit,
        tree: git::line(
            root,
            &["rev-parse", "--verify", &format!("{head}^{{tree}}")],
        )?,
    })
}

/// The latest run of `runs` with `key`, when it holds at `now`: green, within
/// a day, and with lint's other reads unchanged (`lint_holds`); or why not.
fn held<'a>(
    runs: &'a [QuickRun],
    key: &Key,
    now: u64,
    lint_holds: impl FnOnce() -> Result<(), String>,
) -> Result<&'a QuickRun, String> {
    let latest = runs
        .iter()
        .rev()
        .find(|run| run.host == key.host && run.unit == key.unit && run.outcome.tree == key.tree)
        .ok_or_else(|| {
            format!(
                "no run with this form and base is recorded on tree {} on this host",
                short(&key.tree)
            )
        })?;
    if !latest.outcome.green {
        return Err(format!(
            "its latest run on tree {}, by {} on {}, was red",
            short(&key.tree),
            latest.outcome.by.name(),
            short(&latest.head)
        ));
    }
    if past_max_age(now, latest.outcome.at) {
        return Err(format!(
            "its green run on tree {} is older than a day",
            short(&key.tree)
        ));
    }
    lint_holds()?;
    Ok(latest)
}

fn line(run: &QuickRun) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        run.host,
        run.unit,
        if run.outcome.green { "pass" } else { "fail" },
        run.head,
        run.outcome.tree,
        run.outcome.at,
        run.outcome.by.name()
    )
}

fn parse(text: &str) -> Result<Vec<QuickRun>> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [host, unit, status, head, tree, at, by] = fields.as_slice() else {
                bail!(
                    "a run's line is `<host>\\t<unit>\\t<status>\\t<head>\\t<tree>\\t<time>\\t<layer>`, not {line:?}"
                );
            };
            let green = match *status {
                "pass" => true,
                "fail" => false,
                other => bail!("the status of {unit} is pass or fail, not {other}"),
            };
            Ok(QuickRun {
                host: (*host).to_owned(),
                unit: (*unit).to_owned(),
                head: (*head).to_owned(),
                outcome: Outcome {
                    green,
                    origin: (*head).to_owned(),
                    tree: (*tree).to_owned(),
                    at: at
                        .parse()
                        .with_context(|| format!("the time of {unit} is {at:?}"))?,
                    by: Layer::from_name(by)
                        .with_context(|| format!("{unit} names no layer the gate knows: {by:?}"))?,
                    earned_on: None,
                },
            })
        })
        .collect()
}

/// The runs recorded in the gate's records `records`: none when the file is
/// missing, and an error when it cannot be read.
pub(crate) fn read(records: &Path) -> Result<Vec<QuickRun>> {
    let file = records.join(FILE);
    match std::fs::read_to_string(&file) {
        Ok(text) => parse(&text).with_context(|| format!("reading {}", file.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", file.display())),
    }
}

/// Each run as a gate run of its own on its head, which ran its one unit, for
/// the duplicate-run check.
pub(crate) fn as_gate_runs(runs: Vec<QuickRun>) -> Vec<GateRun> {
    runs.into_iter()
        .map(|run| GateRun {
            head: run.head,
            base: String::new(),
            host: run.host,
            units: BTreeMap::from([(run.unit, run.outcome)]),
            earned: BTreeMap::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Key, QuickRun, held, line, parse};
    use crate::gate::plan::{Layer, MAX_AGE, Outcome};

    const AT: u64 = 1_800_000_000;

    fn run(unit: &str, green: bool, at: u64, by: Layer) -> QuickRun {
        QuickRun {
            host: "host".to_owned(),
            unit: unit.to_owned(),
            head: "head".to_owned(),
            outcome: Outcome {
                green,
                origin: "head".to_owned(),
                tree: "tree".to_owned(),
                at,
                by,
                earned_on: None,
            },
        }
    }

    fn key(unit: &str) -> Key {
        Key {
            host: "host".to_owned(),
            unit: unit.to_owned(),
            tree: "tree".to_owned(),
        }
    }

    #[test]
    fn a_green_run_holds_for_its_own_key_alone() {
        // Lint's other reads, read again: unchanged.
        let holds = || Ok::<(), String>(());
        let runs = [run("just-quick:base", true, AT, Layer::Quick)];

        assert_eq!(
            held(&runs, &key("just-quick:base"), AT, holds),
            Ok(&runs[0])
        );
        assert!(held(&runs, &key("just-quick:other"), AT, holds).is_err());
        assert!(held(&runs, &key("just-quick-all"), AT, holds).is_err());
        let mut other_host = key("just-quick:base");
        other_host.host = "another host".to_owned();
        assert!(held(&runs, &other_host, AT, holds).is_err());
        let mut other_tree = key("just-quick:base");
        other_tree.tree = "another tree".to_owned();
        assert!(held(&runs, &other_tree, AT, holds).is_err());
    }

    #[test]
    fn a_red_latest_run_a_run_past_a_day_or_lint_reading_more_holds_nothing() {
        let holds = || Ok::<(), String>(());
        let green = run("just-quick:base", true, AT, Layer::Quick);
        let red_after = [green, run("just-quick:base", false, AT + 1, Layer::PrePush)];
        let red = held(&red_after, &key("just-quick:base"), AT + 1, holds).unwrap_err();
        assert!(red.contains("by pre-push on head, was red"), "{red}");

        let once = [run("just-quick:base", true, AT, Layer::Quick)];
        assert!(held(&once, &key("just-quick:base"), AT + MAX_AGE + 1, holds).is_err());

        let lint = held(&once, &key("just-quick:base"), AT, || {
            Err("lint's register check: an Exception expired".to_owned())
        });
        assert_eq!(
            lint,
            Err("lint's register check: an Exception expired".to_owned())
        );
    }

    #[test]
    fn a_run_reads_back_as_written_and_an_unreadable_line_is_an_error() {
        let written = run("just-quick-all", false, AT, Layer::PrePush);

        assert_eq!(parse(&line(&written)).unwrap(), [written]);
        assert!(parse("host\tjust-quick-all\tpass\thead\ttree\tnoon\tquick\n").is_err());
        assert!(parse("host\tjust-quick-all\tpass\n").is_err());
    }
}
