//! A page's kept tree (spec #1577's #1918; ADR 0138): the untouched tree
//! of a document a fetch tier serves to a handler, parsed at most once for
//! its attempt and read by every call the handler makes on it.
//!
//! One owner holds the tree: a task of its Run's that takes the calls'
//! reads from a bounded channel, one at a time, and runs each on the CPU
//! pool with the tree lent to it, so no read waits for a parse of its own
//! and none changes the tree. A read that changes a page, as the clean-up
//! and the main-content step of `page.formats` do, reads a copy it takes
//! inside its read. The owner parses the document's HTML at its first read
//! when it was handed no tree (a document the streaming path read), so a
//! page whose handler reads nothing of its tree is never parsed for it.
//!
//! The tree is gone once every holder of the page dropped its handle,
//! which the attempt's end does as it closes the page: the owner's channel
//! closes then, and the owner ends, dropping the tree; it ends too as its
//! Run stops. While a handler runs, its page holds one tree, or the HTML
//! it will parse, and at most [`READS_QUEUED`] reads wait for it, each a
//! closure holding its call's params; a read past them waits for room, so
//! what a handler's calls hold beside the tree is bounded whatever it
//! sends.

use std::fmt::Debug;

use bytestring::ByteString;
use goodfetch_counted_work::{ParsedHtml, Row};
use goodfetch_engine::runs::Run;
use tokio::sync::{mpsc, oneshot};

/// How many reads of a page's tree wait for its owner at most.
pub const READS_QUEUED: usize = 8;

/// One read of a kept tree, which answers through its own channel.
type Job = Box<dyn FnOnce(&ParsedHtml) + Send>;

/// What a page's tree is kept from.
pub enum KeptFrom {
    /// Its untouched tree, parsed already.
    Tree(ParsedHtml),
    /// Its HTML, parsed at the first read, on `F1.parses`.
    Html(ByteString),
}

/// The handle to a page's kept tree. Cheap to clone; every clone reads
/// the same tree, which is gone once the last clone dropped.
#[derive(Clone)]
pub struct KeptTree {
    reads: mpsc::Sender<Job>,
}

impl Debug for KeptTree {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("KeptTree").finish_non_exhaustive()
    }
}

impl KeptTree {
    /// Keeps the tree `kept` gives for `run`'s page served from it. Its
    /// owner is a task of the Run's, which ends with the last handle, or
    /// as the Run stops.
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0138: a streamed page's tree, parsed once for its attempt at its handler's first read"
    )]
    pub fn keep(run: &Run, kept: KeptFrom) -> Self {
        let (reads, mut mailbox) = mpsc::channel::<Job>(READS_QUEUED);
        let owner = run.clone();
        run.owner().spawn(async move {
            let stopping = owner.owner().token().clone();
            let mut kept = Some(kept);
            while let Some(Some(read)) = stopping.run_until_cancelled(mailbox.recv()).await {
                let work = owner.metrics().counted_work().clone();
                let held = kept.take();
                kept = owner
                    .owner()
                    .handler_analysis(owner.metrics().counted_work(), move || {
                        let tree = match held? {
                            KeptFrom::Tree(tree) => tree,
                            KeptFrom::Html(html) => work.parse_html(Row::PageParses, &html),
                        };
                        read(&tree);
                        Some(KeptFrom::Tree(tree))
                    })
                    .await;
            }
        });
        Self { reads }
    }

    /// What `read` reads of the tree, run on the CPU pool once the reads
    /// sent before it ran, never beside another read of the same tree;
    /// none once the owner is gone: as its Run stops, or after a read of
    /// it panicked, which ends the owner.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before its read started runs nothing; once it
    /// started, the read runs to its end and its answer is dropped.
    pub async fn read<T, F>(&self, read: F) -> Option<T>
    where
        F: FnOnce(&ParsedHtml) -> T + Send + 'static,
        T: Send + 'static,
    {
        let (answer, answered) = oneshot::channel();
        let job: Job = Box::new(move |tree| {
            let _ = answer.send(read(tree));
        });
        self.reads.send(job).await.ok()?;
        answered.await.ok()
    }
}
