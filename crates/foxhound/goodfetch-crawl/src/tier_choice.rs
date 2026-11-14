//! The crawl's tier choice (spec #579's `comparison` and spec #582's
//! `goodfetch-crawl`; ADRs 0020, 0023, 0048, 0093 and 0103; CO-15): under
//! `fetch.tier: auto`, the tier each crawl request takes is learned per
//! Label on a domain key by sample comparison, and kept in the run file so
//! a resumed crawl takes the same tiers.
//!
//! [`TierChoices::serve_attempt`] plans an attempt: the first
//! `fetch.sample.count` requests of a Label on a domain (3 by default,
//! [`Sampling`]) are samples, each run on `http` and then on `browser`,
//! the browser's run the one that commits; [`TierChoices::classify`] then
//! takes how the [`weighing`] judged the two runs. A Label whose samples
//! all matched is learned onto `http`. One whose samples disagree by the
//! first segment of their URL paths, some segment having matched
//! throughout, is split by that segment: each segment keeps a tier of its
//! own, `http` once its own samples all matched, `browser` at its first
//! sample that did not, so a segment whose own samples disagree stays on
//! `browser`. At most [`MAX_SEGMENTS`] segments are kept per Label and
//! domain; the rest share the Label's choice, `browser`. Any other Label
//! is learned onto `browser`. A Label whose browser sample showed a
//! consent wall stays on `browser` for the Run, never sampled again
//! (ADR 0048).
//!
//! Where the Run allows the `light_browser` tier (`fetch.light_browser`,
//! ADR 0020), each sample also runs on it, before the browser, and a
//! sample whose `http` run differed but whose light run matched counts as
//! matched there: a Label (or segment) whose samples all matched goes on
//! `http` when every one matched over `http`, and on `light_browser` when
//! some matched only there.
//!
//! A choice is sampled again once it served `fetch.sample.refresh_after`'s
//! requests, or once its time passed, whichever comes first (500 requests
//! or 24 hours by default), and at once when [`TierChoices::invalidate`]
//! is told a declared total stopped matching (ADR 0020). A sample run is
//! neither a retry nor a failure.
//!
//! The hand-over of a Collie page on the planned tier, and the attempt's
//! runs, are the delivery's (`attempt`).

use std::collections::BTreeMap;

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_fetch_tier::{ReplayPlan, TierChoice};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::tier_race::Raced;

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

/// How a Run samples: `fetch.sample.count` and
/// `fetch.sample.refresh_after`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sampling {
    /// How many samples a Label on a domain, or one of its segments, takes
    /// before it moves to `http`.
    pub(crate) count: usize,
    /// How many requests a choice serves before it is sampled again.
    pub(crate) requests: u64,
    /// How long, in milliseconds, a choice stands before it is sampled
    /// again.
    pub(crate) after_ms: u64,
}

impl Default for Sampling {
    fn default() -> Self {
        Self {
            count: 3,
            requests: 500,
            after_ms: 24 * 60 * 60 * 1000,
        }
    }
}

/// How one attempt of a crawl request runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttemptPlan {
    /// A sitemap's fetch, on the Run's HTTP tier: it takes no tier choice
    /// and runs no handler.
    Sitemap,
    /// On this tier, forced or learned.
    On(TierChoice),
    /// On `discovered_api`, its calls replayed as the Label's plan says.
    Replayed(ReplayPlan),
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

/// How the weighing judged a sample's runs against the browser's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Weighed {
    /// Its `http` run staged the same results.
    Matched,
    /// Its `http` run did not, but its `light_browser` run did.
    MatchedLight,
    /// Its `http` run did not, but its `discovered_api` run, replaying
    /// `plan`, did; `light` when its `light_browser` run did too.
    MatchedApi { plan: ReplayPlan, light: bool },
    /// They did not, or the cheaper run escalated or failed.
    Differed,
    /// The browser's run met a consent wall: its Label stays on `browser`
    /// (ADR 0048).
    Walled,
}

/// What every Label on every domain of a Run learned so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TierChoices {
    units: BTreeMap<(String, String), Unit>,
    sampling: Sampling,
}

