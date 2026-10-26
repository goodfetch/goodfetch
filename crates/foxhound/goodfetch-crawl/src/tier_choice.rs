//! The crawl's tier choice (spec #582's `goodfetch-crawl`; ADRs 0020, 0023
//! and 0093; CO-15): under `fetch.tier: auto`, the tier each crawl request
//! takes is learned per Label on a domain key by sample comparison, and
//! kept in the run file so a resumed crawl takes the same tiers.
//!
//! [`TierChoices::serve_attempt`] plans an attempt: the first
//! [`SAMPLES`] requests of a Label on a domain are samples, each run on
//! `http` and then on `browser`, the browser's run the one that commits;
//! [`TierChoices::classify`] then takes whether the two runs matched. A
//! Label whose samples all matched is learned onto `http`. One whose
//! samples disagree by the first segment of their URL paths, some segment
//! having matched throughout, is split by that segment: each segment keeps
//! a tier of its own, `http` once its own [`SAMPLES`] samples all matched,
//! `browser` at its first sample that did not, so a segment whose own
//! samples disagree stays on `browser`. At most [`MAX_SEGMENTS`] segments
//! are kept per Label and domain; the rest share the Label's choice,
//! `browser`. Any other Label is learned onto `browser`. A choice once made
//! stands for the Run: re-sampling is #768's.
//!
//! What "matched" means, the sample comparison's weighing, is area 5's
//! (#768); until it is built, [`matches`] stands in: the two runs pushed
//! the same Records, in any order, and the cheaper one neither escalated
//! nor failed.
//!
//! The hand-over of a Collie page on the planned tier, and the attempt's
//! runs, are the delivery's (`attempt`).

use std::collections::BTreeMap;

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_fetch_tier::TierChoice;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::tier_race::Raced;

/// How many samples a Label on a domain, or one of its segments, takes
/// before it moves to `http` (`fetch.sample.count`'s built-in, #768).
pub(crate) const SAMPLES: usize = 3;

/// How many path segments a split Label keeps a tier for on one domain.
pub(crate) const MAX_SEGMENTS: usize = 16;

/// The table that keeps what each Label on each domain learned.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl-tier-choices",
    statements: &["CREATE TABLE IF NOT EXISTS tier_choices (
            label TEXT NOT NULL,
            domain_key TEXT NOT NULL,
            unit TEXT NOT NULL,
            PRIMARY KEY (label, domain_key)
        ) STRICT"],
};

/// How one attempt of a crawl request runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttemptPlan {
    /// No tier: the handler is given its request alone, as under the
    /// built-in `fetch.tier` until #1602.
    Alone,
    /// On this tier, forced or learned.
    On(TierChoice),
    /// A sample: on `http`, then on `browser`, whose run commits.
    Sample(Sampled),
    /// A race of browser-fetch against a navigation (`fetch.tier_race`;
    /// the `tier_race` module), whose winner's run commits.
    Race(Raced),
}

/// The unit a sample counts for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sampled {
    pub(crate) label: String,
    pub(crate) domain_key: String,
    /// The first segment of its URL's path.
    pub(crate) segment: String,
}

/// What every Label on every domain of a Run learned so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TierChoices {
    units: BTreeMap<(String, String), Unit>,
}

/// What one Label on one domain learned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Unit {
    /// Its first samples, in the order they were classified.
    Sampling { samples: Vec<Sample> },
    /// One tier for every request.
    Learned { tier: FetchTier },
    /// A tier per path segment, in the order the segments came; any other
    /// segment takes `browser`.
    Split { segments: Vec<Segment> },
}

/// One classified sample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sample {
    segment: String,
    matched: bool,
}

/// One segment of a split Label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Segment {
    name: String,
    /// Its samples that matched so far.
    matched: usize,
    /// Its tier, once chosen.
    tier: Option<FetchTier>,
}

