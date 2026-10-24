//! What a page's navigation arrived at, read the same way for each load of
//! it: the document's response, or a download, which is published on the
//! call's events; and the Page check's judgement of that document.

use goodfetch_engine::events::Events;
use goodfetch_protocol::page::{PAGE_DOWNLOAD, PageDownload, PageId, PageResponse};

use crate::driver::{Arrived, Destination};
use crate::recovery::Judging;
use crate::sessions::OpenPage;

/// The document's response a navigation of the page `page_id` arrived at;
/// none for a document without one, and for a download, which is published
/// on `events`.
pub(super) async fn arrived(
    page_id: &PageId,
    arrived: Arrived,
    events: &Events,
) -> Option<PageResponse> {
    match arrived {
        Arrived::Document(response) => response,
        Arrived::Download {
            url,
            suggested_filename,
        } => {
            events
                .publish(
                    &PAGE_DOWNLOAD,
                    Some(&page_id.0),
                    &PageDownload {
                        page_id: page_id.clone(),
                        url,
                        suggested_filename,
                    },
                )
                .await;
            None
        }
    }
}

/// The Page check's judgement of the document `page`'s navigation to
/// `destination` reached.
pub(super) fn judging<'a>(
    page: &'a OpenPage,
    page_id: &'a PageId,
    destination: Destination<'a>,
) -> Judging<'a> {
    Judging {
        run: &page.run,
        session: &page.session,
        page_id,
        driver: &page.driver,
        declared: page.page_check.as_deref(),
        requested: match destination {
            Destination::Url(url) => Some(url),
            Destination::Reload | Destination::Back | Destination::Forward => None,
        },
        operation: destination.operation(),
    }
}
