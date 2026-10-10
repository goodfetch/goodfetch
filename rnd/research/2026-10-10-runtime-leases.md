# Work-item leases, run fencing and zombie handling: competitor research

Date: 2026-10-10. Facts only; each claim has a source. Items marked **[Inference]** are my reading, not a source statement. Versions: Crawlee JS v3.18.2 (tag read), Crawlee Python v1.10.4 (master read), scrapy-redis v0.9.1, Colly v2.2.0, spider-rs v2.54.0 (Cargo.toml on master; latest release tag v2.52.2), Temporal server v1.32.1, Hatchet v0.110.5 (master read), Restate docs for v1.7.x.

---

## 1. Crawlers: in-progress work when a worker crashes or hangs

### Apify request queue (platform API)
- Locking exists to avoid "concurrent processing of one request by multiple clients"; each lock expires so "requests which fail processing are eventually unlocked and retried". https://docs.apify.com/platform/storage/request-queue
- Operations: `listAndLockHead` (params `lockSecs` required, `limit` max 25, optional `clientKey` 1–32 chars), `prolongRequestLock`, `deleteRequestLock`. Locked items carry `lockExpiresAt`; the response carries `queueHasLockedRequests` and `hadMultipleClients`. https://docs.apify.com/api/v2/request-queue-head-lock-post
- Lock ownership "works on the client level, as well as the run level": prolonging/unlocking works only with the same client key or from the same Actor run; other clients get an error. No clientKey = treated as a new client. https://docs.apify.com/platform/storage/request-queue
- Rate limits: prolong/delete lock 350 req/s per queue; list-and-lock head 35 req/s per queue. https://docs.apify.com/platform/storage/request-queue
- No documented maximum lockSecs, no documented behaviour of a late "mark handled" from a client whose lock expired. (Absence noted on both pages above.)
- **[Inference]** The lock is a time-based lease identified by client key or run ID. There is no monotonic token, so the lock owner is identified, not ordered. Because a migrated run keeps its run ID, run-level ownership cannot tell the old container from the new one.

### Crawlee JS (v3.18.2) `RequestQueue` (V2, locking queue)
- `RequestProvider.requestLockSecs = 3 * 60` default; BasicCrawler overrides to `Math.max(requestHandlerTimeoutMillis / 1000 + 5, 60)`; default `requestHandlerTimeoutSecs` = 60 s; `maxRequestRetries` default 3. https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_provider.ts (line ~119), https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts (lines ~646, 739, 769)
- Fetches up to `LIST_AND_LOCK_HEAD_LIMIT = 25` via `client.listAndLockHead({ lockSecs })`; before handing out a cached request it calls `prolongRequestLock`; if prolong fails it treats the request as lost ("Cannot find a request ... or lost lock, will be retried later") and drops it. https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_queue_v2.ts (lines ~26, 172, 342–520)
- On `MIGRATING` and `ABORTING` events it calls `_clearPossibleLocks()` so the next incarnation can take the requests immediately. Same file, constructor (~lines 80–90).
- `markRequestHandled` → `client.updateRequest({...request, handledAt})`; `reclaimRequest` → `updateRequest` then `deleteRequestLock`. No lock/epoch is sent with the update. https://github.com/apify/crawlee/blob/v3.18.2/packages/core/src/storages/request_provider.ts (lines ~652–730), request_queue_v2.ts (~298)
- Crawlee master (v4 dev) generalises this as `setExpectedRequestProcessingTimeSecs` ("only ever raise the reservation duration ... otherwise a short-lived consumer could cut short the reservation of a long-lived one and have its in-flight request stolen") and `extendRequestProcessingTimeSecs`. https://github.com/apify/crawlee/blob/master/packages/core/src/storages/request-queue.ts (~lines 851–880)

