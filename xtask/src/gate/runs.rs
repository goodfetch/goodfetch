//! The gate's records, kept with the gate's worktree under
//! `target/gate-runs/` and never committed: `<head>.tsv`, each unit's result
//! for the run on that head; `<head>.plan`, the plan that run follows; and
//! `<head>.credit`, the clean passes of the members its stress run has short
//! of their counts.
//! A record is a `base\t<commit>` line and a `host\t<fingerprint>` line,
//! then one `<unit>\t<pass|fail>\t<origin commit>\t<origin tree>\t<unix time>`
//! line per result, with `\t<commit>` after it for a stress result that counted
//! passes from an earlier head; the last line for a unit wins, so a run appends
//! each step's result as it ends.
//! A credit file is one `<member>\t<origin commit>\t<unix time>\t<passes>\t<test>`
//! line per test. One that is missing or cannot be read is no credit.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};

use super::plan::{Credit, GateRun, Outcome};
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
            [unit, status, origin, tree, at, credited @ ..] if credited.len() <= 1 => {
                let green = match *status {
                    PASS => true,
                    FAIL => false,
                    other => bail!("the status of {unit} is pass or fail, not {other}"),
                };
                units.insert(
                    (*unit).to_owned(),
                    Outcome {
                        green,
                        origin: (*origin).to_owned(),
                        tree: (*tree).to_owned(),
                        at: at
                            .parse()
                            .with_context(|| format!("the time of {unit} is {at:?}"))?,
                        credited: credited.first().map(|commit| (*commit).to_owned()),
                    },
                );
            }
            _ => bail!(
                "a record line is `<unit>\\t<status>\\t<origin>\\t<tree>\\t<time>`, not {line:?}"
            ),
        }
    }
    Ok(GateRun {
        head: head.to_owned(),
        base: base.context("a record names its base on a `base` line")?,
        host: host.context("a record names its host on a `host` line")?,
        units,
        credits: BTreeMap::new(),
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
        "{unit}\t{status}\t{}\t{}\t{}",
        outcome.origin, outcome.tree, outcome.at
    );
    if let Some(credited) = &outcome.credited {
        let _ = write!(text, "\t{credited}");
    }
    text.push('\n');
    text
}

/// The credits a credit file's text holds; a line that cannot be read, or a
/// member's lines that disagree on where its passes began, is an error.
pub(crate) fn parse_credits(text: &str) -> Result<BTreeMap<String, Credit>> {
    let mut credits: BTreeMap<String, Credit> = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [member, origin, at, passes, id] = fields.as_slice() else {
            bail!(
                "a credit line is `<member>\\t<origin>\\t<time>\\t<passes>\\t<test>`, not {line:?}"
            );
        };
        let at = at
            .parse()
            .with_context(|| format!("the time of {member} is {at:?}"))?;
        let passes = passes
            .parse()
            .with_context(|| format!("the passes of {id} are {passes:?}"))?;
        let credit = credits
            .entry((*member).to_owned())
            .or_insert_with(|| Credit {
                origin: (*origin).to_owned(),
                at,
                passes: BTreeMap::new(),
            });
        ensure!(
            credit.origin == *origin && credit.at == at,
            "the lines of {member} name two heads or times"
        );
        credit.passes.insert((*id).to_owned(), passes);
    }
    Ok(credits)
}

pub(crate) fn render_credits(credits: &BTreeMap<String, Credit>) -> String {
    let mut text = String::new();
    for (member, credit) in credits {
        for (test, passes) in &credit.passes {
            let _ = writeln!(
                text,
                "{member}\t{}\t{}\t{passes}\t{test}",
                credit.origin, credit.at
            );
        }
    }
    text
}

/// What each member of `clean` keeps after a red stress run on `head`: the
/// passes it had in `credits` and those the run gave it, begun where and when
/// its earliest ran. No other member keeps any: one that went red, one that has
/// its counts, and every one when the files do not account for the failure.
pub(crate) fn earned(
    credits: &BTreeMap<String, Credit>,
    clean: BTreeMap<String, BTreeMap<String, u32>>,
    head: &str,
    now: u64,
) -> BTreeMap<String, Credit> {
    clean
        .into_iter()
        .filter_map(|(member, passes)| {
            let mut credit = credits.get(&member).cloned().unwrap_or_else(|| Credit {
                origin: head.to_owned(),
                at: now,
                passes: BTreeMap::new(),
            });
            for (test, count) in passes {
                *credit.passes.entry(test).or_default() += count;
            }
            credit.passes.retain(|_, count| *count > 0);
            (!credit.passes.is_empty()).then_some((member, credit))
        })
        .collect()
}

/// The credit file of the run on `head`.
pub(crate) fn credit_file(folder: &Path, head: &str) -> PathBuf {
    folder.join(format!("{head}.credit"))
}

/// The credits in `file`: none when it is missing or cannot be read, which
/// costs a rerun of the passes and never counts one that did not run.
pub(crate) fn read_credits(file: &Path) -> BTreeMap<String, Credit> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(error) => {
            output::error(&format!(
                "Gate: {} gives no credit: {error}",
                file.display()
            ));
            return BTreeMap::new();
        }
    };
    parse_credits(&text).unwrap_or_else(|error| {
        output::error(&format!(
            "Gate: {} gives no credit: {error:#}",
            file.display()
        ));
        BTreeMap::new()
    })
}

/// Writes `credits` as the run on `head`'s, and removes the file when there
/// are none.
pub(crate) fn write_credits(
    folder: &Path,
    head: &str,
    credits: &BTreeMap<String, Credit>,
) -> Result<()> {
    let file = credit_file(folder, head);
    if credits.is_empty() {
        return match std::fs::remove_file(&file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", file.display()))
            }
            _ => Ok(()),
        };
    }
    std::fs::write(&file, render_credits(credits))
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
    run.credits = read_credits(&credit_file(folder, head));
    Ok(run)
}
