//! The Notification payload's writer for the Compatibility corpus (ADR
//! 0066; issue 1391): builds, through the `webhook` channel's own
//! [`payload`], at ADR 0059's test `Clock` start, the body of a `full`
//! delivery and of a `thin` one, and writes them into the folder given as
//! the only argument as `full.json` and `thin.json`. `xtask compat write`
//! runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_notification::message::{EventId, Message};
use goodfetch_notification::webhook::{Payload, payload};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_protocol::point_in_time;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::schedule::ScheduleId;
use url::Url;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let message = Message {
        event_id: EventId::new("evt_01JDQ8X2Y5").expect("invariant: an event ID"),
        event: NotificationEvent::RUN_FAILED,
        run_id: Some(RunId::parse("nightly-prices").expect("invariant: a Run ID")),
        schedule_id: Some(ScheduleId::parse("prices").expect("invariant: a Schedule ID")),
        product: Product::Collie,
        title: "Run failed".to_owned(),
        text: "The Run nightly-prices failed: navigation_failed.".to_owned(),
        link: Some(
            Url::parse("http://127.0.0.1:7777/runs/nightly-prices").expect("invariant: a URL"),
        ),
        occurred_at: point_in_time::from_rfc3339(START).expect("invariant: an RFC 3339 time"),
    };
    for (name, kind) in [("full.json", Payload::Full), ("thin.json", Payload::Thin)] {
        let mut text = serde_json::to_string_pretty(&payload(&message, kind))
            .expect("invariant: a Notification payload serializes");
        text.push('\n');
        std::fs::write(output.join(name), text).expect("invariant: the output folder is writable");
    }
}
