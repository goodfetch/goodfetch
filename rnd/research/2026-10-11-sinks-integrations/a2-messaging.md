# A2. Messaging and streaming sinks: idempotence, fencing, Rust crates, CI doubles

Research date: 2026-10-11. Crate data comes from the crates.io API (`max_stable_version`, release `created_at`, `license`). Anything not read in a primary source is marked **inference**.

Classes used:
- **exactly-once fenced**: the destination can reject a write from a stale writer (epoch or compare-and-set), and can also dedup.
- **idempotent keyed**: the destination drops a repeated ID inside a window. It cannot tell a zombie from a retry.
- **at-least-once**: no server-side dedup. Dedup must happen downstream, on the delivery ID.

## Summary table

| Destination | Strongest primitive | Class | Rust crate (version, date, license, official/community) | CI double | First-party in |
|---|---|---|---|---|---|
| Apache Kafka | Transactions: `transactional.id` + producer epoch. `InitProducerId` bumps the epoch and fences the old instance (`ProducerFenced`). Idempotent producer (PID + per-partition sequence) dedups only within one producer session. | exactly-once fenced | `rdkafka` 0.39.0, 2026-01-25, MIT, community (librdkafka binding; transactions supported). Pure Rust: `rskafka` 0.6.0, 2025-03-20, MIT OR Apache-2.0 (no transactions). | testcontainers-modules `kafka` (`apache/kafka`, default tag 3.8.0; also `confluentinc/cp-kafka`) | Airbyte (community, alpha) |
| Redpanda | Same Kafka API. `enable_idempotence` and `enable_transactions` are cluster properties, both default true. | exactly-once fenced | `rdkafka` (as above) | Docker `redpandadata/redpanda`. No testcontainers-rs module found. | none found |
| Amazon SQS (FIFO) | `MessageDeduplicationId` or content-based dedup (SHA-256 of body). 5-minute window. Scope `queue` or `messageGroup` (`DeduplicationScope`). | idempotent keyed (5 min) | `aws-sdk-sqs` 1.114.0, 2026-10-01, Apache-2.0, official | LocalStack (now needs auth token); ElasticMQ (testcontainers `elasticmq`); moto server | none found |
| Amazon SQS (standard, incl. fair queues) | None. `MessageGroupId` on standard queues enables fair queues only. | at-least-once | `aws-sdk-sqs` | same | none found |
| Amazon SNS (FIFO topic) | Dedup ID or content-based dedup. 5-minute interval. Scope topic or message group (`FifoThroughputScope`). | idempotent keyed (5 min) | `aws-sdk-sns` 1.116.0, 2026-10-01, Apache-2.0, official | LocalStack; moto server | none found |
| Amazon SNS (standard topic) | None | at-least-once | `aws-sdk-sns` | same | none found |
| Kinesis Data Streams | None. `SequenceNumberForOrdering` orders puts; it does not reject or dedup. Producer retries create duplicates with new sequence numbers. | at-least-once | `aws-sdk-kinesis` 1.125.0, 2026-10-01, Apache-2.0, official | LocalStack; `etspaceman/kinesis-mock` (MIT); moto server | none found (Airbyte registry has no Kinesis destination today) |
| Amazon Data Firehose | None. `PutRecord`/`PutRecordBatch` retries can duplicate. Delivery to most destinations is at-least-once. | at-least-once | `aws-sdk-firehose` 1.123.0, 2026-10-01, Apache-2.0, official | LocalStack; moto server | none found |
| Google Cloud Pub/Sub | No publisher dedup. Exactly-once delivery is subscriber-side only (pull/StreamingPull, single region). Ordering keys order, they do not dedup. | at-least-once (publish side) | `google-cloud-pubsub` 1.6.0, 2026-09-30, Apache-2.0, official (googleapis). Old community crate now `gcloud-pubsub` 1.7.1, 2026-06-02, MIT. | Official emulator (`gcloud beta emulators pubsub start`); testcontainers `google_cloud_sdk_emulators` (PubSub, port 8085) | Bright Data (`gcp_pubsub`); Airbyte (community, alpha) |
| Azure Service Bus | Duplicate detection on `MessageId` (+`PartitionKey` if partitioned). Window default 10 min, min 20 s, max 7 days. Standard and Premium only. | idempotent keyed (20 s to 7 days) | Official `azure_messaging_servicebus` rewrite is in the repo but unreleased (`publish = false`). crates.io 0.21.0 (2024-10-15, MIT) is the legacy crate. Community: `azservicebus` 0.25.1, 2025-07-11, MIT (on `fe2o3-amqp` 0.18.2). | Official emulator `mcr.microsoft.com/azure-messaging/servicebus-emulator` + SQL Server container. Config accepts duplicate detection. | none found |
| Azure Event Hubs (AMQP) | No public dedup. Hidden idempotent publishing (producer group, owner level, sequence) exists only in .NET/Java SDKs, not in Rust. | at-least-once | `azure_messaging_eventhubs` 0.16.0, 2026-09-08, MIT, official (pre-1.0) | Official emulator `mcr.microsoft.com/azure-messaging/eventhubs-emulator` + Azurite | none found |
| Azure Event Hubs (Kafka endpoint) | Kafka idempotent producer supported. Kafka transactions in public preview, Premium and Dedicated only. | exactly-once fenced (preview; **inference** that fencing matches Apache Kafka) | `rdkafka` | Emulator speaks Kafka, but only producer and consumer APIs (no transactions) | none found |
| RabbitMQ (queues, AMQP 0-9-1) | Publisher confirms. Optional community plugin `rabbitmq-message-deduplication` (header-keyed cache with size and TTL). | at-least-once; idempotent keyed with plugin | `lapin` 4.12.1, 2026-10-09, MIT, community | testcontainers-modules `rabbitmq` (`rabbitmq:4.2-management`) | Airbyte (community, alpha) |
| RabbitMQ Streams | Named producer + strictly increasing publishing ID. Broker drops any ID <= last stored for that name. No time window. Dropped messages are still confirmed. | idempotent keyed (high-water mark, no window); silent fencing possible (**inference**) | `rabbitmq-stream-client` 0.11.0, 2026-03-18, Apache-2.0 OR MPL-2.0, official (RabbitMQ) | Same image with stream plugin enabled and port 5552 (**inference**: module does not enable it by default) | none found |
| NATS JetStream | `Nats-Msg-Id` dedup (stream `duplicate_window`, default 2 min). CAS headers: `Nats-Expected-Last-Sequence`, `Nats-Expected-Last-Subject-Sequence` (+`-Subject`), `Nats-Expected-Last-Msg-Id`, `Nats-Expected-Stream`. Atomic batch publish since 2.12. | exactly-once fenced | `async-nats` 0.50.0, 2026-07-20, Apache-2.0, official. Batch helper: `jetstream-extra` 0.3.0, 2026-05-11, Apache-2.0 (Synadia). | testcontainers-modules `nats` (`nats:2.10.14` default; override tag for 2.12+ batch) | none found |
| Redis Streams | Lua script or Function: check epoch key, `SET NX` delivery ID, `XADD`, all atomic. `XADD` with explicit ID rejects ID <= top. Redis 8.6+ `XADD IDMP pid iid` dedup. | exactly-once fenced (via script); idempotent keyed via IDMP (default 100 s, max 86,400 s) | `redis` 1.7.1, 2026-09-25, BSD-3-Clause, community (redis-rs; IDMP builder present). `fred` 10.1.0, 2025-02-27, MIT (last commit 2025-02-27). | testcontainers-modules `redis` (`redis:5.0` default; override to 8.6+ for IDMP) | Airbyte (community, alpha) |

