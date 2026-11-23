//! The Record stage of an attempt (ADR 0104): once its handler answered,
//! each Record the attempt is about to commit, those it pushed in order and
//! then those its answer gives, goes to each `before_record_commit` Hook in
//! code its Label and Dataset reach, in the order the crawl declared them,
//! one call at a time, under the attempt's lease ([`super::hook_call`]). A
//! failed attempt commits none of what it staged, its enqueues, Records
//! and Key-value writes discarded together (ADR 0003), and is retried as
//! its failure says.
//!
//! A Hook reads each Record where the attempt staged it, never copied: what
//! the attempt pushed is set aside while its Hooks run and put back after,
//! so what it stages meanwhile, after its handler answered, is discarded.
//! A Run with no Hook at the Record stage sends no message here.

use goodfetch_protocol::foxhound::crawl::CrawlRequestHandled;

use super::hook_call::called;
use super::{Answer, On};
use crate::staging::SetAside;

/// The answer of an attempt whose handler answered `handled`, once each
/// Record it is about to commit went through the Record stage's Hooks:
/// `handled` when every call returned, else how the first that did not
/// ended the attempt.
pub(super) async fn passed(on: &On<'_>, handled: CrawlRequestHandled) -> Answer {
    let label = on.leased.crawl_request.label.as_str();
    let pushed = on.lease.set_aside();
    let records = pushed
        .iter()
        .flat_map(SetAside::records)
        .chain(&handled.records);
    let mut ended = None;
    'records: for record in records {
        for hook in on.hooks.on_record(label, &record.dataset) {
            if let Some(answer) = called(on, hook, Some(record)).await {
                ended = Some(answer);
                break 'records;
            }
        }
    }
    if let Some(pushed) = pushed {
        on.lease.put_back(pushed);
    }
    ended.unwrap_or(Answer::Handled(handled))
}
