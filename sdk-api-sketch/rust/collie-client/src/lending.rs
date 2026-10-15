//! Lent pages (ADR 0089): [`Page::lend`] opens a CDP endpoint that shows only the page's Session,
//! for a library in the same program. Rust has no lending helper: chromiumoxide's `close()` kills
//! the browser, and a Rust program has Collie's page API and its CDP session (ADR 0089).

use std::collections::BTreeMap;

use crate::lifecycle::Page;
use crate::time::Call;

/// A lent page's endpoint: a WebSocket under ADR 0017's rules, its token in [`CdpEndpoint::headers`],
/// never in its URL. The attach ends with [`CdpEndpoint::close`], the library's `Browser.close`, its
/// last connection closing, the page closing or the Run ending; in a crawl handler, with the work
/// item's lease (ADR 0003). A cheap clone (ADR 0081).
#[derive(Debug, Clone)]
pub struct CdpEndpoint {
    _private: (),
}

impl CdpEndpoint {
    /// The endpoint's WebSocket URL.
    pub fn url(&self) -> &str {
        unimplemented!()
    }

    /// The headers that carry the attach's token.
    pub fn headers(&self) -> &BTreeMap<String, String> {
        unimplemented!()
    }

    /// Ends the attach. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Page {
    /// Lends the page to a CDP library in this program. Fails with `session_reserved` while another
    /// attach on the page's Session is open, and with `capability_unavailable` on a Firefox or WebKit
    /// page. While the library is attached, await each of the program's own Collie calls on the page
    /// before the library acts: an AI verb whose document a navigation it did not start replaces
    /// fails with `page_navigated_during_call` (ADR 0114).
    pub fn lend(&self) -> Call<CdpEndpoint> {
        unimplemented!()
    }
}

/// Details of `attach_ended`: why the attach was cut.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AttachEnded {
    /// `closed`, `lease`, `page_closed` or `run_ended`; a newer engine's reason is kept (G3).
    pub reason: String,
}

impl crate::results::ErrorDetails for AttachEnded {}
