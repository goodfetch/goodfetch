//! The gate's records, kept with the gate's worktree under
//! `target/gate-runs/` and never committed: `<head>.tsv`, each unit's result
//! for the run on that head, and `<head>.plan`, the plan that run follows.
//! A record is a `base\t<commit>` line and a `host\t<fingerprint>` line,
//! then one `<unit>\t<pass|fail>\t<origin commit>\t<origin tree>\t<unix time>`
//! line per result; the last line for a unit wins, so a run appends each
//! step's result as it ends.
//!
//! Every line ends in `\t<mac>`, the HMAC-SHA256 of the run's head and the
//! line under the host's gate key, a file kept outside the gate's worktree
//! ([`Key`]): a line the key did not sign for that head, which a test, a
//! tool or a hand wrote, edited or copied from another run's record, makes
//! the whole record unreadable, so nothing of it is reused (ADR 0135).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;

use super::plan::{GateRun, Outcome};

const PASS: &str = "pass";
const FAIL: &str = "fail";

/// The host's gate key, which signs every record line. Its file holds 64
/// hex digits, made once from the operating system's random source and
/// readable by its owner only; the key is those digits' bytes.
pub(crate) struct Key(Vec<u8>);

impl Key {
    #[cfg(test)]
    pub(crate) const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The key in `path`.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the gate's key in {}", path.display()))?;
        let key = text.trim();
        if key.len() < 32 {
            bail!("the gate's key in {} is too short", path.display());
        }
        Ok(Self(key.as_bytes().to_vec()))
    }

    /// The key in `path`, made there first when there is none.
    pub(crate) fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let mut bytes = [0_u8; 32];
            getrandom::fill(&mut bytes)
                .map_err(|error| anyhow::anyhow!("reading random bytes: {error}"))?;
            let digits = bytes.iter().fold(String::new(), |mut digits, byte| {
                let _ = write!(digits, "{byte:02x}");
                digits
            });
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder)
                    .with_context(|| format!("creating {}", folder.display()))?;
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options
                .open(path)
                .with_context(|| format!("creating the gate's key in {}", path.display()))?;
            std::io::Write::write_all(&mut file, format!("{digits}\n").as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
        }
        Self::load(path)
    }

    fn mac(&self, head: &str, line: &str) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0)
            .expect("invariant: HMAC takes a key of any length");
        mac.update(format!("{head}\n{line}").as_bytes());
        mac
    }

    fn sign(&self, head: &str, line: &str) -> String {
        format!("{:x}", self.mac(head, line).finalize().into_bytes())
    }

    fn signed(&self, head: &str, line: &str, mac: &str) -> bool {
        let bytes: Option<Vec<u8>> = (0..mac.len())
            .step_by(2)
            .map(|at| {
                mac.get(at..at + 2)
                    .and_then(|digits| u8::from_str_radix(digits, 16).ok())
            })
            .collect();
        bytes.is_some_and(|bytes| self.mac(head, line).verify_slice(&bytes).is_ok())
    }
}

/// `line`, without its line break, signed for the run on `head`.
pub(crate) fn signed_line(head: &str, line: &str, key: &Key) -> String {
    format!("{line}\t{}\n", key.sign(head, line))
}

pub(crate) fn parse(head: &str, text: &str, key: &Key) -> Result<GateRun> {
    let mut base = None;
    let mut host = None;
    let mut units = BTreeMap::new();
    for (number, signed) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        let line = match signed.rsplit_once('\t') {
            Some((line, mac)) if key.signed(head, line, mac) => line,
            _ => bail!(
                "line {} of the record of {head} is not signed by this host's gate key",
                number + 1
            ),
        };
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

pub(crate) fn render(record: &GateRun, key: &Key) -> String {
    let mut text = signed_line(&record.head, &format!("base\t{}", record.base), key);
    text.push_str(&signed_line(
        &record.head,
        &format!("host\t{}", record.host),
        key,
    ));
    for (unit, outcome) in &record.units {
        text.push_str(&line(&record.head, unit, outcome, key));
    }
    text
}

/// One unit's line in the record of the run on `head`.
pub(crate) fn line(head: &str, unit: &str, outcome: &Outcome, key: &Key) -> String {
    let mut text = String::new();
    let status = if outcome.green { PASS } else { FAIL };
    let _ = write!(
        text,
        "{unit}\t{status}\t{}\t{}\t{}",
        outcome.origin, outcome.tree, outcome.at
    );
    signed_line(head, &text, key)
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

pub(crate) fn read(folder: &Path, head: &str, key: &Key) -> Result<GateRun> {
    let path = file(folder, head);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse(head, &text, key).with_context(|| format!("reading {}", path.display()))
}
