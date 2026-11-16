//! A program's links batch while memory does not hold every visit (#2169;
//! G-11 Q6): its new URLs are looked up in the visited set on a reader
//! before its job is queued, so the writer runs no read for them and
//! answers the batches whose URLs memory holds meanwhile. A batch that
//! holds a URL another batch is reading waits for that read instead of
//! reading it again.

use goodfetch_engine::run_state::Syncing;
use goodfetch_frontier::Lookup;

use super::Stored;
use crate::run_state::StateError;
use crate::run_state::admission::{Link, LinksBatch};
use crate::run_state::asks::{Queued, Staged};

/// What a links batch's job answers.
type Linked = Staged<goodfetch_protocol::foxhound::frontier::EnqueuedLinks>;

impl Stored {
    /// The lookups of the URLs of `links` that memory does not hold:
    /// none while it holds every visit, or the writer does not batch.
    pub(super) fn look_up(&self, links: &LinksBatch) -> Option<Lookup> {
        let unwritten = self.invariants.frontier().unwritten()?;
        unwritten.look_up(links.links.iter().filter_map(|link| match link {
            Link::Keyed { keyed, .. } => keyed.plain(),
            Link::NoHref | Link::Trap { .. } | Link::Skipped { .. } => None,
        }))
    }

    /// The job of `links`, queued with the run-state writer, synced as
    /// `syncing` says, once `lookup` read the URLs it reads, on a reader
    /// in one transaction, and the other batches' reads it waits for
    /// answered: its job reads none of them. The lookup's answers stand
    /// until the job is written.
    pub(super) fn links_after_lookup(
        &self,
        links: LinksBatch,
        mut lookup: Lookup,
        syncing: Syncing,
    ) -> Queued<Linked> {
        let this = self.clone();
        Box::pin(async move {
            if lookup.reads() {
                let reading = this.clone();
                lookup = this
                    .writer
                    .read(move |connection| {
                        let unwritten = reading.invariants.frontier().unwritten().expect(
                            "invariant: a links batch looks its URLs up only beside the unwritten visits",
                        );
                        goodfetch_frontier::read_lookup(connection, unwritten, &lookup)?;
                        Ok(lookup)
                    })
                    .await?;
            }
            lookup.wait().await;
            let place = this.writer.reserve().await?;
            let writing = this.clone();
            let written = place
                .write_with(syncing, move |connection| writing.links(connection, &links))
                .await
                .map_err(StateError::from);
            drop(lookup);
            written
        })
    }
}