impl TierChoices {
    /// How an attempt of a request of `label` at `url` on `domain_key`
    /// runs under `auto`.
    pub(crate) fn serve_attempt(&self, label: &str, domain_key: &str, url: &str) -> AttemptPlan {
        let segment = segment(url);
        let unit = self.units.get(&(label.to_owned(), domain_key.to_owned()));
        match unit.map_or(Some(Plan::Sample), |unit| unit.plan(&segment)) {
            Some(Plan::Sample) | None => AttemptPlan::Sample(Sampled {
                label: label.to_owned(),
                domain_key: domain_key.to_owned(),
                segment,
            }),
            Some(Plan::On(tier)) => AttemptPlan::On(TierChoice { tier, forced: None }),
        }
    }

    /// Takes the sample `sampled`, whose runs `matched` or not; answers
    /// its unit as it now stands, for the run file.
    pub(crate) fn classify(&mut self, sampled: &Sampled, matched: bool) -> &Unit {
        let unit = self
            .units
            .entry((sampled.label.clone(), sampled.domain_key.clone()))
            .or_insert_with(|| Unit::Sampling {
                samples: Vec::new(),
            });
        unit.classify(&sampled.segment, matched);
        unit
    }

    /// Every unit, by Label and domain key.
    #[cfg(test)]
    pub(crate) fn units(&self) -> impl Iterator<Item = (&str, &str, &Unit)> {
        self.units
            .iter()
            .map(|((label, key), unit)| (label.as_str(), key.as_str(), unit))
    }

