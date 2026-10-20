//! The ceilings file, `ci/rss-ceilings.toml`: a whole number of MiB per
//! Product, platform and scenario, committed only with the owner's approval
//! (ADRs 0071 and 0094), and the proposal it is written from.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;

use crate::check::Finding;

use super::peaks::{Peak, parse_name};
use super::platform::Platform;
use super::scenario::Scenario;

const MIB: u64 = 1024 * 1024;

/// ADR 0071: a ceiling is proposed from 20 runs.
pub(crate) const RUNS: usize = 20;

/// The key of a ceiling and of the peaks it holds.
pub(crate) type Key = (Product, Platform, Scenario);

/// The committed ceilings, in bytes.
pub(crate) struct Ceilings {
    ceilings: BTreeMap<Key, u64>,
}

impl Ceilings {
    pub(crate) fn get(&self, key: Key) -> Option<u64> {
        self.ceilings.get(&key).copied()
    }
}

/// The file's shape: `[<product>.<platform>]` tables of `<scenario> = <MiB>`.
type File = BTreeMap<String, BTreeMap<String, BTreeMap<String, u64>>>;

/// The ceilings at `path`, or `None` when no file is there.
pub(crate) fn read(path: &Path) -> Result<Option<Ceilings>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let file: File =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut ceilings = BTreeMap::new();
    for (product, platforms) in file {
        let product: Product = parse_name(&product)?;
        for (platform, scenarios) in platforms {
            let platform: Platform = parse_name(&platform)?;
            for (scenario, mebibytes) in scenarios {
                let scenario: Scenario = parse_name(&scenario)?;
                ceilings.insert((product, platform, scenario), mebibytes * MIB);
            }
        }
    }
    Ok(Some(Ceilings { ceilings }))
}

/// The ceilings file that holds 1.5 times the highest peak of each
/// Product, platform and scenario, rounded up to a whole MiB; or a finding
/// for each one with fewer than [`RUNS`] runs.
pub(crate) fn propose(peaks: &[Peak]) -> (String, Vec<Finding>) {
    let mut runs: BTreeMap<Key, Vec<u64>> = BTreeMap::new();
    for peak in peaks {
        runs.entry((peak.product, peak.platform, peak.scenario))
            .or_default()
            .push(peak.bytes);
    }
    let findings: Vec<Finding> = runs
        .iter()
        .filter(|(_, bytes)| bytes.len() < RUNS)
        .map(|((product, platform, scenario), bytes)| {
            Finding::new(
                "rss-runs",
                subject(*product, *platform, *scenario),
                format!(
                    "{} runs, and a ceiling is proposed from {RUNS} (ADR 0071)",
                    bytes.len()
                ),
            )
        })
        .collect();
    let mut text = format!(
        "# Proposed by `xtask rss propose`: 1.5 times the highest peak of {RUNS} runs, rounded up to a whole MiB (ADR 0071).\n"
    );
    let mut table = None;
    for ((product, platform, scenario), bytes) in &runs {
        let highest = bytes.iter().copied().max().unwrap_or_default();
        let ceiling = (u128::from(highest) * 3).div_ceil(2 * u128::from(MIB));
        if table != Some((*product, *platform)) {
            let _ = write!(text, "\n[{}.{}]\n", product.as_str(), platform.name());
            table = Some((*product, *platform));
        }
        let _ = writeln!(text, "{} = {ceiling}", scenario.name());
    }
    (text, findings)
}

/// How a finding and a JUnit test case name a peak.
pub(crate) fn subject(product: Product, platform: Platform, scenario: Scenario) -> String {
    format!(
        "{} {} {}",
        product.as_str(),
        scenario.name(),
        platform.name()
    )
}
