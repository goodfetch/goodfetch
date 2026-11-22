//! The gate's records, which every worktree of the repository shares in
//! `goodfetch-gate-runs/` of its common directory (`ci/gate-records.sh`;
//! ADR 0135's amendment of 2026-11-22) and which are never committed:
//! `<head>.tsv`, each unit's result for the run on that head; `<head>.plan`,
//! the plan that run follows; and `<head>.earned`, the clean passes of the
//! members its stress run has short of their counts.
//! A record is a `base\t<commit>` line and a `host\t<fingerprint>` line,
//! then one `<unit>\t<pass|fail>\t<origin commit>\t<origin tree>\t<unix time>\t<layer>`
//! line per result, the layer being the one that ran it ([`Layer`]), with
//! `\t<commit>` after it for a stress result that counted passes from an
//! earlier head; the last line for a unit wins, so a run appends each step's
//! result as it ends.
//! An earned file is one `<member>\t<origin commit>\t<unix time>\t<passes>\t<test>`
//! line per test. For a plan or a selection, one that is missing or cannot be
//! read earns nothing; `gate record`, which must mark what the selection
//! counted, fails instead.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};

use super::plan::{EarnedPasses, GateRun, Layer, Outcome};
use crate::output;

const PASS: &str = "pass";
const FAIL: &str = "fail";

pub(crate) fn parse(head: &str, text: &str) -> Result<GateRun> {
    let mut base = None;
    let mut host = None;
    let mut units = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["base", commit] => base = Some((*commit).to_owned()),
            ["host", fingerprint] => host = Some((*fingerprint).to_owned()),
            [unit, status, origin, tree, at, by, earned_on @ ..] if earned_on.len() <= 1 => {
                let green = match *status {
                    PASS => true,
                    FAIL => false,
                    other => bail!("the status of {unit} is pass or fail, not {other}"),
                };
                let by = Layer::from_name(by)
                    .with_context(|| format!("{unit} names no layer the gate knows: {by:?}"))?;
                units.insert(
                    (*unit).to_owned(),
                    Outcome {
                        green,
                        origin: (*origin).to_owned(),
                        tree: (*tree).to_owned(),
                        at: at
                            .parse()
                            .with_context(|| format!("the time of {unit} is {at:?}"))?,
                        by,
                        earned_on: earned_on.first().map(|commit| (*commit).to_owned()),
                    },
                );
            }
            _ => bail!(
                "a record line is `<unit>\\t<status>\\t<origin>\\t<tree>\\t<time>\\t<layer>`, not {line:?}"
            ),
        }
    }
    Ok(GateRun {
        head: head.to_owned(),
        base: base.context("a record names its base on a `base` line")?,
        host: host.context("a record names its host on a `host` line")?,
        units,
        earned: BTreeMap::new(),
    })
}

pub(crate) fn render(record: &GateRun) -> String {
    let mut text = format!("base\t{}\nhost\t{}\n", record.base, record.host);
    for (unit, outcome) in &record.units {
        text.push_str(&line(unit, outcome));
    }
    text
}

/// One unit's line.
pub(crate) fn line(unit: &str, outcome: &Outcome) -> String {
    let mut text = String::new();
    let status = if outcome.green { PASS } else { FAIL };
    let _ = write!(
        text,
        "{unit}\t{status}\t{}\t{}\t{}\t{}",
        outcome.origin,
        outcome.tree,
        outcome.at,
        outcome.by.name()
    );
    if let Some(earned_on) = &outcome.earned_on {
        let _ = write!(text, "\t{earned_on}");
    }
    text.push('\n');
    text
}

/// The earned passes an earned file's text holds; a line that cannot be read,
/// or a member's lines that disagree on where its passes began, is an error.
pub(crate) fn parse_earned(text: &str) -> Result<BTreeMap<String, EarnedPasses>> {
    let mut earned: BTreeMap<String, EarnedPasses> = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [member, origin, at, passes, id] = fields.as_slice() else {
            bail!(
                "an earned line is `<member>\\t<origin>\\t<time>\\t<passes>\\t<test>`, not {line:?}"
            );
        };
        let at = at
            .parse()
            .with_context(|| format!("the time of {member} is {at:?}"))?;
        let passes = passes
            .parse()
            .with_context(|| format!("the passes of {id} are {passes:?}"))?;
        let held = earned
            .entry((*member).to_owned())
            .or_insert_with(|| EarnedPasses {
                origin: (*origin).to_owned(),
                at,
                passes: BTreeMap::new(),
            });
        ensure!(
            held.origin == *origin && held.at == at,
            "the lines of {member} name two heads or times"
        );
        held.passes.insert((*id).to_owned(), passes);
    }
    Ok(earned)
}

