//! The gate's records, kept with the gate's worktree under
//! `target/gate-runs/` and never committed: `<head>.tsv`, each unit's result
//! for the run on that head, and `<head>.plan`, the plan that run follows.
//! A record is a `base\t<commit>` line and a `host\t<fingerprint>` line,
//! then one `<unit>\t<pass|fail>\t<origin commit>\t<origin tree>\t<unix time>`
//! line per result; the last line for a unit wins, so a run appends each
//! step's result as it ends.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::plan::{GateRun, Outcome};

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
            [unit, status, origin, tree, at] => {
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
    let _ = writeln!(
        text,
        "{unit}\t{status}\t{}\t{}\t{}",
        outcome.origin, outcome.tree, outcome.at
    );
    text
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
    parse(head, &text).with_context(|| format!("reading {}", path.display()))
}
