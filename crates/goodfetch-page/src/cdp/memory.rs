//! What a page holds in memory, as its renderer reports it, which a Run
//! past its memory cap reads (ADR 0040's shedding ladder).

use goodfetch_cdp::generated::runtime::{GetHeapUsageParams, GetHeapUsageReturns};

use super::CdpPage;
use super::paused::NotTaken;

impl CdpPage {
    /// What the page holds, in bytes: its JavaScript heap, its embedder's
    /// heap and the buffers it holds.
    ///
    /// # Errors
    ///
    /// When the page's target went.
    pub(crate) async fn memory_held(&self) -> Result<u64, NotTaken> {
        Ok(held(&self.page.command(&GetHeapUsageParams {}).await?))
    }
}

/// The bytes `heap` reports the page holding.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the renderer reports byte counts as JSON numbers: whole, never negative, far below 2^53, and `as` saturates"
)]
fn held(heap: &GetHeapUsageReturns) -> u64 {
    (heap.used_size + heap.embedder_heap_used_size + heap.backing_storage_size) as u64
}
