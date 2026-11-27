//! Conformance tests for Sink delivery retries, `Retry-After`, 410 pauses,
//! three-day failure pauses with Notifications, and Run-report Sink statuses
//! (ADRs 0026, 0059, 0102 and 0129).

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::runs::ReportPart as _;
use goodfetch_protocol::foxhound::report::{CrawlRunReport, SinkReport, SinkStatus};
use goodfetch_protocol::run::{RunReport, RunUsage};
use goodfetch_sink::{
    Batch, Driver, Failure, FailureKind, GuaranteeClass, PauseReason, Primitives, Probe, RunCopy,
    SCHEDULE, SinkNotificationEvent, SinkReportPart, SinkStateRow, Status, Written, dead_letters,
    report,
};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::json;

use crate::run_files::{commit, commit_at, run_file};

/// 2026-01-05T09:00:00Z.
const START_AT: SystemTime = UNIX_EPOCH;

fn copy_of(run_id: &str, run_epoch: u64) -> RunCopy {
    RunCopy {
        run_id: run_id.to_owned(),
        run_epoch,
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "integration test on paused Clock (ADR 0059)"
)]
fn paused_clock(at: SystemTime) -> Clock {
    Clock::paused(at)
}

enum ScriptedReply {
    Ok,
    Fail {
        status: u16,
        retry_after: Option<String>,
        date: Option<String>,
    },
}

/// A scripted HTTP-style destination that answers a sequence of failures (or
/// repeats the last one) before optionally succeeding.
struct ScriptedHttpSink {
    responses: Vec<ScriptedReply>,
    calls: usize,
    delivered: BTreeMap<String, u64>,
    clock: Clock,
}

impl ScriptedHttpSink {
    fn repeating(status: u16, retry_after: Option<&str>, date: Option<&str>, clock: Clock) -> Self {
        Self {
            responses: vec![ScriptedReply::Fail {
                status,
                retry_after: retry_after.map(str::to_owned),
                date: date.map(str::to_owned),
            }],
            calls: 0,
            delivered: BTreeMap::new(),
            clock,
        }
    }

    fn sequence(responses: Vec<ScriptedReply>, clock: Clock) -> Self {
        Self {
            responses,
            calls: 0,
            delivered: BTreeMap::new(),
            clock,
        }
    }
}

struct HttpReply {
    status: u16,
    retry_after: Option<String>,
    date: Option<String>,
    received: SystemTime,
}

impl Primitives for ScriptedHttpSink {
    type Error = HttpReply;

