# Nothing leaves the machine unless configured

Goodfetch sends nothing about its user, their runs or their machine anywhere except to destinations a run, Recipe, user or operator configured. There is no product telemetry, not even opt-in, and no setting can turn it on. The only connections nobody configured are downloads of artifacts pinned by digest, which carry nothing about the user, and an `offline` setting turns them off. The rule covers the binary, the SDKs, the CLI, the MCP server, the Console, Plugins and the browsers Goodfetch launches, and CI enforces it. Before this decision only the Console (ADR 0031) and version checks (ADR 0030) promised it (parity row SC-10). Decided in [Network defaults](https://github.com/Jbmanllr/automan/issues/118).

Goodfetch's users send it the URLs, accounts and tasks they care most about, so telemetry costs more trust here than elsewhere. Browser Use sends PostHog events by default, tied to a device ID persisted on disk or derived from the MAC address and host name, and its docs now say they may include tasks, visited URLs and results ([service.py](https://github.com/browser-use/browser-use/blob/main/browser_use/telemetry/service.py), [telemetry docs](https://docs.browser-use.com/open-source/development/monitoring/telemetry)). Its users report the gap between "anonymous usage data" and what is sent ([browser-use#5216](https://github.com/browser-use/browser-use/issues/5216)) and an empty `ANONYMIZED_TELEMETRY=` turning it on ([browser-use#5837](https://github.com/browser-use/browser-use/issues/5837)). Stagehand's CLI sends events unless `DO_NOT_TRACK`, `BROWSERBASE_TELEMETRY_DISABLED` or `CI` is set, and its SDKs send none ([telemetry.ts](https://github.com/browserbase/stagehand/blob/main/packages/cli/src/lib/telemetry.ts)). Crawlee, Playwright, Scrapy and Spider have no telemetry; self-hosted Firecrawl sends events only with a configured PostHog key.

## Considered Options

- **Opt-out telemetry** (the Next.js and Homebrew model): rejected. It contradicts what Goodfetch's users expect, and Browser Use shows what it does to trust.
- **Opt-in telemetry**: rejected. Few users enable it, so it gives little data, and it is still code that must be maintained and audited on every release.
- **Honouring `DO_NOT_TRACK`** (Stagehand's CLI): unnecessary, since nothing is sent to switch off.
- **Leaving browsers' background traffic on**: rejected. Chrome's component updates, Safe Browsing and field-trial fetches are calls to Google from the user's machine that nobody configured.

## Consequences

**The rule.**
- **Changes ADR 0016:** this rule joins the controls that cannot be switched off.
- Configured destinations are those set in a run, a Recipe, user or project configuration, or Operator policy: targets and the pages they lead to, proxies, Sinks, webhooks and Notifications, Model providers, Secret providers, OTLP export, the Plugin index and registries, and payment endpoints (ADR 0045).
- **Changes ADRs 0024 and 0033:** unconfigured connections are limited to downloads of artifacts pinned by digest: browsers on first use (ADR 0024), Lightpanda when its tier is chosen (ADR 0020), browsers a Plugin declares and Plugin language runtimes (ADR 0033), and Cursory's recordings (ADR 0035). They are plain HTTPS GETs to the pinned URL with no cookie, no identifier, no query beyond the pinned path and no user agent naming the install.
- The `offline` setting, which Operator policy can lock, turns each of those downloads into the Error code `artifact_unavailable_offline`, which is not retryable (ADR 0038). They can be supplied beforehand: `goodfetch browsers install` fetches browsers, Lightpanda and Cursory's recordings, `GOODFETCH_BROWSERS_PATH` points at a copy, and `goodfetch plugins add` fetches a Plugin's runtime along with its artifact.
- The rule binds Goodfetch's own code. A third-party Plugin's manifest domains and network level are approved by a person when it is installed or updated (ADR 0033), which makes them configured. At the `direct` level the engine does not see the Plugin's connections, and approving `direct` is what grants that.
- Any future call that is neither configured nor a pinned download needs a decision that changes this ADR.

**Diagnostics.**
- Crash reports and panics are written to a local file and never sent.
- `goodfetch doctor` builds a redacted bug-report file (versions, settings with secrets removed, recent errors) that the user chooses to attach to an issue.

**Browsers.**
- Every browser Goodfetch launches has its background traffic turned off, starting from Playwright's switches ([chromiumSwitches.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/chromium/chromiumSwitches.ts)): `--disable-background-networking`, `--disable-component-update`, `--disable-breakpad`, `--disable-sync`, `--disable-client-side-phishing-detection`, `--metrics-recording-only`, `--disable-field-trial-config`, and the features `OptimizationHints`, `NetworkTimeServiceQuerying`, `Translate` and `MediaRouter`. Firefox and Camoufox get the matching preferences.
- **Changes ADR 0035:** the Goodfetch Chromium build turns this traffic off at build time rather than by switches. It also ships the variations seed and its Google signature, captured when each Stable major is built and loaded through `--variations-test-seed-path` with `--disable-variations-seed-fetch`, so Google sites receive the `X-Client-Data` header a real Chrome sends ([variations_http_headers.cc](https://chromium.googlesource.com/chromium/src/+/main/components/variations/net/variations_http_headers.cc)) with no call at run time. The seed is Google's signed data, so its redistribution gets a licence check before the first public preview and an entry in ADR 0018's notice inventory, like every third-party input of the build. Chrome for Testing is unbranded and never sends the header, as in every Playwright and Puppeteer setup; the docs say so.

**Enforcement.**
- **Changes ADR 0032:** every step that runs Goodfetch's product (the surfaces of this ADR's opening paragraph), in every CI tier and on every operating system it covers, denies outbound traffic except to the fixtures and the pinned artifact mirror, and fails on any other connection. The conformance suite therefore proves the rule wherever it runs, including the nightly installed-Chrome matrix. Settled in [Tighten ADR 0043's CI rule scope](https://github.com/Jbmanllr/automan/issues/129) and [Release promotion, release branches, vendor conformance and docs hosting](https://github.com/Jbmanllr/automan/pull/131), on the gaps the [Repository, CI and release spec](https://github.com/Jbmanllr/automan/issues/130) listed, as the [owner settled them](https://github.com/Jbmanllr/automan/issues/130#issuecomment-6114909646):
  - **Before the deny:** checkout, toolchain and dependency installs, pinned container images and cache restores.
  - **Outside the rule:** jobs that run none of the product, such as publishing, signing, the Plugin index signature, deploying the docs, deploying the online Proving ground and the watchlist job (ADR 0035).
  - **The pinned artifact mirror** is the set of digest-pinned downloads this ADR's rule lists, or a CI cache of them.
  - **A canary** in every job under the deny tries a forbidden connection after the deny starts and must be refused.
- Only these jobs may also reach named endpoints, and only those:
  - the nightly real SaaS sink accounts (ADR 0026);
  - the release-candidate benchmark's online Proving ground behind real vendor protections and the model, proxy, CAPTCHA and hosted-browser services it pays for (ADRs 0025 and 0035);
  - the nightly public testnets that check Paid fetch proofs (ADR 0045);
  - the online conformance of fetch backends, browser backends and Challenge solvers against real vendors (ADR 0033). It runs on every release candidate, on every patch that carries a new Chrome major or new stealth material, and on manual dispatch. It reaches only the online Proving ground, the hosts from which the challenges of the vendors protecting it load, and, for a Challenge solver that uses one, its solving service; the online Proving ground reads the vendors' verdicts server-side. It reports in the release notes and never blocks a release, because a vendor's own scoring change is outside Goodfetch's control and must not hold back a same-day security patch;
  - a fixture's licence endpoint where the fixture needs one, named in its job.
- The Console's Content Security Policy (ADR 0031) and the offline version checks (ADRs 0030 and 0035) are instances of this rule.
- The Clean-up rules join the downloads pinned by digest, their daily channel is a configured destination once `clean_up.update = daily` is set, and the nightly job that builds it joins the CI jobs that reach named endpoints (ADR 0047).
