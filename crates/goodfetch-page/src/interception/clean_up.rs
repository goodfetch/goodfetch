//! The Clean-up rules the interception matches at step 4 (ADRs 0047, 0057
//! and 0117): the embedded Filter lists, loaded on the first page whose
//! Session turns one on, never at engine start, through a blocking task of
//! the page's Run (ADR 0060), then shared by every Session of the engine
//! behind one `Arc` with no lock; the never-block list, compiled once; and
//! what a Run's settings choose of them.

use std::sync::Arc;

use goodfetch_clean_up_rules::matching::{Choices, ResourceKind};
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_protocol::page::BROWSER_BLOCK_RESOURCES;
use goodfetch_protocol::settings::{
    FILTER_LISTS_ADS, FILTER_LISTS_ANNOYANCES, FILTER_LISTS_FIRST_PARTY_SCRIPTS,
    FILTER_LISTS_TRACKERS,
};
use tokio::sync::OnceCell;

use super::steps::Blocking;

/// The engine's Clean-up rules, each part loaded when first needed.
#[derive(Default)]
pub(super) struct CleanUp {
    never_block: OnceCell<Arc<NeverBlock>>,
    lists: OnceCell<Arc<FilterLists>>,
}

impl CleanUp {
    /// Step 4 as `choices` make it, its parts loaded through `owner`'s
    /// blocking tasks if no page loaded them yet.
    pub(super) async fn blocking(&self, choices: Choices, owner: &TaskOwner) -> Blocking {
        let never_block = self
            .never_block
            .get_or_init(|| async { Arc::new(owner.blocking(NeverBlock::new).await) })
            .await;
        let lists = if choices.needs_lists() {
            let lists = self
                .lists
                .get_or_init(|| async {
                    let loaded = owner
                        .blocking(FilterLists::embedded)
                        .await
                        .expect("invariant: the build compiled the embedded Filter lists with this binary's own adblock");
                    Arc::new(loaded)
                })
                .await;
            Some(Arc::clone(lists))
        } else {
            None
        };
        Blocking {
            never_block: Arc::clone(never_block),
            lists,
            choices,
        }
    }
}

/// What `settings`, a Run's resolved settings, choose of the Clean-up
/// rules; the defaults without any.
pub(super) fn choices(settings: Option<&Resolved>) -> Choices {
    let boolean = |key: &str, default: bool| {
        settings
            .and_then(|settings| settings.boolean(key))
            .unwrap_or(default)
    };
    let block_resources = settings
        .and_then(|settings| settings.value(BROWSER_BLOCK_RESOURCES.name).as_array())
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
        ads: boolean(FILTER_LISTS_ADS.name, true),
        trackers: boolean(FILTER_LISTS_TRACKERS.name, true),
        annoyances: boolean(FILTER_LISTS_ANNOYANCES.name, true),
        first_party_scripts: boolean(FILTER_LISTS_FIRST_PARTY_SCRIPTS.name, false),
        block_resources,
    }
}
