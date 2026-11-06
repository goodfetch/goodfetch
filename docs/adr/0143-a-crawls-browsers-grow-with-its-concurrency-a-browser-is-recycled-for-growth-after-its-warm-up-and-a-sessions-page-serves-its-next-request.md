# A crawl's browsers grow with its concurrency, a browser is recycled for growth after its warm-up, and a Session's page serves its next request

A crawl's Session pool now opens a Session only when every open Session is busy. Its browser contexts therefore follow the requests in flight rather than `session.pool.max_size`, and its browsers follow its contexts. A Session's page serves the Session's next request instead of closing, so a page target is set up once for many requests. A browser is recycled for memory only when its footprint grows after a warm-up that lasts until it has reached its full load. Browser memory is read once per governor sample, and the Browser cache is digested once per build and engine process. Every Session keeps a browser context of its own, so no isolation changes. Each change is a setting, and its off state restores today's behaviour.

On 2026-11-06, in `just compare`'s headless browser scenario (1,000 pages, 16 at a time, spec #1577's branch at cc72b0f93), Foxhound took 86.5 s, scrapy-playwright 33.8 s and Spider 35.0 s. Agent `gap-browser`'s measurements on the Hetzner server (`.claude/handoff/wiki/research-browser-gap-20261106.md`, raw data in `/g/gap-browser/`) found that the pages in flight and the wait condition were the same as the other tools'. The gap is CPU: Foxhound spends 795 CPU-s per 1,000 pages against scrapy-playwright's 164, and keeps 12.4 cores busy. The CPU goes to:

