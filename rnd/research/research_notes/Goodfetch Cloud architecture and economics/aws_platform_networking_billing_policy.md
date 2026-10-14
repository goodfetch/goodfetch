# AWS platform around Goodfetch Cloud's compute layer: storage, front door, networking and egress, IP reputation, policy, metering, observability, programs and security (as of 2026-10-12)

*Scope: everything around the compute layer (compute and isolation choices belong to another researcher; exact AWS prices to another). Prices below are only the ones needed to size a trap, mostly us-east-1 list prices. The shared web-search budget for this session ran out part-way through (200 searches across all agents), so the second half of the work relied on direct fetches of official pages; anything not confirmed that way is marked secondary or unverified, and listed under Gaps.*

*Discrepancy to flag first: the assignment says resuming on another machine "needs shared storage (ADR 0002)". ADR 0002's own title and text say the opposite: runs resume from a checkpoint (a self-contained copy of the run's SQLite file written to a location the caller gives), not from shared live storage. The cloud needs shared storage for checkpoints (S3), not a shared file system under the live database. This governs the storage recommendation in section 1.*

## 1. Storage per need: datasets, results and recordings; checkpoints and run state; control-plane metadata; queues and crawl frontiers; usage event streams

### Takeaway
The live run database must stay on the run host's local disk, because SQLite's WAL mode does not work on network file systems; checkpoints go to S3, fenced by run epoch with S3 conditional writes. Results and recordings go to S3 Standard or Intelligent-Tiering as large batched objects. Control-plane metadata fits Aurora PostgreSQL better than Aurora DSQL, because DSQL uses optimistic concurrency and caps transactions. SQS standard queues with fair queues should dispatch runs. Usage events should be batched into S3 through Firehose; Kinesis On-demand Advantage and MSK Serverless have high fixed floors.

### Cited Findings

**Run state and checkpoints (what the engine requires)**
- Each run's state (frontier, staged results, sessions, effect journal, usage) is its own SQLite database in WAL mode with one writer. A checkpoint is a consistent, self-contained copy written to a location the caller gives and reported with size, digest, run epoch and journal position. The cloud stores checkpoints wherever it likes and never swaps storage inside the engine; Goodfetch Cloud always uses one database per run. — [ADR 0002](../../../../docs/adr/0002-portable-run-state-resume-from-checkpoints.md)
- By default an acknowledged commit survives a power cut (`synchronous=FULL`). An opt-in fast mode survives only a process crash. Checkpoints are taken on `checkpoint` and `migrate` signals, on a clean stop, and at a caller-set interval. After a machine is lost, the run redoes the work since its last checkpoint. — [ADR 0002](../../../../docs/adr/0002-portable-run-state-resume-from-checkpoints.md)
- The run epoch cannot be enforced inside the run's own file. Every write leaving the engine (checkpoint, export, sink write) carries the epoch, and the destination must enforce it: the cloud either kills the old copy before resuming or rejects stale epochs with a compare-and-set. — [ADR 0003](../../../../docs/adr/0003-two-level-fencing-at-least-once-delivery.md)
- Secret fields in run state are encrypted with a random per-run data key, wrapped by a key-encryption key (KEK) the caller holds: a KMS key in Goodfetch Cloud. A checkpoint is useless without it, and discarding a run destroys the key. Results at rest are left to disk or volume encryption, and DuckDB must be able to read run files. — [ADR 0015](../../../../docs/adr/0015-secret-references-and-caller-held-run-keys.md)
- SQLite's WAL mode requires every process using a database to be on the same host. It does not work over a network file system, because readers share a memory-mapped wal-index. Setting `locking_mode=EXCLUSIVE` before first access lets WAL work without shared memory, but only when one process is guaranteed to be the sole accessor. — [SQLite: Write-Ahead Logging](https://www.sqlite.org/wal.html)
- S3 conditional writes:
  - `If-None-Match: *` makes a write create-only. When several conditional writes race on one key, the first to finish wins and the others get `412 Precondition Failed`.
  - `If-Match: <ETag>` makes a write a compare-and-set against the current ETag.
  - Both work on PutObject, CompleteMultipartUpload and CopyObject, and bucket policies can make conditional writes mandatory.
  - A concurrent delete can cause `409 Conflict` or `404`.
  - [S3 conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html)
