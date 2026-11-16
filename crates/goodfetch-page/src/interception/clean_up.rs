//! The Clean-up rules the interception matches at step 4 (ADRs 0047, 0057
//! and 0117): the Filter lists of the version the page's Run holds to,
//! loaded as a Run whose own settings turn one on starts, else on the
//! first page whose Session turns one on, never at engine start, through a
//! blocking task of the page's Run (ADR 0060), then shared
//! by every Session of the Runs that hold to that version behind one `Arc`
//! with no lock; the never-block list, compiled once; and what a Session's
//! own settings, over its Run's, choose of them.

use std::sync::Arc;

use goodfetch_clean_up_rules::matching::{Choices, ResourceKind};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{
    Resolved, RunSettings, Values, catalog_value, get_or_default, value_as,
};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::page::{
    BROWSER_BLOCK_RESOURCES, BROWSER_SERVICE_WORKERS, LIVE_VIEW_HIDING,
};
use goodfetch_protocol::settings::{
    CONSENT_HANDLING, FILTER_LISTS_ADS, FILTER_LISTS_ANNOYANCES, FILTER_LISTS_FIRST_PARTY_SCRIPTS,
    FILTER_LISTS_TRACKERS,
};
use serde_json::Value;
use tokio::sync::OnceCell;

use super::steps::Blocking;
use crate::clean_up_versions::Versions;

/// What a Run's `consent.handling` lets the Consent prompt runner and the
/// cosmetic rules do (ADR 0048, #2434).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsentHandling {
    /// The runner answers each prompt: the default.
    Answer,
    /// The runner only looks, and still runs the wall test.
    Detect,
    /// The runner is neither registered nor started, and no cosmetic rule
    /// acts, while the Filter lists still block requests.
    Off,
}

impl ConsentHandling {
    /// `run`'s `consent.handling`.
    pub(crate) fn of(run: &Run) -> Self {
        let resolved = run.settings().map(RunSettings::resolved);
        match get_or_default::<&str>(resolved, &CONSENT_HANDLING) {
            "detect" => Self::Detect,
            "off" => Self::Off,
            // The resolver takes only the catalog's values: this is `answer`.
            _ => Self::Answer,
        }
    }
}

/// The engine's Clean-up rules, each part loaded when first needed.
pub(super) struct CleanUp {
    never_block: OnceCell<Arc<NeverBlock>>,
    versions: Arc<Versions>,
}

impl Default for CleanUp {
    fn default() -> Self {
        Self {
            never_block: OnceCell::new(),
            versions: Versions::new(run_needs_lists),
        }
    }
}

impl CleanUp {
    /// The versions of the Clean-up rules the engine's Runs hold to.
    pub(super) fn versions(&self) -> Arc<Versions> {
        Arc::clone(&self.versions)
    }

    /// Step 4 as `choices` make it for a page of `run`, its parts loaded
    /// through the Run's blocking tasks if no page loaded them yet, and the
    /// digest of the Filter lists' version, when any list is on.
    pub(super) async fn blocking(&self, choices: Choices, run: &Run) -> (Blocking, Option<String>) {
        let owner = run.owner();
        let never_block = self
            .never_block
            .get_or_init(|| async { Arc::new(owner.blocking(NeverBlock::new).await) })
            .await;
        let (lists, version) = if choices.needs_lists() {
            let held = self.versions.lists(run, owner).await;
            (Some(held.lists), Some(held.digest))
        } else {
            (None, None)
        };
        (
            Blocking {
                never_block: Arc::clone(never_block),
                lists,
                choices,
                work: run.metrics().counted_work().clone(),
            },
            version,
        )
    }
}

/// Whether `run`'s own settings turn a Filter list on, so that its pages
/// match the lists unless their Session turns every one off.
pub(crate) fn run_needs_lists(run: &Run) -> bool {
    choices(
        run.settings().map(RunSettings::resolved),
        &Values::default(),
    )
    .needs_lists()
}

/// The settings a Session's own `settings` may set, which its pages take
/// over the Run's: the Clean-up rules' choices, and whether its sites'
/// service workers run (ADR 0121).
pub(crate) const SESSION_KEYS: [&str; 7] = [
    FILTER_LISTS_ADS.name,
    FILTER_LISTS_TRACKERS.name,
    FILTER_LISTS_ANNOYANCES.name,
    FILTER_LISTS_FIRST_PARTY_SCRIPTS.name,
    BROWSER_BLOCK_RESOURCES.name,
    BROWSER_SERVICE_WORKERS.name,
    LIVE_VIEW_HIDING.name,
];

/// The value of `setting` a Session's pages take: `own`, the Session's own
/// settings, else `run`, its Run's resolved settings; its catalog default
/// without either.
pub(super) fn effective<'a>(
    run: Option<&'a Resolved>,
    own: &'a Values,
    setting: &Setting,
) -> &'a Value {
    own.get(setting.name)
        .or_else(|| run.map(|run| run.value(setting)))
        .unwrap_or_else(|| catalog_value(setting))
}

/// The `live_view.hiding` a Session's own settings `own`, over `run`, its
/// Run's resolved settings, choose: its catalog default, `auto`, without
/// either.
pub(super) fn live_hiding(run: Option<&Resolved>, own: &Values) -> String {
    value_as::<&str>(&LIVE_VIEW_HIDING, effective(run, own, &LIVE_VIEW_HIDING)).to_owned()
}

/// What a Session's own settings `own`, over `run`, its Run's resolved
/// settings, choose of the Clean-up rules; the defaults without either.
pub(super) fn choices(run: Option<&Resolved>, own: &Values) -> Choices {
    let boolean = |setting: &Setting| value_as(setting, effective(run, own, setting));
    let block_resources = effective(run, own, &BROWSER_BLOCK_RESOURCES)
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|kind| match kind.as_str()? {
            "image" => Some(ResourceKind::Image),
            "media" => Some(ResourceKind::Media),
            "font" => Some(ResourceKind::Font),
            "stylesheet" => Some(ResourceKind::Stylesheet),
            _ => None,
        })
        .collect();
    Choices {
        ads: boolean(&FILTER_LISTS_ADS),
        trackers: boolean(&FILTER_LISTS_TRACKERS),
        annoyances: boolean(&FILTER_LISTS_ANNOYANCES),
        first_party_scripts: boolean(&FILTER_LISTS_FIRST_PARTY_SCRIPTS),
        block_resources,
    }
}
