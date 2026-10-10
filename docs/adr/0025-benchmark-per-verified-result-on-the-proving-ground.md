# The V1 benchmark measures per verified result, on the Proving ground, at equal guarantees

Every figure Automan publishes about its performance comes in three views: raw throughput or latency, the same work per verified result (time, CPU, memory), and dollars per 1,000 verified results. A raw figure never appears without its verified count and its false-success count beside it. A result is verified when an oracle independent of the tool confirms it correct and complete, and every failed attempt and retry counts in the cost. Figures come only from the Proving ground, a website Automan owns and generates from a seed so that every page has a known expected result. None of the public benchmarks surveyed for Crawlee, Spider and Stagehand can rank them: Spider's harness never checks that each competitor fetched the same pages, and an agent benchmark counts cost on successful attempts only ([benchmark audit](https://github.com/Jbmanllr/automan/blob/main/rnd/research/2026-10-07-benchmark-audit-followup.md)). Decided in [V1 benchmark protocol](https://github.com/Jbmanllr/automan/issues/79).

## Considered Options

- **Raw speed alone** (pages per second, milliseconds per click), as Spider and Lightpanda publish: rejected as a standalone figure. A tool that does less work, or reports success without delivering, looks faster. Raw speed is kept as one of the three views.
- **Cost alone**: rejected as the only view. It hides latency, which decides interactive automation.
- **Live third-party sites**: rejected for published figures. They change between runs and rate-limit, and scraping them for marketing carries the legal exposure found in [Legal exposure of stealth and challenge-solving tooling](https://github.com/Jbmanllr/automan/issues/65). Challenges are measured on the Proving ground's online copy instead, behind real vendors on a domain Automan owns.
- **Laptops or shared cloud instances**, as competitors measure (Spider on an M1 Max, Lightpanda on AWS m5): rejected. Only the lab's `reference` host class, dedicated bare-metal Linux, is published.

## Consequences

- **Families.** HTTP crawl; rendered crawl; extraction, speed and quality; scripted actions; remote browser; AI; repeated runs with Drift; incident and recovery; startup and binding overhead (ADR 0024); the cost of each switchable control (ADRs 0016 and 0023); and Access, whose content comes from [Stealth and challenge-solving catalog](https://github.com/Jbmanllr/automan/issues/78).
- **AI use cases.** First run that compiles a Plan; replay with Drift and Repair; autonomous agent; Automan driven by an outside agent through MCP, CLI and skills, or code mode; AI extraction to a schema; AI-guided crawl; search then crawl; vision against DOM actions; the decision-provider fast path; AI safety against prompt injection, fake success messages and allowlist escapes; Human pause; an agent woken by a change on a watched page; a fleet of agents on one host; an agent resumed from a Checkpoint after the engine is killed; an authenticated agent with MFA and account rotation; extraction rules written once by AI and then run on every page without a model; and documents and visuals. Each runs where it applies with a cloud or a local model (ADR 0012) and a local or hosted browser (ADR 0022).
- **Measurements handed on by earlier decisions.**
  - The guard stays at 390 of 390 and is no slower than Playwright on the lab's click and form tasks on the same machine; a miss reopens ADR 0019.
  - A third-party remote browser driven over raw CDP with the guard is measured against Stagehand v4 with equal guarantees; if Automan is clearly slower, ADR 0022 opens a ticket for an in-browser runtime.
  - The cost of every switchable control is published (ADR 0023).
- **Proving ground.**
  - **Pages.** Millions of pages generated from a seed. It covers, among others:
    - CSS class churn, honeypot links and every kind of CAPTCHA and protection
    - flagged IPs and per-IP rate limits with `Retry-After`
    - every HTTP error, soft 404s, redirect loops, crawler traps and duplicate URLs
    - schema.org data and hidden APIs
    - the same entity under small structure changes
    - AJAX loading, infinite scroll, shadow DOM and iframes
    - logins with MFA and expiring sessions
    - geo and language variants, odd encodings, huge pages, PDFs and downloads, sitemaps
    - content that changes on a known schedule, for change tracking
    - prompt-injection pages, fake success messages, and TLS and HTTP/2 fingerprint checks
  - **Two copies.** A local copy serves every published speed figure, with declared network profiles: added round trips of 0, 20, 100 and 300 ms, and loss through `tc netem`. An online copy on a domain Automan owns adds real networks and real challenge vendors.
  - **Open source.** The Proving ground is published, so anyone, competitors included, can rerun the benchmark.
- **Competitors.**
  - **Tools.** Crawlee (JavaScript and Python), Spider, Scrapy, Colly in async mode, Stagehand v4, Playwright, Crawl4AI, Browser Use, Vercel's `agent-browser`, Firecrawl self-hosted (its cloud-only anti-bot engine noted as absent) and Scrapling; ScrapeGraphAI, Skyvern and changedetection.io where their use case applies.
  - **Configuration.** Each runs in its best documented configuration for the family, pinned at its latest stable release when the benchmark freezes. Configurations are published, and maintainers get 14 days to review theirs before publication.
  - **Equal guarantees.** Where a competitor lacks a guarantee, Automan appears twice: by default, with every control on, and matched, with the controls the competitor lacks switched off and marked as ADR 0023 requires.
- **Reference host.** One rented bare-metal x86_64 server with 16 cores and 32 threads, 128 GB of RAM and NVMe, running Ubuntu 24.04 LTS. Turbo is off and SMT is on, fixed for a series, under the lab's `reference` gates. The Proving ground is served from a second machine on the same switch, so the reference host measures only the tool. arm64 runs stay unpublished `local-controlled` checks. The owner approves both machines when they are provisioned.
- **Cost.** Dollars per 1,000 verified results sum:
  - the reference host's public hourly price multiplied by wall time
  - model, proxy and CAPTCHA-service spend at list price, failed attempts included
  - hosted browsers at the vendor's list price per browser-minute

  Plans show their cost spread over 1, 10, 100 and 1,000 runs, a count declared before measuring. Human time is reported apart, never folded in.
- **Statistics and load.**
  - **Statistics.** The lab's rules: claims declared in `claims/` before measuring, with their metric, comparator, direction and minimum effect Δ; verdicts computed by fixed rules from block-bootstrap intervals; at least five blocks, more when results scatter widely; the page or the task as the pairing unit.
  - **Load.** Concurrency runs at 1, 8, 64 and 256, plus a search for the highest concurrency where verified throughput still rises and false success stays at zero.
- **Publication.** Every result is published, losses and inconclusive verdicts included, with raw data, harness, configurations and redacted traces. Results go on a Benchmarks page in the documentation and in a versioned folder of the repository. A figure used in marketing must quote a published claim and link to it.
- **Releases.** The benchmark reruns at every minor release against the then-current competitor versions, and earlier results stay published with their dates. A correctness failure always blocks a release. A primary metric worse than the previous release by more than its Δ blocks it unless the release notes explain it. [Release, versioning and documentation policy](https://github.com/Jbmanllr/automan/issues/100) adopts this rule.