### Crawlee Python (v1.10.4) storage clients
- SQL client: `_BLOCK_REQUEST_TIME = 300` s; fetch sets `time_blocked_until` and `client_key` (random 32-char per client), uses `SELECT ... FOR UPDATE SKIP LOCKED` where supported, else a conditional UPDATE. `mark_request_as_handled` updates by `request_id` only and clears `client_key`; **it does not check that the caller still owns the block**. https://github.com/apify/crawlee-python/blob/master/src/crawlee/storage_clients/_sql/_request_queue_client.py (lines ~96, 118, 424–520)
- Redis client: `_BLOCK_REQUEST_TIME = 300_000` ms, `_RECLAIM_INTERVAL = 30 s`; in-progress hash stores `{client_id, blocked_until_timestamp}`; `mark_request_as_handled` does an atomic `HDEL` on the in-progress hash and warns if absent. It checks presence, not the client id. https://github.com/apify/crawlee-python/blob/master/src/crawlee/storage_clients/_redis/_request_queue_client.py (lines ~83–86, 366, 395–410)
- File-system client: an in-process `asyncio.Lock`; `in_progress_requests` set kept in `RecoverableState` (persisted) for single-process restart. No lease/expiry. https://github.com/apify/crawlee-python/blob/master/src/crawlee/storage_clients/_file_system/_request_queue_client.py (lines ~54, 95–120, 484–522)
- **[Inference]** In both SQL and Redis clients, a slow worker whose block expired can still mark a request handled after another client re-fetched it, so the late commit is accepted.

### Scrapy JOBDIR
- Persists the scheduler queue (pending requests on disk), the dupefilter (`requests.seen`), and `spider.state`. Stop "safely" with Ctrl-C or a signal. "Forced, sudden or otherwise unclean shutdown can lead to data corruption in the job directory". The directory "must not be shared by different spiders, or even different jobs of the same spider". Unpicklable requests are kept only in memory and lost on pause. Resume needs the same Scrapy version. https://docs.scrapy.org/en/latest/topics/jobs.html
- No leases, epochs or retry of in-flight requests after a hard kill are documented (same page).
- **[Inference]** Requests popped from the disk queue and already recorded as seen in `requests.seen` but not finished when the process is killed are neither in the queue nor re-scheduled, so they are lost on resume.

### scrapy-redis (v0.9.1)
- Queues pop destructively: FIFO `brpop`/`rpop`, LIFO `blpop`/`lpop`, priority queue `MULTI`+`zrange`+`zremrangebyrank`. No ack or processing list. https://github.com/rmax/scrapy-redis/blob/master/src/scrapy_redis/queue.py (lines ~66–150)
- **[Inference]** Requests a crashed worker has already popped are lost. There is no lock, so there is nothing to expire or fence.

### Colly (v2.2.0) redis storage
- `GetRequest` = `LPop`. No ack/lease. https://github.com/gocolly/redisstorage/blob/master/redisstorage.go (line ~127)

### spider-rs (spider crate 2.54.0)
- Offers `pause`/`resume`/`shutdown` controls under the `control` feature (in-process); a `disk` feature for "SQLite hybrid disk storage to balance memory usage"; a `decentralized` feature that moves IO to `spider_worker` processes. https://github.com/spider-rs/spider/blob/main/spider/README.md (lines ~136–140, 179–197, 268–296)
- Code search for "heartbeat" found nothing. "lease" matched only unrelated files (h2 tracker, interner). No durable crash-resume of in-flight work is documented. (gh code search over spider-rs/spider, 2026-10-10)

### Others
- Stagehand v4: a browser-agent SDK with no request queue, so it has no work-item lease model. Not investigated further.
- Frontera: not investigated (archived project).

**Answer to Q1:** No crawler found uses epochs or fencing tokens. Apify/Crawlee use expiring, owner-identified locks (client key / run) with prolong. Crawlee Python uses expiring "blocks" with a client key but accepts late commits. Scrapy, scrapy-redis and Colly have no lease at all, so popped work is lost on a crash.

---

## 2. Apify actor migration

- Causes: "Server workload optimization", "Server crashes (rare)", "New feature releases and bug fixes". Migrations can happen at any time. After `migrating`, "the Apify platform will shut it down and restart it on a new server within one minute". "You only have a few seconds to save your work". Without saved state the run restarts "with an empty internal state". https://docs.apify.com/platform/actors/development/builds-and-runs/state-persistence
- What survives: "the Actor run's default dataset, key-value store and request queue are preserved across migrations". `Actor.useState()` / `Actor.use_state()` auto-save to the KV store. Same page.
- Events: `migrating` (payload `timeRemainingSecs`); `aborting` (graceful abort: user-initiated, max cost limit, Standby inactivity/worker shutdown; no cleanup window stated); `persistState` (payload `isMigrating`; every 60 s by default; generated by the SDK, not sent over the WebSocket); `cpuInfo` (~1 s). https://docs.apify.com/platform/actors/development/programming-interface/system-events
- `PERSIST_STATE` is also emitted when migration starts, with `is_migrating` set. You can call `Actor.reboot` after persisting to trigger migration immediately; with `gracefulShutdown`, the JS SDK calls `Actor.reboot` automatically on `migrating`. https://docs.apify.com/sdk/python/docs/next/concepts/actor-events , https://docs.apify.com/sdk/js/reference/class/PlatformEventManager
- Crawlee's queue releases its locks on `MIGRATING` / `ABORTING` (see section 1).
- **Two copies acting at once:** no Apify page I found documents a mechanism, such as fencing or a run epoch, that prevents the old container from writing after the new one starts. The docs only promise shutdown "within one minute". (Absence: state-persistence and system-events pages above.)
- **[Inference]** Apify relies on the platform killing the old container, a one-minute bound, plus expiring request locks. Because storage keys on run ID, the old container could still write to the KV store or dataset until it is killed.

