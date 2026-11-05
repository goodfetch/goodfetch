# Bursting past one machine: local-first Foxhound or Collie that overflows to remote capacity

**Status:** analysis for the owner, not a decision. **Date:** 2026-10-24.
**Question (owner):** can a Foxhound or Collie running on a person's machine send work to remote capacity (Goodfetch Cloud, the person's own cloud account, other machines) automatically when the machine is full, for example 10 million browser-rendered URLs in one hour? What do competitors do, what is feasible, what would be new? Politeness is out of scope by the owner's choice (ADR 0086 already lets a run press harder, recorded).
**Built on:** ADRs 0001, 0002, 0003, 0020, 0022, 0024, 0040, 0041, 0043, 0049, 0054, 0055, 0096; the cloud notes in [`research_notes/Goodfetch Cloud architecture and economics/`](../research_notes/Goodfetch%20Cloud%20architecture%20and%20economics/) (2026-10-12); two web surveys made for this note: [precedents.md](precedents.md) (hybrid and burst products) and [vendor-capacity.md](vendor-capacity.md) (concurrency ceilings, creation rates, batch endpoints).

## 1. What 10 million browser pages in one hour costs physically

2,778 pages/s. By Little's law, concurrency = rate × wall time per page.

| Quantity | Light pages | Heavy pages | Basis |
|---|---|---|---|
| Wall time per page | 4 s | 8 s | assumption |
| Concurrent pages | 11,100 | 22,200 | 2,778 × wall time |
| CPU per page | 0.5 vCPU-s | 2.5 vCPU-s | 0.12 s light JS (Lightpanda benchmark), 1.2 s lab task, heavier SPA (workload_resource_profiles.md) |
| vCPU busy | 1,400 | 6,900 | |
| RAM (300 / 500 MB per open page) | 3.3 TB | 10.9 TB | |
| 16-vCPU nodes at 85 % packing | ~100 | ~510 | |
| Compute for the hour, AWS c7g.4xlarge on-demand / spot | $59 / $25 | $296 / $124 | $0.580 / $0.242 per hour (provider_alternatives_vs_aws.md) |
| Same on Hetzner CCX43 (16 vCPU), €0.531/h | ~€46 | ~€230 | |
| Bandwidth (0.6 MB with blocking / 2.6 MB median page) | 13 Gbit/s, 6 TB | 58 Gbit/s, 26 TB | Web Almanac 2025 page weight |
| Residential proxy at $1.8–8/GB | $11k–48k | $47k–208k | pricing_hosted_browsers_and_agents.md |

Conclusions:
- **Compute is cheap** (tens to hundreds of dollars). What costs money is proxies (if residential) and hosted per-request pricing; what blocks is ramp-up and allowances.
- **Ramp-up and quotas are the wall.** A new AWS account has a 5 vCPU default for Standard on-demand and Spot; new capacity takes 30 s to minutes; Lambda adds 1,000 environments per 10 s per function; Hetzner raises limits by hand ([vendor-capacity.md](vendor-capacity.md)). A burst of this size must be reserved ahead.
- **The person's machine cannot be in the data path.** 13–58 Gbit/s of page traffic, 11–22 k concurrent sessions and, with a remote browser per page, 14–28 k WAN round trips per second (5–10 operations per page, ADR 0022) are beyond a home or office link, and a laptop that sleeps stops everything. Above a few hundred remote pages, the work has to move to where the browsers are.
- **The cheapest browser is the one not opened.** ADR 0020 already samples each route both ways; in the lab, the discovered API cost 0.008 and HTTP 0.004 of the browser's time while matching it exactly. If most of the 10 M URLs share a few routes, the fleet shrinks by one to two orders of magnitude before any burst.

## 2. What competitors offer (2026-10-24)

No product documents "use local capacity until full, then overflow to remote" for browsers or crawls ([precedents.md](precedents.md)).