Competitor checks with no messaging sink found: Apify integrations page, Scrapy built-in feed storages (file, stdout, S3, GCS, FTP), Spider Cloud connectors (`s3`, `gcs`, `google_sheets`, `azure_blob`, `supabase`), Zyte (custom item pipeline only), Firecrawl (HTTPS webhooks only), Browse AI, Octoparse, dlt (no messaging destination in `dlt/destinations/impl`).

## Per-destination notes

### Apache Kafka
- Idempotent producer: the broker gives each producer an ID and dedups on a sequence number. Source: https://kafka.apache.org/43/design/design/
- Dedup holds only inside one producer session, because a new instance gets a new PID. Source: KIP-98, https://cwiki.apache.org/confluence/display/KAFKA/KIP-98+-+Exactly+Once+Delivery+and+Transactional+Messaging
- Fencing: `InitProducerId` with a `transactional.id` bumps the epoch. A zombie with the same id then gets `ProducerFencedException`. Source: KIP-98 (same URL).
- KIP-890 part 2 bumps the epoch on every commit or abort, so late writes from an old transaction are fenced. Confluent's Kafka 4.0 notes say part 2 is complete. Sources: https://cwiki.apache.org/confluence/display/KAFKA/KIP-890%3A+Transactions+Server-Side+Defense and https://www.confluent.io/blog/introducing-apache-kafka-4-0/
- KIP-939 adds 2PC participation (`prepareTransaction`, `keepPreparedTxn`). Status Accepted; no release named on the page. Source: https://cwiki.apache.org/confluence/display/KAFKA/KIP-939%3A+Support+Participation+in+2PC
- KIP-447 changes fencing for consume-transform-produce to the consumer group generation. It does not matter for a pure producer sink. (**inference**, page not re-fetched.)
- Broker defaults: `transactional.id.expiration.ms` 7 days, `producer.id.expiration.ms` 1 day, `transaction.max.timeout.ms` 15 min, `transaction.partition.verification.enable` true. Source: https://kafka.apache.org/43/configuration/broker-configs/
- Mapping to Automan: Kafka compares its own producer epoch, not the run epoch. A resumed run that calls `init_transactions` with the same `transactional.id` fences the zombie. (**inference**)
- `rdkafka` exposes `init_transactions`, `begin_transaction`, `send_offsets_to_transaction`, `commit_transaction`, `abort_transaction`. Source: https://docs.rs/rdkafka/0.39.0/rdkafka/producer/trait.Producer.html
- `rdkafka-sys` on main bundles librdkafka 2.12.1; latest librdkafka is 2.16.0. librdkafka's changelog does not mention KIP-890, so client-side part 2 is likely absent; broker-side verification still applies. (**inference**) Sources: https://github.com/fede1024/rust-rdkafka/blob/master/rdkafka-sys/Cargo.toml and https://github.com/confluentinc/librdkafka/blob/master/CHANGELOG.md
- `rskafka` README lists "No support for ... transactions". Source: https://github.com/influxdata/rskafka
- Other crates: `kafka` 0.10.0 (2023-09-24, stale); `samsa` 0.1.8 (2025-09-24, license "non-standard"); `kafka-protocol` 0.18.0 (2026-08-20, codec only).
- testcontainers default tag 3.8.0 predates Kafka 4.0. Override the tag to test KIP-890 behavior. (**inference** on the need; tag read from module source.)

