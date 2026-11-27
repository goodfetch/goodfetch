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
//!
//! Every local record line ends in `\t<mac>`, the HMAC-SHA256 of the run's
//! head and the line under the host's gate key, a file kept outside every
//! worktree ([`Key`]): a line the key did not sign for that head, which a
//! test, a tool or a hand wrote, edited or copied from another run's record,
//! makes the whole record unreadable, so nothing of it is reused (ADR 0135).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;

use super::plan::{EarnedPasses, GateRun, Layer, Outcome};
use crate::output;

const PASS: &str = "pass";
const FAIL: &str = "fail";

/// The host's gate key, which signs every local record line. Its file holds
/// 64 hex digits made once from the operating system's random source and
/// readable by its owner only; the key is those digits' bytes.
pub(crate) struct Key(Vec<u8>);

impl Key {
    #[cfg(test)]
    pub(crate) const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Default path of this host's gate key, outside every worktree:
    /// `GOODFETCH_GATE_KEY`, else `$XDG_CONFIG_HOME/goodfetch/gate-records.key`,
    /// else `$HOME/.config/goodfetch/gate-records.key`.
    pub(crate) fn default_path() -> Result<PathBuf> {
        if let Some(explicit) = std::env::var_os("GOODFETCH_GATE_KEY")
            && !explicit.is_empty()
        {
            return Ok(PathBuf::from(explicit));
        }
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            && !xdg.is_empty()
        {
            return Ok(PathBuf::from(xdg).join("goodfetch/gate-records.key"));
        }
        if let Some(home) = std::env::var_os("HOME")
            && !home.is_empty()
        {
            return Ok(PathBuf::from(home).join(".config/goodfetch/gate-records.key"));
        }
        bail!("cannot locate the host's gate key: set GOODFETCH_GATE_KEY or HOME")
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

    /// The key in `path`, created atomically with mode 0600 when absent.
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
            let mut nonce = [0_u8; 8];
            getrandom::fill(&mut nonce)
                .map_err(|error| anyhow::anyhow!("reading random nonce: {error}"))?;
            let nonce_hex = nonce.iter().fold(String::new(), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            });
            let tmp_name = format!(".gate-key.{}.{nonce_hex}.tmp", std::process::id());
            let tmp = path.with_file_name(tmp_name);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options
                .open(&tmp)
                .with_context(|| format!("creating temporary key {}", tmp.display()))?;
            let write_result =
                std::io::Write::write_all(&mut file, format!("{digits}\n").as_bytes());
            drop(file);
            if let Err(error) = write_result {
                let _ = std::fs::remove_file(&tmp);
                return Err(error).with_context(|| format!("writing {}", tmp.display()));
            }
            match std::fs::hard_link(&tmp, path) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&tmp);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_file(&tmp);
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(error).with_context(|| {
                        format!("installing the gate's key in {}", path.display())
                    });
                }
            }
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
        let bytes = self.mac(head, line).finalize().into_bytes();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    fn signed(&self, head: &str, line: &str, mac: &str) -> bool {
        if mac.len() != 64 {
            return false;
        }
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

fn is_mac_field(field: &str) -> bool {
    field.len() == 64 && field.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_payload_line(
    line: &str,
    base: &mut Option<String>,
    host: &mut Option<String>,
    units: &mut BTreeMap<String, Outcome>,
) -> Result<()> {
    let fields: Vec<&str> = line.split('\t').collect();
    match fields.as_slice() {
        ["base", commit] => *base = Some((*commit).to_owned()),
        ["host", fingerprint] => *host = Some((*fingerprint).to_owned()),
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
    Ok(())
}

/// Parses a local record, requiring every non-empty line to carry a valid
/// HMAC-SHA256 signature under `key` for `head`.
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
        parse_payload_line(line, &mut base, &mut host, &mut units)?;
    }
    Ok(GateRun {
        head: head.to_owned(),
        base: base.context("a record names its base on a `base` line")?,
        host: host.context("a record names its host on a `host` line")?,
        units,
        earned: BTreeMap::new(),
    })
}

/// Parses a hosted run's record produced on another host (GitHub Actions),
/// stripping a trailing MAC field if the hosted runner signed its own copy.
pub(crate) fn parse_unsigned(head: &str, text: &str) -> Result<GateRun> {
    let mut base = None;
    let mut host = None;
    let mut units = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let stripped = match line.rsplit_once('\t') {
            Some((rest, tail)) if is_mac_field(tail) => {
                let count = rest.split('\t').count();
                if matches!(count, 2 | 6 | 7) {
                    rest
                } else {
                    line
                }
            }
            _ => line,
        };
        parse_payload_line(stripped, &mut base, &mut host, &mut units)?;
    }
    Ok(GateRun {
        head: head.to_owned(),
        base: base.context("a record names its base on a `base` line")?,
        host: host.context("a record names its host on a `host` line")?,
        units,
        earned: BTreeMap::new(),
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

/// One unit's signed line in the record of the run on `head`.
pub(crate) fn line(head: &str, unit: &str, outcome: &Outcome, key: &Key) -> String {
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
    signed_line(head, &text, key)
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

/// Each test's clean passes in the earned file `xtask gate plan` wrote, which
/// `xtask stress select` takes from their counts; none when it is missing or
/// cannot be read, and none of a member's past the age limit by now, since the
/// stress run reads the file later than the plan that kept them.
pub(crate) fn earned_passes(file: &Path) -> BTreeMap<String, u32> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(u64::MAX, |since_the_epoch| since_the_epoch.as_secs());
    read_earned(file)
        .into_values()
        .filter(|earned| !super::plan::past_max_age(now, earned.at))
        .flat_map(|earned| earned.passes)
        .collect()
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

pub(crate) fn read(folder: &Path, head: &str, key: &Key) -> Result<GateRun> {
    let path = file(folder, head);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut run = parse(head, &text, key).with_context(|| format!("reading {}", path.display()))?;
    run.earned = read_earned(&earned_file(folder, head));
    Ok(run)
}