    fn probe(&mut self, _run: &RunCopy) -> Result<Probe, HttpReply> {
        Ok(Probe {
            class: GuaranteeClass::Keyed,
            delivered: self.delivered.clone(),
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, HttpReply> {
        let index = self.calls.min(self.responses.len().saturating_sub(1));
        self.calls += 1;
        match &self.responses[index] {
            ScriptedReply::Ok => {
                if let Some(last) = batch.records.last() {
                    self.delivered.insert(batch.dataset.clone(), last.number);
                }
                Ok(Written::Delivered)
            }
            ScriptedReply::Fail {
                status,
                retry_after,
                date,
            } => Err(HttpReply {
                status: *status,
                retry_after: retry_after.clone(),
                date: date.clone(),
                received: self.clock.wall(),
            }),
        }
    }

    fn map_failure(&self, error: HttpReply) -> Failure {
        Failure::from_http_response(
            error.status,
            error.retry_after.as_deref(),
            error.date.as_deref(),
            error.received,
            format!("HTTP {}", error.status),
        )
    }
}

/// 408, 429 and 5xx responses retry on the exact Standard Webhooks schedule on
/// a paused `Clock`, pause after three days of total failure with a
/// `sink_paused` Notification, and emit `sink_recovered` once resumed and
/// delivered.
///
/// Covers: story:6-290
#[rstest]
#[case::timeout_408(408)]
#[case::too_many_429(429)]
#[case::server_500(500)]
#[case::bad_gateway_502(502)]
#[case::unavailable_503(503)]
#[case::gateway_timeout_504(504)]
fn retryable_http_statuses_follow_standard_webhooks_schedule_and_pause_after_three_days(
    #[case] status: u16,
) {
    goodfetch_test_support::runtime().block_on(async {
        let clock = paused_clock(START_AT + Duration::from_hours(491_001));
        let run = run_file(&[]);
        commit(&run, "pages", 1, &[json!({ "url": "https://a.test" })]);

        let mut driver = Driver::new(
            "webhook",
            ScriptedHttpSink::repeating(status, None, None, clock),
            copy_of("run-retry", 1),
            vec!["pages".to_owned()],
        )
        .with_clock(clock);

        let mut observed_waits = Vec::new();
        loop {
            match driver.deliver(&run).unwrap() {
                Status::Behind {
                    failure,
                    retry_in,
                    next_at,
                } => {
                    assert_eq!(failure.kind(), FailureKind::Transient);
                    assert_eq!(driver.report_status(), SinkStatus::Behind);
                    assert_eq!(next_at, clock.wall() + retry_in);
                    observed_waits.push(retry_in);
                    tokio::time::advance(retry_in).await;
                }
                Status::Paused { reason, .. } => {
                    assert_eq!(reason, PauseReason::ThreeDaysFailing);
                    assert_eq!(driver.report_status(), SinkStatus::Failed);
                    assert_eq!(driver.paused_reason(), Some(PauseReason::ThreeDaysFailing));
                    break;
                }
                other => panic!("unexpected status for HTTP {status}: {other:?}"),
            }
        }
        assert_eq!(observed_waits, SCHEDULE.to_vec());

        // Three days of failure queued a `sink_paused` Notification.
        let notes = driver.take_notifications();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].event, SinkNotificationEvent::Paused);
        assert_eq!(notes[0].sink, "webhook");

        // Exercise resume after a 3-day pause -> queues `sink_recovered`.
        let mut driver2 = Driver::new(
            "webhook",
            ScriptedHttpSink::sequence(
                vec![
                    ScriptedReply::Fail {
                        status,
                        retry_after: None,
                        date: None,
                    },
                    ScriptedReply::Fail {
                        status,
                        retry_after: None,
                        date: None,
                    },
                    ScriptedReply::Ok,
                ],
                clock,
            ),
            copy_of("run-retry-2", 1),
            vec!["pages".to_owned()],
        )
        .with_clock(clock);
        assert!(matches!(
            driver2.deliver(&run).unwrap(),
            Status::Behind { .. }
        ));
        tokio::time::advance(Duration::from_hours(72)).await;
        assert!(matches!(
            driver2.deliver(&run).unwrap(),
            Status::Paused {
                reason: PauseReason::ThreeDaysFailing,
                ..
            }
        ));
        assert_eq!(
            driver2.take_notifications()[0].event,
            SinkNotificationEvent::Paused
        );
        driver2.resume();
        assert_eq!(driver2.deliver(&run).unwrap(), Status::Complete);
        assert_eq!(driver2.report_status(), SinkStatus::Complete);
        let recovered_notes = driver2.take_notifications();
        assert_eq!(recovered_notes.len(), 1);
        assert_eq!(recovered_notes[0].event, SinkNotificationEvent::Recovered);
    });
}

/// `Retry-After` in both seconds and HTTP-date form is honoured on a paused
/// `Clock` so retries never fire earlier than the destination asked.
///
/// Covers: story:6-290
#[test]
fn retry_after_seconds_and_http_date_are_honoured_on_paused_clock() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = paused_clock(START_AT + Duration::from_hours(491_001));
        let run = run_file(&[]);
        commit(&run, "pages", 1, &[json!({ "url": "https://a.test" })]);

