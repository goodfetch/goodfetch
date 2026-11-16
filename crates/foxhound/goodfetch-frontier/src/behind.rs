//! Learning's evidence written behind (#2169; ADR 0142's note of
//! 2026-11-10): once a Run learns behind, neither a fetched page nor an
//! admission reads or writes the run file for learning on its page's path.
//! A page's lesson, its redirect hops and its document, is noted as its
//! task meets it; an admission that writes notes its URL and its page's,
//! tagged with the writer's job, whose failure drops what it noted. The
//! end of each batch the writer commits teaches them all, in the order
//! they were noted, as a write of its own: a rule learned there merges the
//! duplicates admitted before it, ending those still pending `skipped`
//! with `duplicate`, one batch late. A batch end that fails loses what it
//! would have taught, as a page's lesson that failed did before.

#![expect(
    clippy::disallowed_types,
    reason = "the evidence noted, guarded for one push, one drop or one swap, never held with another lock, across an await or while the run file is read, so there is no lock order to test under loom"
)]

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::run_state::WriteWatch;
use rusqlite::Connection;
use url::Url;

use crate::identity;
use crate::learn::{self, EndMerged, PageSeen, Redirect};
use crate::rules;

/// What a fetched page teaches the Frontier, kept until a batch's end
/// teaches it: each redirect hop its chain followed and its document, the
/// evidence among them alone, keyed.
#[derive(Clone, Debug, Default)]
pub struct Lesson {
    hops: Vec<(Url, Url)>,
    page: Option<Page>,
}

/// A fetched page's document, keyed, as a lesson keeps it.
#[derive(Clone, Debug)]
struct Page {
    crawl_request: String,
    key: Url,
    signature: [u8; 32],
    canonical: Option<Url>,
}

impl Lesson {
    /// Adds the redirect hop `redirect`, when it is evidence.
    pub fn redirect(&mut self, redirect: Redirect<'_>) {
        if learn::is_evidence(redirect.method, redirect.body) {
            self.hops
                .push((identity::keyed(redirect.from), identity::keyed(redirect.to)));
        }
    }

    /// Adds the document `page`, when it is evidence.
    pub fn page(&mut self, page: PageSeen<'_>) {
        if learn::is_evidence(page.method, page.body) {
            self.page = Some(Page {
                crawl_request: page.crawl_request.to_owned(),
                key: identity::keyed(page.url),
                signature: page.signature,
                canonical: page.canonical.map(identity::keyed),
            });
        }
    }

    /// Whether it teaches nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hops.is_empty() && self.page.is_none()
    }

    /// Teaches it in `connection`'s transaction, each hop and then the
    /// document, handing `pending` the requests a rule it teaches merges.
    fn teach(
        &self,
        connection: &Connection,
        pending: &mut dyn EndMerged,
        work: &CountedWork,
    ) -> rusqlite::Result<()> {
        for (from, to) in &self.hops {
            let change = rules::learn_redirect(connection, from, to, work)?;
            learn::apply(connection, change.into_iter().collect(), pending)?;
        }
        if let Some(page) = &self.page {
            let changes = rules::learn(
                connection,
                &rules::Page {
                    crawl_request: &page.crawl_request,
                    key: &page.key,
                    signature: &page.signature,
                    canonical: page.canonical.as_ref(),
                },
                work,
            )?;
            learn::apply(connection, changes, pending)?;
        }
        Ok(())
    }
}

/// One piece of evidence a batch's end teaches.
#[derive(Debug)]
enum Evidence {
    /// The URL whose key is `key` was admitted, a link of the page whose
    /// key is `from` when an attempt enqueued it.
    Admitted { key: Url, from: Option<Url> },
    /// A fetched page's lesson.
    Lesson(Lesson),
}

/// The evidence noted and not taught yet, in order, each tagged with the
/// writer's job that noted it when one did.
#[derive(Debug, Default)]
struct Noted {
    entries: Vec<(Option<u64>, Evidence)>,
    /// The writer's job that began last.
    job: u64,
}

/// Learning's evidence, noted until a batch's end teaches it.
#[derive(Debug, Default)]
pub(crate) struct Behind(Mutex<Noted>);

impl Behind {
    fn lock(&self) -> std::sync::MutexGuard<'_, Noted> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes `lesson`, a page's, which no job of the writer takes back.
    pub(crate) fn lesson(&self, lesson: Lesson) {
        if !lesson.is_empty() {
            self.lock().entries.push((None, Evidence::Lesson(lesson)));
        }
    }

    /// Notes, for the writer's job running now, that the URL whose key is
    /// `key` was admitted from the page whose key is `from`.
    pub(crate) fn admitted(&self, key: Url, from: Option<Url>) {
        let mut noted = self.lock();
        let job = noted.job;
        noted
            .entries
            .push((Some(job), Evidence::Admitted { key, from }));
    }

    /// Whether any evidence waits to be taught.
    pub(crate) fn waiting(&self) -> bool {
        !self.lock().entries.is_empty()
    }

    /// Teaches, in `connection`'s transaction, every piece of evidence
    /// noted, in order, handing `pending` the requests a rule it teaches
    /// merges; `work` counts the rows of evidence it reads and writes.
    /// What it took is no longer noted, whether it succeeds or not.
    pub(crate) fn teach(
        &self,
        connection: &Connection,
        pending: &mut dyn EndMerged,
        work: &CountedWork,
    ) -> rusqlite::Result<()> {
        let entries = std::mem::take(&mut self.lock().entries);
        for (_, evidence) in entries {
            match evidence {
                Evidence::Admitted { key, from } => {
                    rules::admitted(connection, &key, from.as_ref(), Some(work))?;
                }
                Evidence::Lesson(lesson) => lesson.teach(connection, pending, work)?,
            }
        }
        Ok(())
    }
}

impl WriteWatch for Behind {
    fn began(&self) {
        let mut noted = self.lock();
        noted.job = noted.job.wrapping_add(1);
    }

    fn failed(&self) {
        let mut noted = self.lock();
        let job = Some(noted.job);
        noted.entries.retain(|(tag, _)| *tag != job);
    }
}

/// The watch of a Run's writes the Frontier keeps: what learning noted
/// behind, and the visits its writer has not committed when it holds them.
pub(crate) struct Watches(pub(crate) Vec<Arc<dyn WriteWatch>>);

impl fmt::Debug for Watches {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Watches")
            .field("watches", &self.0.len())
            .finish()
    }
}

impl WriteWatch for Watches {
    fn began(&self) {
        for watch in &self.0 {
            watch.began();
        }
    }

    fn failed(&self) {
        for watch in &self.0 {
            watch.failed();
        }
    }
}
