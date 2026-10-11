# Filter lists block ads and trackers by default, from Clean-up rules that each run pins

Every browser Session blocks ads and trackers by default, in every Access mode. Brave's `adblock-rust` matches each request inside the interception ADR 0014 already runs, and a never-block list keeps challenge, anti-bot, anti-fraud and payment scripts loading whatever the Filter lists say. The same rules remove ads, Consent prompt leftovers and page annoyances from extracted content in every tier. They hide those elements on the live page only where the site cannot see the hiding, which Chromium allows and Playwright's Firefox and WebKit do not. The rules come as **Clean-up rules**:
- a permissively licensed part, embedded precompiled in the binary;
- the full set, a digest-pinned download refreshed in every release;
- an opt-in, signed daily channel.

Each run holds to one version of them for its whole life and names it in its results. Decided in [Built-in page clean-up](https://github.com/Jbmanllr/automan/issues/119) (parity row SA-17); Consent prompts are handled by ADR 0048.

Competitors split on the default:
- **On by default.** Firecrawl's `blockAds` ([API](https://docs.firecrawl.dev/api-reference/endpoint/scrape)), spider.cloud's `block_ads` and `block_analytics` in browser mode ([API](https://spider.cloud/docs/api)), Steel's `blockAds` ([requests.ts](https://github.com/steel-dev/steel-browser/blob/main/api/src/utils/requests.ts)), and Browser Use, which loads uBlock Origin Lite ([profile.py](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/profile.py)).
- **Off.** The Spider crate's `block_ads` behind its `adblock` feature ([chrome_common.rs](https://github.com/spider-rs/spider/blob/main/spider/src/features/chrome_common.rs)), Browserbase's `blockAds` ([API](https://docs.browserbase.com/reference/api/create-a-session)), Crawl4AI's `avoid_ads`, and ScrapingBee's `block_ads`.
- **No ad blocking at all.** Crawlee, whose `blockRequests` blocks URL substrings through CDP and does nothing on Firefox or WebKit in its JavaScript version ([docs](https://crawlee.dev/js/api/playwright-crawler/namespace/playwrightUtils)).

The real hazard is that a tracker list blocks the scripts that bot protection needs. EasyPrivacy blocks DataDome, PerimeterX, Akamai's `/akam/` paths, iovation, ThreatMetrix and Forter ([EasyPrivacy](https://easylist.to/easylist/easyprivacy.txt)). Cloudflare documents that blockers cause challenge loops ([Turnstile troubleshooting](https://developers.cloudflare.com/turnstile/troubleshooting/challenge-solve-issues/)). uBlock Origin's unbreak list re-allows Turnstile, hCaptcha and reCAPTCHA ([unbreak.txt](https://github.com/uBlockOrigin/uAssets/blob/master/filters/unbreak.txt)), and Spider leaves DataDome and reCAPTCHA unblocked on purpose ([scripts.rs](https://github.com/spider-rs/spider_network_blocker/blob/main/src/scripts.rs)). About 29.5% of internet users run a blocker ([GWI via Backlinko](https://backlinko.com/ad-blockers-users)), so blocking is consistent with `standard`'s ordinary browser (ADR 0021).

## Considered Options

- **Off by default** (the Spider crate, Browserbase, Crawl4AI, ScrapingBee): rejected. Ads and trackers cost requests, bytes on proxies billed by volume, and noise in screenshots and AI input. A breakage check (below) handles the pages that need them.
- **A browser extension** (uBlock Origin Lite in Browserless and Browser Use): rejected.
  - It runs on Chromium only and is GPLv3.
  - Browserless documents that it compiles about 69,000 rules on every fresh launch ([launch options](https://docs.browserless.io/baas/launch-options)).
  - Chrome 139's removal of Manifest V2 broke the extensions Browser Use loaded ([browser-use#2724](https://github.com/browser-use/browser-use/issues/2724)).
- **Chromium's own ad filter** (`Page.setAdBlockingEnabled`): rejected. Its rules arrive through Chrome's component updater ([subresource_filter](https://chromium.googlesource.com/chromium/src/+/main/components/subresource_filter/README.md)), which ADR 0043 turns off.
- **URL patterns through `Network.setBlockedURLs`** (Crawlee): rejected as the mechanism. It cannot run filter-list logic, works on Chromium only, and the engine already pauses every request for ADR 0014's policy.
- **Hard-coded domain lists** (Firecrawl's open-source service, Crawl4AI, Steel): rejected. Coverage is thin, and Firecrawl's list blocks the whole of `fbcdn.net` ([api.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/playwright-service-ts/api.ts)).
- **Fetching the lists from their authors at run start** (Ghostery's library, puppeteer-extra, Browser Use): rejected as the default. It is an unpinned call nobody configured (ADR 0043), and launches fail when it fails ([puppeteer-extra#840](https://github.com/berstend/puppeteer-extra/issues/840), [ghostery/adblocker#3994](https://github.com/ghostery/adblocker/issues/3994)). It stays available as `clean_up.update = upstream`.
- **Downloading the lists while building the binary** (Spider's `adblock_easylist`): rejected. Spider's build fetches them unpinned and silently ships empty lists when the download fails ([build.rs](https://github.com/spider-rs/chromey/blob/main/build.rs)), and GPL lists compiled into the binary risk making it "a larger program" under the GPL.
- **The daily channel on by default**: rejected. One request a day from every install would show Goodfetch each install's address, the census ADR 0043 refuses.
- **Hiding on the live page through page scripts or a `<style>` element**: rejected on Chromium, where the site would see it. Firefox and WebKit have no other way, so there it happens only on demand.
- **Differential updates for the daily channel** (uBlock Origin's and AdGuard's): rejected as over-engineering. They serve millions of clients hourly; the channel is one opt-in download a day.
- **Non-commercial lists**: excluded by licence. Disconnect's and DuckDuckGo's lists are CC BY-NC-SA ([Disconnect](https://github.com/disconnectme/disconnect-tracking-protection/blob/master/LICENSE), [DuckDuckGo](https://github.com/duckduckgo/tracker-blocklists)), and Steven Black's merged hosts file includes non-commercial sources ([readme](https://github.com/StevenBlack/hosts)). Peter Lowe's list has no formal licence ([site](https://pgl.yoyo.org/adservers/)) and stays out until it has one.

## Consequences

**Matching.**
- **The library.** `adblock` (adblock-rust, MPL-2.0, Brave), unmodified ([repository](https://github.com/brave/adblock-rust)). It handles network rules, cosmetic rules, redirects and uBlock Origin syntax. Brave measured 5.7 µs per request ([blog](https://brave.com/blog/improved-ad-blocker-performance/)) and about 45 MB on its default lists ([blog](https://brave.com/privacy-updates/36-adblock-memory-reduction)).
- **Changes ADR 0018:** the licence allowlist accepts MPL-2.0 for unmodified files, with the notice saying where their source is.
- **Where it runs.**
  - On Chromium, matching runs inside ADR 0014's interception. On Firefox and WebKit, it runs inside Playwright's routing.
  - It covers every request type that interception covers, service workers included.
  - A blocked request fails as `BlockedByClient`, the error a real blocker produces.
- **Redirect rules.** These serve harmless stand-ins from Brave's `adblock-resources` (MPL-2.0, [repository](https://github.com/brave/adblock-resources)) through the same interception, so a page calling a blocked script does not break.
- **Memory.** One engine loads each version of the Clean-up rules once, shares it between the runs that hold to it, and unloads it when none does.

**Defaults and the never-block list.**
- **Settings, each settable per run (ADR 0028).**
  - `filter_lists.ads`, `filter_lists.trackers` and `filter_lists.annoyances` are on in every browser Session and every Access mode.
  - `filter_lists.first_party_scripts` allows a site's own scripts, as Spider's `allow_first_party_javascript` does.
  - `filter_lists.extra` adds the user's own lists, as files or URLs. A URL is a configured destination (ADR 0043).
- **The never-block list.** Goodfetch maintains it, under MIT. It names:
  - challenge and CAPTCHA providers;
  - anti-bot sensors;
  - anti-fraud and payment scripts.

  It wins over every Filter list, the user's own included, and is tested against the real vendors on the online Proving ground (ADR 0025).
- **Breakage, caught two ways.**
  - **Changes ADR 0020:** the per-route sample comparison also fetches its sample with blocking off. If the extracted results differ beyond what the rules remove, blocking is off for that route, learned as the tier choice is.
  - A page that fails its Page check, or shows an ad-blocker wall, while blocking is on is retried once with blocking off, and that choice is learned per domain, keyed as in ADR 0004.
- **Provenance.** Each page's Provenance records whether blocking was on, the Clean-up rules it used and how many requests were blocked.

**Removing and hiding elements.**
- **Changes ADR 0007:** before dom-smoothie, htmd or any AI input, the engine removes from the page's HTML the elements the cosmetic rules match. It walks shadow roots and does this in every tier, so hidden text never leaks into output, as it did in [jina-ai/reader#1138](https://github.com/jina-ai/reader/issues/1138). Raw HTML, WARC, Run traces and the user's own selectors see the page untouched.
- **Live page, `live_hiding` (`auto` by default).**
  - **On Chromium**, `auto` hides matched elements through `CSS.createStyleSheet`, in every frame and every new document. That call builds a style sheet with no DOM node ([style_engine.cc](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/css/style_engine.cc)). `document.styleSheets` is built only from DOM nodes, so the site cannot see the sheet ([style_sheet_collection.cc](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/css/style_sheet_collection.cc)). Computed styles still show the hiding, as they do for any person using a blocker.
  - **Shadow trees.** These sheets never reach them, so Consent prompts there are handled by ADR 0048's runner.
  - **Cost.** Enabling the CSS domain has a cost, which the benchmark measures. `live_hiding = off` removes it, and Provenance records it.
  - **On Firefox and WebKit,** Playwright offers only `addStyleTag`, a visible `<style>` element. There `auto` hides only on demand: when the guard (ADR 0019) finds an action's target covered by an element a rule matches. `live_hiding = always` hides everywhere, with a warning that the site can see it.
- **Never hidden.** A paywall overlay is never hidden or removed, whatever a list says (ADR 0045), and neither is a consent wall (ADR 0048).

**Scriptlets and resource types.**
- **Storage scriptlets** (`set-cookie`, `set-local-storage-item`, `set-session-storage-item`) are done by the engine through CDP before the page loads.
- **Click scriptlets** (`trusted-click-element`) go through the guard's trusted input.
- **Scriptlets that patch the page's JavaScript** (`abort-on-property-read`, `json-prune`, `set-constant` and the like) need the main world.
  - They are off by default.
  - They come only from Brave's MPL resources, never from uBlock Origin's GPL code.
  - `filter_lists.page_scriptlets` turns them on, with the warning of ADR 0035's `js-injection` Fingerprint method.
- **`block_resources`** blocks whole types (`image`, `media`, `font`, `stylesheet`). It is empty by default and works through the same interception. In `standard` and `stealth` it warns that the site can see the missing loads, since few people block images or fonts. Finer blocking stays a Hook (ADR 0033).

**What the Clean-up rules hold.**

| Content | Licence | Embedded in the binary |
| --- | --- | --- |
| EasyList, EasyPrivacy, EasyList Cookie List | GPLv3+ or CC BY-SA 3.0+, credited to "The EasyList authors" ([licence](https://easylist.to/pages/licence.html)) | yes, under CC BY-SA |
| Fanboy Annoyance | CC BY 3.0 in its header | yes |
| Brave's unbreak list and resources | MPL-2.0 | yes |
| autoconsent and its rules (ADR 0048) | MPL-2.0 | yes |
| The never-block list | MIT | yes |
| uBlock filters, privacy, unbreak and quick-fixes | GPLv3 ([uAssets](https://github.com/uBlockOrigin/uAssets)) | no, download only |
| AdGuard's Cookie Notices, Popups, Mobile App Banners and Widgets | GPLv3 ([AdguardFilters](https://github.com/AdguardTeam/AdguardFilters)) | no, download only |

**How they arrive.**
- **Embedded.**
  - The embedded part, about 2.5 MB compressed, is precompiled at build time with `Engine::serialize` from the binary's own `adblock` version.
  - The serialized format can change between minor versions of the crate ([docs](https://docs.rs/adblock/latest/adblock/struct.Engine.html)), which is why it is built with the binary's own version.
  - It is loaded on the first page that needs it, never at engine start.
- **Changes ADR 0018:** the compiled form of the CC BY-SA lists is an adaptation. It carries CC BY-SA in the notice inventory, as Chromium does for the EasyList-derived rules file in its BSD tree ([README.chromium](https://chromium.googlesource.com/chromium/src/+/main/third_party/subresource-filter-ruleset/README.chromium)). The GPL lists are never compiled by Goodfetch.
- **Downloaded.**
  - The full set is pinned by digest in each release and refreshed in every patch release (ADR 0030).
  - It is downloaded on first use by any run, HTTP-only runs included, into the browser cache.
  - Downloaded sets are compiled once, in a tracked blocking task off the async runtime, and the compiled form is cached by set digest and `adblock` version.
  - **Changes ADR 0024:** `goodfetch browsers install` also fetches the Clean-up rules, and `GOODFETCH_BROWSERS_PATH` covers them.
  - **Changes ADR 0043:** they join the downloads pinned by digest, and `offline` turns their download into `artifact_unavailable_offline` for an explicit `goodfetch rules update`. A run then uses what it has.
- **`clean_up.update`** is one of three values:
  - `release` (default): the set pinned by the release, with the embedded part as fallback;
  - `daily`: the channel below;
  - `upstream`: the Filter lists from their own URLs, at the expiry each declares. Consent prompt rules and the never-block list still come from Goodfetch, and the never-block list still wins.

  `goodfetch rules update` pulls once on demand.
- **The daily channel.**
  - **Built and gated.**
    - A nightly CI job builds it from the upstream lists, autoconsent's latest release and the never-block list.
    - It publishes only after the never-block, breakage and Consent prompt suites pass, on owned fixtures and on the online Proving ground.
    - Online, each page is judged against the same page with blocking off, so a vendor's own scoring change, which moves both, never holds the channel back; this differs from the release conformance against real vendors, which only reports (ADR 0043).
    - A failed gate publishes nothing and alerts the maintainers.
  - **Published.** It is an OCI artifact in the registry Goodfetch already publishes to, pulled by the Plugin store's code (ADR 0033).
  - **Signed.** It is signed with Sigstore to the workflow's identity and checked with `sigstore-verify` against the bundled trust root, as the Plugin index is.
  - **Versioned.** It carries a version and a 14-day expiry. The engine refuses a set older than the last one it saw, and falls back with a warning from a set past its expiry.
  - **Fetched.** Once a day per user, as one conditional request with no identifier and no cookie, and never when `offline`. Operator policy can lock `clean_up.update`.
  - **Changes ADR 0043:** setting `daily` makes the channel a configured destination.
- **Changes ADRs 0043 and 0032:** the nightly job that builds the channel joins the CI jobs allowed to reach named endpoints: the upstream list URLs, autoconsent's package and the online Proving ground. Publishing the channel is a publishing job, outside the deny.
- **Changes ADR 0030:** the Clean-up rules' manifest is a contract format with its own `major.minor`. The channel publishes a set for every format major that a supported product major reads. As with the Response cache, an engine skips a set whose minor it cannot read and uses the newest one it can.

**One version per run.**
- **Choosing.** A run uses the newest valid version available: the daily channel's, then the downloaded release set, then the embedded part.
- **Changes ADR 0002:** a run records that version's digest in its state at start and holds to it until it ends. A resumed run uses the same digest, fetched as a pinned download if the machine lacks it. When that fails, it uses the newest version available and Provenance records the change.
- **Named in the results.** Provenance names the version and its date. The run report warns when it is older than 14 days.
- **Changes ADR 0036:** change tracking says when a run and its Baseline used different Clean-up rules, so a difference the rules caused is not reported as a change on the site.

**Proof.**
- **The conformance suite** runs on the Proving ground (ADR 0025), on Chromium, Firefox and WebKit, with cross-origin iframes and shadow DOM. It covers:
  - ad and tracker pages with a known blocked set;
  - challenge and anti-bot pages that must keep working;
  - pages relying on redirect stand-ins;
  - pages whose hidden text must not reach extracted output.
- **Changes ADR 0025:** the matched Goodfetch rows set blocking as each competitor's default does. The benchmark publishes what the clean-up costs and saves in time and bytes, as it does for ADR 0023's controls.