- **Contexts and browsers.** The Session pool opens a new Session while it has room (ADR 0006's #920 note), so 100 pages make 100 Sessions. Those are 100 browser contexts in 13 browsers, with 2.6 renderers per page. One Session gives 1.05 renderers per page, and the browsers' CPU falls from 268 to 98 CPU-s.
- **Recycling.** ADR 0040's cap is twice the footprint read at a browser's second page, while the browser still has its contexts and pages to fill. It recycled healthy browsers: 43 launches per crawl instead of 13. Off, the crawl took 64.3 s instead of 82.7 s (−22%).
- **Per-launch and per-page set-up.** Each launch digests the 403 MB Chrome folder. Each page creates a target and sends 11 commands before its navigation, among them the 583 KB Consent prompt runner bundle. Each page opened also walks the browser's processes for memory.

scrapy-playwright and Spider run one browser and one context for the whole crawl and never recycle. The owner's answer to these findings (2026-11-06, verbatim): « mesurer quoi? on va changer. ». The design changes, and Foxhound's browser crawl must match those two tools or beat them, with every feature a setting that has a free off state.

## Decision

**A Session pool grows on demand.** `session.pool.growth` takes `on_demand`, the default, or `eager`, which is ADR 0006's #920 rule and Crawlee's.
- **On demand,** a request goes to an open Session at its domain's proxy tier that is serving fewer than `session.pool.session_concurrency` requests (1 by default). It is drawn from the Run's `Entropy` among those Sessions.
- **A new Session opens** only when no open Session at the tier has room and fewer than `session.pool.max_size` are open. A full pool draws among its busy Sessions as today, and retires one (`pool_full`) only when none is open at the tier.
- **Unchanged:** rotation (`max_uses`, `max_age`, the error score), escalation by new Session (ADR 0021), which opens its Session under either rule, and a Label's own pool (ADR 0093), which follows the same rule.
- **What follows:** the open Sessions follow the peak number of requests in flight, so at 16 requests at once a crawl opens about 16 browser contexts in two browsers (`governor.browser.contexts_per_process` stays 8).

**A Session keeps a browser context of its own.** Two Sessions never share a context. Cookies, storage and the proxy (`Target.createBrowserContext`'s `proxyServer`) are per context, so concurrent pages of two Sessions in one context would mix identities, which ADRs 0006, 0021 and 0040 forbid. The pool follows the crawl's concurrency by keeping fewer Sessions open, not by putting more Sessions in a context.

**A Session's page serves its next request.** `crawl.browser.page_reuse` is on by default.
- **Reuse.** When a browser-tier attempt ends, its page goes back to its Session if it is in a healthy state. The Session's next browser-tier request then navigates that page instead of creating a target.
- **A healthy state** means its renderer did not crash and it holds no Human pause, Live view or raw CDP hold. It has no dialog open and no popup still open, its Session is not retired, and its browser is not being recycled. Any other page closes as today.
- **The handler's handle ends when the handler returns,** and a later call on it fails as on a closed page.
- **The page stays the Session's,** which is the Glossary's Page: the history, `sessionStorage` and window state it keeps belong to that one identity, as in a person's tab.
- **A spare page closes** when its Session retires or its browser is recycled, under memory pressure, or after 30 s with no request.
- **What it saves:** the target, its enables and `Target.setAutoAttach`, the guard marker and the Consent prompt runner bundle (`Page.addScriptToEvaluateOnNewDocument`) are set up once per page, not once per request. At the default `session_concurrency` of 1, that is once per browser context.
- **Unchanged:** the runner still starts in every document, as ADR 0048 requires. An out-of-process iframe's target still gets the bundle when it attaches.
- **Off,** each request opens a new page, as today.

**A browser is recycled for growth after its warm-up.**
- **The warm-up size** is the highest footprint the governor's samples read for the browser. The window runs until it has opened `governor.browser.warm_up_pages` pages (32 by default, twice the default concurrency) since it last held more open pages than ever before. Filling up its contexts and pages therefore extends the warm-up instead of ending it.
- **Recycled for memory.** After its warm-up, a browser is recycled when a sample reads its footprint above `governor.browser.memory_growth` percent of its warm-up size (200 by default, ADR 0040's "twice"). Unsetting it turns growth recycling off.
- **`governor.browser.memory_cap`** is now only an absolute cap, unset by default, and is judged at every sample, warm-up included. It no longer means "twice the warm-up size" when unset.
- **`governor.browser.recycle_pages`** becomes unset by default instead of 100. A page count recycles a browser only when it is set, and with page reuse it counts the navigations the crawl sent in the browser's pages. This drops ADR 0040's default of recycling every 100 pages. Its reason was Crawlee's guard against leaks, and the growth rule catches leaks wherever they show as memory.
- **Memory is the sampler's.** The pool reads the governor sampler's last measure of each browser (ADR 0140's E4: 0 memory walks per page opened) and judges at each sample, not at each page opening.
- **Unchanged:** idle recycling, held browsers, the move of a Session's cookies and storage on a recycle, the critical shedding and the per-page cap. During a heavy single-page app's warm-up, those and an absolute `memory_cap` still bound it.

**The Browser cache is digested once per build and engine process.**
- **The first launch or install** digests the build's files, as ADR 0024 requires, and records each file's path, size, modification time and file identity (inode and device, or the Windows file ID).
- **A later launch in the same engine process** compares that record. A difference digests the build again, and a mismatch then fails as today.
- **What it gives up:** a change that keeps every file's size, time and identity goes unseen until the next engine process. The digest guards against a corrupt or partial build. A program that can write to the user's Browser cache can already change anything the engine runs, so the digest is no defence against it.

## Considered Options

- **Measuring the current design further:** rejected by the owner.
- **One browser context for every Session, with cookies swapped per page** (Crawlee's shared-context pages): rejected. Concurrent pages of different Sessions would share cookies, storage and the proxy.
- **One browser per crawl** (scrapy-playwright, Spider): rejected as the default. At Foxhound's 57 CDP commands per page, one browser took 197 s, because its browser process answered each command in 40–290 ms. `governor.browser.contexts_per_process` still allows it.
- **Raising `contexts_per_process` alone:** rejected. It gives fewer browsers but still 100 contexts and 2.6 renderers per page.
- **The Consent prompt runner as a Chrome extension's content script** (CDP `Extensions.loadUnpacked`), loaded once per browser: rejected. It needs `--enable-unsafe-extension-debugging`, a page can detect an extension, and Firefox and WebKit have no such path. It would also move the runner out of the guard's isolated world (ADR 0048).
- **A warm-up read at a fixed page count:** rejected. A browser the governor fills later, as its targets rise, would still be recycled for normal growth.
- **Footprint per open page:** rejected. At the end of a crawl, a browser with one page left reads high per page and would be recycled.
- **Keeping `recycle_pages` at 100:** rejected as the default. With two browsers per crawl it means about ten launches and their Session moves per 1,000 pages, a cost that neither scrapy-playwright nor Spider pays. It stays a setting.

## Consequences

- **Amends ADR 0006** (#920's note): the pool's growth rule above replaces "a request goes to a new Session while fewer than `session.pool.max_size` are open" by default. `eager` keeps it.
- **Amends ADR 0040:**
  - the recycling defaults and the warm-up above, for its Recycling bullet and #235's note ("When", and the key defaults);
  - pages reused by their Session, for "a page finishes when it closes";
  - fewer, busier browsers per Run, with Sessions and contexts kept one to one.
- **Amends ADR 0140:**
  - **E4's unit "one page opened in the browser pool"** also counts 0 Browser cache digests per launch after a build's first in an engine process, at most 1 browser context created per Session opened, and at most 1 Consent prompt runner registration per page target created (0 on a reused page).
  - **B2 gains a pool cell.** The pool launches through a scripted launcher over the in-memory CDP end (ADR 0059's values), so that `E4.pool_walks` and these rows are reached. Today's Borrowed browser skips the pool (X17).
- **ADR 0024's digest, ADR 0048's runner on every document, ADR 0021's identity and escalation, and ADR 0035's process per per-process Fingerprint method** are unchanged.
- **New settings:** `session.pool.growth`, `session.pool.session_concurrency`, `crawl.browser.page_reuse`, `governor.browser.warm_up_pages` and `governor.browser.memory_growth`. All are declared in `goodfetch-protocol`, the first three in its Foxhound module. Foxhound reads `crawl.browser.page_reuse` and tells the page crate through its `BrowserPages` interface, so no crate outside `crates/foxhound/` depends on one inside it (ADR 0087).
- **Changed defaults,** said here as the owner asked: `governor.browser.recycle_pages` 100 becomes unset, `governor.browser.memory_cap` unset no longer means twice the warm-up size, and the pool's growth is on demand.
- **Fewer identities at once.** On demand, a crawl with a rotating proxy uses about as many sticky exits at once as it has requests in flight, rather than up to `max_size`. `eager` restores the wider spread.