        let responses = vec![
            // Seconds form: 120s > 5s schedule base -> must wait 120s.
            ScriptedReply::Fail {
                status: 429,
                retry_after: Some("120".to_owned()),
                date: None,
            },
            // HTTP-date form: 15 min after Date header > 5m schedule base -> must wait 15 min.
            ScriptedReply::Fail {
                status: 503,
                retry_after: Some("Mon, 05 Jan 2026 09:17:00 GMT".to_owned()),
                date: Some("Mon, 05 Jan 2026 09:02:00 GMT".to_owned()),
            },
            ScriptedReply::Ok,
        ];
        let mut driver = Driver::new(
            "api",
            ScriptedHttpSink::sequence(responses, clock),
            copy_of("run-ra", 1),
            vec!["pages".to_owned()],
        )
        .with_clock(clock);

        match driver.deliver(&run).unwrap() {
            Status::Behind {
                retry_in, next_at, ..
            } => {
                assert_eq!(retry_in, Duration::from_mins(2));
                assert_eq!(next_at, clock.wall() + Duration::from_mins(2));
                tokio::time::advance(retry_in).await;
            }
            other => panic!("expected Behind on 429, got {other:?}"),
        }

        match driver.deliver(&run).unwrap() {
            Status::Behind {
                retry_in, next_at, ..
            } => {
                assert_eq!(retry_in, Duration::from_mins(15));
                assert_eq!(next_at, clock.wall() + Duration::from_mins(15));
                tokio::time::advance(retry_in).await;
            }
            other => panic!("expected Behind on 503 with HTTP-date, got {other:?}"),
        }

        assert_eq!(driver.deliver(&run).unwrap(), Status::Complete);
        assert_eq!(driver.report_status(), SinkStatus::Complete);
    });
}

/// HTTP 410 pauses the Sink immediately without retrying, and non-retryable
/// 4xx responses (401, 403, 404, 405) fail immediately without retrying.
///
/// Covers: story:6-290
#[test]
fn http_410_pauses_the_sink_and_non_retryable_4xx_fail_immediately() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = paused_clock(START_AT + Duration::from_hours(491_001));
        let run = run_file(&[]);
        commit(&run, "pages", 1, &[json!({ "url": "https://a.test" })]);

        // 410 pauses the Sink; subsequent deliver() calls do not touch the destination.
        let mut driver_410 = Driver::new(
            "gone-sink",
            ScriptedHttpSink::repeating(410, None, None, clock),
            copy_of("run-410", 1),
            vec!["pages".to_owned()],
        )
        .with_clock(clock);
        assert!(matches!(
            driver_410.deliver(&run).unwrap(),
            Status::Paused {
                reason: PauseReason::Gone,
                ..
            }
        ));
        assert_eq!(driver_410.paused_reason(), Some(PauseReason::Gone));
        assert_eq!(driver_410.report_status(), SinkStatus::Failed);
        assert!(matches!(
            driver_410.deliver(&run).unwrap(),
            Status::Paused {
                reason: PauseReason::Gone,
                ..
            }
        ));
        assert_eq!(driver_410.into_sink().calls, 1);

        // 401, 403, 404, 405 fail immediately (never retried on the schedule).
        for code in [401u16, 403, 404, 405] {
            let mut driver = Driver::new(
                "perm-sink",
                ScriptedHttpSink::repeating(code, None, None, clock),
                copy_of("run-perm", 1),
                vec!["pages".to_owned()],
            )
            .with_clock(clock);
            assert!(
                matches!(driver.deliver(&run).unwrap(), Status::Failed(_)),
                "HTTP {code} must fail immediately without retrying"
            );
            assert_eq!(driver.report_status(), SinkStatus::Failed);
        }
    });
}