---

## 3. Durable-execution engines

### Temporal (server v1.32.1)
- Timeouts: Schedule-To-Start ("non-retryable by design"), Start-To-Close (single attempt; on fire, `ActivityTaskTimedOut` and retry per policy), Schedule-To-Close (all attempts), Heartbeat Timeout (max time between heartbeats; capped at Start-To-Close; `0s` disables). "Activities that don't Heartbeat can't receive a Cancellation." Heartbeat throttle = min(0.8 × heartbeatTimeout, or 30 s by default, and the max interval of 60 s). "The Temporal Server doesn't detect failures when a Worker loses communication with the Server or crashes". https://docs.temporal.io/encyclopedia/detecting-activity-failures
- The internal task token contains `scheduled_event_id`, `attempt`, `version`, `start_version`, `activity_attempt_stamp`, among others. https://github.com/temporalio/temporal/blob/main/proto/internal/temporal/server/api/token/v1/message.proto
- Completion validation: `IsActivityTaskNotFoundForToken` returns not-found if `token.Attempt != ai.Attempt` or start versions differ. The server then returns `ErrActivityTaskNotFound` ("invalid activityID or activity already timed out or invoking workflow is completed"). https://github.com/temporalio/temporal/blob/v1.32.1/service/history/api/activity_util.go , https://github.com/temporalio/temporal/blob/v1.32.1/service/history/api/respondactivitytaskcompleted/api.go (line ~84), service/history/consts/const.go (line ~45)
- Docs list NOT_FOUND on `RespondActivityTaskCompleted` as normal when a task timed out or the worker restarted. https://docs.temporal.io/troubleshooting/request-failures.md
- Side effects: "Temporal guarantees that the Activity will be observed as completed exactly once. However, the Activity may be executed multiple times and may even partially complete more than once". Idempotency keys "are enforced by the service you are calling from your Activity". https://docs.temporal.io/activity-definition
- A timeout does not stop the activity code (community answer). https://community.temporal.io/t/activityworker-error-invalid-activityid-or-activity-already-timed-out-or-invoking-workflow-is-completed/2955
- **[Inference]** The attempt number acts as a per-work-item fencing token, checked by the server (the journal owner), together with timeouts and heartbeats. External effects are not fenced.

### Restate (docs v1.7.x)
- One active leader per partition. A promoted follower "obtains a new epoch, appends an epoch-bump record". "Late appends from superseded leaders or stale handler attempts (carrying lower epochs) are fenced". Invocation "attempts carry monotonically increasing epochs; the processor rejects any events from superseded epochs". A step "happened" when its log append reaches quorum and is not re-executed afterwards. https://docs.restate.dev/references/architecture
- Invoker timers: `inactivity-timeout = "1m"` (on expiry, Restate asks the invocation to suspend) and `abort-timeout = "10m"` (starts after the inactivity timeout and "potentially interrupts user code"). Retry policies: "Infinite retries if unset". https://docs.restate.dev/references/server-config
- Docs elsewhere claim side effects in tool steps run "exactly once" and recommend deterministic idempotency IDs. https://docs.restate.dev/ai/patterns/durable-agents , https://docs.restate.dev/tour/microservice-orchestration
- **[Inference]** A crash after the effect but before the journal append re-runs the effect, so external effects are at-least-once in practice.
- **Restate is the closest match to our two-level design:** leader epochs (process/partition incarnation) plus invocation attempt epochs (work item).

