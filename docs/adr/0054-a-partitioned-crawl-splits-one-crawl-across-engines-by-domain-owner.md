# A Partitioned crawl splits one crawl across engines, each Partition owning its domains

One run belongs to one engine (ADRs 0001 and 0002), and inside that bound it already scales out: handler processes through `workers=N` on any machine (ADR 0024) and browsers through Browser hosts (ADR 0022). What one engine cannot exceed is its own fetching bandwidth and the writes to its Frontier; a crawl of 500 million pages a day (hn-105) is about 5,800 pages a second. A **Partitioned crawl** is one crawl carried out as several runs, its **Partitions**, on several Group members of a Daemon group (ADR 0055). Each Partition owns the domains whose key hashes to it, and a link found for a domain another Partition owns is forwarded to that Partition. Decided in [Scaling past one engine](https://github.com/goodfetch/goodfetch/issues/116).

How comparable tools spread one crawl:
- **A shared queue with competing workers:** Crawlee's `RequestQueueV2` locking ([docs](https://crawlee.dev/docs/3.10/experiments/experiments-request-locking)), Crawlee Python's experimental Redis and SQL storage clients, scrapy-redis and Firecrawl. Capacity can be added at any time, but the Frontier has many writers and per-domain load grows with the number of workers.
- **Partitioning by host:** Nutch's Generator (`partition.url.mode`, [URLPartitioner](https://nutch.apache.org/documentation/javadoc/api/org/apache/nutch/crawl/URLPartitioner.html)), StormCrawler's status index routed by key, and Heritrix's `HashCrawlMapper`, which writes links owned by another machine to `.divert` files and leaves moving them to the operator ([wiki](https://github.com/internetarchive/heritrix3/wiki/Multiple-Machine-Crawling)). Politeness is exact because one worker owns each host. Their partition count is fixed per job or per index.
- **Fetching offloaded, Frontier kept:** Spider's `decentralized` feature sends fetches to `spider_worker` instances, which have no Chrome.

## Considered Options

- **A per-engine ceiling and a guide to splitting a crawl by hand**: rejected as the only answer. A link found in one hand-split run for a domain of another is either crawled twice or lost, which is Heritrix's `.divert` problem left to the user. The ceiling is still documented, so users know when a Partitioned crawl helps.
- **A shared queue**: rejected, already excluded by ADR 0002 and by CO-28 (ADR 0037): it brings back multiple writers, and it multiplies per-domain load.
- **A fixed number of Partitions** (Nutch, StormCrawler): rejected. Autoscaling (ADR 0055) could not help a crawl already running, and a removed Group member's Partition would have to move whole.

## Consequences

**Ownership.**
- A domain's key (ADR 0004) is assigned to the Partition with the highest rendezvous hash score for it, so adding or removing a Partition moves only the domains it now wins or held, about 1/N of them.
- Each Partition is an ordinary run: its own single writer, Frontier, run epoch (ADR 0003), Effect journal and Completeness.
- Politeness inside the crawl is exact without a Politeness store, since one Partition owns each domain; ADR 0053 still shares it with other crawls.
- A crawl of one large site stays in one Partition. Politeness caps that site at 8 requests in flight by default, far below one engine's capacity.
  > Amended by [ADR 0086](0086-a-crawl-obeys-robots-txt-and-sends-one-request-at-a-time-per-domain-by-default-and-pressing-harder-is-recorded.md): the default is now 1 request in flight and at least 1 s between starts, still far below one engine's capacity.

**Forwarding links.**
- A link whose key another Partition owns is sent to that Partition over the peer link (ADR 0055), at least once, and the owner deduplicates it against its Frontier, so it lands there exactly once. Forwarding uses ADR 0024's credit-based backpressure and is fenced by the sender's run epoch.
- Each forward carries the *partition-map epoch* under which it was routed. A Partition that no longer owns the key re-routes it under the current map.

**Changing the number of Partitions while the crawl runs.**
- Adding a Group member to the crawl, or removing one, publishes a new partition map under a higher partition-map epoch.
- A moving domain is handed off fenced: the old owner stops admitting it, waits until its requests in flight to it have ended, sends its pending URLs and the fingerprints of URLs already seen, and the new owner starts admitting it only once the handoff is acknowledged.
- A Group member removed from the group (ADR 0055) hands all its Partition's domains to the others this way, so a Partitioned crawl never needs a checkpoint transfer; on a restart, the Partition resumes in place. A handoff the grace period does not cover is completed from the kept volume by `runs adopt`, which hands the remaining domains to their owners under the current partition map rather than resuming the Partition as a run. A Partition left with no domains ends `finished`, and its Completeness counts only the domains it still owned at its end, so a handoff never marks the crawl `stopped` or `incomplete`.

**Starting and reporting.**
- `runs.start` takes `partitions`: a number, or `auto` for one per Group member that can run the crawl. It returns a Partitioned crawl ID beside the run IDs of its Partitions. The Lead Daemon places Partitions on distinct Group members (ADR 0055); a number above the Group members that can run the crawl is refused with the Error code `partitions_exceed_group_members`.
- On a Daemon outside a group or an Attached engine, `partitions` can only be 1; more is refused with the Error code `partitions_need_group`.
- A crawl of more than one Partition whose `session.source` is a Profile directory, an installed browser, a `state_file` path, a Borrowed browser on one machine or an import bound to one machine, or a run-only import on direct egress, is refused with `session_bound_to_machine`, and one on any other Borrowed browser with `browser_borrowed` (ADR 0056).
- It is reported on ADR 0029's three axes:
  - **Lifecycle** is `running` while any Partition runs; then `failed` if any Partition failed, else `stopped` if any stopped, else `finished`.
  - **Verdict** is the worst of the Partitions' Verdicts: `rejected`, then `needs-review`, then `verified`.
  - **Completeness** is `incomplete` if any Partition's is, else `unverified` if any Partition's is, else `complete`.
- Usage is summed. A Limit set on the crawl spans its Partitions through a new Limit scope, *Partitioned crawl*, held in the Lead Daemon's Usage ledger (ADR 0041).
  > Amended by [ADR 0107](0107-a-runs-model-allowance-refuses-its-model-calls-once-spent-while-the-run-goes-on.md): a Model allowance (`ai.allowance.*`) set on the crawl is the whole crawl's too, held by the Lead Daemon with the crawl's coordination state, outside its Usage ledger; each Partition reserves its model calls against it in slices, and once it is reached, every Partition's model calls are refused while the crawl goes on.
- Its Dataset is the union of its Partitions' Datasets, shown as one in the Console and exported as one. Each Partition's Sinks keep their own fencing (ADRs 0003 and 0008).
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.
- Abort, checkpoint and resume (ADR 0001) act on every Partition. A Schedule may target a Partitioned crawl (ADR 0036), and its Baseline is the whole crawl.
- Goodfetch Cloud serves `partitions` through its gateway, with one engine per Partition (ADR 0001), forwarding links between them as Group members do.