/// What one Label on one domain learned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Unit {
    /// Its first samples, in the order they were classified, and the plan
    /// the last that matched on `discovered_api` replayed.
    Sampling {
        samples: Vec<Sample>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<ReplayPlan>,
    },
    /// One tier for every request, chosen at `since` (milliseconds since
    /// the Unix epoch), having `served` requests since, with the plan
    /// `discovered_api` replays.
    Learned {
        tier: FetchTier,
        since: u64,
        #[serde(default, skip_serializing_if = "is_zero")]
        served: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<ReplayPlan>,
    },
    /// A tier per path segment, in the order the segments came, chosen at
    /// `since`, having `served` requests since, with the plan
    /// `discovered_api` replays; any other segment takes `browser`.
    Split {
        segments: Vec<Segment>,
        since: u64,
        #[serde(default, skip_serializing_if = "is_zero")]
        served: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<ReplayPlan>,
    },
    /// `browser` for every request, never sampled again: a browser sample
    /// showed a consent wall (ADR 0048).
    Walled,
}

/// Whether a choice served no request yet, its count then left out of
/// the run file.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_zero(served: &u64) -> bool {
    *served == 0
}

/// One classified sample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sample {
    segment: String,
    matched: bool,
    /// Whether it matched on `light_browser`, not over `http`.
    #[serde(default)]
    light: bool,
    /// Whether it matched on `discovered_api`, not over `http`.
    #[serde(default)]
    api: bool,
}

/// The tiers cheaper than the browser that served every sample of a unit
/// so far: a sample that matched over `http` is served by each, as each
/// serves the document `http` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Serves {
    http: bool,
    api: bool,
    light: bool,
}

impl Serves {
    const ALL: Self = Self {
        http: true,
        api: true,
        light: true,
    };

    fn all() -> Self {
        Self::ALL
    }

    /// The tiers that served `sample`.
    fn of(sample: &Sample) -> Self {
        if !sample.matched {
            return Self {
                http: false,
                api: false,
                light: false,
            };
        }
        if !sample.light && !sample.api {
            return Self::ALL;
        }
        Self {
            http: false,
            api: sample.api,
            light: sample.light,
        }
    }

    fn and(self, other: Self) -> Self {
        Self {
            http: self.http && other.http,
            api: self.api && other.api,
            light: self.light && other.light,
        }
    }

    /// The cheapest of them (ADR 0020): `http`, then `discovered_api`,
    /// then `light_browser`; `browser` when none.
    fn tier(self) -> FetchTier {
        if self.http {
            FetchTier::Http
        } else if self.api {
            FetchTier::DiscoveredApi
        } else if self.light {
            FetchTier::LightBrowser
        } else {
            FetchTier::Browser
        }
    }
}

/// One segment of a split Label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Segment {
    name: String,
    /// Its samples that matched so far.
    matched: usize,
    /// The tiers that served every one of them.
    #[serde(default = "Serves::all")]
    serves: Serves,
    /// Its tier, once chosen.
    tier: Option<FetchTier>,
}