### DBOS
- Recovery: a single node scans PENDING workflows at startup. Distributed recovery goes through Conductor, which detects failed executors by a closed websocket and, after a grace period, reassigns their workflows. Checkpointed steps are never re-executed. https://docs.dbos.dev/architecture
- Zombies: "a 'zombie' executor could still be running your workflow". "steps get at-least-once guarantees and workflow outcomes are persisted exactly-once". Each execution "generates a unique ownership token, records it in the workflow's row". Each checkpoint write "first checks, in the same database transaction, that the recorded token still matches its own". The loser's checkpoint "is not written" and the execution parks. After a conflict error, "no subsequent work in the workflow will be made durable". https://docs.dbos.dev/explanations/concurrent-executions
- **[Inference]** This is antirez-style fencing: a random ownership token plus check-and-set in the same transaction as the journal write, not a monotonic epoch. It fences the journal only.

### Inngest
- Per-step retries (default 1 attempt + up to 4 retries); completed step results are reused. The docs state that a step does not make an external action exactly once and recommend idempotency keys derived from the business operation. Function-level `idempotency` dedupes events for 24 h. https://www.inngest.com/docs/guides/multi-step-functions , https://www.inngest.com/docs/durable-execution/primitives/step-run , https://www.inngest.com/docs/durable-execution/guides-and-advanced/error-handling/retries
- No public fencing/epoch mechanism found.

### Hatchet (v0.110.5)
- Workers heartbeat every 4 s. After 30 s without one, the worker is INACTIVE and its in-flight tasks are re-queued. Execution timeout default 60 s; scheduling timeout default 5 m. A timeout counts as a failure and retries. "A timed out step does not guarantee that the step will be stopped immediately". `refreshTimeout` is additive. HARD sticky tasks are not reassigned when the worker dies. https://docs.hatchet.run/v1/faq , https://docs.hatchet.run/home/guarantees-and-tradeoffs
- "Hatchet is at least once ... a task can run more than once". https://docs.hatchet.run/home/guarantees-and-tradeoffs
- Source: failure/retry SQL matches `(task_id, inserted_at, retry_count)` against the current `v1_task.retry_count` and requires a `v1_task_runtime` row for the current retry count ("a late failure event should not trigger a retry"). https://github.com/hatchet-dev/hatchet/blob/main/pkg/repository/sqlcv1/tasks.sql (lines ~341–400)
- **[Inference]** `retry_count` works as an attempt-level fencing token, like Temporal's attempt.

### Cloudflare Workflows / Durable Objects
- Steps may be retried and "should (ideally) be idempotent". The docs' pattern is to check whether a charge was already made before charging. Code outside steps may run twice. https://developers.cloudflare.com/workflows/build/rules-of-workflows/
- Durable Objects (the substrate): one active instance at a time. https://developers.cloudflare.com/durable-objects/learning/in-memory-state
- Split brain: a new instance cannot start until the old one "can no longer confirm writes"; 3 of 5 followers are told to stop confirming writes for it. Output gates hold outgoing messages until writes are confirmed. This applies "not only to responses sent back to a client, but also to new outgoing requests made with fetch()". If a write fails, the outgoing messages are discarded and the object restarts. https://blog.cloudflare.com/durable-objects-easy-fast-correct-choose-three/
- After a code update, old instances are "disconnected from the persistent storage API" to keep the single-instance invariant. https://developers.cloudflare.com/workers/platform/changelog/historical-changelog/
- **[Inference]** This is storage-level fencing (quorum refuses the old instance) plus output gating: external I/O is released only after the journal confirms. It is the strongest zombie-acting prevention found, and it is the model for "quarantine the browser if the zombie may still act".

---

## 4. Fencing tokens: Kleppmann and critiques

- Kleppmann (2016): a client pauses (GC), its lease expires, another client acquires the lock, and the first then writes stale data. Checking expiry before writing doesn't help. Fix: the lock service issues a monotonically increasing token, and the storage server rejects writes with a lower token than one already accepted, so the resource must enforce it. Locks for efficiency tolerate occasional double work; locks for correctness need this. Redlock "does not have any facility for generating fencing tokens" and depends on timing assumptions. https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html
- antirez's reply: (1) if the resource can reject stale writes, the distributed lock adds little; (2) a random unique token with check-and-set suffices, so no monotonicity is needed; (3) many effects (external APIs, emails, physical actions) cannot check tokens, so fencing doesn't apply; (4) token issue order may not match action order. http://antirez.com/news/101
- Kubernetes client-go leader election: "This implementation does not guarantee that only one client is acting as a leader (a.k.a. fencing)". It tolerates clock skew but not clock skew rate. https://github.com/kubernetes/client-go/blob/master/tools/leaderelection/leaderelection.go
- When the resource can't compare tokens, "the fencing token is only evidence, not enforcement". Recommended fix: route through an authoritative row/service that records the highest token plus an idempotency key before emitting the effect. https://hashnode.com/posts/fencing-tokens-in-net/6a723685183addc17be0b86b , https://system-design.space/en/chapter/distributed-locks-leases-fencing/
- Hazelcast FencedLock: a stale client's write is rejected by the external service only if that service checks the token. https://docs.hazelcast.com/hazelcast/5.0/data-structures/fencedlock