/// The Run report (`SinkReportPart`) carries each Sink's status: `complete`,
/// `complete_with_rejections`, `behind` and `failed`.
///
/// Covers: story:6-290
#[test]
fn run_report_gives_each_sink_status_complete_rejections_behind_or_failed() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE run (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             run_id TEXT NOT NULL,
             kind TEXT NOT NULL,
             run_epoch INTEGER NOT NULL
         ) STRICT;
         INSERT INTO run VALUES (1, 'run-report', 'crawl', 1);",
    )
    .unwrap();
    dead_letters::ensure_table(&conn).unwrap();
    report::ensure_tables(&conn).unwrap();

    for (name, status) in [
        ("alpha", SinkStatus::Complete),
        ("beta", SinkStatus::CompleteWithRejections),
        ("gamma", SinkStatus::Behind),
        ("delta", SinkStatus::Failed),
    ] {
        report::record_state(
            &conn,
            &SinkStateRow {
                sink: name.to_owned(),
                guarantee_class: GuaranteeClass::Fenced,
                status,
                paused_reason: (status == SinkStatus::Failed).then_some(PauseReason::Gone),
                outside_dedup_window: 0,
                undeclared_datasets: Vec::new(),
            },
        )
        .unwrap();
    }
    dead_letters::record(
        &conn,
        &dead_letters::DeadLetter {
            sink: "beta".to_owned(),
            delivery_id: goodfetch_sink::DeliveryId::new("run-report", "beta", "pages", 1),
            run: copy_of("run-report", 1),
            dataset: "pages".to_owned(),
            number: 1,
            committed_at: "2026-01-05T09:00:00Z".to_owned(),
            value: json!({ "url": "https://bad.test" }),
            reason: "rejected".to_owned(),
            attempts: 1,
        },
        &Masks::new(),
    )
    .unwrap();

    let mut run_report: RunReport = serde_json::from_value(json!({
        "run_id": "run-report",
        "lifecycle": "finished",
        "verdict": "verified",
        "completeness": "complete",
        "started_at": "2026-01-05T09:00:00Z",
        "ended_at": "2026-01-05T09:01:00Z",
        "usage": serde_json::to_value(RunUsage::default()).unwrap(),
    }))
    .unwrap();
    SinkReportPart.fill(&conn, &mut run_report).unwrap();

    let encoded = serde_json::to_value(&run_report).unwrap();
    let crawl_report: CrawlRunReport = serde_json::from_value(encoded).unwrap();
    let sinks: Vec<SinkReport> = crawl_report.sinks.unwrap();
    let by_name: BTreeMap<&str, (SinkStatus, u64)> = sinks
        .iter()
        .map(|s| (s.name.as_str(), (s.status, s.dead_letters)))
        .collect();
    assert_eq!(by_name["alpha"], (SinkStatus::Complete, 0));
    assert_eq!(by_name["beta"], (SinkStatus::CompleteWithRejections, 1));
    assert_eq!(by_name["gamma"], (SinkStatus::Behind, 0));
    assert_eq!(by_name["delta"], (SinkStatus::Failed, 0));
}

/// A destination with a finite deduplication window (e.g. a 5-minute FIFO
/// queue) that rejects Records containing `"bad": true`.
struct DedupWindowSink {
    window: Duration,
    delivered: BTreeMap<String, u64>,
}

impl Primitives for DedupWindowSink {
    type Error = &'static str;

