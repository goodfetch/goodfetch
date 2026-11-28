//! Conformance tests for Sink rate limiting, batch sizing, adaptive
//! concurrency, quota exhaustion (`behind`, not `failed`), and
//! `timeout.sink_write` bounding hung writes (ADRs 0026, 0059 and 0080).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests run on Tokio's paused Clock (ADR 0059)"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_protocol::foxhound::report::SinkStatus;
use goodfetch_protocol::foxhound::sink::{SinkBatch, SinkRate, SinkType};
use goodfetch_sink::{
    Batch, BatchPolicy, Driver, Failure, FailureKind, GuaranteeClass, Preset, Primitives, Probe,
    RateLimit, RunCopy, Status, Throughput, Written,
};
use serde_json::json;

use crate::run_files::run_file;

const START_AT: SystemTime = UNIX_EPOCH;

fn copy_of(run_id: &str) -> RunCopy {
    RunCopy {
        run_id: run_id.to_owned(),
        run_epoch: 1,
    }
}

fn paused_clock(at: SystemTime) -> Clock {
    Clock::paused(at)
}

#[derive(Default)]
struct RecordingSink {
    batches: Vec<Vec<u64>>,
    batch_bytes: Vec<usize>,
    delivered_at: Vec<SystemTime>,
    clock: Option<Clock>,
    script: Vec<Result<(), Failure>>,
    cursor: usize,
}

impl RecordingSink {
    fn new(clock: Clock) -> Self {
        Self {
            clock: Some(clock),
            ..Self::default()
        }
    }

    fn with_script(mut self, script: Vec<Result<(), Failure>>) -> Self {
        self.script = script;
        self
    }

    fn last_delivered(&self) -> u64 {
        self.batches.iter().flatten().copied().max().unwrap_or(0)
    }
}

impl Primitives for RecordingSink {
    type Error = Failure;

