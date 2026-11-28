//! Sink rate limiting, batch sizing, adaptive concurrency and write timeouts
//! (ADRs 0026, 0059 and 0080):
//!
//! - [`Preset::for_sink`] gives every catalog destination a token bucket,
//!   batch bounds and adaptive concurrency window matched to its published
//!   quotas (such as Sheets' 60 writes/min and Airtable's 5 req/s with 10
//!   records each), overridden by `rate.*` and `batch.*` settings.
//! - [`TokenBucket`] refills deterministically from wall-clock readings of the
//!   engine [`Clock`], reporting quota exhaustion as `behind` rather than
//!   `failed`.
//! - [`BatchPolicy`] bounds each write by `batch.max_records` and
//!   `batch.max_bytes` while always admitting at least one Record so an
//!   oversized Record never stalls delivery.
//! - [`AdaptiveConcurrency`] lowers multiplicatively on HTTP 429, quota
//!   exhaustion or write timeout and recovers additively on consecutive
//!   successes (additive-increase, multiplicative-decrease, as in Vector).
//! - [`write_bounded`] bounds an async write with `timeout.sink_write` (30 s
//!   by default) on the engine [`Clock`].

use std::future::Future;
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_protocol::foxhound::sink::{SinkBatch, SinkRate, SinkType};

use crate::contract::{Delivered, Failure};

/// Default `timeout.sink_write` (30 s, ADR 0080).
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Nanoseconds in one second, used for exact integer token-bucket math.
const NANOS_PER_SEC: u128 = 1_000_000_000;

/// Token-bucket rate parameters for a Sink (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Sustained writes per second (at least 1).
    pub per_second: u64,
    /// Maximum burst capacity in tokens (at least 1).
    pub burst: u64,
}

impl RateLimit {
    /// A rate limit of `per_second` writes per second and `burst` capacity,
    /// each clamped to at least 1.
    #[must_use]
    pub const fn new(per_second: u64, burst: u64) -> Self {
        Self {
            per_second: if per_second == 0 { 1 } else { per_second },
            burst: if burst == 0 { 1 } else { burst },
        }
    }
}

/// Per-write batch bounds (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchPolicy {
    /// Maximum Records in one write (at least 1).
    pub max_records: usize,
    /// Maximum serialized Record bytes in one write (at least 1).
    pub max_bytes: usize,
}

impl BatchPolicy {
    /// A batch policy bounded by `max_records` and `max_bytes`, each clamped
    /// to at least 1.
    #[must_use]
    pub const fn new(max_records: usize, max_bytes: usize) -> Self {
        Self {
            max_records: if max_records == 0 { 1 } else { max_records },
            max_bytes: if max_bytes == 0 { 1 } else { max_bytes },
        }
    }
}

/// Destination preset for rate limiting, batch sizing and adaptive concurrency
/// (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    /// Token bucket parameters.
    pub rate: RateLimit,
    /// Batch size bounds.
    pub batch: BatchPolicy,
    /// Starting adaptive concurrency limit.
    pub initial_concurrency: usize,
    /// Upper adaptive concurrency bound.
    pub max_concurrency: usize,
}