- S3 Express One Zone:
  - Stores data in one Availability Zone that you choose, designed for 99.95% availability within that zone.
  - Single-digit-millisecond access, up to 10x faster than S3 Standard, up to 2 million requests per second per directory bucket.
  - Request costs up to 80% lower than S3 Standard; supports appending to existing objects.
  - [S3 Express One Zone product page](https://aws.amazon.com/s3/storage-classes/express-one-zone/); [S3 Express AZs and Regions](https://docs.aws.amazon.com/AmazonS3/latest/userguide/s3-express-Endpoints.html)
- S3 Express One Zone in Europe, conflicting sources:
  - A July 7, 2026 announcement added Frankfurt and said the class was then in 8 Regions — [What's New, July 2026](https://aws.amazon.com/about-aws/whats-new/2026/07/s3-express-one-zone-europe-frankfurt/).
  - The current documentation table (fetched 2026-10-12) lists 15 Regions: eu-central-1 (2 AZ IDs), eu-west-1 (2), eu-west-3 (1), eu-north-1 (3), us-east-1 (3), us-east-2 (2), us-west-2 (3), and others — [S3 Express AZs and Regions](https://docs.aws.amazon.com/AmazonS3/latest/userguide/s3-express-Endpoints.html).
  - The documentation is the more recent source.

**Datasets, results and session recordings**
- S3 Intelligent-Tiering:
  - Moves an object to the Infrequent Access tier after 30 days without access, and to Archive Instant Access after 90.
  - Archive Access (90 days or more) and Deep Archive Access (180 days or more) are optional tiers, and objects there must be restored before they can be read.
  - A small per-object monitoring and automation charge applies.
  - Objects under 128 KB are never monitored or tiered and always stay in the Frequent Access tier.
  - [How S3 Intelligent-Tiering works](https://docs.aws.amazon.com/AmazonS3/latest/userguide/intelligent-tiering-overview.html)
- S3 request and transfer prices (us-east-1):
  - GET requests cost $0.0004 per 1,000.
  - The first 100 GB per month of internet egress is free, aggregated across services.
  - S3 Tables (managed Apache Iceberg) list $0.0265 per GB-month, $0.005 per 1,000 PUTs and $0.0004 per 1,000 GETs.
  - The S3 Standard storage and PUT table did not render in the fetch.
  - [S3 pricing](https://aws.amazon.com/s3/pricing/)

**Control-plane metadata**
- Aurora Serverless v2 auto-pause:
  - A minimum of 0 ACUs (Aurora capacity units) lets an instance pause after 300 to 86,400 seconds with no user connections.
  - No instance charge while paused; storage is still billed.
  - A resume typically takes about 15 seconds, and 30 seconds or more after a pause longer than 24 hours.
  - An attached RDS Proxy, logical replication, a global-database primary or a zero-ETL integration prevents pausing.
  - [Aurora Serverless v2 auto-pause](https://docs.aws.amazon.com/AmazonRDS/latest/AuroraUserGuide/aurora-serverless-v2-auto-pause.html)
- Aurora DSQL (current docs), what it supports:
  - It is PostgreSQL 16-compatible and now supports foreign keys, views and sequences or identity columns.
  - It has no temporary tables and no PL/pgSQL (only SQL functions); AWS advises moving trigger logic into the application.
  - Isolation is fixed at Repeatable Read, under optimistic concurrency: conflicts surface at commit as serialization errors that the application must retry.
  - [DSQL PostgreSQL compatibility and migration](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/working-with-postgresql-compatibility-unsupported-features.html)
- Aurora DSQL limits:
  - Transactions: DDL and DML go in separate transactions, with one DDL statement per transaction. A transaction may modify up to 3,000 rows and 10 MiB, and may last at most 5 minutes.
  - Connections: each lasts at most 60 minutes, with 10,000 per cluster and 100 new connections per second.
  - Objects: 1 database per cluster, 10 schemas and 1,000 tables.
  - Storage: 10 TiB by default, up to 256 TiB on request.
  - 5 CDC (change data capture) streams per cluster.
  - [DSQL compatibility](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/working-with-postgresql-compatibility-unsupported-features.html); [DSQL quotas](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/CHAP_quotas.html)
- Reports from 2025 said DSQL lacked foreign keys, views, triggers and sequences. The current docs supersede them on foreign keys, views and sequences. — [DevClass](https://devclass.com/?p=7160); [Andrew Baker, Nov 2025](https://andrewbaker.ninja/2025/11/19/amazon-aurora-dsql-performance-limits-architecture/)
- DSQL Regions:
  - Europe: eu-central-1, eu-west-1, eu-west-2, eu-west-3, eu-south-2 and eu-north-1; US: us-east-1, us-east-2 and us-west-2.
  - A multi-Region cluster must stay inside one continental Region set; cross-continent clusters are not supported.
  - [What is Aurora DSQL](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/what-is-aurora-dsql.html)

**Queues, schedules and crawl frontiers**
- Each run keeps its frontier as bounded per-domain queues inside the engine, journaled and surviving a resume. Politeness defaults are 8 requests in flight per domain, with backoff on 429 and 503. — [ADR 0004](../../../../docs/adr/0004-fair-per-domain-admission.md); [ADR 0021](../../../../docs/adr/0021-standard-access-by-default-and-escalation-by-new-session.md)
- SQS fair queues (July 2025):
  - Producers mark each message's tenant with a `MessageGroupId` on a standard queue.
  - SQS detects a tenant with a disproportionate share of in-flight messages and returns other tenants' messages first, which keeps quiet tenants' dwell time low.
  - No consumer change and no throughput limit; there is no limit on the number of tenants.
  - SQS does not rate-limit a noisy tenant: its messages still flow when consumers have spare capacity.
  - CloudWatch adds metrics for the quiet groups.
  - [SQS fair queues](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-fair-queues.html); [What's New, July 2025](https://aws.amazon.com/about-aws/whats-new/2025/07/amazon-sqs-introduces-fair/)
- Goodfetch Cloud's gateway must implement the `schedules.*` operations with its own durable scheduler and the same semantics as the Daemon. A caller-chosen run ID makes `runs.start` idempotent. — [ADR 0036](../../../../docs/adr/0036-the-daemon-schedules-runs-above-the-engine.md)
- EventBridge Scheduler:
  - 10,000,000 schedules per Region by default, adjustable to billions; completed one-time schedules count until deleted.
  - 1,000 invocations per second by default in primary Regions, including eu-central-1 and eu-west-1, adjustable.
  - 256 KB input payload.
  - [EventBridge Scheduler quotas](https://docs.aws.amazon.com/scheduler/latest/UserGuide/scheduler-quotas.html)
- ElastiCache Serverless for Valkey (us-east-1):
  - $0.084 per GB-hour and $0.0023 per million ECPUs (the request compute unit).
  - 100 MB minimum metered storage, against 1 GB for Redis OSS.
  - Priced 33% below other engines.
  - [ElastiCache pricing](https://aws.amazon.com/elasticache/pricing/)
- Lambda durable functions now exist as an AWS-native durable-execution option: 5 million running executions per Region by default, 3,000 operations and 100 MB of state per execution. — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)

**Usage event streams**
- The engine counts usage and enforces limits but never knows prices. The cloud drives it through the same JSON-RPC protocol, where usage is an ordinary message. — [ADR 0001](../../../../docs/adr/0001-single-tenant-engine-cloud-as-protocol-client.md)
- Goodfetch Cloud's gateway keeps its own ledger per Tenant, with the local ledger's scopes and semantics. Limits are reserved in slices, each taken in one transaction, sized as the smaller of 1% of the Limit and the remainder divided by twice the number of live runs. — [ADR 0041](../../../../docs/adr/0041-limits-across-runs-through-a-per-user-usage-ledger.md)
- Kinesis Data Streams On-demand Advantage (November 4, 2025):
  - Ingest $0.032/GB, retrieval $0.016/GB, extended retention $0.023/GB-month, and no per-stream fee.
  - Once enabled, the account is billed for at least 25 MB/s of ingest and 25 MB/s of retrieval across all its on-demand streams.
  - AWS says the mode pays off at 10 MB/s or more in aggregate, more than two consumers, or hundreds of streams.
  - [What's New](https://aws.amazon.com/about-aws/whats-new/2025/11/amazon-kinesis-data-streams-ondemand-advantage); [Kinesis pricing](https://aws.amazon.com/kinesis/data-streams/pricing/)
- Amazon Data Firehose, Direct PUT:
  - $0.029/GB for the first 500 TB per month.
  - Each record is rounded up to the nearest 5 KB.
  - Delivery to Apache Iceberg tables is billed per GB without that rounding.
  - Conversion to Parquet or ORC costs $0.018/GB.
  - [Firehose pricing](https://aws.amazon.com/firehose/pricing/)
- MSK Serverless (us-east-1): $0.75 per cluster-hour, $0.0015 per partition-hour, $0.10/GB in, $0.05/GB out and $0.10/GB-month storage. — [MSK pricing](https://aws.amazon.com/msk/pricing/)

### Inferences

#### Recommendations
- **Live run state goes on local disk:** NVMe instance store or EBS gp3 on the run's host. It never goes on EFS, FSx, NFS or an S3 file mount, because WAL with a single writer, `synchronous=FULL` and the shared-memory index all assume one host.
- **Checkpoints use S3 Standard (Regional, multi-AZ) as the system of record:**
  - Key layout: `tenants/{tenant}/runs/{run}/e{epoch}/ckpt-{journal_position}.sqlite.zst`, written with `If-None-Match`.
  - Fencing: a small per-run manifest (or the run row in the control-plane database) is advanced with `If-Match` or a conditional update. That makes S3 or the database the compare-and-set that ADR 0003 requires, and a stale epoch's write fails with 412.
  - Resume: download the manifest's checkpoint, verify its digest, start the next epoch.
- **S3 Express One Zone, only as a staging tier:** a measured option for frequent periodic checkpoints in the runner's own AZ, always promoted to Standard, because an AZ outage would otherwise lose the very checkpoints that exist to survive machine loss. Express is in Frankfurt, Ireland, Paris (one AZ) and Stockholm, so the same design can serve the US and the EU.
- **Datasets and results:** S3 Standard, written as large chunk objects (JSONL or Parquet of several MB each, or the per-run SQLite export DuckDB reads), indexed by tenant, run and dataset in the control-plane database. Never one object per item: request charges scale per object (S3 Tables' $0.005 per 1,000 PUTs prices 100 million items at about $500 in PUTs alone), and objects under 128 KB never tier down. Parquet (optionally S3 Tables/Iceberg) also serves analytics and DuckDB or Athena queries.
- **Session recordings:** large objects, so S3 Intelligent-Tiering works for them (Infrequent Access after 30 days without access), with lifecycle expiry per plan retention, served through CloudFront signed URLs.
- **Control-plane database: Aurora PostgreSQL** (Serverless v2 or provisioned), one cluster per geography (US and EU), with a minimum ACU above 0 in production. Auto-pause only in dev and staging, because a 15 to 30 second resume would stall API calls. Reasons this beats Aurora DSQL here (correctness and maintainability, not effort):
  - The per-tenant ledger of ADR 0041 is a set of hot rows updated by many concurrent runs. Under DSQL's optimistic concurrency, concurrent slice-takers conflict and retry, while PostgreSQL row locks serialize them.
  - DSQL's caps (3,000 rows, 10 MiB and 5 minutes per transaction) and its lack of temporary tables and PL/pgSQL constrain invoicing, retention sweeps and backfills.
  - DSQL becomes the better choice only if active-active writes across Regions of one continent are required. Since it cannot span continents, the US and the EU would be separate clusters either way.
- **DynamoDB is optional,** for very high-rate key-value items: session-to-Browser-host routing, API-key lookup cache, idempotency keys.
- **Queues and schedules:**
  - Run dispatch, webhook delivery and sink retries: SQS standard queues with fair queues (`MessageGroupId` = tenant).
  - Tenant schedules: EventBridge Scheduler as the time trigger only. It should fire into SQS with a deterministic run ID (schedule ID plus occurrence time), so `runs.start` stays idempotent. ADR 0036's semantics (skips on Limit, missed occurrences, after-run and after-Watch triggers) stay in Goodfetch's own scheduler logic.
- **Crawl frontier:** stays inside each run (ADR 0002, 0004), so no ElastiCache frontier. The cloud does need a limiter per target domain across runs and tenants, because ADR 0004's politeness is per engine, and fifty tenants' runs on one site add up. Valkey Serverless token buckets fit (100 MB minimum). This also cuts abuse complaints (section 5).
- **Usage stream:** the gateway, a protocol client, receives usage messages.
  - It applies ledger slices transactionally in Aurora (the real-time caps).
  - It appends raw usage records, batched many to a record, to Firehose, which writes them to S3 as Iceberg tables for audit and Athena or DuckDB.
  - Kinesis On-demand Advantage only once aggregate traffic passes about 10 to 25 MB/s; MSK only if Kafka itself is wanted.

#### Cost traps (storage and streams)
- **Kinesis On-demand Advantage floor:** 25 MB/s is about 64,800 GB a month each of ingest and retrieval. At $0.032 and $0.016 per GB that is about $3,100 a month before any real traffic (computed from the listed rates).
- **MSK Serverless floor:** about $548 a month per cluster ($0.75 × 730 hours) before partitions and data.
- **Firehose rounding:** a usage message of about 300 bytes is billed as 5 KB, roughly 17 times its size, unless many are batched into each record.
- **Small objects:** many sub-128 KB objects pay per-request costs and never tier down.
- **Aurora:** an RDS Proxy silently prevents auto-pause, and auto-pause adds 15 to 30 seconds or more of first-connection latency.
- **S3 Express One Zone:** single-AZ, so it is unsafe as the only copy of a checkpoint.

### Gaps
- Exact S3 Standard storage and PUT prices, the Intelligent-Tiering monitoring fee, and S3 Express storage and per-GB charges were not captured (the pricing table did not render); the pricing researcher has them.
- Whether EC2 traffic to an S3 Express directory bucket in another AZ of the same Region is charged was not verified.
- EBS gp3 against instance store for SQLite fsync latency, and EBS attach limits for a same-AZ volume reattach after host loss, were not researched.
- The July 2026 "8 Regions" count against the 15-Region documentation table is unresolved.
- DynamoDB's November 2024 price change could not be verified: the announcement URL tried returned 404.
- RDS for PostgreSQL against Aurora PostgreSQL price and performance was not researched.

## 2. API front door and live connections (API Gateway, ALB, CloudFront with Lambda function URLs, NLB; WebSocket limits; tenant authentication)

### Takeaway
API Gateway cannot carry hosted-browser traffic. Its WebSockets close after 2 hours or 10 idle minutes and refuse messages over 128 KB, and its HTTP APIs cut off at 30 seconds. Lambda caps at 15 minutes. The live plane should therefore be a Regional NLB (TCP, idle timeout up to 6,000 s) in front of a Goodfetch-owned session gateway, and the REST and crawl API should be CloudFront plus WAF in front of an ALB (idle timeout up to 4,000 s). For tenant identity, WorkOS or Clerk provide B2B organizations and per-customer enterprise SSO that Cognito leaves to the integrator. Programmatic access should use Goodfetch-issued scoped API keys.

### Cited Findings
- API Gateway WebSocket APIs:
  - Fixed limits: 2-hour maximum connection, 10-minute idle timeout.
  - Messages: 32 KB frames and 128 KB message payloads; anything larger closes the connection with code 1009.
  - 500 new connections per second per account per Region, adjustable.
  - [WebSocket API quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/apigateway-execution-service-websocket-limits-table.html)
- API Gateway HTTP APIs: 30-second maximum integration timeout, not adjustable; 10 MB payload. — [HTTP API quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/http-api-quotas.html)
- API Gateway REST APIs: 29-second integration timeout by default; Regional and private APIs can go higher, possibly at the cost of a lower Region throttle. 10 MB payload; 310-second idle connection timeout. — [REST API quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-execution-service-limits-table.html)
- Lambda:
  - 15-minute maximum timeout.
  - 6 MB synchronous request and response; 200 MB streamed response, throttled to 2 MBps after the first 6 MB.
  - The new Lambda MicroVMs run up to 8 hours.
  - [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- ALB:
  - Idle timeout 1 to 4,000 seconds, 60 by default; HTTP/2 PING frames do not reset it.
  - HTTP client keepalive from 60 seconds to 7 days.
  - [ALB attributes](https://docs.aws.amazon.com/elasticloadbalancing/latest/application/edit-load-balancer-attributes.html)
- ALB can verify JWTs in request headers (November 2025): signature, `exp`, `iss`, and up to 10 extra claims, aimed at machine-to-machine traffic and needing no application change. — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/11/application-load-balancer-jwt-verification/); [ALB JWT docs](https://docs.aws.amazon.com/de_de/elasticloadbalancing/latest/application/listener-verify-jwt.html)
- NLB:
  - TCP idle timeout 350 seconds by default, configurable from 60 to 6,000.
  - TLS listeners have a fixed 350-second idle timeout, with keepalives the load balancer generates every 20 seconds.
  - With an NLB timeout above 350 seconds, the target ENI's connection-tracking timeout must be raised: on 6th-generation Nitro instances it defaults to 350 seconds and would drop connections.
  - [NLB docs](https://docs.aws.amazon.com/elasticloadbalancing/latest/network/network-load-balancers.html)
- CloudFront supports WebSockets on any distribution once the `Sec-WebSocket-*` headers are forwarded, over HTTP/1.1 only. — [CloudFront WebSockets](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/distribution-working-with.websockets.html)
- CloudFront flat-rate plans:

  | Plan | Price per month | Requests | Data transfer |
  |---|---|---|---|
  | Free | $0 | 1M | 100 GB |
  | Pro | $15 | 10M | 50 TB |
  | Business | $200 | 125M | 50 TB |
  | Premium | $1,000 | 500M (adjustable to 6B) | 50 TB (adjustable to 600 TB) |

  - No overage charges; DDoS-blocked and WAF-blocked requests do not count.
  - Transfer from AWS origins to CloudFront is waived.
  - [CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/)
- The flat-rate plans bundle WAF, DDoS protection, Route 53 DNS, CloudWatch log ingestion and edge compute (secondary). — [BlazingCDN (a competitor)](https://blog.blazingcdn.com/en-us/cloudfront-flat-rate-plans-explained-limits-and-fine-print-2026)
- Every engine endpoint (MCP Streamable HTTP, JSON-RPC daemon, Console, live view, raw CDP) binds to loopback by default and checks `Host`, `Origin` and a per-launch token. Remote exposure needs TLS and a token, and OAuth 2.1 for remote MCP. — [ADR 0017](../../../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md)
- The Browser host speaks the engine's JSON-RPC, needs TLS and a token when reached remotely, takes one network round trip per action, and serves one Session before it is wiped. Raw CDP is opt-in. — [ADR 0022](../../../../docs/adr/0022-remote-browsers-driven-through-a-browser-host.md)
- Cognito:
  - Feature plans: Lite, Essentials (the default; managed login, passkeys) and Plus (risk-based adaptive authentication). Machine-to-machine authorization is priced on usage, with no free tier for its token requests. — [Cognito pricing](https://aws.amazon.com/cognito/pricing/)
  - Essentials costs $0.015 per MAU after 10,000 free; Plus costs $0.020 per MAU (secondary). — [Logto](https://blog.logto.io/amazon-cognito-pricing)
  - The Stack called the new pricing complicated and potentially costly. — [The Stack](https://www.thestack.technology/awss-new-cognito-pricing-complicated-potentially-costly/)
- WorkOS:
  - AuthKit is free for the first 1 million MAU, then $2,500 per extra million.
  - SSO and Directory Sync cost $125 per connection for the first 15, falling to $65 at 51 to 100.
  - Audit-log streaming is $125 per month per connection.
  - [WorkOS pricing](https://workos.com/pricing)
- Clerk:
  - 50,000 monthly retained users (MRU) free per app, then $0.02 per MRU, decreasing with volume.
  - 100 monthly retained organizations included, then $1 each.
  - One SSO connection on Pro and Business, then $75 per month each; a B2B add-on at $100 per month.
  - Built-in API keys and machine-to-machine tokens with included quotas.
  - [Clerk pricing](https://clerk.com/pricing)
- Ory Kratos is Apache-2.0, but organization SSO, SAML, SCIM and multi-tenancy require Ory's enterprise license. — [Ory Kratos](https://github.com/ory/kratos)

### Inferences
- **API Gateway WebSockets are disqualified for hosted browsers** on three counts:
  - The 2-hour cap ends long agent sessions.
  - The 128 KB message cap breaks screenshots, DOM snapshots and screencast frames, which routinely exceed it.
  - Per-message pricing scales with chatty JSON-RPC traffic.
- **The HTTP API's fixed 30-second timeout is a trap for synchronous scrape and extract calls,** which can exceed 30 seconds with browser rendering and model extraction. Use an ALB, or an asynchronous job with polling or a webhook.
- **Lambda function URLs** fit webhook receivers and light endpoints, not sessions.
- **Recommended live plane** (JSON-RPC to Browser hosts, raw CDP when opted in, live view): a Regional NLB on a TCP listener, not a TLS listener (fixed 350-second idle), with WebSocket pings, in front of a Goodfetch session gateway written in Rust. The gateway:
  - terminates TLS and validates the per-session token of ADR 0017;
  - looks up which Browser host owns the session;
  - proxies to it over the private network;
  - meters bytes per tenant, since internet egress is billed (section 3);
  - caps connections per tenant.

  An ALB with WebSocket and JWT verification of the upgrade request works too, but it still needs this gateway tier to route to thousands of individual hosts.
- **Recommended REST, crawl API, Console and MCP Streamable HTTP:** CloudFront with WAF and DDoS protection (a flat-rate plan gives a predictable bill), in front of an ALB, in front of the services.
- **CloudFront for live traffic** terminates TLS at the edge, but every message still travels to the Region, so placing Regions near customers matters more for ADR 0022's per-action round trip: us-east-1 plus eu-central-1 or eu-west-1.
- **Human identity:** WorkOS (AuthKit free to 1 million MAU, enterprise SSO and SCIM billed per customer connection) or Clerk (organizations and API keys built in, billed on retained users). Cognito is the cheapest AWS-native option, but per-tenant organizations, roles and per-customer SSO become Goodfetch's code. Ory's open-source edition lacks organizations and SSO.
- **Programmatic identity:** Goodfetch-issued API keys and scoped, expiring tokens per tenant and project, mirroring ADR 0031's program tokens. Validate them in the gateway, or mint short-lived JWTs from them for ALB to verify.

### Gaps
- ALB and NLB capacity-unit pricing (new connections, active connections, processed bytes) for long-lived sessions was not captured.
- CloudFront's origin response and keepalive timeouts for long synchronous API calls were not verified.
- Whether flat-rate plans cover WebSocket traffic or restrict use cases was not verified (the AWS re:Post explainer returned 403).
- AWS Global Accelerator for live-session latency was not researched.
- WorkOS and Clerk data-residency options for EU tenants were not verified.
- Whether ALB JWT verification applies only to the WebSocket upgrade request was not verified.

## 3. Networking and egress traps (NAT Gateway, public IPv4, IPv6 egress-only gateways, NAT instances, VPC endpoints, inter-AZ, ingress against egress, CloudFront)

### Takeaway
Internet ingress is free, but a NAT Gateway charges $0.045 for every GB it processes in either direction. A crawler pulling terabytes through NAT pays about $45 per TB for bytes that cost nothing through an internet gateway. Crawler and browser hosts should sit in public subnets on an internet gateway, using public IPv4 at $0.005 an hour each plus free IPv6, keep S3 and DynamoDB traffic on free gateway endpoints, and stay within one AZ. NAT (the Regional NAT Gateway or fck-nat) is for low-volume control-plane egress that needs stable, allowlistable IPs. Egress to customers costs $0.09/GB in the first tier and should go through CloudFront where it can.

### Cited Findings
- NAT Gateway is billed per hour and per GB processed; in the US East (Ohio) example, $0.045 an hour and $0.045 a GB. AWS says the processing charge applies to each GB regardless of the traffic's source or destination, so responses count too, and standard data transfer charges still apply. — [VPC pricing](https://aws.amazon.com/vpc/pricing/); [NAT gateway pricing docs](https://docs.aws.amazon.com/vpc/latest/userguide/nat-gateway-pricing.html)
- AWS's own advice: keep resources in the NAT gateway's AZ (or run one per AZ), and use gateway or interface endpoints for traffic to AWS services. — [NAT gateway pricing docs](https://docs.aws.amazon.com/vpc/latest/userguide/nat-gateway-pricing.html)
- Regional NAT Gateway (November 19, 2025):
  - One gateway expands and contracts across a VPC's AZs and needs no public subnet.
  - Billing is per AZ-hour covered plus the same per-GB processing.
  - Each AZ scales to 32 IPs, each IP holds 55,000 concurrent connections per unique destination, and scale-up starts at about 40,000 connections.
  - Expanding into a new AZ takes 15 to 20 minutes on average and up to 60, with traffic crossing AZs (and charged) meanwhile.
  - Private connectivity was not supported at launch. Available in all commercial Regions except GovCloud and China.
  - [What's New](https://aws.amazon.com/about-aws/whats-new/2025/11/aws-nat-gateway-regional-availability); [AWS blog](https://aws.amazon.com/blogs/networking-and-content-delivery/introducing-amazon-vpc-regional-nat-gateway); [VPC pricing](https://aws.amazon.com/vpc/pricing/); [InfoQ](https://www.infoq.com/news/2025/12/aws-regional-nat-gateway)
- Public IPv4 addresses:
  - Since February 1, 2024 every public IPv4 address costs $0.005 an hour, in use or idle, Amazon-provided or Elastic IP; billed per second with a 60-second minimum.
  - EC2's free tier covers 750 hours a month for 12 months; BYOIP addresses are not charged.
  - AWS recommends IPv6, and offers Public IP Insights in IPAM free of charge.
  - [AWS blog](https://aws.amazon.com/blogs/aws/new-aws-public-ipv4-address-charge-public-ip-insights/); [VPC pricing](https://aws.amazon.com/vpc/pricing/)
- An egress-only internet gateway (IPv6 outbound from private subnets) has no charge of its own; normal data transfer charges apply. — [Egress-only internet gateway](https://docs.aws.amazon.com/vpc/latest/userguide/egress-only-internet-gateway.html)
- Data transfer rules:
  - Inbound from the internet is free for all services in all Regions; outbound is charged by the originating Region.
  - Traffic within one AZ is free, and traffic between AZs is charged.
  - Gateway endpoints to S3 and DynamoDB carry no data transfer charge; interface endpoints charge per hour and per GB.
  - Reaching AWS services through a NAT gateway incurs processing.
  - [AWS Architecture blog, 2021](https://aws.amazon.com/blogs/architecture/overview-of-data-transfer-costs-for-common-architectures/)
- Egress pricing:
  - AWS's Regional NAT example prices internet egress at $0.09/GB — [VPC pricing](https://aws.amazon.com/vpc/pricing/).
  - The first 100 GB a month is free, aggregated across services; above 500 TB a month the rate is negotiated with sales — [EC2 pricing](https://aws.amazon.com/ec2/pricing/on-demand/).
  - EU customers can request reduced rates for eligible use cases under the EU Data Act — [EC2 pricing](https://aws.amazon.com/ec2/pricing/on-demand/).
- fck-nat (a NAT-instance AMI):
  - A t4g.nano costs $0.0042 an hour with no per-GB fee, against $0.045 an hour plus $0.045 a GB for the managed gateway. Idle, it costs about 10% of a NAT Gateway.
  - Its maintainers put the cost-effective ceiling at about 5 Gbps. — [fck-nat](https://github.com/AndrewGuenther/fck-nat)
  - A third-party example at 1 TB a month: $99.65 with a NAT instance against $170.64 with a NAT Gateway. — [dev.to](https://dev.to/enforza/how-to-reduce-aws-nat-gateway-costs-7-ways-ranked-by-savings-ll4)
- Corey Quinn (Duckbill, November 2021): the NAT processing fee has no volume discount, and a petabyte a month through it costs about $45,000. A free S3 gateway endpoint bypasses it, and self-managed NAT instances are cheaper. — [Last Week in AWS](https://www.lastweekinaws.com/blog/the-aws-managed-nat-gateway-is-unpleasant-and-not-recommended/)
- CloudFront:
  - Transfer from AWS origins to CloudFront is free — [Vantage handbook, 2021](https://handbook.vantage.sh/aws/services/cloudfront-pricing/); [CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/).
  - Paid flat-rate plans include 50 TB a month with no overage — [CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/).
- 31.9% of all websites use IPv6 (W3Techs, October 2026). — [W3Techs](https://w3techs.com/technologies/details/ce-ipv6)
- AWS's own AgentCore Browser bills network transfer at standard EC2 rates. — [AgentCore pricing](https://aws.amazon.com/bedrock/agentcore/pricing/)
- Browserbase meters proxy usage on all bytes, headers and encryption overhead included, with a 1 MB minimum per proxied session, rounded up to the next MB. — [Browserbase proxies](https://docs.browserbase.com/features/proxies)

### Inferences
- **Trap size:**
  - A fleet downloading 100 TB a month through a NAT Gateway pays about $4,500 a month in processing (100,000 GB × $0.045) on bytes that are free through an internet gateway.
  - Giving 100 hosts a public IPv4 each costs about $365 a month (100 × $0.005 × 730).
  - A NAT Gateway in each of 3 AZs adds about $99 a month in hourly fees.
- **Recommended crawler and browser networking:**
  - Public subnets on an internet gateway, dual-stack.
  - Each host gets a public IPv4 (secondary IPs only where per-IP politeness needs them, $3.65 a month each), and IPv6 through the internet gateway at no processing fee.
  - Security groups deny all inbound traffic. Browser hosts are reached only through the NLB and session gateway of section 2.
  - IMDS (the instance metadata service at 169.254.169.254) is blocked from runs, as defense in depth under ADR 0017.
  - IPv6 reaches only about a third of sites, so IPv4 remains required.
- **If policy forces private subnets:** fck-nat per AZ (several instances, given the roughly 5 Gbps ceiling, behind an Auto Scaling group for availability), plus IPv6 through an egress-only gateway for targets that support it.
- **Control-plane egress:** webhook deliveries to tenants, sink writes and model API calls. Customers often allowlist webhook source IPs, so this is where a Regional NAT Gateway with fixed Elastic IPs (or BYOIP) earns its fee: low volume, stable addresses.
- **Gateway endpoints for S3 and DynamoDB in every VPC.** Otherwise checkpoint uploads and dataset writes from private subnets pay NAT processing of about $45 per TB.
- **AZ affinity:** keep a run, its disk, any S3 Express bucket, any NAT path and its session gateway in one AZ, because cross-AZ bytes are billed.
- **Customer-facing egress:**
  - Dataset and recording downloads should leave through CloudFront (origin fetch free; flat-rate plans cap the bill) using signed URLs.
  - Hosted-browser live view, JSON-RPC responses and screenshots are internet egress at about $0.09/GB, so meter them per tenant and price browser time or bandwidth to match.
- **Residential proxies dominate the bill when used.** Vendors bill every byte at per-GB prices, far above AWS rates, and free AWS ingress does not help, so block media and ads on proxied sessions.

### Gaps
- The cross-AZ rate per GB was not verified this session; $0.01/GB in each direction is commonly cited.
- NLB and ALB per-GB processing charges were not captured.
- Whether BYOIP addresses can back a (Regional) NAT Gateway was not verified.
- CloudFront's pay-as-you-go per-GB rates, and whether the 1 TB-a-month always-free tier still exists, were not captured.
- AWS's EU Data Act page returned 404, so the reduced-rate terms were not verified.

## 4. IP reputation (how anti-bot vendors treat AWS ranges; rotating Elastic IPs; residential and ISP proxies; BYOIP)

### Takeaway
AWS publishes its address space, so any site can recognize AWS traffic, and anti-bot vendors fingerprint far beyond IPs. Cloudflare kept tracking a crawler across rotated IPs and ASNs. Rotating Elastic IPs therefore buys little. For protected sites the industry routes through residential or ISP proxies, whose vendors' KYC and category restrictions pass down to their customers. For agents, the sanctioned path is a cryptographic identity: Web Bot Auth, and Cloudflare's verified "Intermediary" agents. AWS's own AgentCore Browser already signs this way. BYOIP plus BYOASN would give Goodfetch its own prefix and origin network number, but not residential reputation.

### Cited Findings
- AWS publishes its IP ranges in `ip-ranges.json` so anyone can identify AWS traffic, plus an RFC 8805 geolocation feed. BYOIP ranges are not in the file. — [AWS IP address ranges](https://docs.aws.amazon.com/vpc/latest/userguide/aws-ip-ranges.html)
- Cloudflare's detection engines:
  - Heuristics against known bad fingerprints on every plan.
  - JavaScript detections for headless browsers.
  - A supervised machine-learning model scoring 1 to 99 from request features (headers, session, browser signals) seen across its network, on Business and Enterprise.
  - Anomaly detection, now deprecated.
  - The page lists no IP-reputation, ASN or datacenter signal. — [Cloudflare bot detection engines](https://developers.cloudflare.com/bots/concepts/bot-detection-engines/)
- Secondary and unverified: Cloudflare keeps reputation per ASN (autonomous system number, a network's routing identity) and pre-scores datacenter ranges low; its free Bot Fight Mode crudely blocks known datacenter traffic. — [Scrappey](https://scrappey.com/qa/web-automation/how-cloudflare-detects-bots)
- Cloudflare, August 4, 2025: Perplexity's undeclared crawler used a generic Chrome user agent and rotated IPs outside its published ranges and across ASNs. Cloudflare still fingerprinted it with machine-learning and network signals, delisted it as a verified bot, and added heuristics and signatures, including to the AI-crawler block rule available on every plan. — [Cloudflare blog](https://blog.cloudflare.com/perplexity-is-using-stealth-undeclared-crawlers-to-evade-website-no-crawl-directives/)
- Since July 1, 2025 Cloudflare asks every new domain whether to allow AI crawlers, with blocking as the default; crawlers need permission. Cloudflare handles about 20% of the web. — [Cloudflare press release](https://www.cloudflare.com/press/press-releases/2025/cloudflare-just-changed-how-ai-crawlers-scrape-the-internet-at-large/)
- Cloudflare verified bots and signed agents, as of July 1, 2026:
  - Signed agents now sit inside Verified bots, labelled Direct (a single operator) or Intermediary: an agentic service that many end users operate, so the operator and the end user differ. Cloudflare is testing the `Forwarded` header (RFC 7239) to pass end-user information.
  - Listing requires a BotBase submission and validation by Web Bot Auth or by IP. The bot must identify itself honestly, obey robots.txt and crawl delays, keep rates reasonable and keep a consistent purpose. A listing can be removed.
  - [Cloudflare signed agents](https://developers.cloudflare.com/bots/concepts/bot/signed-agents/)
- How Web Bot Auth works:
  - The operator publishes Ed25519 keys as a signed JWK set at `/.well-known/http-message-signatures-directory`.
  - Each request carries `Signature-Input`, `Signature` and `Signature-Agent` headers.
  - Cloudflare does not check nonces, so short expiry windows are the replay defense.
  - [Cloudflare Web Bot Auth](https://developers.cloudflare.com/bots/reference/bot-verification/web-bot-auth/)
- AgentCore Browser:
  - Supports Web Bot Auth in preview since October 2025, signing every HTTP request, in 9 Regions including us-east-1 and eu-west-1. AWS warns that not every site recognizes signed traffic. — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/10/amazon-bedrock-agentcore-browser-web-bot-auth-preview); [AgentCore docs](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-web-bot-auth.html)
  - Supports external proxies matched by domain pattern, with credentials in Secrets Manager. — [AgentCore browser proxies](https://docs.aws.amazon.com/de_de/bedrock-agentcore/latest/devguide/browser-proxies.html)
- Secondary: DataDome treats datacenter IPs as one signal among fingerprint and behavior signals in models trained per site. — [Scrappey](https://scrappey.com/qa/web-automation/how-datadome-detects-bots)
- Browserbase:
  - Built-in proxies are residential, from third-party providers, and off by default; routing is best-effort to US exits; 200+ countries.
  - Custom proxies and routing by domain are supported.
  - Some categories are less reliable because of the providers' own use policies: banking and crypto, government, streaming and gaming, ticketing, webmail, gambling.
  - It recommends pairing proxies with its Verified identity. — [Browserbase proxies](https://docs.browserbase.com/features/proxies)
  - The built-in pool assigns and rotates IPs for you. — [Browserbase blog](https://browserbase.com/blog/how-to-rotate-proxies-automatically-in-a-browser)
- Bright Data residential proxies:
  - Only for verified companies that pass a human-reviewed KYC (know-your-customer) check, for zones created after July 7, 2026.
  - A corporate email is required.
  - Requests outside the approved use case are blocked, and government sites are blocked across all networks.
  - [Bright Data network access](https://docs.brightdata.com/products/residential/network-access)
  - Another Bright Data page describes immediate access without KYC; the two conflict. — [Bright Data (alternate page)](https://docs.brightdata.com/proxy-networks/residential/network-access.md)
- BYOIP (bring your own IP addresses) on AWS:
  - Smallest prefixes are a /24 for IPv4 and a /48 for advertised IPv6.
  - The range must be registered with ARIN, RIPE or APNIC to a business, not a person.
  - AWS may reject a range with a poor reputation.
  - 5 ranges per Region per account, raised through Support.
  - BYOIP addresses carry no public IPv4 fee.
  - [EC2 BYOIP](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-byoip.html); [AWS blog](https://aws.amazon.com/blogs/aws/new-aws-public-ipv4-address-charge-public-ip-insights/)
- BYOASN (November 2023): advertise a BYOIP range from AWS under your own public ASN. Switching from Amazon's ASN to your own is limited to once an hour; 5 ASNs per IPAM by default. — [What's New](https://aws.amazon.com/about-aws/whats-new/2023/11/amazon-vpc-ip-address-manager-bring-your-own-asn-aws); [BYOASN tutorial](https://docs.aws.amazon.com/vpc/latest/ipam/tutorials-byoasn.html)

### Inferences
- **Rotating Elastic IPs is not a strategy.**
  - Every AWS address is published as AWS, so rotation changes neither the network nor the hosting classification.
  - Cloudflare's Perplexity case shows that rotation even across ASNs gets fingerprinted.
  - Each extra address costs $3.65 a month, and churn may itself look abusive.
- **BYOIP plus BYOASN** gives Goodfetch:
  - a prefix and BGP origin of its own, and its own WHOIS record and geofeed;
  - no listing in `ip-ranges.json` and no IPv4 fee;
  - isolation from other AWS tenants' reputation;
  - something site owners can allowlist, and an address base for Cloudflare's IP validation.

  It almost certainly still reads as "hosting" to IP-intelligence vendors (unverified). Its value is accountability and isolation, not stealth.
- **A three-tier egress design** matches the market (Browserbase: residential plus Verified; AgentCore: Web Bot Auth plus your own proxy):
  1. **Direct** from AWS or BYOIP addresses, for unprotected targets and polite crawling.
  2. **Signed identity:** Web Bot Auth, registered with Cloudflare as an Intermediary signed agent, carrying the end user through `Forwarded`, for agent browsing where sites accept verified agents. AWS's AgentCore sets this bar.
  3. **Residential or ISP proxies,** from partners or the customer's own, for protected targets. Partners' KYC and category blocks must pass into Goodfetch's own terms.
- **Stealth Chromium on a datacenter IP is a mismatch.** A consumer-browser fingerprint from a cloud ASN is itself a signal, so stealth on AWS addresses will mostly help on weakly protected sites. The Proving ground (ADR 0025) should measure it per target class.
- **Keep an egress attribution log** (tenant, run, egress IP or proxy session, target domain, time). It is the only way to answer an AWS abuse notice within 24 hours (section 5) and to keep a verified-bot listing.

### Gaps
- No official statement from Akamai, DataDome or HUMAN on how they score cloud-provider ASNs was retrieved (search budget exhausted). Cloudflare's official docs name no ASN or IP-reputation signal, and the secondary claims are unverified.
- Amazon's ASN numbers, the default Elastic IP quota, and any AWS policy on rapid address churn were not verified.
- How IP-intelligence databases (MaxMind, IPinfo, Spur) classify BYOIP or BYOASN prefixes announced from AWS was not researched.
- Residential and ISP proxy prices per GB were not researched.
- Which cloud competitors (Apify, Browserbase, Firecrawl) actually run on AWS was not verified.

## 5. AWS Acceptable Use Policy and Service Terms on crawling, scraping and hosting browsers; abuse reports; what competitors do

### Takeaway
The current AWS Acceptable Use Policy (last updated July 1, 2021) never names crawling or scraping. It prohibits illegal use, violating others' rights, and violating the security, integrity or availability of any system, and it covers facilitating others, so Goodfetch Cloud answers to AWS for its tenants' traffic. AWS weighs a customer's own prevention processes when it enforces, and an abuse notice left unanswered for 24 hours can get resources blocked or the account suspended. An abuse desk, KYC tiers, attribution logs and account-level blast-radius isolation are therefore required.

### Cited Findings
- The AWS Acceptable Use Policy, fetched 2026-10-12 (last updated July 1, 2021):
  - Prohibits using the services, or facilitating or allowing others to use them, for illegal or fraudulent activity, violating others' rights, threats and violence, child sexual abuse material, violating the security, integrity or availability of any user, network, computer, communications system, software or device, and spam.
  - Contains no section on network abuse and does not mention crawling, scraping or proxies.
  - [AWS Acceptable Use Policy](https://aws.amazon.com/aup/)
- The policy's enforcement clause:
  - AWS may investigate and may remove or disable content or resources, and customers must cooperate.
  - In judging a violation AWS may weigh your ability and willingness to comply, including your policies and processes for preventing, identifying and removing prohibited activity.
  - Anyone can report violations.
  - [AWS Acceptable Use Policy](https://aws.amazon.com/aup/)
- Archived older versions had a "No Network Abuse" section: no connections to systems without permission, and no monitoring or crawling that impairs or disrupts a system. Third-party trackers hold these versions, with unclear dates; one labels a provision "Network and Security Abuse", likely the tracker's own label. — [ToS Tracker](https://tostracker.app/document/aws/version/855035); [ConductAtlas](https://conductatlas.com/platform/amazon/aws-acceptable-use-policy/network-and-security-abuse-prohibition/)
- AWS Service Terms (last updated October 1, 2026; about 94% read):
  - No clause on crawling third-party sites or operating proxies.
  - Adjacent clauses:
    - Amazon Location Service forbids scraping its location data (82.4).
    - Device Farm forbids generating internet traffic from its EC2 instances without approval (35.2).
    - WorkSpaces may not serve web traffic (26.3).
    - Bedrock may keep inputs and outputs of some models up to 30 days for abuse detection (50.12.2).
    - AgentCore Payments leaves transaction responsibility with the customer (50.15).
    - Cryptocurrency mining is restricted (1.25).
    - Promotional credits fall under separate terms (1.16), and AWS Marketplace sellers under separate seller terms (20.2).
  - [AWS Service Terms](https://aws.amazon.com/service-terms/)
- AWS abuse notices:
  - AWS Trust & Safety emails the account's primary contact and its security alternate contact.
  - With no response within 24 hours, AWS might block resources or suspend the account.
  - Reply in the same email thread without changing its subject; AWS Support cannot help with these notices.
  - For blocked resources with no email received, write to trustandsafety@support.aws.com.
  - [re:Post article](https://www.repost.aws/articles/ARZVtmQf5CS_-UPgIJSCyr3A/securing-compromised-ec2-instances-implicated-in-abuse-reports); [re:Post Knowledge Center](https://repost.aws/knowledge-center/aws-abuse-report); [re:Post question](https://repost.aws/questions/QUFKwmf19zRASvusw1hjMuqQ/aws-abuse-report-ec2-access-revoked)
- Apify's acceptable use policy (February 20, 2026):
  - Bans denial of service and undue load on servers, fraud and phishing, fake accounts, and engagement manipulation.
  - Apify may block, delete or restrict users or Actors without notice and will help law enforcement.
  - Misuse is reported by email. The policy contains no KYC requirement.
  - [Apify acceptable use policy](https://docs.apify.com/legal/acceptable-use-policy)
  - Apify updated its Store publishing terms and acceptable use policy after a wave of fraudulent reviews. — [Apify Discord](https://discord.apify.com/m/1474330608904699957)
- Competitors' gates:
  - Bright Data requires KYC for residential proxies (section 4) — [Bright Data](https://docs.brightdata.com/products/residential/network-access).
  - Browserbase's proxy providers restrict categories (section 4) — [Browserbase proxies](https://docs.browserbase.com/features/proxies).
  - Cloudflare's verified-bot rules require obeying robots.txt and keeping rates reasonable (section 4) — [Cloudflare](https://developers.cloudflare.com/bots/concepts/bot/signed-agents/).
- The framework does not respect robots.txt by default, to stay competitive with Crawlee and Spider. The engine always reads robots.txt, Content Signals, aipref and TDMRep and records them with the run's declared Purpose, but enforcement (`respect_site_preferences`) is opt-in. — [ADR 0016](../../../../docs/adr/0016-security-safe-by-default-compliance-switchable.md); [V1 scope matrix SC-01](../../../../docs/adr/0037-v1-scope-matrix.md)
- Goodfetch Cloud switches on a "no private endpoints" policy, after Firecrawl's SSRF CVEs (CVE-2024-56800, CVE-2025-57818, CVE-2026-32857). — [ADR 0017](../../../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md)

### Inferences
- **AWS does not forbid scraping as such.** What triggers enforcement is site owners' complaints (load, attacks, unauthorized access) traced to Goodfetch's IPs. Goodfetch Cloud is the AWS customer of record for every tenant's traffic, so its own terms must be at least as strict, and enforceable.
- **Required controls:**
  1. An abuse desk answering within 24 hours, with a security contact set on every AWS account.
  2. The tenant, run, egress IP, domain and time attribution log, retained under a GDPR-compatible period.
  3. A per-tenant kill switch, plus the cross-tenant limit per target domain from section 1.
  4. KYC tiers: card and email for basic use, business verification for high concurrency, proxy tiers and logged-in sessions (as Bright Data does).
  5. The category blocks inherited from proxy partners.
  6. A public abuse-report form.
  7. Review of Recipes and listings in the marketplace (Apify's fraud update shows the need).
- **Tension to decide:** ADR 0016 leaves robots.txt enforcement off by default, while Cloudflare's signed-agent listing requires obeying it, and AWS weighs prevention processes. The cloud can set Operator policy (as it does for "no private endpoints"), for example requiring `respect_site_preferences` on signed or verified traffic and on the crawl API. That is a product decision for the owner, not settled here.
- **Blast radius:** put egress-heavy runner and browser fleets in their own AWS accounts per Region, separate from the control plane, data stores and billing. A Trust & Safety block on runner resources then cannot take down the API, datasets or invoicing. A separate account for the proxy and stealth tier could isolate it further.
- **AWS sells hosted browsers for agents itself (AgentCore Browser),** which signals that AWS accepts the business model when the traffic is identifiable.

### Gaps
- AWS's own text of the pre-2021 policy and the date it changed were not verified (only third-party trackers).
- The last roughly 20,500 characters of the Service Terms were not read, so no AgentCore-specific terms were found.
- The use policies, KYC and abuse processes of Browserbase, Firecrawl, Steel and Hyperbrowser were not retrieved.
- No public source describes how AWS Trust & Safety escalates repeated scraping complaints.
- Legal exposure (CFAA, EU law) is out of scope.

## 6. Metering and billing pipeline from engine usage messages to invoices (AWS-native, SaaS, open-source; real-time spend caps and prepaid credits)

### Takeaway
No billing platform enforces caps in real time: Stripe processes meter events asynchronously and applies credits only when an invoice is finalized. Goodfetch's own per-tenant ledger (ADR 0041) must therefore hold prepaid balances and spend caps, while a billing platform rates and invoices aggregated usage. Stripe, which now owns Metronome, is the default. Lago (AGPLv3) and OpenMeter (Apache-2.0, now owned by Kong) are the self-hostable options. AWS Marketplace buyers need hourly BatchMeterUsage calls fed from the same aggregates.

### Cited Findings
- The engine reserves each model call's or paid solve's worst case before sending it. Usage known only afterwards (browser time, proxy bytes) is charged against the run's slice afterwards. Check-then-charge overshoots under concurrency, as LiteLLM's bugs show. — [ADR 0041](../../../../docs/adr/0041-limits-across-runs-through-a-per-user-usage-ledger.md); [ADR 0012](../../../../docs/adr/0012-engine-owned-model-calls-through-gateways-and-local-servers.md)
- Stripe meter events:
  - Processing is asynchronous, so meter summaries and upcoming invoices may not reflect the latest events.
  - Idempotency comes from an event identifier. Timestamps must fall within the past 35 days and no more than 5 minutes ahead; an accepted event may still miss an invoice already finalized.
  - The v1 endpoint takes 1,000 calls per second per account, with one concurrent call per customer and meter. The v2 meter event streams take 10,000 events per second (200,000 on request), with 15-minute session tokens.
  - Dimension limits: 10,000 combinations per meter per hour and 100 per customer.
  - Errors arrive as asynchronous events.
  - [Stripe: recording usage](https://docs.stripe.com/billing/subscriptions/usage-based/recording-usage-api)
- Stripe billing credits:
  - Credit grants, prepaid or promotional, apply only to metered subscription items, at invoice finalization, after discounts and before taxes.
  - A customer may hold at most 100 unused grants.
  - Grants may not be used as stored value, gift cards or for third-party purchases.
  - [Stripe billing credits](https://docs.stripe.com/billing/subscriptions/usage-based/billing-credits)
- Stripe and Metronome:
  - Patrick Collison announced the acquisition on December 2, 2025; it was reported at about $1 billion. — [PYMNTS](https://www.pymnts.com/acquisitions/2025/stripe-acquires-metronome-to-enhance-metered-pricing-capabilities-for-ai-companies)
  - Stripe has completed it; a secondary source dates the close to January 14, 2026. — [Stripe newsroom](https://stripe.com/newsroom/news/stripe-completes-metronome-acquisition)
  - OpenAI, Anthropic, Databricks and NVIDIA use Metronome, and the brand continues. — [PYMNTS](https://www.pymnts.com/acquisitions/2025/stripe-acquires-metronome-to-enhance-metered-pricing-capabilities-for-ai-companies)
- Kong acquired OpenMeter on September 3, 2025. OpenMeter stays open source under Apache-2.0, with integration into Kong Konnect planned for early 2026 and customer migration by mid-2026. — [Kong press release](https://konghq.com/company/press-room/press-release/kong-acquires-openmeter-to-unlock-ai-and-api-monetization-for-the-agentic-era)
- Lago: AGPLv3, with real-time event ingestion, deduplication by transaction ID, wallets and credit top-ups, and usage alerts. Some capabilities, and Lago Cloud, are premium. — [Lago](https://github.com/getlago/lago)
- AWS Marketplace metering for SaaS:
  - `BatchMeterUsage` is mandatory, with hourly reporting recommended in batches of up to 25 records and a 1 MB payload limit.
  - Records are deduplicated per customer, hour and dimension, and the first quantity wins.
  - Records may be up to 24 hours late, with a 6-hour grace period after month end.
  - After an unsubscribe notice the seller has 1 hour to send the remaining usage.
  - Vendor-metered tags (5 tags, 2,500 allocations) are available; buyers' reports are not real time.
  - [Marketplace SaaS metering](https://docs.aws.amazon.com/marketplace/latest/userguide/metering-for-usage.html)
- Secondary (from a competitor): Stripe cannot gate a request in real time, so allowance counters belong in your own system. — [Flexprice](https://flexprice.io/questions/how-do-i-add-metering-api-can-bill-customers-based-many-calls-make)

### Inferences
- **Recommended pipeline:**
  1. The engine emits usage messages per run to the cloud gateway, its protocol client.
  2. **Enforcement:** the gateway applies ADR 0041 slice transactions and decrements prepaid balances in Aurora. This is the only real-time cap.
  3. **Audit trail:** it appends an immutable usage record (tenant, run, run epoch, sequence number, kind, quantity, model, timestamps) through Firehose to S3 Iceberg tables, for disputes, analytics and reconciliation.
  4. **Invoicing:** an hourly aggregator sends usage to the billing platform, or through `BatchMeterUsage` for tenants who subscribed via AWS Marketplace, never both for one tenant.
  - The idempotency key is run ID plus run epoch plus sequence number, so a run resumed under ADR 0003 never counts usage twice.
- **Who rates** (turns quantities into money with a price book)? Pricing here is multi-dimensional: per-model token prices, browser-seconds, proxy GB by kind, solver attempts, commitments and enterprise contracts. Metronome, now Stripe, is the established platform for that, used by large AI vendors. Stripe Billing meters alone strain at 100 dimension combinations per customer and credits applied only at finalization. Either way, the live credit balance and burn-down shown to tenants come from Goodfetch's ledger.
- **Self-hosting:** Lago is mature, but AGPL means any modifications must be published if users interact with them over the network. OpenMeter is permissively licensed metering and entitlements, but its roadmap now belongs to Kong.
- **Prepaid credits:** the ledger holds the money balance; top-ups are Stripe payments, and grants are mirrored to Stripe only for how the invoice presents them.
- **Spend caps:** use ADR 0041's scopes, notifications at 50%, 80% and 100%, and the "uncapped" mark, implemented in the cloud ledger. Raising a Limit stays a person's action in the Console.
- **AWS Marketplace:** map each tenant to its Marketplace customer (`ResolveCustomer`). The 1-hour unsubscribe window makes hourly flushing mandatory.

### Gaps
- The features and prices of Orb and Amberflo, and Metronome's packaging after the acquisition, were not retrieved.
- Stripe Billing's own fee (a percentage of billed volume) was not captured.
- EU VAT and merchant-of-record options (Stripe Tax, Paddle, Polar) were not researched.
- OpenMeter's open-source activity during 2026 was not verified.

## 7. Observability cost traps and alternatives (CloudWatch Logs, custom metrics, X-Ray; Grafana Cloud, self-hosted Loki or ClickHouse, Better Stack, Sentry)

### Takeaway
CloudWatch charges $0.30 a month for every unique metric-and-dimension combination and $0.50/GB for custom logs, so per-tenant or per-run metrics and verbose browser or CDP logs explode costs. Keep CloudWatch for AWS-vended metrics and alarms. Send OpenTelemetry from the engine's `tracing` output to a columnar backend: self-hosted ClickStack on ClickHouse, or a SaaS such as Better Stack, which is cheaper per GB for logs, or Grafana Cloud. Use Sentry for errors. Store tenant-facing run logs in S3 as product data.

### Cited Findings
- CloudWatch prices (us-east-1):
  - Custom metrics: $0.30 per metric-month for the first 10,000.
  - Custom application logs: $0.50/GB (Standard) or $0.25/GB (Infrequent Access).
  - Vended logs (logs AWS services emit): tiered $0.50, $0.25, $0.10, then $0.05/GB above 50 TB a month.
  - Log archive: $0.03/GB-month.
  - X-Ray: $5 per million traces recorded, $0.50 per million retrieved.
  - Application Signals: $1.50 per million signals for the first 100 million.
  - Transaction Search: $0.35/GB ingested plus $0.75 per million indexed spans.
  - Free tier: 5 GB of logs, 10 metrics and 100,000 traces.
  - [CloudWatch pricing](https://aws.amazon.com/cloudwatch/pricing/)
- Lambda logs moved to tiered vended-log pricing on May 1, 2025. — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/05/amazon-cloudwatch-tiered-pricing-additional-destinations-aws-lambda-logs)
- Grafana Cloud:
  - Free: 10,000 metric series and 50 GB each of logs, traces and profiles, with 14 days' retention.
  - Pro: $19 a month plus $6.50 per 1,000 series, and logs at about $0.05/GB processing, $0.40/GB writing and $0.10/GB retention.
  - Enterprise: a $25,000 yearly minimum.
  - [Grafana pricing](https://grafana.com/pricing/)
- Better Stack: log ingestion $0.10/GB in the EU and $0.15/GB in the US; retention $0.05 and $0.08 per GB-month. — [Better Stack pricing](https://betterstack.com/pricing)
- Sentry: Team plan $26 a month with 50,000 errors, 5 million spans and 5 GB of logs; extra logs or metrics at $0.50/GB. — [Sentry pricing](https://sentry.io/pricing/)
- ClickStack:
  - Combines ClickHouse, the HyperDX interface and an OpenTelemetry collector; described as fully open source.
  - Can be self-hosted (with MongoDB for interface state) or run managed on ClickHouse Cloud.
  - Built for high-cardinality data.
  - [ClickStack overview](https://clickhouse.com/docs/use-cases/observability/clickstack/overview)
- Only the protocol transport, the CLI's output writer and the `tracing` subscriber's writer touch stdout or stderr; everything else logs through `tracing`. — [AGENTS.md](../../../../AGENTS.md)
- Nothing leaves the machine unless configured (local telemetry is off by default). — [ADR 0043](../../../../docs/adr/0043-nothing-leaves-the-machine-unless-configured.md)

### Inferences
- **Cardinality trap:** tenant or run IDs as CloudWatch dimensions create a custom metric per combination. 10,000 tenants × 5 usage kinds is 50,000 metrics, up to about $15,000 a month at the first-tier rate (higher tiers, not captured, lower this). Never put tenant or run IDs in CloudWatch dimensions.
- **Log-volume trap:**
  - 10 TB a month of engine, browser-host and CDP logs costs about $5,000 a month in CloudWatch ingestion alone.
  - Better Stack costs about $1,000 to $1,500 for the same, plus retention.
  - Grafana Cloud's logs, at about $0.55/GB all-in, are not cheaper than CloudWatch per GB; its edge is metrics and the interface.
  - Self-hosted ClickHouse on S3-backed storage should be cheapest at scale (an inference, not quantified).
- **Recommended stack:**
  - `tracing` exports through `tracing-opentelemetry` over OTLP to an OpenTelemetry collector.
  - The collector writes to ClickStack in the shared-services account, or to Better Stack or Grafana Cloud early on.
  - Traces are sampled, and metrics are kept to bounded cardinality.
  - Sentry covers control-plane and SDK errors.
  - CloudWatch stays for AWS service metrics, alarms and the vended logs that AWS sends there anyway.
- **Tenant-facing run logs are product data** (Apify, for one, shows its users their run logs): compressed objects per run in S3 with an index, served by the API.

### Gaps
- The CloudWatch Logs Insights price per GB scanned and the custom-metric tiers above 10,000 were not captured.
- Self-hosted ClickHouse total cost was not quantified, and the ClickStack page names no license.
- Embedded Metric Format and high-resolution metric pricing were not verified.

## 8. Commitments and programs (Savings Plans, Reserved Instances, Spot; AWS Activate; AWS Marketplace SaaS; EU regions and the AWS European Sovereign Cloud)

### Takeaway
- **Compute pricing:** use Spot for checkpointable batch runs, since its two-minute notice maps onto ADR 0002's migrate signal, and on-demand for live hosted-browser sessions until a baseline exists. Then commit through Compute Savings Plans (up to 72%) and the new Database Savings Plans (up to 35% on Aurora Serverless v2 and DSQL; 1-year, no upfront), not Reserved Instances.
- **Credits:** AWS Activate now goes up to $200,000 (Portfolio), with more for AI startups by invitation, but credits cannot pay Marketplace purchases or Savings Plan and Reserved Instance upfront fees.
- **Marketplace:** AWS takes 3% on SaaS listings (2% or 1.5% on large private offers). Its new AI Agents and Tools category is where AWS's own AgentCore Browser competes.
- **EU:** Frankfurt and Ireland have every service this design needs. The AWS European Sovereign Cloud (generally available January 2026, in Brandenburg) is a separate partition: a second deployment target, for customers who require sovereignty.

### Cited Findings
- Savings Plans cut up to 72% from on-demand prices, cover Lambda and Fargate as well as EC2, and apply across an organization. — [Savings Plans](https://aws.amazon.com/savingsplans/)
- Database Savings Plans (December 2025):
  - A 1-year, no-upfront commitment in dollars per hour.
  - Discounts: up to 35% on serverless (including Aurora Serverless v2 and DSQL), up to 20% on provisioned instances, and on DynamoDB up to 18% on on-demand and 12% on provisioned capacity.
  - They cannot combine with RDS Reserved Instances or DynamoDB reserved capacity for the same usage, and mostly cover 7th-generation and newer instances.
  - [AWS News Blog](https://aws.amazon.com/blogs/aws/introducing-database-savings-plans-for-aws-databases/); [Database Savings Plans pricing](https://aws.amazon.com/savingsplans/database-pricing); [InfoQ](https://www.infoq.com/news/2025/12/aws-database-savings-plans)
- Spot interruption notice:
  - A warning two minutes ahead, delivered as an EventBridge event and as `instance-action` in instance metadata, best-effort; AWS advises checking every 5 seconds.
  - With hibernation as the interruption behavior there is no two-minute warning.
  - [Spot interruption notices](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/spot-instance-termination-notices.html)
- AWS Activate (official page):
  - Founders: up to $5,000 (starting at $1,000), for self-funded startups.
  - Portfolio: up to $200,000, with an Organization ID from an Activate Provider (accelerator, angel or VC).
  - Eligibility: pre-Series B, founded within 10 years, on a paid-tier account.
  - AI startups: $200,000 and more by invitation, through an account manager.
  - Credits can pay for Bedrock, Kiro and migration.
  - [AWS Activate credits](https://aws.amazon.com/startups/credits); [AWS guide, June 2026](https://aws.amazon.com/aws-startups/learn/everything-you-need-to-know-about-aws-activate-credits/)
  - A third-party guide claims a $300,000 AI tier gated by VC nomination, and puts Portfolio at $100,000, contradicting AWS's page. — [guptadeepak](https://guptadeepak.com/startup-offers/guides/aws-activate)
- Promotional credits cannot pay for:
  - AWS Marketplace purchases;
  - Savings Plan and Reserved Instance upfront fees;
  - Route 53 domain registration;
  - cryptocurrency mining;
  - some support plans, Professional Services, training, or taxes.

  They are non-transferable except within an AWS Organization, and they expire. — [AWS Promotional Credit terms](https://aws.amazon.com/awscredits/)
- AWS Marketplace listing fees (effective January 5, 2024):
  - SaaS public offers: 3%.
  - Private offers: 3% under $1 million, 2% from $1 million to under $10 million, 1.5% from $10 million; renewals 1.5%.
  - Channel-partner private offers add 0.5%.
  - Fees are charged on pre-tax total contract value, and only when a transaction happens.
  - [Listing fees](https://docs.aws.amazon.com/marketplace/latest/userguide/listing-fees.html); [What's New](https://aws.amazon.com/about-aws/whats-new/2024/01/aws-marketplace-simplified-reduced-listing-fees/)
  - A seller-tools vendor's August 2026 post claims other professional-services and South Korea rates (unverified). — [Clazar](https://clazar.io/blog/aws-marketplace-fees)
- AWS Marketplace's AI Agents and Tools category:
  - Lists agents, agent tools (MCP servers and web search tools among them) and development solutions.
  - Sold as SaaS, container, AMI or API products, pay-as-you-go or by contract.
  - Some listings run on AgentCore Runtime or connect through AgentCore Gateway.
  - [AI Agents and Tools](https://aws.amazon.com/marketplace/solutions/ai-agents-and-tools)
- AgentCore Browser pricing:
  - About $0.0895 per vCPU-hour, charged on active CPU only (waiting on a model costs nothing when nothing else runs).
  - About $0.00945 per GB-hour on peak memory, billed per second with a 128 MB minimum.
  - Network at EC2 rates.
  - [AgentCore pricing](https://aws.amazon.com/bedrock/agentcore/pricing/)
- What the EU Regions have:
  - S3 Express One Zone is in eu-central-1, eu-west-1, eu-west-3 (one AZ) and eu-north-1 — [S3 Express Regions](https://docs.aws.amazon.com/AmazonS3/latest/userguide/s3-express-Endpoints.html).
  - DSQL is in six EU Regions, with an EU-only multi-Region set — [DSQL](https://docs.aws.amazon.com/aurora-dsql/latest/userguide/what-is-aurora-dsql.html).
  - AgentCore Web Bot Auth is in preview in eu-west-1 — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/10/amazon-bedrock-agentcore-browser-web-bot-auth-preview).
  - EventBridge Scheduler's high default quotas cover eu-central-1, eu-west-1 and eu-west-2 — [Scheduler quotas](https://docs.aws.amazon.com/scheduler/latest/UserGuide/scheduler-quotas.html).
- AWS European Sovereign Cloud:
  - Generally available on January 15, 2026 per AWS's press release (one secondary source says January 14).
  - Its first Region is in Brandenburg, Germany, with more than 90 services at launch.
  - It is run only by EU residents, through a new parent company and three German subsidiaries led by EU citizens, with an EU advisory board.
  - It is physically and logically separate from other Regions, and keeps customer metadata in the EU, including IAM, billing and usage metering.
  - Sovereign Local Zones are planned in Belgium, the Netherlands and Portugal.
  - [AWS press release](https://press.aboutamazon.com/aws/2026/1/aws-launches-aws-european-sovereign-cloud-and-announces-expansion-across-europe); [Intelligent CIO](https://www.intelligentcio.com/eu/2026/01/15/aws-launches-european-sovereign-cloud-expands-footprint-across-eu/)
  - Region code `eusc-de-east-1` and an investment above €7.8 billion (secondary). — [Towards the Cloud](https://towardsthecloud.com/blog/aws-european-sovereign-cloud); [KuppingerCole](https://www.kuppingercole.com/blog/small/aws-eu-sovereign-cloud-announcement-january-2026)

### Inferences
- **Spot:** crawl and actor runs (batch) fit. On an interruption notice the cloud sends ADR 0002's `migrate` signal, so the run checkpoints and uploads within two minutes; a checkpoint of a few GB uploads in seconds to S3 in the same Region (an inference to measure).
- **On-demand:** live hosted-browser sessions belong on on-demand capacity, because a live browser cannot move between machines invisibly and an interruption would end a customer's session. That argues for a mixed fleet.
- **Commitments:** once 2 to 3 months show a steady baseline, buy a Compute Savings Plan for the control plane and the warm browser pool, and a Database Savings Plan for Aurora. Reserved Instances are superseded.
- **Activate:** apply for Founders now, and for Portfolio through an accelerator or VC. Since credits do not cover upfront Savings Plan fees, buy plans with no upfront.
- **Marketplace channel:**
  - List the SaaS (3% fee) with usage dimensions mirroring Goodfetch's units; private offers serve enterprises that want to spend their committed AWS budgets (a common motivation, not verified here). The AI Agents and Tools category fits hosted browsers and MCP tools.
  - AgentCore Browser's rates set a price anchor: one fully busy vCPU with 2 GB costs about $0.11 an hour, and much less for idle-heavy agent sessions.
- **EU and US Regions:**
  - Primary EU Region: eu-central-1 (Frankfurt) or eu-west-1 (Ireland), both carrying S3 Express, DSQL and the high Scheduler quotas. Ireland also has Web Bot Auth for AgentCore and more S3 Express AZs. Choose by latency to customers and to the sites they target. Paris is weaker, with S3 Express in one AZ.
  - US: us-east-1, optionally us-west-2.
- **Sovereign cloud:** EU data residency is met in the standard EU Regions. The Sovereign Cloud adds EU-only operations and metadata, but as a separate partition it means separate accounts, IAM, billing and pipeline, and a narrower service set. Whether to support it is a scope decision for the owner, with that cost known up front.

### Gaps
- Whether Activate credits pay the hourly charges of no-upfront Savings Plans was not verified.
- Spot interruption rates for instance families suited to browsers were not researched (compute researcher).
- The Sovereign Cloud's service list (S3 Express, DSQL, SQS fair queues and the rest) was not verified.
- AWS Marketplace's EMEA seller and VAT handling, and "Buy with AWS", were not researched.
- AgentCore Browser's feature parity (live view, recordings, CAPTCHA handling) was not researched.

## 9. Security baseline for a multi-tenant cloud on AWS (accounts per environment, per-tenant IAM scoping, KMS, guardrails)

### Takeaway
- **Accounts:** use separate AWS accounts per environment and per blast radius (control plane, data, runner and egress fleets, security and logging), under AWS Organizations with service control policies (SCPs) and the newer resource control policies (RCPs).
- **Credentials:** runs get no AWS credentials, since the cloud is their protocol client. Tenant data access is scoped by short-lived STS sessions tagged with the tenant ID.
- **Keys:** one KMS key per Region, environment and purpose wraps ADR 0015's per-run data keys, with an encryption context binding tenant and run. Per-tenant keys are only for bring-your-own-key customers, at $1 per key per month.

### Cited Findings
- AWS's multi-account whitepaper (April 30, 2025):
  - An account is an isolation boundary for identity and access, and no access crosses accounts by default.
  - Production and non-production belong in separate accounts. Charges follow usage, not the number of accounts.
  - Keep a clean Security organizational unit as AWS Control Tower requires.
  - [Organizing your AWS environment](https://docs.aws.amazon.com/whitepapers/latest/organizing-your-aws-environment/organizing-your-aws-environment.html)
- Resource control policies (November 13, 2024):
  - Organization-wide ceilings on what resources allow, complementing SCPs (which limit principals).
  - Launched for S3, STS, KMS, SQS and Secrets Manager, at no charge.
  - Main use is a data perimeter: deny outside access even where a bucket policy grants it.
  - [AWS News Blog](https://aws.amazon.com/blogs/aws/introducing-resource-control-policies-rcps-a-new-authorization-policy/)
- KMS:
  - $1 per key per month, prorated; symmetric requests cost about $0.03 per 10,000, with 20,000 free requests a month.
  - Rotation adds $1 a month for each of the first two rotations.
  - [KMS pricing](https://aws.amazon.com/kms/pricing/)
- AWS's SaaS tenant isolation whitepaper (August 2020) is now marked historical. — [SaaS Tenant Isolation Strategies](https://docs.aws.amazon.com/whitepapers/latest/saas-tenant-isolation-strategies/saas-tenant-isolation-strategies.html)
- Engine-side security:
  - Secret providers include the caller over JSON-RPC (how Goodfetch Cloud injects a tenant's secrets) and AWS Secrets Manager; discarding a run destroys its data key, crypto-shredding its secrets — [ADR 0015](../../../../docs/adr/0015-secret-references-and-caller-held-run-keys.md).
  - A permanent reset gate plants canary secrets and scans everything reachable — [ADR 0001](../../../../docs/adr/0001-single-tenant-engine-cloud-as-protocol-client.md).
  - Chromium's sandbox stays on, images run as non-root with a seccomp profile, and Goodfetch Cloud forbids private endpoints — [ADR 0017](../../../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md).
  - The Browser host enforces the egress policy on hosted browsers — [ADR 0022](../../../../docs/adr/0022-remote-browsers-driven-through-a-browser-host.md).
  - A third-party Plugin at the `direct` level opens its own sockets, and on Linux nothing is checked locally beyond its configured destinations — [ADR 0017](../../../../docs/adr/0017-one-egress-policy-and-one-endpoint-rule.md).
- Instance metadata is served at 169.254.169.254 to code on the instance (IMDSv2 token flow). — [Spot interruption notices](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/spot-instance-termination-notices.html)
- Sharing a BYOIP range across accounts requires IPAM integrated with Organizations. — [EC2 BYOIP](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-byoip.html)

### Inferences
- **Account layout, per environment (dev, staging, prod):**
  - management account (billing only);
  - security tooling and log archive (organization CloudTrail, Config);
  - shared services (CI, ECR, IPAM, the observability stack);
  - control plane (API, gateway, Aurora, billing pipeline);
  - data (S3 for datasets, checkpoints and recordings, plus the KMS keys);
  - runner and egress accounts per Region (Browser hosts, engines, Recipe builds), optionally a separate one for the proxy and stealth tier;
  - a sandbox organizational unit.

  US and EU workloads go in Region-pinned accounts. This also contains a Trust & Safety block (section 5).
- **SCPs:**
  - deny leaving the organization;
  - deny disabling CloudTrail, GuardDuty or Config;
  - allow only the chosen Regions;
  - deny creating IAM users;
  - require IMDSv2.
- **RCPs:** deny access from outside the organization to S3, KMS, SQS and Secrets Manager, so a misconfigured bucket policy cannot expose tenant data.
- **Runs and user code hold no AWS credentials.**
  - The engine writes checkpoints and exports to a local path the cloud gives (ADR 0002's "location the caller gives"). A host agent uploads them under a session policy limited to that tenant's and run's prefix.
  - Tenant code (Recipes, and Plugins at the `direct` level) therefore cannot reach other tenants' data even if it escapes the engine's checks.
  - Below ADR 0017's engine checks, add network-level defenses: block instance metadata from containers (hop limit 1 plus host firewall rules), and deny private and link-local ranges in security groups, NACLs or a VPC firewall.
- **Control-plane data access:** STS AssumeRole with a `tenant_id` session tag, and attribute-based policies on S3 prefixes (`aws:PrincipalTag`): the standard pooled-SaaS pattern. This is an inference, since the whitepaper's details were not captured.
- **KMS:** one symmetric key per Region, environment and purpose: checkpoint KEK, dataset encryption, secrets.
  - The encryption context binds tenant and run, and key policies and grants restrict who can decrypt.
  - Data keys come from GenerateDataKey once per run start or resume, a negligible cost.
  - Per-tenant keys only for bring-your-own-key enterprise customers; for every tenant they would cost 10,000 × $1 = $10,000 a month.

#### Cross-cutting: recommended control plane and data plane (synthesis of sections 1 to 9)
- **Front door:**
  - REST, crawl API, Console and MCP: CloudFront with WAF and DDoS protection, then an ALB, then the services.
  - Live hosted-browser traffic: a Regional NLB on a TCP listener, then a Goodfetch session gateway (Rust; TLS, ADR 0017 tokens, session routing, byte metering), then Browser hosts.
  - Human identity through WorkOS or Clerk; Goodfetch-issued scoped API keys for programs.
- **Control plane:** Aurora PostgreSQL per geography (metadata, Run index, per-tenant ledger with transactional slices); SQS fair queues (dispatch, webhooks, sink retries); EventBridge Scheduler as the time trigger under ADR 0036's semantics; Valkey Serverless for limits per target domain across tenants; KMS for KEKs; Secrets Manager for tenant secrets injected over JSON-RPC.
- **Data plane:**
  - Runner and browser hosts sit in public subnets on an internet gateway, dual-stack, with no NAT on the bulk path, gateway endpoints for S3 and DynamoDB, and AZ-local placement.
  - Live SQLite on local NVMe or EBS; checkpoints in S3 Standard, written with conditional writes keyed by epoch, optionally staged in S3 Express in the same AZ.
  - Datasets in S3 as Parquet or JSONL chunks (optionally S3 Tables); recordings in Intelligent-Tiering; customer downloads through CloudFront.
- **Usage and billing:** gateway, then the Aurora ledger (real-time caps) and Firehose into S3 Iceberg (audit), then hourly aggregates to Stripe or Metronome, and to Marketplace `BatchMeterUsage` for Marketplace tenants.
- **Egress tiers:** direct (AWS or BYOIP addresses), signed (Web Bot Auth as a Cloudflare Intermediary), proxied (partner residential or ISP proxies, or the customer's own), with an attribution log on every byte.
- **Observability:** OpenTelemetry into ClickStack or a SaaS, Sentry for errors, CloudWatch only for AWS-vended signals and alarms.
- **Regions:** us-east-1 (and optionally us-west-2), plus eu-central-1 or eu-west-1. The Sovereign Cloud is a separate decision.

#### Cross-cutting: cost-trap checklist (with size)
- **NAT processing** of $0.045/GB in both directions is about $45 per TB (about $45,000 per PB) on crawler ingress that would otherwise be free.
- **Public IPv4** costs $0.005 an hour (about $3.65 a month) per address, idle or not, so rotating Elastic IPs costs money and gains little.
- **Internet egress** of about $0.09/GB applies to live view, screenshots and API responses, so meter it per tenant and serve bulk downloads through CloudFront.
- **Cross-AZ bytes** (rate not verified) appear whenever a run, its storage and its NAT path straddle AZs, including during a Regional NAT Gateway's expansion window.
- **Kinesis On-demand Advantage** has a floor of about $3,100 a month; **MSK Serverless** about $548 a month per cluster; **Firehose** bills each tiny record as 5 KB.
- **CloudWatch** custom metrics cost $0.30 per combination per month (per-tenant dimensions explode this), and custom logs $0.50/GB.
- **S3 small objects:** per-request costs, and nothing under 128 KB ever tiers down.
- **S3 Express** is single-AZ and must not be a checkpoint's only copy.
- **Aurora:** auto-pause adds 15 to 30 seconds or more to resume, and an RDS Proxy disables it.
- **API Gateway:** WebSockets cap at 2 hours, 10 idle minutes and 128 KB messages; HTTP APIs at 30 seconds.
- **KMS** keys per tenant cost $1 each per month.
- **Proxy vendors** bill every byte, with per-session minimums (Browserbase: 1 MB per session, rounded up per MB).

#### Cross-cutting: policy-constraint checklist
- **AWS AUP:** Goodfetch Cloud is responsible for traffic it facilitates. AWS weighs its prevention processes, an abuse notice unanswered after 24 hours risks blocks or suspension, and runner fleets should be isolated in their own accounts.
- **AWS Service Terms:** no crawling clause found. Bedrock may keep some model inputs and outputs up to 30 days for abuse detection, which matters if Goodfetch routes model calls through Bedrock.
- **Cloudflare:** AI crawlers are blocked by default on new domains (about 20% of the web). Verified or signed agents must obey robots.txt and keep reasonable rates, which conflicts with ADR 0016's default and needs an owner decision on cloud Operator policy.
- **Proxy partners:** KYC (Bright Data, for new residential zones from July 2026) and category blocks (government, banking, ticketing, webmail and others) pass down to Goodfetch's tenant terms.
- **AWS Marketplace:** hourly metering, a 1-hour flush after unsubscribe, and a 3% fee or less; promotional credits cannot pay for Marketplace purchases.
- **EU residency:** standard EU Regions suffice for residency. The Sovereign Cloud is a separate partition, with separate IAM and billing.

### Gaps
- Declarative policies (December 2024), GuardDuty runtime monitoring features and prices, Control Tower costs and VPC firewall pricing were not verified this session.
- The SaaS tenant-isolation whitepaper's silo, pool and bridge details beyond its introduction were not captured.
- Per-tenant IAM limits (session policy size, session-tag limits) were not verified.
- **Method note:** the shared web-search budget (200 searches across all agents) ran out after the first five rounds. Later findings rest on direct fetches of known official pages. Items needing a fresh search (official vendor statements from Akamai, DataDome and HUMAN; competitors' hosting and abuse processes; current inter-AZ and CloudFront per-GB rates) remain open, and a follow-up message can resume the search.