    fn probe(&mut self, _run: &RunCopy) -> Result<Probe, Failure> {
        let mut delivered = BTreeMap::new();
        let last = self.last_delivered();
        if last > 0 {
            delivered.insert("items".to_owned(), last);
        }
        Ok(Probe {
            class: GuaranteeClass::Keyed,
            delivered,
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Failure> {
        if let Some(outcome) = self.script.get(self.cursor).cloned() {
            self.cursor += 1;
            outcome?;
        }
        let numbers: Vec<u64> = batch.records.iter().map(|r| r.number).collect();
        let bytes: usize = batch
            .records
            .iter()
            .map(|r| serde_json::to_vec(&r.value).unwrap().len())
            .sum();
        self.batches.push(numbers);
        self.batch_bytes.push(bytes);
        if let Some(clock) = self.clock {
            self.delivered_at.push(clock.wall());
        }
        Ok(Written::Delivered)
    }

    fn map_failure(&self, error: Failure) -> Failure {
        error
    }
}

/// A Sink under a low token-bucket quota delivers at the bucket's rate on a
/// paused `Clock`, and both local and destination quota exhaustion report
/// `behind`, never `failed` (ADR 0026).
///
/// Covers: story:6-293
#[tokio::test(start_paused = true)]
async fn low_quota_delivers_at_bucket_rate_on_paused_clock_and_reports_behind() {
    let clock = paused_clock(START_AT);
    let file = run_file(&[(
        "items",
        (1..=4).map(|i| json!({ "id": i })).collect::<Vec<_>>(),
    )]);

    let throughput = Throughput::from_settings(
        Some(SinkType::GoogleSheets),
        Some(&SinkRate {
            per_second: Some(2),
            burst: Some(1),
        }),
        Some(&SinkBatch {
            max_records: Some(1),
            max_bytes: None,
        }),
        Duration::from_secs(30),
    );
    let mut driver = Driver::new(
        "sheets",
        RecordingSink::new(clock),
        copy_of("quota-run"),
        vec!["items".to_owned()],
    )
    .with_clock(clock)
    .with_throughput(throughput);

    // First call sends Record 1 immediately, then hits empty bucket for Record 2.
    let mut delivered_steps = 0usize;
    loop {
        match driver.deliver(&file).unwrap() {
            Status::Complete => break,
            Status::Behind {
                failure, retry_in, ..
            } => {
                assert_eq!(failure.kind(), FailureKind::QuotaExhausted);
                assert_eq!(driver.report_status(), SinkStatus::Behind);
                assert_eq!(retry_in, Duration::from_millis(500));
                tokio::time::advance(retry_in).await;
                delivered_steps += 1;
            }
            other => panic!("expected Behind or Complete under quota, got {other:?}"),
        }
    }
    assert_eq!(delivered_steps, 3);
    assert_eq!(driver.report_status(), SinkStatus::Complete);

    let sink = driver.into_sink();
    assert_eq!(sink.batches, vec![vec![1], vec![2], vec![3], vec![4]]);
    let offsets_ms: Vec<u128> = sink
        .delivered_at
        .iter()
        .map(|at| at.duration_since(START_AT).unwrap().as_millis())
        .collect();
    assert_eq!(offsets_ms, vec![0, 500, 1000, 1500]);

    // Destination-signaled quota exhaustion also reports `behind`, never `failed`,
    // even across repeated exhaustions.
    let quota_sink = RecordingSink::new(clock).with_script(vec![
        Err(
            Failure::new(FailureKind::QuotaExhausted, "daily quota reached")
                .with_retry_after(Duration::from_secs(60)),
        ),
        Err(
            Failure::new(FailureKind::QuotaExhausted, "daily quota reached")
                .with_retry_after(Duration::from_secs(60)),
        ),
    ]);
    let mut dest_driver = Driver::new(
        "sheets-dest",
        quota_sink,
        copy_of("quota-dest"),
        vec!["items".to_owned()],
    )
    .with_clock(clock);
    for _ in 0..2 {
        let status = dest_driver.deliver(&file).unwrap();
        assert!(
            matches!(
                status,
                Status::Behind { ref failure, retry_in, .. }
                    if failure.kind() == FailureKind::QuotaExhausted
                        && retry_in == Duration::from_secs(60)
            ),
            "{status:?}"
        );
        assert_eq!(dest_driver.report_status(), SinkStatus::Behind);
        tokio::time::advance(Duration::from_secs(60)).await;
    }
}

/// Batch sizes respect both `batch.max_records` and `batch.max_bytes`, and
/// oversized singletons still deliver without stalling; destination presets
/// carry published API defaults (ADR 0026).
///
/// Covers: story:6-293
#[tokio::test(start_paused = true)]
async fn batch_sizes_respect_max_records_and_max_bytes() {
    let clock = paused_clock(START_AT);
    assert_eq!(
        Preset::for_sink(Some(SinkType::Airtable)).batch.max_records,
        10
    );
    assert_eq!(
        Preset::for_sink(Some(SinkType::Webhook)).batch.max_bytes,
        256 * 1024
    );

    // 1. max_records = 2 splits 5 records into [2, 2, 1].
    let file = run_file(&[(
        "items",
        (1..=5).map(|i| json!({ "n": i })).collect::<Vec<_>>(),
    )]);
    let mut by_count = Driver::new(
        "count-bounded",
        RecordingSink::new(clock),
        copy_of("batch-count"),
        vec!["items".to_owned()],
    )
    .with_clock(clock)
    .with_batch_policy(BatchPolicy::new(2, 1024 * 1024));
    assert_eq!(by_count.deliver(&file).unwrap(), Status::Complete);
    assert_eq!(
        by_count.into_sink().batches,
        vec![vec![1, 2], vec![3, 4], vec![5]]
    );

    // 2. max_bytes = 120 splits 5 records of ~55 bytes into [2, 2, 1], plus
    // an oversized 200-byte singleton that still delivers alone without deadlock.
    let payload_55 = "x".repeat(45);
    let payload_200 = "y".repeat(200);
    let byte_file = run_file(&[(
        "items",
        vec![
            json!({ "v": payload_55 }),
            json!({ "v": payload_55 }),
            json!({ "v": payload_200 }),
            json!({ "v": payload_55 }),
            json!({ "v": payload_55 }),
        ],
    )]);
    let mut by_bytes = Driver::new(
        "byte-bounded",
        RecordingSink::new(clock),
        copy_of("batch-bytes"),
        vec!["items".to_owned()],
    )
    .with_clock(clock)
    .with_batch_policy(BatchPolicy::new(100, 120));
    assert_eq!(by_bytes.deliver(&byte_file).unwrap(), Status::Complete);
    let sink = by_bytes.into_sink();
    assert_eq!(sink.batches, vec![vec![1, 2], vec![3], vec![4, 5]]);
    assert!(sink.batch_bytes[0] <= 120);
    assert!(sink.batch_bytes[1] > 120);
    assert!(sink.batch_bytes[2] <= 120);
}

/// Adaptive concurrency lowers multiplicatively on HTTP 429 and recovers
/// additively across subsequent successful writes (ADR 0026).
///
/// Covers: story:6-293
#[tokio::test(start_paused = true)]
async fn adaptive_concurrency_lowers_on_429_and_recovers() {
    let clock = paused_clock(START_AT);
    let file = run_file(&[(
        "items",
        (1..=4).map(|i| json!({ "id": i })).collect::<Vec<_>>(),
    )]);

    let sink = RecordingSink::new(clock).with_script(vec![
        Err(Failure::from_http_status(
            429,
            Some(Duration::from_secs(1)),
            "rate limited",
        )),
        Err(Failure::from_http_status(
            429,
            Some(Duration::from_secs(1)),
            "rate limited",
        )),
        Ok(()),
        Ok(()),
        Ok(()),
        Ok(()),
    ]);
    let mut driver = Driver::new(
        "adaptive",
        sink,
        copy_of("adaptive-run"),
        vec!["items".to_owned()],
    )
    .with_clock(clock)
    .with_batch_records(1)
    .with_concurrency(4, 8);
    assert_eq!(driver.concurrency_limit(), 4);

    // First 429 halves concurrency: 4 -> 2.
    let first = driver.deliver(&file).unwrap();
    assert!(matches!(first, Status::Behind { .. }), "{first:?}");
    assert_eq!(driver.concurrency_limit(), 2);
    tokio::time::advance(Duration::from_secs(1)).await;

    // Second 429 halves concurrency: 2 -> 1.
    let second = driver.deliver(&file).unwrap();
    assert!(matches!(second, Status::Behind { .. }), "{second:?}");
    assert_eq!(driver.concurrency_limit(), 1);
    tokio::time::advance(Duration::from_secs(1)).await;

    // Four consecutive successes recover concurrency additively:
    // 1 --(1 success)--> 2 --(2 successes)--> 3 (+1 progress toward 4).
    assert_eq!(driver.deliver(&file).unwrap(), Status::Complete);
    assert_eq!(driver.concurrency_limit(), 3);
    assert_eq!(
        driver.into_sink().batches,
        vec![vec![1], vec![2], vec![3], vec![4]]
    );
}

/// `timeout.sink_write` bounds a hung write on the paused `Clock`, lowers
/// adaptive concurrency, and returns `Behind` so the delivery retries and
/// completes once the destination responds (ADRs 0026, 0059 and 0080).
///
/// Covers: story:6-293, setting:timeout.sink_write
#[tokio::test(start_paused = true)]
async fn timeout_sink_write_bounds_hung_write_and_delivery_retries() {
    let clock = paused_clock(START_AT);
    let file = run_file(&[("items", vec![json!({ "id": 1 }), json!({ "id": 2 })])]);

    let mut driver = Driver::new(
        "hung",
        RecordingSink::new(clock),
        copy_of("timeout-run"),
        vec!["items".to_owned()],
    )
    .with_clock(clock)
    .with_rate(RateLimit::new(10, 10))
    .with_write_timeout(Duration::from_secs(3))
    .with_concurrency(4, 8);

    // Attempt 1 hangs forever; `timeout.sink_write` (3 s) bounds it cleanly.
    let started_at = clock.wall();
    let hung = driver
        .deliver_with(&file, |_batch| {
            std::future::pending::<Result<Written, Failure>>()
        })
        .await
        .unwrap();
    let elapsed = clock.wall().duration_since(started_at).unwrap();
    assert_eq!(elapsed, Duration::from_secs(3));
    let retry_in = match hung {
        Status::Behind {
            failure, retry_in, ..
        } => {
            assert_eq!(failure.kind(), FailureKind::Transient);
            assert!(failure.is_timeout());
            retry_in
        }
        other => panic!("expected Behind on hung write timeout, got {other:?}"),
    };
    assert_eq!(driver.report_status(), SinkStatus::Behind);
    assert_eq!(driver.concurrency_limit(), 2);

    // Attempt 2 retries after the scheduled backoff and delivers all Records.
    tokio::time::advance(retry_in).await;
    let received: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let status = driver
        .deliver_with(&file, |batch| {
            let received = Rc::clone(&received);
            async move {
                received
                    .borrow_mut()
                    .extend(batch.records.iter().map(|r| r.number));
                Ok(Written::Delivered)
            }
        })
        .await
        .unwrap();
    assert_eq!(status, Status::Complete);
    assert_eq!(driver.report_status(), SinkStatus::Complete);
    assert_eq!(*received.borrow(), vec![1, 2]);
}