impl Preset {
    /// The published quota preset for `sink_type` (or a conservative default
    /// for a program's own Sink handler when `None`).
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "exhaustive match over every catalog SinkType variant keeps every destination's preset in one table"
    )]
    pub const fn for_sink(sink_type: Option<SinkType>) -> Self {
        match sink_type {
            // Google Sheets: 60 writes/min per user (1/s, burst 10).
            Some(SinkType::GoogleSheets) => Self {
                rate: RateLimit::new(1, 10),
                batch: BatchPolicy::new(500, 2 * 1024 * 1024),
                initial_concurrency: 2,
                max_concurrency: 4,
            },
            // Airtable: 5 requests/sec, 10 records per batch.
            Some(SinkType::Airtable) => Self {
                rate: RateLimit::new(5, 5),
                batch: BatchPolicy::new(10, 512 * 1024),
                initial_concurrency: 2,
                max_concurrency: 5,
            },
            // Notion: 3 requests/sec average.
            Some(SinkType::Notion) => Self {
                rate: RateLimit::new(3, 3),
                batch: BatchPolicy::new(100, 512 * 1024),
                initial_concurrency: 2,
                max_concurrency: 3,
            },
            // BigQuery Storage Write API: 10 MiB batches.
            Some(SinkType::Bigquery) => Self {
                rate: RateLimit::new(20, 20),
                batch: BatchPolicy::new(10_000, 10 * 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // Snowflake: 100 MiB files/batches (100-250 MB range).
            Some(SinkType::Snowflake) => Self {
                rate: RateLimit::new(10, 10),
                batch: BatchPolicy::new(100_000, 100 * 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // Webhook: 256 KB default payload cap (ADR 0026).
            Some(SinkType::Webhook) => Self {
                rate: RateLimit::new(20, 20),
                batch: BatchPolicy::new(256, 256 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // Embedded local stores: high local throughput.
            Some(SinkType::Sqlite | SinkType::Duckdb) => Self {
                rate: RateLimit::new(1_000, 1_000),
                batch: BatchPolicy::new(256, 16 * 1024 * 1024),
                initial_concurrency: 1,
                max_concurrency: 4,
            },
            // Object stores: 256 MiB part sizing.
            Some(SinkType::S3 | SinkType::Gcs | SinkType::AzureBlob) => Self {
                rate: RateLimit::new(50, 50),
                batch: BatchPolicy::new(100_000, 256 * 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // Relational, warehouse, document and search stores.
            Some(
                SinkType::Postgres
                | SinkType::Pgvector
                | SinkType::Supabase
                | SinkType::Mysql
                | SinkType::Sqlserver
                | SinkType::Mongodb
                | SinkType::Elasticsearch
                | SinkType::Opensearch
                | SinkType::Redis
                | SinkType::Redshift
                | SinkType::Clickhouse
                | SinkType::Databricks
                | SinkType::Iceberg
                | SinkType::Motherduck,
            ) => Self {
                rate: RateLimit::new(50, 50),
                batch: BatchPolicy::new(1_000, 4 * 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // Vector stores.
            Some(
                SinkType::Qdrant
                | SinkType::Pinecone
                | SinkType::Weaviate
                | SinkType::Milvus
                | SinkType::Chroma
                | SinkType::Lancedb,
            ) => Self {
                rate: RateLimit::new(20, 20),
                batch: BatchPolicy::new(100, 2 * 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // Queues and streams.
            Some(
                SinkType::Kafka
                | SinkType::Sqs
                | SinkType::Sns
                | SinkType::Kinesis
                | SinkType::Firehose
                | SinkType::Pubsub
                | SinkType::ServiceBus
                | SinkType::EventHubs
                | SinkType::Rabbitmq
                | SinkType::Nats
                | SinkType::RedisStreams,
            ) => Self {
                rate: RateLimit::new(100, 100),
                batch: BatchPolicy::new(500, 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
            // File transfer and cloud drives.
            Some(
                SinkType::Sftp
                | SinkType::Ftp
                | SinkType::GoogleDrive
                | SinkType::Dropbox
                | SinkType::Onedrive
                | SinkType::Box,
            ) => Self {
                rate: RateLimit::new(10, 10),
                batch: BatchPolicy::new(10_000, 16 * 1024 * 1024),
                initial_concurrency: 2,
                max_concurrency: 8,
            },
            // Email providers.
            Some(
                SinkType::Smtp
                | SinkType::Ses
                | SinkType::Resend
                | SinkType::Postmark
                | SinkType::Sendgrid
                | SinkType::Mailgun,
            ) => Self {
                rate: RateLimit::new(14, 14),
                batch: BatchPolicy::new(50, 1024 * 1024),
                initial_concurrency: 2,
                max_concurrency: 8,
            },
            // Bridge Sinks and program handlers.
            Some(SinkType::Airbyte | SinkType::Singer) | None => Self {
                rate: RateLimit::new(20, 20),
                batch: BatchPolicy::new(256, 4 * 1024 * 1024),
                initial_concurrency: 4,
                max_concurrency: 16,
            },
        }
    }
}

/// Effective throughput configuration for one Sink: preset merged with
/// `rate.*`, `batch.*`, and `timeout.sink_write`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throughput {
    /// Effective token bucket parameters.
    pub rate: RateLimit,
    /// Effective batch bounds.
    pub batch: BatchPolicy,
    /// Starting adaptive concurrency limit.
    pub initial_concurrency: usize,
    /// Upper adaptive concurrency limit.
    pub max_concurrency: usize,
    /// Per-write timeout (`timeout.sink_write`).
    pub write_timeout: Duration,
}

impl Throughput {
    /// Builds effective throughput settings from `sink_type`'s preset,
    /// overridden by any `rate` and `batch` keys and bounded by
    /// `write_timeout`.
    #[must_use]
    pub fn from_settings(
        sink_type: Option<SinkType>,
        rate: Option<&SinkRate>,
        batch: Option<&SinkBatch>,
        write_timeout: Duration,
    ) -> Self {
        let preset = Preset::for_sink(sink_type);
        let per_second = rate
            .and_then(|r| r.per_second)
            .unwrap_or(preset.rate.per_second);
        let burst = rate.and_then(|r| r.burst).unwrap_or(preset.rate.burst);
        let max_records = batch
            .and_then(|b| b.max_records)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(preset.batch.max_records);
        let max_bytes = batch
            .and_then(|b| b.max_bytes)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(preset.batch.max_bytes);
        Self {
            rate: RateLimit::new(per_second, burst),
            batch: BatchPolicy::new(max_records, max_bytes),
            initial_concurrency: preset.initial_concurrency,
            max_concurrency: preset.max_concurrency,
            write_timeout,
        }
    }
}

/// Deterministic token bucket driven by wall-clock readings of [`Clock`].
#[derive(Clone, Debug)]
pub struct TokenBucket {
    rate: RateLimit,
    tokens: u64,
    remainder_nanos: u128,
    last_refill: Option<SystemTime>,
}

impl TokenBucket {
    /// Creates a full token bucket for `rate`.
    #[must_use]
    pub const fn new(rate: RateLimit) -> Self {
        Self {
            rate,
            tokens: rate.burst,
            remainder_nanos: 0,
            last_refill: None,
        }
    }

    /// Refills tokens up to `now` using exact integer nanosecond arithmetic.
    fn refill(&mut self, now: SystemTime) {
        let Some(last) = self.last_refill else {
            self.last_refill = Some(now);
            return;
        };
        let elapsed = now.duration_since(last).unwrap_or(Duration::ZERO);
        if elapsed.is_zero() {
            return;
        }
        self.last_refill = Some(now);
        if self.tokens >= self.rate.burst {
            self.remainder_nanos = 0;
            return;
        }
        let scaled = elapsed
            .as_nanos()
            .saturating_mul(u128::from(self.rate.per_second))
            .saturating_add(self.remainder_nanos);
        let added = u64::try_from(scaled / NANOS_PER_SEC).unwrap_or(u64::MAX);
        self.tokens = self.tokens.saturating_add(added).min(self.rate.burst);
        self.remainder_nanos = if self.tokens >= self.rate.burst {
            0
        } else {
            scaled % NANOS_PER_SEC
        };
    }

    /// Consumes one write token at `now`, or returns the exact wait until the
    /// next token becomes available.
    ///
    /// # Errors
    ///
    /// Returns the `Duration` until one token is ready when the bucket is
    /// empty.
    pub fn try_acquire(&mut self, now: SystemTime) -> Result<(), Duration> {
        self.refill(now);
        if self.tokens >= 1 {
            self.tokens -= 1;
            return Ok(());
        }
        let needed_scaled = NANOS_PER_SEC.saturating_sub(self.remainder_nanos);
        let wait_nanos = needed_scaled.div_ceil(u128::from(self.rate.per_second));
        Err(Duration::from_nanos(
            u64::try_from(wait_nanos).unwrap_or(u64::MAX),
        ))
    }

    /// Whole tokens currently available at `now`.
    #[must_use]
    pub fn available(&mut self, now: SystemTime) -> u64 {
        self.refill(now);
        self.tokens
    }
}

/// Additive-increase, multiplicative-decrease adaptive concurrency controller
/// (as in Vector, ADR 0026):
/// multiplicatively halves on throttling (HTTP 429, quota exhaustion,
/// write timeout) and additively recovers after consecutive successes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdaptiveConcurrency {
    limit: usize,
    max: usize,
    consecutive_successes: usize,
}

impl AdaptiveConcurrency {
    /// Creates a controller starting at `initial` and capped at `max` (each at
    /// least 1).
    #[must_use]
    pub const fn new(initial: usize, max: usize) -> Self {
        let max = if max == 0 { 1 } else { max };
        let initial = if initial == 0 {
            1
        } else if initial > max {
            max
        } else {
            initial
        };
        Self {
            limit: initial,
            max,
            consecutive_successes: 0,
        }
    }

    /// Current concurrency limit.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Records a successful write; after `limit` consecutive successes,
    /// increases the limit by 1 up to `max`.
    pub const fn record_success(&mut self) {
        self.consecutive_successes = self.consecutive_successes.saturating_add(1);
        if self.consecutive_successes >= self.limit {
            self.consecutive_successes = 0;
            if self.limit < self.max {
                self.limit += 1;
            }
        }
    }

    /// Records throttling (HTTP 429, quota exhaustion or timeout): halves the
    /// limit down to a floor of 1 and resets the recovery window.
    pub const fn record_throttle(&mut self) {
        self.consecutive_successes = 0;
        let halved = self.limit / 2;
        self.limit = if halved == 0 { 1 } else { halved };
    }
}

/// Serialized JSON size in bytes of `record`'s value, used to enforce
/// `batch.max_bytes`.
#[must_use]
pub fn record_wire_bytes(record: &Delivered) -> usize {
    serde_json::to_vec(&record.value).map_or(0, |bytes| bytes.len())
}

/// Bounds an async Sink write by `timeout` (`timeout.sink_write`) on `clock`
/// (ADRs 0026, 0059 and 0080). If `work` does not complete before `timeout`
/// elapses, `work` is dropped and [`Failure::deadline_exceeded`] is returned.
///
/// # Errors
///
/// Returns `work`'s [`Failure`], or [`Failure::deadline_exceeded`] when
/// `timeout` elapses first.
///
/// # Cancel safety
///
/// Dropping the returned future drops `work` and the timer; any side effect
/// `work` already sent over the wire is governed by the Sink's delivery ID and
/// run epoch.
pub async fn write_bounded<F, T>(clock: Clock, timeout: Duration, work: F) -> Result<T, Failure>
where
    F: Future<Output = Result<T, Failure>>,
{
    tokio::select! {
        biased;
        result = work => result,
        () = clock.sleep(timeout) => Err(Failure::deadline_exceeded(timeout)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use goodfetch_protocol::foxhound::sink::{SinkBatch, SinkRate, SinkType};

    use super::{
        AdaptiveConcurrency, DEFAULT_WRITE_TIMEOUT, Preset, RateLimit, Throughput, TokenBucket,
    };

    #[test]
    fn presets_match_published_quotas_and_settings_override_them() {
        let sheets = Preset::for_sink(Some(SinkType::GoogleSheets));
        assert_eq!(sheets.rate.per_second, 1);
        let airtable = Preset::for_sink(Some(SinkType::Airtable));
        assert_eq!(airtable.rate.per_second, 5);
        assert_eq!(airtable.batch.max_records, 10);
        let webhook = Preset::for_sink(Some(SinkType::Webhook));
        assert_eq!(webhook.batch.max_bytes, 256 * 1024);

        let overridden = Throughput::from_settings(
            Some(SinkType::Airtable),
            Some(&SinkRate {
                per_second: Some(2),
                burst: Some(1),
            }),
            Some(&SinkBatch {
                max_records: Some(4),
                max_bytes: Some(1024),
            }),
            Duration::from_secs(5),
        );
        assert_eq!(overridden.rate, RateLimit::new(2, 1));
        assert_eq!(overridden.batch.max_records, 4);
        assert_eq!(overridden.batch.max_bytes, 1024);
        assert_eq!(overridden.write_timeout, Duration::from_secs(5));
        assert_eq!(DEFAULT_WRITE_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn token_bucket_refills_exactly_without_drift() {
        let mut bucket = TokenBucket::new(RateLimit::new(2, 1));
        let t0 = UNIX_EPOCH;
        assert_eq!(bucket.try_acquire(t0), Ok(()));
        assert_eq!(bucket.try_acquire(t0), Err(Duration::from_millis(500)));
        assert_eq!(
            bucket.try_acquire(t0 + Duration::from_millis(250)),
            Err(Duration::from_millis(250))
        );
        assert_eq!(bucket.try_acquire(t0 + Duration::from_millis(500)), Ok(()));
    }

    #[test]
    fn adaptive_concurrency_halves_on_throttle_and_recovers_additively() {
        let mut concurrency = AdaptiveConcurrency::new(4, 8);
        assert_eq!(concurrency.limit(), 4);
        concurrency.record_throttle();
        assert_eq!(concurrency.limit(), 2);
        concurrency.record_throttle();
        assert_eq!(concurrency.limit(), 1);
        concurrency.record_throttle();
        assert_eq!(concurrency.limit(), 1);
        // Limit 1 needs 1 success to reach 2.
        concurrency.record_success();
        assert_eq!(concurrency.limit(), 2);
        // Limit 2 needs 2 successes to reach 3.
        concurrency.record_success();
        assert_eq!(concurrency.limit(), 2);
        concurrency.record_success();
        assert_eq!(concurrency.limit(), 3);
    }
}