**Applied to external websites [Inference, grounded in the above]:** A website will never check our epoch. Fencing can guarantee that (a) a zombie's results, journal entries, enqueued links, dataset rows and KV writes are rejected at our journal/storage, and (b) actions gated on a journal round trip (DO-style output gate) are not issued after fencing takes effect. It cannot guarantee that (c) HTTP requests, clicks or form posts already in flight, or issued between the last successful check and the zombie noticing, are blocked (the Kleppmann pause window), or (d) that a non-idempotent external action is not duplicated by the retry. Those need idempotency keys or check-before-act (Cloudflare's "check if already charged"), or killing the process/browser (STONITH-style: Apify's one-minute kill, Hatchet SIGKILL, DO quorum refusal).

---

## 5. Is the two-level design overkill, missing pieces, or replaceable by expiring locks?

Facts:
- Every crawler surveyed uses expiring locks at most; none fences late commits (Apify/Crawlee, Crawlee Python SQL/Redis) or has any lease (Scrapy, scrapy-redis, Colly, spider-rs). Sources in section 1.
- Every durable-execution engine with retries fences late commits per attempt: Temporal `attempt`, Hatchet `retry_count`, Restate invocation epochs, DBOS ownership token. Sources in section 3.
- Process/instance-level fencing is explicit in Restate (leader epoch) and Cloudflare DO (quorum + output gate). DBOS covers zombies with the per-workflow ownership token. Apify documents nothing beyond "shut down within one minute". Sources in sections 2–3.
- All engines state that external side effects remain at-least-once and require idempotency. Sources: Temporal activity-definition, DBOS concurrent-executions, Hatchet guarantees, Inngest step-run, Cloudflare rules-of-workflows.

Inferences for a one-engine-per-run setup:
- **Level 1 (work-item lease + epoch)** is justified as soon as handlers are out-of-process (JSON-RPC SDKs) and can hang past their timeout. JS promises and Python threads can't be force-cancelled, and Temporal and Hatchet both say timed-out code keeps running. The epoch is cheap: an integer column checked in the same SQLite transaction as the commit, the same pattern as Temporal attempt, Hatchet retry_count and DBOS's token check. Expiring locks alone (Crawlee Python) accept a late commit and can double-write results or links.
- **Level 2 (run epoch)** is needed only if two engine processes can ever hold a writable copy of the run's state. With a local SQLite file plus portable snapshot, the zombie writes to *its own* file, so a SQLite-level epoch check alone does not fence it. The fence must sit where the copies converge: the checkpoint/snapshot store (conditional put on the epoch), result sinks (dataset/KV, exported artifacts) and any shared external resources (proxy session, browser profile, account cookies). This is the missing piece to verify: the run epoch must be enforced by whatever authority hands out the resume (control plane or object-store CAS), not only inside the run's own DB. This mirrors DO's quorum refusal and Restate's epoch-bump record in a shared log.
- **A simpler alternative** that the sources support: if the platform guarantees the old container is dead before the resume starts (kill/STONITH, as with DO "cannot start until ... can no longer confirm writes"), level 2 reduces to "resume requires a successful CAS on the epoch at the checkpoint store, after confirmed termination". A run epoch is still cheap insurance for self-hosted users without such a platform guarantee.
- **Missing pieces to consider:** (1) an output-gate equivalent: browser actions and outbound requests marked side-effecting are released only after the lease/epoch check, or after a journal intent record (DO pattern); (2) idempotency keys or check-before-act for non-idempotent site actions, since fencing can't cover them (section 4); (3) prolong/heartbeat with "only raise, never lower" lease duration (Crawlee master comment); (4) lease expiry measured on a monotonic clock of the lease grantor only (client-go skew note); (5) Scrapy's lesson that requests marked "seen" but in flight are lost on a hard kill unless the lease state is persisted.