| Product | How remote capacity is reached | Automatic? | Ceiling relevant to a 10 M/h burst |
|---|---|---|---|
| Browserbase | Swap the CDP endpoint; one session per connection | No; 429 at the plan limit | Scale "250+" concurrent, "150+" new sessions/min; blog claims customers at "6,000+ concurrent" (verified, 2026-06-16): 10 M pages take ≥2.3 h |
| Bright Data Browser API / Web Unlocker | Remote CDP / per-request API | No | "Unlimited" concurrency, no published cap or rate; per-GB billing |
| Zyte API | Per-request HTTP API | No | 3,000 RPM per key; temporary raise by ticket ≥24 h ahead |
| Cloudflare Browser Run | Workers binding, sessions, `/crawl` (one seed, ≤100k pages) | No | 200 concurrent, 3 new browsers/s on Paid |
| Firecrawl | Batch scrape queue | Queue only | 100 concurrent on Scale: 10 M pages ≈ 139 h |
| Kernel | Pre-warmed browser pools | Resized by hand | 150 concurrent on Start-Up |
| Steel, Browserless, Anchor | Same API self-hosted or cloud | No router between them | — |
| Apify | `apify run` local, `apify call` remote; local Crawlee can use cloud storage (`forceCloud`) and Apify Proxy | No auto-offload | Account memory limits |
| Spider `decentralized` | Static list of `spider_worker` URLs that fetch over HTTP; frontier stays local | Static | No Chrome in workers |
| Playwright Workspaces (Azure App Testing, successor of Microsoft Playwright Testing) | Test runner local, browsers in Azure, chosen by a config file | Manual, all or nothing | Per-workspace worker limit |
| Selenium Grid 4 + KEDA | Workers scale on session-queue length | Yes, inside one cluster | — |
| Modal, Coiled/Dask, Ray | Local entrypoint, remote functions/cluster; autoscale on pending work | Yes for cluster size; the author chooses what is remote | Ray Client drops the job after 30 s of lost connection |
| Bazel dynamic execution, icecream, distcc | Local build offloads compile jobs to a remote pool; Bazel races local and remote, icecream prefers local when fast enough | Yes | — |

Patterns: (a) endpoint swap; (b) static pool of remote I/O workers; (c) workers pull from a queue and a scaler watches its depth; (d) remote capacity disguised as a local node (virtual-kubelet, Liqo, Slurm cloud bursting); (e) per-task race or fallback (Bazel); (f) reverse tunnel so remote browsers reach local resources (BrowserStack Local, Playwright `exposeNetwork`).

Gaps nobody fills: pressure-based local-then-remote overflow for browsers or crawls; a frontier that stays local while work runs remote and can then move whole; deadline- or cost-aware sizing; falling back elsewhere on a vendor 429.

## 3. What Goodfetch already has

| Piece | ADR | What it gives a burst |
|---|---|---|
| Browser host | 0022, 0096 | A remote browser driven from a local engine at one round trip per action; run state stays local; reports its memory pressure to the engine |
| Resource governor | 0040 | Measures memory per open page per route, CPU and Tokio saturation; knows when the machine is full and how much each page costs |
| Fetch tiers by sample | 0020 | Measures whether a route needs the browser at all |
| Portable run state, checkpoint, `migrate` | 0001, 0002 | A run can stop here and resume on another engine |
| Leases and fencing | 0003 | Work handed out can be lost and redone without duplicates |
| Daemon group, placement by headroom, KEDA on saturation | 0055 | Several machines as one; the Lead Daemon places runs where there is room |
| Partitioned crawl | 0054 | One crawl split across engines by domain owner, resizable while running |
| Usage ledger and Limits | 0041 | Spending caps that hold across machines |
| Same published executable everywhere; cloud is an ordinary protocol client | 0001, 0096, 0049 | Anything Goodfetch Cloud does is reproducible on the person's own cloud |
| Nothing leaves the machine unless configured | 0043 | Bursting must be opt-in and name its destination |

Missing: a **capacity pool** concept (a set of Browser hosts or engines the governor can draw from), an **overflow policy** in the governor, **remote handler workers** across machines (`workers=N` is local processes, ADR 0024), a **planner** that turns a deadline and a budget into capacity, and a **provisioner** that rents and returns machines.

## 4. Proposed architecture: four units of offload, one policy