pub(crate) fn render_earned(earned: &BTreeMap<String, EarnedPasses>) -> String {
    let mut text = String::new();
    for (member, held) in earned {
        for (test, passes) in &held.passes {
            let _ = writeln!(
                text,
                "{member}\t{}\t{}\t{passes}\t{test}",
                held.origin, held.at
            );
        }
    }
    text
}

/// What each member of `clean` keeps after a red stress run on `head` that
/// began at `started`: the passes it had in `earned` and those the run gave
/// it, begun where and when its earliest run did. No other member keeps any:
/// one that went red, one that has its counts, and every one when the files do
/// not account for the failure.
pub(crate) fn add_earned(
    earned: &BTreeMap<String, EarnedPasses>,
    clean: BTreeMap<String, BTreeMap<String, u32>>,
    head: &str,
    started: u64,
) -> BTreeMap<String, EarnedPasses> {
    clean
        .into_iter()
        .filter_map(|(member, passes)| {
            let mut kept = earned
                .get(&member)
                .cloned()
                .unwrap_or_else(|| EarnedPasses {
                    origin: head.to_owned(),
                    at: started,
                    passes: BTreeMap::new(),
                });
            for (test, count) in passes {
                *kept.passes.entry(test).or_default() += count;
            }
            kept.passes.retain(|_, count| *count > 0);
            (!kept.passes.is_empty()).then_some((member, kept))
        })
        .collect()
}

/// The earned file of the run on `head`.
pub(crate) fn earned_file(folder: &Path, head: &str) -> PathBuf {
    folder.join(format!("{head}.earned"))
}

/// The earned passes in `file`: none when it is missing, and an error when it
/// cannot be read.
pub(crate) fn read_earned_strictly(file: &Path) -> Result<BTreeMap<String, EarnedPasses>> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", file.display())),
    };
    parse_earned(&text).with_context(|| format!("reading {}", file.display()))
}

/// The earned passes of the run on `head`, for its record. The selection counted
/// what the plan kept, the members `kept`, and a green result must carry that
/// mark for the backstop: a file that cannot be read or has lost one of them is
/// an error, never a green result without it.
pub(crate) fn earned_the_plan_kept(
    folder: &Path,
    head: &str,
    kept: &[String],
) -> Result<BTreeMap<String, EarnedPasses>> {
    let file = earned_file(folder, head);
    let earned = read_earned_strictly(&file)?;
    if let Some(lost) = kept.iter().find(|member| !earned.contains_key(*member)) {
        bail!(
            "the plan kept passes of {lost}, and {} no longer holds them",
            file.display()
        );
    }
    Ok(earned)
}

/// The earned passes in `file`: none when it is missing or cannot be read,
/// which costs a rerun of the passes and never counts one that did not run.
pub(crate) fn read_earned(file: &Path) -> BTreeMap<String, EarnedPasses> {
    read_earned_strictly(file).unwrap_or_else(|error| {
        output::error(&format!("Gate: no earned passes: {error:#}"));
        BTreeMap::new()
    })
}

/// Writes `earned` as the run on `head`'s, and removes the file when there are
/// none.
pub(crate) fn write_earned(
    folder: &Path,
    head: &str,
    earned: &BTreeMap<String, EarnedPasses>,
) -> Result<()> {
    let file = earned_file(folder, head);
    if earned.is_empty() {
        return match std::fs::remove_file(&file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", file.display()))
            }
            _ => Ok(()),
        };
    }
    std::fs::write(&file, render_earned(earned))
        .with_context(|| format!("writing {}", file.display()))
}

/// The record file of the run on `head`.
pub(crate) fn file(folder: &Path, head: &str) -> PathBuf {
    folder.join(format!("{head}.tsv"))
}

/// The plan file of the run on `head`.
pub(crate) fn plan_file(folder: &Path, head: &str) -> PathBuf {
    folder.join(format!("{head}.plan"))
}

/// The heads that have a record in `folder`.
pub(crate) fn heads(folder: &Path) -> Result<Vec<String>> {
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut heads = Vec::new();
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry
            .with_context(|| format!("reading {}", folder.display()))?
            .path();
        if path.extension().is_some_and(|extension| extension == "tsv")
            && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        {
            heads.push(stem.to_owned());
        }
    }
    heads.sort();
    Ok(heads)
}

pub(crate) fn read(folder: &Path, head: &str) -> Result<GateRun> {
    let path = file(folder, head);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut run = parse(head, &text).with_context(|| format!("reading {}", path.display()))?;
    run.earned = read_earned(&earned_file(folder, head));
    Ok(run)
}