    fn probe(&mut self, _run: &RunCopy) -> Result<Probe, &'static str> {
        Ok(Probe {
            class: GuaranteeClass::Keyed,
            delivered: self.delivered.clone(),
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, &'static str> {
        if batch
            .records
            .iter()
            .any(|r| r.value.get("bad").and_then(serde_json::Value::as_bool) == Some(true))
        {
            return Err("rejected");
        }
        if let Some(last) = batch.records.last() {
            self.delivered.insert(batch.dataset.clone(), last.number);
        }
        Ok(Written::Delivered)
    }

    fn dedup_window(&self) -> Option<Duration> {
        Some(self.window)
    }

    fn map_failure(&self, error: &'static str) -> Failure {
        Failure::new(FailureKind::RejectedRecords, error)
    }
}

/// After a Run with both a dead letter and a late delivery past the
/// destination's deduplication window, the Run report carries the Sink's
/// Guarantee class, its dead-letter count and its `outside_dedup_window`
/// count (ADRs 0026 and 0102).
///
/// Covers: story:6-298
#[test]
fn late_delivery_outside_dedup_window_and_dead_letters_appear_in_run_report() {
    goodfetch_test_support::runtime().block_on(async {
        let committed_at = START_AT + Duration::from_hours(491_001);
        let committed_rfc3339 = "2026-01-05T09:00:00Z";
        let clock = paused_clock(committed_at);
        let run = run_file(&[]);
        run.execute_batch(
            "CREATE TABLE run (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 run_id TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 run_epoch INTEGER NOT NULL
             ) STRICT;
             INSERT INTO run VALUES (1, 'run-late', 'crawl', 1);",
        )
        .unwrap();
        dead_letters::ensure_table(&run).unwrap();
        report::ensure_tables(&run).unwrap();

        commit_at(
            &run,
            "pages",
            1,
            &[
                json!({ "url": "https://a.test/ok" }),
                json!({ "url": "https://a.test/bad", "bad": true }),
            ],
            committed_rfc3339,
        );

        let mut driver = Driver::new(
            "fifo",
            DedupWindowSink {
                window: Duration::from_mins(5),
                delivered: BTreeMap::new(),
            },
            copy_of("run-late", 1),
            vec!["pages".to_owned()],
        )
        .with_clock(clock);

        // Batch 1 lands within the 5-minute window: 1 good Record, 1 dead letter.
        tokio::time::advance(Duration::from_mins(1)).await;
        assert_eq!(
            driver.deliver(&run).unwrap(),
            Status::CompleteWithRejections
        );
        assert_eq!(driver.outside_dedup_window(), 0);
        for entry in driver.take_dead_letters() {
            dead_letters::record(&run, &entry, &Masks::new()).unwrap();
        }

        // Batch 2 was committed at `committed_at` but delivered after 10 minutes
        // (outside the 5-minute dedup window).
        commit_at(
            &run,
            "pages",
            3,
            &[json!({ "url": "https://a.test/late" })],
            committed_rfc3339,
        );
        tokio::time::advance(Duration::from_mins(9)).await;
        assert_eq!(driver.deliver(&run).unwrap(), Status::Complete);
        assert_eq!(driver.outside_dedup_window(), 1);

        report::record_state(
            &run,
            &SinkStateRow {
                sink: "fifo".to_owned(),
                guarantee_class: driver.class().unwrap(),
                status: driver.report_status(),
                paused_reason: driver.paused_reason(),
                outside_dedup_window: driver.outside_dedup_window(),
                undeclared_datasets: Vec::new(),
            },
        )
        .unwrap();

        let mut run_report: RunReport = serde_json::from_value(json!({
            "run_id": "run-late",
            "lifecycle": "finished",
            "verdict": "verified",
            "completeness": "complete",
            "started_at": "2026-01-05T09:00:00Z",
            "ended_at": "2026-01-05T09:10:00Z",
            "usage": serde_json::to_value(RunUsage::default()).unwrap(),
        }))
        .unwrap();
        SinkReportPart.fill(&run, &mut run_report).unwrap();

        let crawl_report: CrawlRunReport =
            serde_json::from_value(serde_json::to_value(&run_report).unwrap()).unwrap();
        assert_eq!(
            crawl_report.sinks.unwrap(),
            vec![SinkReport {
                name: "fifo".to_owned(),
                guarantee_class: GuaranteeClass::Keyed,
                dead_letters: 1,
                outside_dedup_window: 1,
                status: SinkStatus::CompleteWithRejections,
                undeclared_datasets: Vec::new(),
            }]
        );
    });
}