    /// The segments `label` keeps a tier for on `domain_key`.
    #[cfg(test)]
    pub(crate) fn segments(&self, label: &str, domain_key: &str) -> Vec<String> {
        match self.units.get(&(label.to_owned(), domain_key.to_owned())) {
            Some(Unit::Split { segments }) => segments
                .iter()
                .map(|segment| segment.name.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// What a unit plans for one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    Sample,
    On(FetchTier),
}

impl Unit {
    /// The plan for a request in `segment`; `None` for a sample that opens
    /// a segment of a split Label.
    fn plan(&self, segment: &str) -> Option<Plan> {
        match self {
            Self::Sampling { .. } => Some(Plan::Sample),
            Self::Learned { tier } => Some(Plan::On(*tier)),
            Self::Split { segments } => match segments.iter().find(|kept| kept.name == segment) {
                Some(Segment {
                    tier: Some(tier), ..
                }) => Some(Plan::On(*tier)),
                Some(Segment { tier: None, .. }) => Some(Plan::Sample),
                None if segments.len() < MAX_SEGMENTS => None,
                // The rest share the Label's choice.
                None => Some(Plan::On(FetchTier::Browser)),
            },
        }
    }

    /// Takes a sample in `segment` that `matched` or not. A choice once
    /// made stands.
    fn classify(&mut self, segment: &str, matched: bool) {
        match self {
            Self::Learned { .. } => {}
            Self::Sampling { samples } => {
                samples.push(Sample {
                    segment: segment.to_owned(),
                    matched,
                });
                if samples.len() >= SAMPLES {
                    *self = Self::chosen(samples);
                }
            }
            Self::Split { segments } => {
                Segment::take(segments, segment, matched);
            }
        }
    }

    /// What a Label on a domain learned from its first `samples`: `http`
    /// when every one matched; split by first segment when some segment
    /// matched throughout and another did not; `browser` otherwise.
    fn chosen(samples: &[Sample]) -> Self {
        if samples.iter().all(|sample| sample.matched) {
            return Self::Learned {
                tier: FetchTier::Http,
            };
        }
        let mut segments: Vec<Segment> = Vec::new();
        for sample in samples {
            Segment::take(&mut segments, &sample.segment, sample.matched);
        }
        let some_matched = segments
            .iter()
            .any(|kept| kept.tier != Some(FetchTier::Browser));
        if segments.len() > 1 && some_matched {
            Self::Split { segments }
        } else {
            Self::Learned {
                tier: FetchTier::Browser,
            }
        }
    }
}

impl Segment {
    /// Takes a sample in `segment` that `matched` or not among `segments`,
    /// opening the segment while fewer than [`MAX_SEGMENTS`] are kept.
    fn take(segments: &mut Vec<Self>, segment: &str, matched: bool) {
        match segments.iter().position(|kept| kept.name == segment) {
            Some(found) => segments[found].classify(matched),
            None if segments.len() < MAX_SEGMENTS => {
                let mut kept = Self {
                    name: segment.to_owned(),
                    matched: 0,
                    tier: None,
                };
                kept.classify(matched);
                segments.push(kept);
            }
            None => {}
        }
    }

    /// Takes one of its samples: its first that did not match keeps it on
    /// `browser`; its [`SAMPLES`]th that matched, all before it matching,
    /// moves it to `http`.
    fn classify(&mut self, matched: bool) {
        if self.tier.is_some() {
            return;
        }
        if !matched {
            self.tier = Some(FetchTier::Browser);
            return;
        }
        self.matched += 1;
        if self.matched >= SAMPLES {
            self.tier = Some(FetchTier::Http);
        }
    }
}

/// The first segment of `url`'s path, empty for its root or a URL that
/// does not parse.
pub(crate) fn segment(url: &str) -> String {
    goodfetch_frontier::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path()
                .split('/')
                .find(|segment| !segment.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Whether a sample's cheaper run, which pushed `cheaper` (`None` when it
/// escalated or failed), matched its browser run, which pushed `browser`.
pub(crate) fn matches(cheaper: Option<&[PushedRecord]>, browser: &[PushedRecord]) -> bool {
    let Some(cheaper) = cheaper else {
        return false;
    };
    if cheaper.len() != browser.len() {
        return false;
    }
    let mut left: Vec<&PushedRecord> = browser.iter().collect();
    cheaper.iter().all(|record| {
        left.iter()
            .position(|other| *other == record)
            .map(|found| left.swap_remove(found))
            .is_some()
    })
}

/// Whether a sample's cheaper run, which read its document in `cheaper`,
/// and its browser run, whose `document.characterSet` is `browser`, read
/// it alike (ADR 0119): a run that names no encoding leaves the Records to
/// decide.
pub(crate) fn same_encoding(cheaper: Option<&str>, browser: Option<&str>) -> bool {
    match (cheaper, browser) {
        (Some(cheaper), Some(browser)) => cheaper.eq_ignore_ascii_case(browser),
        _ => true,
    }
}

/// Records what `label` learned on `domain_key`.
///
/// # Errors
///
/// The run file's, when the row cannot be written.
pub(crate) fn record(
    connection: &Connection,
    label: &str,
    domain_key: &str,
    unit: &Unit,
) -> rusqlite::Result<()> {
    let unit = serde_json::to_string(unit).expect("invariant: a tier choice encodes as JSON");
    connection.execute(
        "INSERT OR REPLACE INTO tier_choices (label, domain_key, unit) VALUES (?1, ?2, ?3)",
        params![label, domain_key, unit],
    )?;
    Ok(())
}

/// What the Run's Labels learned, as [`record`] kept it; a row that no
/// longer reads is left out, its Label sampled again.
///
/// # Errors
///
/// The run file's, when the rows cannot be read.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<TierChoices> {
    let mut select =
        connection.prepare("SELECT label, domain_key, unit FROM tier_choices ORDER BY label")?;
    let rows = select.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut units = BTreeMap::new();
    for row in rows {
        let (label, domain_key, unit) = row?;
        match serde_json::from_str(&unit) {
            Ok(unit) => {
                units.insert((label, domain_key), unit);
            }
            Err(error) => {
                tracing::warn!(%error, %label, %domain_key, "a Label's tier choice in the run file no longer reads");
            }
        }
    }
    Ok(TierChoices { units })
}

#[cfg(test)]
mod tests;