### Redpanda
- Cluster properties `enable_idempotence` and `enable_transactions` default to true. Idempotence holds only within a session. `transactional.id` is needed for cross-session guarantees. Sources: https://docs.redpanda.com/streaming/current/develop/produce-data/idempotent-producers/ and https://docs.redpanda.com/streaming/25.3/develop/transactions/
- No Redpanda module in testcontainers-modules 0.15.0 (feature list checked).

### Amazon SQS
- FIFO: within a 5-minute window, a repeated `MessageDeduplicationId` is acknowledged but not delivered. Tracking continues after the message is deleted. Source: https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/using-messagededuplicationid-property.html
- `ContentBasedDeduplication` hashes the body (not attributes) with SHA-256. An explicit ID overrides it. `DeduplicationScope` is `messageGroup` or `queue`; high throughput mode needs `messageGroup`. Source: https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/API_SetQueueAttributes.html
- Standard queues: `MessageDeduplicationId` applies only to FIFO. `MessageGroupId` on a standard queue enables fair queues, not dedup or ordering. Source: https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/API_SendMessage.html
- No compare-and-set, so no epoch rejection.

### Amazon SNS
- FIFO topics: same 5-minute dedup interval, content-based option, scope set by `FifoThroughputScope` (`Topic` or `MessageGroup`). With message filtering, delivery becomes at-most-once. Source: https://docs.aws.amazon.com/sns/latest/dg/fifo-message-dedup.html
- Standard topics have no dedup.

### Kinesis Data Streams
- Producer retries can store two records with the same data and different sequence numbers. AWS advises embedding a primary key and deduping downstream. Source: https://docs.aws.amazon.com/streams/latest/dev/kinesis-record-processor-duplicates.html
- `SequenceNumberForOrdering` only makes sequence numbers strictly increasing for one client and partition key. No error path rejects a stale writer. Source: https://docs.aws.amazon.com/kinesis/latest/APIReference/API_PutRecord.html

### Amazon Data Firehose
- Delivery is at-least-once. Retried Put calls can duplicate. HTTP endpoint deliveries keep the same request ID across retries, best effort. Iceberg "update" uses primary keys. Source: https://docs.aws.amazon.com/firehose/latest/dev/basic-deliver.html and https://docs.aws.amazon.com/firehose/latest/APIReference/API_PutRecordBatch.html

