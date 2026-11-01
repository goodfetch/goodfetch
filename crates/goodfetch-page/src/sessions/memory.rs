//! A page past its memory cap (ADR 0040's shedding ladder): at each sample
//! that finds the Resource governor's memory critical, each open page of a
//! Run is measured, its JavaScript heap and the buffers it holds as its
//! renderer reports them, and one past `governor.browser.page_memory_cap`
//! ends at its safe point, once no call on it is in flight: it closes,
//! emits `close`, and its later calls raise `page_memory_exceeded`, which
//! is retryable, as a crashed page's raise `page_crashed`.

use goodfetch_cdp::generated::runtime::{GetHeapUsageParams, GetHeapUsageReturns};
use goodfetch_engine::runs::Run;
use goodfetch_protocol::governor::{BROWSER_PAGE_MEMORY_CAP, GovernorLevel};
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use serde_json::json;

use super::life::Ending;
use super::lifecycle::PageEffect;
use super::{OpenPage, Sessions};

/// `governor.browser.page_memory_cap` when the Run's settings name none.
const PAGE_MEMORY_CAP: u64 = 1 << 30;

impl Sessions {
    /// Ends each page of `run` past its memory cap at every sample that
    /// finds memory critical, until the Run stops.
    pub(super) fn end_pages_past_their_cap(&self, run: &Run) {
        let sessions = self.clone();
        let mut samples = run.governor().samples();
        let stopping = run.cancellation().clone();
        let limit = page_memory_cap(run);
        let run_id = run.id().clone();
        run.clone().spawn(async move {
            loop {
                tokio::select! {
                    () = stopping.cancelled() => return,
                    changed = samples.changed() => if changed.is_err() { return },
                }
                if *samples.borrow_and_update() != GovernorLevel::Critical {
                    continue;
                }
                let pages: Vec<(PageId, OpenPage)> = sessions
                    .state
                    .lock()
                    .await
                    .pages
                    .iter()
                    .filter(|(_, page)| page.run.id() == &run_id)
                    .map(|(page_id, page)| (page_id.clone(), page.clone()))
                    .collect();
                for (page_id, page) in pages {
                    let Ok(heap) = page.driver.command(&GetHeapUsageParams {}).await else {
                        continue;
                    };
                    let memory = held(&heap);
                    let cap = limit.value.as_u64().unwrap_or(PAGE_MEMORY_CAP);
                    if memory > cap && page.life.past_cap() {
                        let ending = sessions.clone();
                        let limit = limit.clone();
                        page.run.clone().spawn(async move {
                            ending.end_past_cap(page_id, page, memory, limit).await;
                        });
                    }
                }
            }
        });
    }

    /// Ends `page`, which held `memory` past `limit`, once no call on it is
    /// in flight.
    async fn end_past_cap(
        &self,
        page_id: PageId,
        page: OpenPage,
        memory: u64,
        limit: SettingLimit,
    ) {
        page.life.idle().await;
        tracing::warn!(
            page_id = %page_id.0,
            memory,
            cap = %limit.value,
            "memory is critical: a page past `governor.browser.page_memory_cap` ends"
        );
        match page.life.end(&Ending::MemoryExceeded { memory, limit }) {
            Ok(PageEffect::Close) => {
                page.driver.close().await;
                // The close the engine sent is confirmed (P29), unless the
                // page's watch saw its target's end first. Whichever acts
                // on the end, the page leaves for the ended pages, whose
                // later calls raise `page_memory_exceeded` again.
                match page.life.end(&Ending::Destroyed) {
                    Ok(effect) => {
                        self.page_ended(&page_id, &page.run, &page.life, effect)
                            .await;
                    }
                    Err(error) => tracing::error!(%error, "a page's end was refused"),
                }
            }
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "a page past its memory cap did not end"),
        }
    }
}

/// `governor.browser.page_memory_cap` as `run`'s settings set it, with its
/// layer.
fn page_memory_cap(run: &Run) -> SettingLimit {
    run.settings()
        .and_then(|settings| settings.resolved().get(BROWSER_PAGE_MEMORY_CAP.name))
        .map_or_else(
            || SettingLimit {
                key: BROWSER_PAGE_MEMORY_CAP.name.to_owned(),
                value: json!(PAGE_MEMORY_CAP),
                layer: SettingsLayer::Default,
            },
            |effective| SettingLimit {
                key: BROWSER_PAGE_MEMORY_CAP.name.to_owned(),
                value: effective.value.clone(),
                layer: effective.layer,
            },
        )
}

/// What a page holds, in bytes: its JavaScript heap, its embedder's heap
/// and the buffers it holds.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the renderer reports byte counts as JSON numbers: whole, never negative, far below 2^53, and `as` saturates"
)]
fn held(heap: &GetHeapUsageReturns) -> u64 {
    (heap.used_size + heap.embedder_heap_used_size + heap.backing_storage_size) as u64
}
