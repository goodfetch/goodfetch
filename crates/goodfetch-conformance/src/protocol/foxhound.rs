//! The names of Foxhound's entries the cases send, each checked when the
//! harness compiles against `goodfetch_protocol::foxhound_names`, the
//! generated list of what only Foxhound carries. The harness sits outside
//! `crates/foxhound/`, so it never names the protocol's `foxhound` module
//! (ADR 0099); a name the catalog renames or drops then fails the build
//! here, rather than a case sending an unknown method and blaming the
//! engine.

use goodfetch_protocol::foxhound_names::{HANDLER_REQUESTS, OPERATIONS, SETTINGS};

/// `crawl.start`.
pub(crate) const CRAWL_START: &str = listed("crawl.start", OPERATIONS);

/// `crawl_request.enqueue`.
pub(crate) const CRAWL_REQUEST_ENQUEUE: &str = listed("crawl_request.enqueue", OPERATIONS);

/// `run.add_seed`.
pub(crate) const RUN_ADD_SEED: &str = listed("run.add_seed", OPERATIONS);

/// `run.export`.
pub(crate) const RUN_EXPORT: &str = listed("run.export", OPERATIONS);

/// `run.failed_requests`.
pub(crate) const RUN_FAILED_REQUESTS: &str = listed("run.failed_requests", OPERATIONS);

/// `run.outcomes`, the reader of a crawl's outcomes in commit order.
pub(crate) const RUN_OUTCOMES: &str = listed("run.outcomes", OPERATIONS);

/// `run.records`, the cursor over a Dataset's Records.
pub(crate) const RUN_RECORDS: &str = listed("run.records", OPERATIONS);

/// `run.retry_failed`.
pub(crate) const RUN_RETRY_FAILED: &str = listed("run.retry_failed", OPERATIONS);

/// `run.seeds`.
pub(crate) const RUN_SEEDS: &str = listed("run.seeds", OPERATIONS);

/// `crawl_request.handle`, the handler request a crawl sends its Client.
pub(crate) const CRAWL_REQUEST_HANDLE: &str = listed("crawl_request.handle", HANDLER_REQUESTS);

/// `crawl.max_requests`, a setting only Foxhound carries.
pub(crate) const CRAWL_MAX_REQUESTS: &str = listed("crawl.max_requests", SETTINGS);

/// `$/record`, the notification that carries one Record `run.records`
/// reads. `foxhound_names` lists no notifications yet, so this one is
/// spelled here unchecked.
pub(crate) const RECORD: &str = "$/record";

/// `name`, which must be one of `list`: every caller is a constant, so a
/// name the list does not hold fails the build.
const fn listed(name: &'static str, list: &[&str]) -> &'static str {
    let mut index = 0;
    while index < list.len() {
        if same(name, list[index]) {
            return name;
        }
        index += 1;
    }
    panic!("a name the cases send is not one of Foxhound's in the catalog");
}

/// Whether `a` and `b` hold the same bytes, in a constant context.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut index = 0;
    while index < a.len() {
        if a[index] != b[index] {
            return false;
        }
        index += 1;
    }
    true
}