### AWS CI doubles
- LocalStack moved to one image on 2026-03-23. Latest releases need an auth token; a free plan exists for non-commercial use. Old community tags (for example 4.12) remain usable. Source: https://blog.localstack.cloud/localstack-single-image-next-steps/
- The `localstack/localstack` GitHub repo is archived (last push 2026-03-23).
- testcontainers `localstack` module defaults to tag 4.5, a pre-change tag.
- Token-free options: ElasticMQ (SQS, FIFO and content-based dedup config, Apache-2.0, v1.7.1 2026-04-05); moto server (Apache-2.0, 5.2.3 2026-08-22); kinesis-mock (MIT). Whether moto enforces the FIFO dedup window is not verified.

### Google Cloud Pub/Sub
- Exactly-once delivery covers acknowledgement and redelivery on pull and StreamingPull subscriptions, within one region. Publish-side duplicates can still arrive; client retries get new message IDs. Source: https://docs.cloud.google.com/pubsub/docs/exactly-once-delivery
- So a sink cannot dedup or fence on publish. Put the delivery ID in an attribute for downstream dedup. (**inference** on the attribute approach.)
- The `google-cloud-pubsub` crate is the googleapis Rust library. Its docs say @yoshidan donated the name; the old crate lives on as `gcloud-pubsub`. Source: https://docs.rs/google-cloud-pubsub/latest/google_cloud_pubsub/
- Emulator: `gcloud beta emulators pubsub start`. Limits include no IAM and no protobuf schemas. Exactly-once is not mentioned. Source: https://docs.cloud.google.com/pubsub/docs/emulator

### Azure Service Bus
- With duplicate detection on, a repeated `MessageId` is accepted and dropped. Partitioned entities key on `MessageId` + `PartitionKey`. Window default 10 min, min 20 s, max 7 days. Basic tier lacks it. Source: https://learn.microsoft.com/en-us/azure/service-bus-messaging/duplicate-detection
- No epoch check. Sessions lock receivers, not senders. (**inference**)
- Official Rust crate: `sdk/servicebus/azure_messaging_servicebus` in Azure/azure-sdk-for-rust has `publish = false` and a changelog "0.1.0 (Unreleased)". Source: https://github.com/Azure/azure-sdk-for-rust/tree/main/sdk/servicebus
- Emulator image `mcr.microsoft.com/azure-messaging/servicebus-emulator`, needs a SQL Server container. Config has `RequiresDuplicateDetection` and `DuplicateDetectionHistoryTimeWindow`. Source: https://learn.microsoft.com/en-us/azure/service-bus-messaging/test-locally-with-service-bus-emulator

### Azure Event Hubs
- .NET has `PartitionPublishingOptions` (`ProducerGroupId`, `OwnerLevel`, `StartingSequenceNumber`). A higher owner level disconnects lower ones in the same group, which is a real fence. But `EnableIdempotentPartitions` is `protected internal` and hidden, and it works only for direct partition publishing. Source: https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/eventhub/Azure.Messaging.EventHubs/src/Producer/PartitionPublishingOptions.cs
- The Rust crate has `owner_level` only on receivers, no idempotent publishing. Source: https://github.com/Azure/azure-sdk-for-rust/blob/main/sdk/eventhubs/azure_messaging_eventhubs/api.md
- Kafka endpoint: transactions are public preview on Premium and Dedicated. Source: https://learn.microsoft.com/en-us/azure/event-hubs/apache-kafka-transactions
- Emulator image `mcr.microsoft.com/azure-messaging/eventhubs-emulator` with Azurite. With Kafka, only producer and consumer APIs work. Source: https://learn.microsoft.com/en-us/azure/event-hubs/test-locally-with-event-hub-emulator

### RabbitMQ
- Queues: publisher confirms give at-least-once. Source: https://www.rabbitmq.com/docs/confirms (not re-fetched).
- Dedup plugin (noxdafox, MPL-2.0, latest release 0.8.0 on 2026-07-05): exchange type `x-message-deduplication` keyed on `x-deduplication-header`, bounded by `x-cache-size` and optional `x-cache-ttl`. Queue-level dedup does not support quorum queues. Needs RabbitMQ 3.13+. Disable it before upgrading to 4.3.0. Source: https://github.com/noxdafox/rabbitmq-message-deduplication
- Streams: the producer name must be unique per stream, and only one producer per name may run at a time. The broker drops IDs <= the last stored one and still confirms them. On restart, the client can query the last publishing ID. Source: https://www.rabbitmq.com/docs/streams
- Fencing idea: put the run epoch in the high bits of the publishing ID. A zombie's lower IDs are then dropped. The zombie is not told, because drops are confirmed. (**inference**)
- The Rust stream client has a `Dedup` producer type from `ProducerBuilder::name()`, and messages carry `publishing_id`. Source: https://github.com/rabbitmq/rabbitmq-stream-rust-client/blob/main/src/producer.rs

