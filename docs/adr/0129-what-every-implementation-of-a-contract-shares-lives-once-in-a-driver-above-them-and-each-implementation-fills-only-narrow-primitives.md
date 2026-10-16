# What every implementation of a contract shares lives once in a driver above them, and each implementation fills only narrow primitives

Where several built-in implementations of one contract sit side by side (the fetch tiers, the Sinks, the Model providers, a local browser and a Browser host), behaviour common to all of them lives once, in a concrete **driver** type above them: redirects, retries, cookies, the typed failures and what each leads to, validation and timeouts. Each implementation provides only a narrow trait of **primitives**, the steps that differ between implementations. Review holds the rule, and the conformance suites already decided keep the implementations' behaviour equal. Each area spec names its drivers and their primitives.

Tests already made implementations agree: every contract with several implementations has a conformance suite as its interface test, Sinks first (ADR 0026), then every Plugin kind, fetch backends included (ADR 0033), and `goodfetch-conformance` for engines (ADR 0069). A crawl handler gets one context and Collie's page on every fetch tier (ADR 0093), defaults and timeouts live in the engine rather than in each SDK (ADR 0080), and a trait exists only where two implementations do (`STYLE.md`). Nothing said that the code itself is shared. The [competitor codebase design research](https://github.com/goodfetch/goodfetch/blob/research/competitor-codebase-design/rnd/research/reports/Competitor%20codebase%20design.md) ([#555](https://github.com/goodfetch/goodfetch/pull/555)) found every project copying along this axis, and the copies drifting:
- **Crawlee JS's** Playwright and Puppeteer helpers are 54% to 82% identical, and `infiniteScroll` already stops at `>=` in one and `>` in the other; only its browser path checks `navigationTimeoutSecs > 0`.
- **Crawlee Python's** three HTTP clients each re-implement redirects and cookies, which its maintainers plan to move into a shared base with a per-hop primitive (apify/crawlee-python#2307), and one warning block is copied seven times across four storage implementations.
- **Browser Use's** 17 model providers each re-map vendor errors, and output-truncation detection exists in 3 of them.
- **Spider's** crawl loops are 47% to 70% identical across HTTP, Chrome and WebDriver, and users report its Chrome and HTTP paths behaving differently.
- **Both halves are needed.** Crawlee Python runs one suite over its storage implementations and Playwright one suite across its browsers, and both still carry copies.
- **Crawlee JS shows the fix.** With the shared steps in a template-method base, each of its HTTP clients is 48 to 83 lines.
- Duplication ties with errors as the field's lowest-scored dimension, a mean of 2.1 of 5.

Decided in [Parallel implementations of one contract have no rule that their shared logic is written once above them](https://github.com/goodfetch/goodfetch/issues/574) ([grilling record](https://github.com/goodfetch/goodfetch/issues/574#issuecomment-6214831034)). The owner decided the rules; the agent decided their details on the owner's delegation of 2026-10-16. One detail departs from the grilling record, which listed error translation among the shared behaviour: the driver owns the failure enum and what each variant leads to, and reading a party's text stays with each implementation, which reaches its own library or format (ADR 0127). No SDK API changes, so ADR 0077's principles do not apply.

## Decision

### A driver above, primitives below

- **The rule.** When two or more built-in implementations of one contract exist, any behaviour they all must have is written once, in a driver, and an implementation holds only what differs. Redirects, retries and their backoff, cookies, the enum of typed failures and what each variant leads to, validation of settings and inputs, timeouts and cancellation are such behaviour wherever they apply. Turning a party's own text into that enum stays with each implementation, its translation point (ADR 0127), since each reaches a different library or format.
- **The driver is a concrete type** that owns that behaviour and calls the implementation through a narrow trait of primitives, such as one `send` per hop for a fetch tier. The trait is the seam `STYLE.md` allows where two implementations exist; the driver holds it behind a newtype of our own if it needs `dyn`.
- **A primitive does one step.** A wide trait that each implementation fills in whole, re-implementing the shared steps, is the shape this rule refuses.
- **Process Plugins and SDK handlers** are implementations too. For them, the driver is the engine's side of the plugin protocol (ADR 0033): the shared behaviour runs in the engine, and the Plugin answers only for its primitives.

### Where it applies

- **Fetch tiers** (area 5, ADRs 0020 and 0118): the tiers that send HTTP themselves share one fetch driver for redirects, retries, cookies, decoding (ADR 0119) and the transport-failure enum; each provides its per-hop request and maps its library's failures into that enum. On a browser tier the browser follows redirects and keeps cookies itself, so those tiers go through the Session driver and the CDP client; area 5's spec decides what else of the fetch driver they share.
- **Sinks** (area 6, ADR 0026): one delivery driver for retries, the delivery ID and the run epoch (ADR 0008) and the delivery-failure enum; each Sink provides its write and maps its service's failures into that enum.
- **The other built-in Plugin kinds** (area 7, ADR 0033): a kind with two or more built-in implementations gets a driver, which its area spec names.
- **Model providers** (area 9, ADRs 0012 and 0101): ADR 0012 already puts JSON repair and the bounded retry above the formats; Usage, the deadline and the model-failure enum join them, and each adapter provides its request, its response mapping and the mapping of its provider's failures into that enum.
- **A local browser and a Browser host** (areas 3 and 5, ADRs 0022 and 0096): one Session driver; each provides its connection.
- **Each area spec names its drivers and their primitives.** A spec that adds a second implementation of a contract adds its driver in the same change.

### How it is checked

- **Review** checks that a new implementation fills primitives and adds no shared behaviour of its own.
- **The conformance suites** (ADRs 0026, 0033 and 0069) check that every implementation behaves the same.

## Considered Options

- **Conformance suites alone:** rejected. Crawlee Python and Playwright run such suites and still carry copies.
- **A code-similarity check across sibling implementations:** rejected. It is noisy, and code that looks alike across implementations is sometimes right.
- **A base trait with default methods** that implementations may override: rejected. Any implementation can override a shared step and drift; a driver it cannot reach keeps the shared step shared.
- **Shared helper functions each implementation calls:** rejected. Each implementation decides the order and whether to call them, which is where Crawlee's copies diverged.

## Consequences

- **`STYLE.md`.** Design states the rule beside "a trait only where two adapters exist".
- **ADR 0037.** Areas 3 (Engine runtime), 5 (Site access), 6 (Collection), 7 (Plugin protocol) and 9 (AI and agent) list this ADR. Each spec names its drivers and primitives.
