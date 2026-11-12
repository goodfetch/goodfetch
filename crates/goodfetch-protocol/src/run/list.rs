//! Listing the Run index (ADRs 0031, 0036 and 0117): `run.list` answers the
//! rows of every Run of the user and major, of either Product, filtered and
//! sorted by the engine, so every Client, the Kennel among them, sees the
//! same view of the same filter. A Run started by a Recipe or a Schedule
//! carries reserved tags, `goodfetch.<key>:<value>`, from which the list
//! reads its Recipe, Schedule, occurrence, trigger and role; a Watch run,
//! role `watch`, is left out unless the filter asks for Watch runs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};

use super::{Completeness, Lifecycle, PointInTime, RunIndexRow, Tag, Tags, Verdict};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What begins each tag only Goodfetch's own starters set, the scheduler
/// and a Recipe's (ADR 0036; spec #589): `goodfetch.<key>:<value>`.
pub const RESERVED_TAG_PREFIX: &str = "goodfetch.";

/// The reserved tag naming the Recipe a Run ran, by its name (ADR 0031).
pub const RECIPE_TAG: &str = "recipe";

/// The reserved tag naming the Schedule that started a Run.
pub const SCHEDULE_TAG: &str = "schedule";
/// The reserved tag naming the occurrence that started a Run.
pub const OCCURRENCE_TAG: &str = "occurrence";
/// The reserved tag naming what fired that occurrence, a [`RunTrigger`].
pub const TRIGGER_TAG: &str = "trigger";
/// The reserved tag naming who created the Schedule.
pub const CREATOR_TAG: &str = "creator";
/// The reserved tag naming a scheduled Run's role: `watch` or `target`.
pub const ROLE_TAG: &str = "role";
/// The role of a Watch run (ADR 0036).
pub const WATCH_ROLE: &str = "watch";

impl Tags {
    /// The value of the reserved tag `goodfetch.<key>:<value>` the Run
    /// carries, the first when it carries several.
    pub fn reserved(&self, key: &str) -> Option<&str> {
        self.iter().find_map(|tag| {
            tag.as_str()
                .strip_prefix(RESERVED_TAG_PREFIX)?
                .strip_prefix(key)?
                .strip_prefix(':')
        })
    }
}

/// What fired a Schedule's occurrence (ADR 0036), as its run's reserved
/// `goodfetch.trigger` tag names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    /// A time trigger: cron or an interval.
    Time,
    /// A Watch that found a change.
    Watch,
    /// The end of another run.
    AfterRun,
    /// A person's or a program's "run now".
    RunNow,
}

impl RunTrigger {
    /// Every value, in declaration order.
    pub const ALL: [Self; 4] = [Self::Time, Self::Watch, Self::AfterRun, Self::RunNow];

    /// Its name in the `goodfetch.trigger` tag.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }

    /// The trigger `name` names, if it is one this version knows.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Time, Self::Watch, Self::AfterRun, Self::RunNow]
            .into_iter()
            .find(|trigger| trigger.as_str() == name)
    }
}

/// A span of time: from its first point, included, until its last,
/// excluded; an end left out leaves that side open.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    /// Its first point, included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub from: Option<PointInTime>,
    /// The point it ends before, excluded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub until: Option<PointInTime>,
}

/// What `run.list` sorts its rows by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunSortKey {
    /// When the Run started.
    #[default]
    StartedAt,
    /// When it ended; a Run that has not ended comes after every one that
    /// did, in ascending order.
    EndedAt,
    /// Its Run ID.
    RunId,
    /// Its Recipe's name.
    Recipe,
    /// The Schedule that started it.
    Schedule,
    /// The occurrence that started it.
    Occurrence,
    /// What fired that occurrence.
    Trigger,
    /// The Product it needs.
    Product,
    /// Its Lifecycle.
    Lifecycle,
    /// Its Verdict.
    Verdict,
    /// Its Completeness.
    Completeness,
}

/// Which way `run.list` sorts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Smallest first.
    Ascending,
    /// Largest first: the newest first, when sorting by time.
    #[default]
    Descending,
}

/// What `run.list` takes: which rows of the Run index it answers, each
/// field given narrowing them, and in which order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunListParams {
    /// Tags each row carries, every one of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tags: Vec<Tag>,
    /// The Recipe the Runs ran, by its name, as their reserved
    /// `goodfetch.recipe` tag names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recipe: Option<String>,
    /// The Schedule that started the Runs, by its ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule: Option<String>,
    /// The occurrence that started the Runs, by its ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub occurrence: Option<String>,
    /// What fired the occurrence that started the Runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub trigger: Option<RunTrigger>,
    /// The Product the Runs need (ADR 0098).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Option<Product>,
    /// Their Lifecycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lifecycle: Option<Lifecycle>,
    /// Their Verdict: only Runs that ended have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verdict: Option<Verdict>,
    /// Their Completeness: only Runs that ended have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub completeness: Option<Completeness>,
    /// When they started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub started: Option<TimeRange>,
    /// When they ended: only Runs that ended match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended: Option<TimeRange>,
    /// Text found, ignoring case, in their Run ID or one of their tags,
    /// their Recipe's among them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: Option<String>,
    /// Whether Watch runs are listed too; they are left out by default
    /// (ADR 0036).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub watch_runs: bool,
    /// What the rows are sorted by, when they started by default; rows
    /// that compare equal follow in Run ID order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sort: Option<RunSortKey>,
    /// Which way, descending by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub order: Option<SortOrder>,
}

/// What `run.list` answers: the rows of its filter, in its order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunList {
    /// The rows, every presence kept: a Run whose state is gone is listed
    /// `missing`, one discarded on purpose `erased` (ADR 0031).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runs: Vec<RunIndexRow>,
}

/// `run.list`.
pub const RUN_LIST: Operation = Operation {
    name: "run.list",
    description: "Lists the Runs of the Run index of the user and major, of either Product (ADRs 0031 and 0098): each row with its tags, Lifecycle, Verdict, Completeness, start and end, the engine that owns it while it is live, and its presence, a Run whose state is gone `missing` and one discarded on purpose `erased`, never left out. The engine filters by tags, the Recipe, Schedule, occurrence and trigger its reserved `goodfetch.` tags name, Product, each axis, start and end time, and a text found in its Run ID or tags, and sorts by the key given; Watch runs are left out unless `watch_runs` asks for them (ADR 0036).",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunListParams>,
    result: schema_of::<RunList>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