impl TierChoices {
    /// The same choices, sampled as `sampling` says.
    #[must_use]
    pub(crate) fn sampled_as(mut self, sampling: Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    /// How an attempt of a request of `label` at `url` on `domain_key`
    /// runs under `auto` at `now` (milliseconds since the Unix epoch): a
    /// choice that served its requests or outlived its time is sampled
    /// again, and each request a choice serves counts toward its refresh.
    /// Answers the plan, and the unit whose count it moved, for the run
    /// file, so a resumed crawl goes on counting.
    pub(crate) fn serve_attempt(
        &mut self,
        label: &str,
        domain_key: &str,
        url: &str,
        now: u64,
    ) -> (AttemptPlan, Option<&Unit>) {
        let segment = segment(url);
        let id = (label.to_owned(), domain_key.to_owned());
        if self
            .units
            .get(&id)
            .is_some_and(|unit| unit.stale(now, self.sampling))
        {
            self.units.remove(&id);
        }
        let plan = match self
            .units
            .get(&id)
            .map_or(Some(Plan::Sample), |unit| unit.plan(&segment))
        {
            Some(Plan::Sample) | None => {
                return (
                    AttemptPlan::Sample(Sampled {
                        label: label.to_owned(),
                        domain_key: domain_key.to_owned(),
                        segment,
                    }),
                    None,
                );
            }
            Some(Plan::On(FetchTier::DiscoveredApi)) => {
                let plan = self.units.get(&id).and_then(Unit::replay_plan).cloned();
                // A unit holds a plan once one of its samples matched on
                // `discovered_api`; one without stays on the browser.
                plan.map_or(
                    AttemptPlan::On(TierChoice {
                        tier: FetchTier::Browser,
                        forced: None,
                    }),
                    AttemptPlan::Replayed,
                )
            }
            Some(Plan::On(tier)) => AttemptPlan::On(TierChoice { tier, forced: None }),
        };
        let unit = self.units.get_mut(&id).map(|unit| {
            unit.count_served();
            &*unit
        });
        (plan, unit)
    }

    /// Takes the sample `sampled`, as the weighing judged its runs at
    /// `now`; answers its unit as it now stands, for the run file.
    pub(crate) fn classify(&mut self, sampled: &Sampled, weighed: Weighed, now: u64) -> &Unit {
        let count = self.sampling.count;
        let unit = self
            .units
            .entry((sampled.label.clone(), sampled.domain_key.clone()))
            .or_insert_with(|| Unit::Sampling {
                samples: Vec::new(),
                plan: None,
            });
        match weighed {
            Weighed::Walled => *unit = Unit::Walled,
            Weighed::Matched
            | Weighed::MatchedLight
            | Weighed::MatchedApi { .. }
            | Weighed::Differed => {
                unit.classify(&sampled.segment, weighed, (count, now));
            }
        }
        unit
    }

    /// Samples `label` on `domain_key` again at once, as a declared total
    /// that stopped matching says (ADR 0020); a walled Label stays on
    /// `browser`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no contract declares a total yet: the crawl calls it once one does"
        )
    )]
    pub(crate) fn invalidate(&mut self, label: &str, domain_key: &str) {
        let id = (label.to_owned(), domain_key.to_owned());
        if self.units.get(&id) != Some(&Unit::Walled) {
            self.units.remove(&id);
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
    /// The plan `discovered_api` replays for it, once a sample matched
    /// there.
    fn replay_plan(&self) -> Option<&ReplayPlan> {
        match self {
            Self::Sampling { plan, .. } | Self::Learned { plan, .. } | Self::Split { plan, .. } => {
                plan.as_ref()
            }
            Self::Walled => None,
        }
    }

    /// Whether the choice it holds is to be sampled again at `now`, having
    /// served its requests or outlived its time.
    fn stale(&self, now: u64, sampling: Sampling) -> bool {
        match self {
            Self::Learned { since, served, .. } | Self::Split { since, served, .. } => {
                *served >= sampling.requests || now.saturating_sub(*since) >= sampling.after_ms
            }
            Self::Sampling { .. } | Self::Walled => false,
        }
    }

    /// Counts one more request its choice served; a unit still sampling, or
    /// walled, counts none.
    fn count_served(&mut self) {
        match self {
            Self::Learned { served, .. } | Self::Split { served, .. } => *served += 1,
            Self::Sampling { .. } | Self::Walled => {}
        }
    }

    /// The plan for a request in `segment`; `None` for a sample that opens
    /// a segment of a split Label.
    fn plan(&self, segment: &str) -> Option<Plan> {
        match self {
            Self::Sampling { .. } => Some(Plan::Sample),
            Self::Learned { tier, .. } => Some(Plan::On(*tier)),
            Self::Walled => Some(Plan::On(FetchTier::Browser)),
            Self::Split { segments, .. } => {
                match segments.iter().find(|kept| kept.name == segment) {
                    Some(Segment {
                        tier: Some(tier), ..
                    }) => Some(Plan::On(*tier)),
                    Some(Segment { tier: None, .. }) => Some(Plan::Sample),
                    None if segments.len() < MAX_SEGMENTS => None,
                    // The rest share the Label's choice.
                    None => Some(Plan::On(FetchTier::Browser)),
                }
            }
        }
    }

    /// Takes a sample in `segment` weighed as `weighed`, of `count`
    /// samples a choice takes, at `now`. A choice once made stands until
    /// it is sampled again.
    fn classify(&mut self, segment: &str, weighed: Weighed, (count, now): (usize, u64)) {
        let (sample, replayed) = match weighed {
            Weighed::MatchedApi { plan, light } => (
                Sample {
                    segment: segment.to_owned(),
                    matched: true,
                    light,
                    api: true,
                },
                Some(plan),
            ),
            weighed => (
                Sample {
                    segment: segment.to_owned(),
                    matched: matches!(weighed, Weighed::Matched | Weighed::MatchedLight),
                    light: weighed == Weighed::MatchedLight,
                    api: false,
                },
                None,
            ),
        };
        match self {
            Self::Learned { .. } | Self::Walled => {}
            Self::Sampling { samples, plan } => {
                if replayed.is_some() {
                    *plan = replayed;
                }
                samples.push(sample);
                if samples.len() >= count {
                    let plan = plan.take();
                    *self = Self::chosen(samples, count, now, plan);
                }
            }
            Self::Split { segments, plan, .. } => {
                if replayed.is_some() {
                    *plan = replayed;
                }
                Segment::take(segments, &sample, count);
            }
        }
    }

    /// What a Label on a domain learned at `now` from its first `samples`,
    /// `plan` the one its samples replayed on `discovered_api`: the
    /// cheapest tier that served every one ([`Serves`]) when every one
    /// matched; split by first segment when some segment matched
    /// throughout and another did not; `browser` otherwise.
    fn chosen(samples: &[Sample], count: usize, now: u64, plan: Option<ReplayPlan>) -> Self {
        if samples.iter().all(|sample| sample.matched) {
            let tier = samples
                .iter()
                .fold(Serves::ALL, |serves, sample| serves.and(Serves::of(sample)))
                .tier();
            return Self::Learned {
                tier,
                since: now,
                served: 0,
                plan,
            };
        }
        let mut segments: Vec<Segment> = Vec::new();
        for sample in samples {
            Segment::take(&mut segments, sample, count);
        }
        let some_matched = segments
            .iter()
            .any(|kept| kept.tier != Some(FetchTier::Browser));
        if segments.len() > 1 && some_matched {
            Self::Split {
                segments,
                since: now,
                served: 0,
                plan,
            }
        } else {
            Self::Learned {
                tier: FetchTier::Browser,
                since: now,
                served: 0,
                plan: None,
            }
        }
    }
}

impl Segment {
    /// Takes `sample` among `segments`, opening its segment while fewer
    /// than [`MAX_SEGMENTS`] are kept.
    fn take(segments: &mut Vec<Self>, sample: &Sample, count: usize) {
        match segments.iter().position(|kept| kept.name == sample.segment) {
            Some(found) => segments[found].classify(sample, count),
            None if segments.len() < MAX_SEGMENTS => {
                let mut kept = Self {
                    name: sample.segment.clone(),
                    matched: 0,
                    serves: Serves::ALL,
                    tier: None,
                };
                kept.classify(sample, count);
                segments.push(kept);
            }
            None => {}
        }
    }

    /// Takes one of its samples: its first that did not match keeps it on
    /// `browser`; its `count`th that matched, all before it matching,
    /// moves it to the cheapest tier that served them all ([`Serves`]).
    fn classify(&mut self, sample: &Sample, count: usize) {
        if self.tier.is_some() {
            return;
        }
        if !sample.matched {
            self.tier = Some(FetchTier::Browser);
            return;
        }
        self.matched += 1;
        self.serves = self.serves.and(Serves::of(sample));
        if self.matched >= count {
            self.tier = Some(self.serves.tier());
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
    connection
        .prepare_cached(RECORD)
        .and_then(|mut cached| cached.execute(params![label, domain_key, encoded(unit)]))?;
    Ok(())
}

/// The statement that keeps a Label's choice on a domain key: its Label,
/// its domain key and its unit, [`encoded`].
pub(crate) const RECORD: &str =
    "INSERT OR REPLACE INTO tier_choices (label, domain_key, unit) VALUES (?1, ?2, ?3)";

/// `unit` as the run file keeps it.
pub(crate) fn encoded(unit: &Unit) -> String {
    serde_json::to_string(unit).expect("invariant: a tier choice encodes as JSON")
}

/// What the Run's Labels learned, as [`record`] kept it, each choice with
/// the requests it served so far; a row that no longer reads is left out,
/// its Label sampled again.
///
/// # Errors
///
/// The run file's, when the rows cannot be read.
pub(crate) fn read(connection: &Connection) -> rusqlite::Result<TierChoices> {
    let mut select = connection
        .prepare_cached("SELECT label, domain_key, unit FROM tier_choices ORDER BY label")?;
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
    Ok(TierChoices {
        units,
        ..TierChoices::default()
    })
}

pub(crate) mod weighing;

#[cfg(test)]
mod tests;