The governor already decides how much work fits locally. Add a policy that, when backlog exceeds what local capacity finishes in time, offloads the smallest unit that removes the bottleneck:

| Unit | What moves | What stays local | Round trips | Fits |
|---|---|---|---|---|
| 1. Remote page | The browser (Browser host from a pool) | Engine, frontier, handlers, run state, Sinks | One per action | Tens to a few hundred extra pages; agents; sessions with local secrets |
| 2. Remote worker | Fetch + render + handler for leased URL batches; returns Records and found links | Engine as single writer of the frontier | One per batch | Hundreds to low thousands of pages in flight; handlers packaged as a Recipe |
| 3. Remote run | The whole run, by checkpoint and `migrate`, then a Partitioned crawl on N cloud engines | Nothing but a Client watching events; Sinks written from the cloud | None on the data path | Thousands of pages/s; the laptop may close |
| 4. Hybrid group | Runs and Partitions placed across the person's machine and rented Group members | The person's machine is one member | Peer link | Teams with their own machines plus overflow; the Lead Daemon must not be a laptop |

Capacity sources, all running the published executable (ADR 0096): the person's other machines; **their own cloud account** (an API key for AWS, Hetzner, GCP: Goodfetch rents ephemeral servers and deletes them after the run); Goodfetch Cloud (managed, billed); third-party CDP browsers (Browserbase, Bright Data) under their concurrency caps (ADR 0022).

A 10 M pages/h job is unit 3 with capacity reserved ahead; units 1 and 2 cover the everyday "my laptop is full" case.

## 5. What would be new

1. **Local-first overflow, automatic and opt-in.** Nobody does it for browsers or crawls; Bazel and icecream prove the model for builds.
2. **Deadline and budget as inputs.** "Finish by 15:00, at most €200": the engine samples the routes (ADR 0020), measures cost per page (ADR 0040), and returns a plan (local only: ETA 69 h; with 300 machines: ETA 58 min, estimated cost) before renting anything. Competitors sell a concurrency number and let the customer do the arithmetic.
3. **Avoid the browser before bursting.** The plan first moves routes to HTTP or the discovered API, then sizes the browser fleet for what remains.
4. **Bring your own cloud.** The same executable on the person's AWS or Hetzner account: no vendor concurrency cap, data stays in their account, and it costs Goodfetch nothing (AGENTS.md Money rule).
5. **Data gravity.** Sinks are written from where the work runs; the person's machine receives events, not pages.
6. **Move and come back.** Start locally, migrate when the plan says so, close the laptop, pull the run back from a checkpoint afterwards.
7. **Spot by default for batch work.** Leases and checkpoints make preemption cheap (~60 % below on-demand).
8. **Capacity reservation.** "Reserve 300 machines at 14:00" hides the 30 s–3 min ramp and the quota wall.
9. **Sell throughput, not concurrency.** With a fleet that scales on backlog, Goodfetch Cloud can price per page or per browser-minute without a concurrency cap or a 429.

Optional, not by default: a Bazel-style race for the slowest tail of a crawl. ADR 0040 refuses redundant requests unless the run asks, so this stays a setting.

## 6. Open questions for a grilling session

- Is the overflow policy part of the governor (ADR 0040) or a layer above it like the Daemon's scheduler (ADR 0036)?
- How does a remote worker get handler code: only Recipes, or does the SDK package a script?
- Which Sessions may leave the machine: a Session using a Secret or a logged-in identity needs Operator policy per domain (ADR 0022), and the egress IP changes from residential to datacenter.
- Who holds the Lead Daemon when a laptop starts a group with cloud members?
- Money: BYOC first (no Goodfetch spend), Goodfetch Cloud only when the owner allows spending.

## Sources not in the linked notes

- Browserbase, "What is a Browserbase browser" (2026-06-16): "Customers run 6,000+ concurrent sessions on us today" — fetched 2026-10-24.
- Microsoft Learn, "What is Playwright Workspaces?" (updated 2026-09-29) — fetched 2026-10-24. The retirement date of Microsoft Playwright Testing (2026-03-08) comes from a secondary source and is unverified.