### NATS JetStream
- Header semantics (`Nats-Msg-Id`, all `Nats-Expected-*`, `Nats-Batch-Id/-Sequence/-Commit`): https://docs.nats.io/reference/jetstream/api/headers
- Default dedup window is 2 minutes, set per stream. The server-side `duplicate_window` limit caps it. Source: https://docs.nats.io/nats-concepts/jetstream/streams
- Atomic batch (ADR-50): server 2.12.0. `Nats-Expected-Last-Sequence` is allowed on the first message only. `Nats-Expected-Last-Subject-Sequence` works if earlier batch entries do not write that subject. `Nats-Expected-Last-Msg-Id` is rejected (error 10177). `Nats-Msg-Id` is allowed from 2.12.1. Limits: 1,000 messages per batch, 50 in-flight per stream, 10 s idle timeout. Source: https://github.com/nats-io/nats-architecture-and-design/blob/main/adr/ADR-50.md
- Fencing recipe: on resume, write an epoch marker to `fence.<run>`. Every data publish carries `Nats-Expected-Last-Subject-Sequence` = the marker's sequence and `...-Subject` = `fence.<run>`. A zombie still holds the old sequence and is rejected. Add `Nats-Msg-Id` = delivery ID for retry dedup. (**inference**, not tested)
- Latest nats-server release: v2.15.1 (2026-10-09).

### Redis Streams
- Explicit `XADD` IDs must be greater than every ID in the stream, or the command errors. Source: https://redis.io/docs/latest/commands/xadd/
- IDMP (8.6+): `XADD key IDMP pid iid *` returns the original entry ID on a repeat. `IDMPAUTO pid` hashes the content. Only with `*` IDs. `IDMP-DURATION` 1 to 86,400 s (default 100). `IDMP-MAXSIZE` 1 to 10,000 per producer (default 100); the size cap wins over the duration. Persisted in RDB/AOF. Changing settings with `XCFGSET` clears the map. Source: https://redis.io/docs/latest/develop/data-types/streams/idempotency/
- Fencing: a Lua script or Function runs atomically. It can compare a stored epoch, `SET NX` the delivery ID, then `XADD`. In Redis Cluster, all keys must share a hash slot (hash tag). Sources: https://redis.io/docs/latest/develop/interact/programmability/eval-intro/ and https://redis.io/docs/latest/develop/interact/transactions/ (not re-fetched)
- `redis` 1.7.1 has typed IDMP options (tag `redis-1.7.1`, `redis/src/commands/streams.rs`). Latest Redis release: 8.10.2 (2026-09-17).

### Competitor first-party support
- Airbyte OSS registry today: Kafka 0.1.11, Google PubSub 0.2.3, RabbitMQ 0.1.43, Redis 0.1.4. All are community support, alpha. Only PubSub and Redis are in the Cloud registry. No SQS, SNS, Kinesis, Firehose, Event Hubs, Service Bus, NATS or Redpanda destination. Source: https://connectors.airbyte.com/files/registries/v0/oss_registry.json
- Bright Data snapshot delivery types: `azure`, `build`, `email`, `gcs`, `gcp_pubsub`, `s3`, `sftp`, `snowflake`, `webhook`, `ali_oss`. Source: https://docs.brightdata.com/api-reference/scrapers/delivery-apis/deliver-snapshot
- Spider Cloud connectors: `s3`, `gcs`, `google_sheets`, `azure_blob`, `supabase`; webhooks are separate. Source: https://spider.cloud/docs/core/data-connectors/
- Apify: no messaging integration on https://docs.apify.com/platform/integrations (webhooks only).
- Scrapy: built-in feed storages are file, stdout, S3, GCS, FTP. Source: https://github.com/scrapy/scrapy/blob/master/scrapy/extensions/feedexport.py
- Zyte: file exports and BigQuery; message queues need a custom item pipeline. Source: https://docs.zyte.com/web-scraping/guides/export/index.html
- Firecrawl: HTTPS webhooks only. Source: https://docs.firecrawl.dev/webhooks/overview
- Browse AI: webhooks, Zapier, Make, Sheets, Airtable, S3. Source: https://help.browse.ai/what-integrations-are-available-on-browse-ai
- Octoparse: MySQL, SQL Server, PostgreSQL, Google Sheets, plus API and Zapier. Source: https://helpcenter.octoparse.com/en/articles/6470955-export-data-to-a-database-automatic
- dlt: destinations are warehouses, databases, vector stores and filesystem; none is a message broker. Source: https://github.com/dlt-hub/dlt/tree/devel/dlt/destinations/impl
