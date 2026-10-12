# Filter lists block ads and trackers by default, from Clean-up rules that each run pins

Every browser Session blocks ads and trackers by default, in every Access mode. Brave's `adblock-rust` matches each request inside the interception ADR 0014 already runs, and a never-block list keeps challenge, anti-bot, anti-fraud and payment scripts loading whatever the Filter lists say. The same rules remove ads, Consent prompt leftovers and page annoyances from extracted content in every tier. On the live page they hide those elements through a style sheet the site cannot see on Chromium, and only on demand on Firefox and WebKit, where any hiding is visible. The rules come as **Clean-up rules**:
- a permissively licensed part, embedded precompiled in the binary;
- the full set, a digest-pinned download refreshed in every release;
- an opt-in, signed daily channel.

Each run holds to one version of them for its whole life and names it in its results. Decided in [Built-in page clean-up](https://github.com/Jbmanllr/automan/issues/119) (parity row SA-17); Consent prompts are handled by ADR 0048.

Competitors split on the default:
- **On by default.** Firecrawl's `blockAds` ([API](https://docs.firecrawl.dev/api-reference/endpoint/scrape)), spider.cloud's `block_ads` and `block_analytics` in browser mode ([API](https://spider.cloud/docs/api)), Steel's `blockAds` ([requests.ts](https://github.com/steel-dev/steel-browser/blob/main/api/src/utils/requests.ts)), and Browser Use, which loads uBlock Origin Lite ([profile.py](https://github.com/browser-use/browser-use/blob/main/browser_use/browser/profile.py)).
- **Off.** The Spider crate's `block_ads` behind its `adblock` feature ([chrome_common.rs](https://github.com/spider-rs/spider/blob/main/spider/src/features/chrome_common.rs)), Browserbase's `blockAds` ([API](https://docs.browserbase.com/reference/api/create-a-session)), Crawl4AI's `avoid_ads`, and ScrapingBee's `block_ads`.
- **No ad blocking at all.** Crawlee, whose `blockRequests` blocks URL substrings through CDP and does nothing on Firefox or WebKit in its JavaScript version ([docs](https://crawlee.dev/js/api/playwright-crawler/namespace/playwrightUtils)).

The real hazard is that a tracker list blocks the scripts that bot protection needs. EasyPrivacy blocks DataDome, PerimeterX, Akamai's `/akam/` paths, iovation, ThreatMetrix and Forter ([EasyPrivacy](https://easylist.to/easylist/easyprivacy.txt)). Cloudflare documents that ad blockers may block the scripts Turnstile needs, which makes challenges fail ([Turnstile troubleshooting](https://developers.cloudflare.com/turnstile/troubleshooting/challenge-solve-issues/)). uBlock Origin's unbreak list re-allows Turnstile, hCaptcha and reCAPTCHA ([unbreak.txt](https://github.com/uBlockOrigin/uAssets/blob/master/filters/unbreak.txt)), and Spider leaves DataDome and reCAPTCHA unblocked on purpose ([scripts.rs](https://github.com/spider-rs/spider_network_blocker/blob/main/src/scripts.rs)). About 29.5% of internet users run a blocker ([GWI via Backlinko](https://backlinko.com/ad-blockers-users)), so blocking is consistent with `standard`'s ordinary browser (ADR 0021).

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
- **The library.** `adblock` (adblock-rust, MPL-2.0, Brave), unmodified ([repository](https://github.com/brave/adblock-rust)). It handles network rules, cosmetic rules, redirects and uBlock Origin syntax. Brave measured 5.7 µs per request ([blog](https://brave.com/blog/improved-ad-blocker-performance/)), and its FlatBuffers rewrite cut the memory of its default lists by 75%, about 45 MB saved ([blog](https://brave.com/privacy-updates/36-adblock-memory-reduction)).
- **Changes ADR 0018:** the licence allowlist accepts MPL-2.0 for unmodified files, with the notice saying where their source is.
- **Where it runs.**
  - On Chromium, matching runs inside ADR 0014's interception. On Firefox and WebKit, it runs inside Playwright's routing.
  - It covers every request type that interception covers, service workers included.
  - A blocked request fails as `BlockedByClient`, the error a real blocker produces.
- **Redirect rules.** These serve harmless stand-ins through the same interception, so a page calling a blocked script does not break.
  - The stand-ins are DuckDuckGo's tracker surrogates (Apache-2.0, with MPL-2.0 parts; [repository](https://github.com/duckduckgo/tracker-surrogates)), covering Google Analytics, Tag Manager and Publisher Tag, AdSense, Facebook's SDK, Criteo and others, and Mozilla's web-compatibility shims (MPL-2.0; [source](https://searchfox.org/mozilla-central/source/browser/extensions/webcompat/shims)).
  - Lists name stand-ins by uBlock Origin's resource names, so Goodfetch keeps an MIT alias table from those names to these files.
  - A redirect rule whose stand-in is missing simply blocks, as `adblock-rust` does when no resource matches ([blocker.rs](https://github.com/brave/adblock-rust/blob/master/src/blocker.rs)).
  - Brave's own `adblock-resources` holds only Brave-specific fixes; Brave assembles its stand-ins and scriptlets from uBlock Origin's GPL-3.0 files ([brave-core-crx-packager](https://github.com/brave/brave-core-crx-packager)).
- **Memory.** One engine loads each version of the Clean-up rules once, shares it between the runs that hold to it, and unloads it when none does. `adblock` is built without its default `single-thread` feature, which makes `Engine` neither `Send` nor `Sync`, so one loaded version serves every task of the engine.

**Defaults and the never-block list.**
- **Settings, each settable per run (ADR 0028).**
  - `filter_lists.ads`, `filter_lists.trackers` and `filter_lists.annoyances` are on by default in every Access mode. In browser Sessions they block requests and hide elements; in every tier, HTTP included, they remove elements from extracted content.
  - `filter_lists.first_party_scripts` allows a site's own scripts, as Spider's `allow_first_party_javascript` does.
  - `filter_lists.extra` adds the user's own lists, as files or URLs. A URL is a configured destination (ADR 0043). As ADR 0017 rules for Plugin endpoints, it may use a private range only when set in Operator policy or user and project configuration, or by a Recipe or run value that Operator policy allows; otherwise it is checked as an address a page can reach. The lists it adds are part of the version a run holds to (below).
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
- **Changes ADR 0007:** before dom-smoothie, htmd or any AI input, the engine removes from the page's HTML the elements the cosmetic rules match. It walks shadow roots and does this in every tier, so hidden text never leaks into output, as it did in [jina-ai/reader#1138](https://github.com/jina-ai/reader/issues/1138). Raw HTML, WARC, Run traces, the Frontier's link reading (ADR 0007's lol-html pass) and the user's own selectors see the page untouched. On a route whose browser sample showed a consent wall, nothing is removed and the page is `consent_required` in every tier (ADR 0048).
- **Live page, `live_hiding` (`auto` by default).**
  - **On Chromium**, `auto` hides matched elements through `CSS.createStyleSheet`, in every frame and every new document. That call builds a style sheet with no DOM node ([style_engine.cc](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/css/style_engine.cc)). `document.styleSheets` is built only from DOM nodes, so the site cannot see the sheet ([style_sheet_collection.cc](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/css/style_sheet_collection.cc)). Computed styles still show the hiding, as they do for any person using a blocker.
  - **Shadow trees.** These sheets never reach them, so Consent prompts there are handled by ADR 0048's runner.
  - **Cost.** Enabling the CSS domain has a cost, which the benchmark measures. `live_hiding = off` removes it, and Provenance records it.
  - **On Firefox and WebKit,** Playwright offers only `addStyleTag`, a visible `<style>` element. There `auto` hides only on demand: when the guard (ADR 0019) finds an action's target covered by an element a rule matches. `live_hiding = always` hides everywhere, with a warning that the site can see it.
  - **What else sees the hiding.** The guard refuses an action whose target live hiding hid, naming the rule; `live_hiding = off` lifts it. ADR 0035's trap-link rule judges visibility as the page renders without Goodfetch's own sheet, so a link inside a hidden ad is not taken for a trap.
- **Never hidden.** A paywall is never hidden or removed, whatever a list says, and neither is a consent wall (ADR 0048).
  - **Detecting a paywall.** Rules written like ADR 0035's block detection recognise the overlays of paywall vendors (Piano, Poool, Zephr and others), and a page that marks paid sections with schema.org's `isAccessibleForFree: false`, the markup Google asks publishers to use ([Google](https://developers.google.com/search/docs/appearance/structured-data/paywalled-content)), declares its own.
  - **What follows.** Their elements stay on the live page and in extracted content, and Provenance records the paywall.
  - **Changes ADR 0045:** these rules join its paywall detection. Only a 402 or a paywall redirect is `payment_required`; an overlay leaves the page fetched as the site served it.

**Scriptlets and resource types.**
- **Storage scriptlets** (`set-cookie`, `set-local-storage-item`, `set-session-storage-item`) are done by the engine before the page's own scripts run: cookies through the browser driver's cookie calls, storage items from an isolated world as the document starts, on all three browsers.
- **Click scriptlets** (`trusted-click-element`) go through the guard's trusted input.
- **Scriptlets that patch the page's JavaScript** (`abort-on-property-read`, `json-prune`, `set-constant` and the like) need the main world.
  - They are off by default.
  - No permissive implementation exists: uBlock Origin's and AdGuard's are GPL-3.0. They come from uBlock Origin's resources, downloaded with the GPL lists and never embedded or compiled into the binary.
  - `filter_lists.page_scriptlets` turns them on, with the warning of ADR 0035's `js-injection` Fingerprint method.
- **`block_resources`** blocks whole types (`image`, `media`, `font`, `stylesheet`). It is empty by default and works through the same interception. In `standard` and `stealth` it warns that the site can see the missing loads, since few people block images or fonts. Finer blocking stays a Hook (ADR 0033).

**What the Clean-up rules hold.**

| Content | Licence | Embedded in the binary |
| --- | --- | --- |
| EasyList, EasyPrivacy, EasyList Cookie List and Fanboy Annoyance, pinned from easylist.to | GPLv3+ or CC BY-SA 3.0+, credited to "The EasyList authors" ([licence](https://easylist.to/pages/licence.html)); fanboy.co.nz's copies say CC BY 3.0, and the inventory records the stricter CC BY-SA | yes, under CC BY-SA |
| Brave's unbreak list | MPL-2.0 ([adblock-lists](https://github.com/brave/adblock-lists)) | yes |
| DuckDuckGo's tracker surrogates, Mozilla's web-compatibility shims | Apache-2.0 and MPL-2.0 | yes |
| autoconsent and its rules (ADR 0048) | MPL-2.0 | yes |
| The never-block list | MIT | yes |
| uBlock filters, privacy, unbreak and quick-fixes | GPLv3 ([uAssets](https://github.com/uBlockOrigin/uAssets)) | no, download only |
| AdGuard's Cookie Notices, Popups, Mobile App Banners and Widgets | GPLv3 ([AdguardFilters](https://github.com/AdguardTeam/AdguardFilters)) | no, download only |
| uBlock Origin's resources, for opt-in patching scriptlets | GPLv3 ([uBlock](https://github.com/gorhill/uBlock)) | no, download only |

**How they arrive.**
- **Embedded.**
  - The embedded part is precompiled at build time with `Engine::serialize` from the binary's own `adblock` version. Serialized, the embedded lists measured 9.06 MB, or 3.67 MB compressed, with `adblock` 0.13.3. autoconsent's rules and library add about 0.1 MB compressed (measured), and the stand-ins are small.
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
    - A nightly CI job builds it from the upstream lists, the rules of autoconsent's latest release that the library Goodfetch shipped can read, and the never-block list.
    - It carries rule data only. autoconsent's library, its main-world snippets and the stand-ins are code, which changes only with Goodfetch releases and their gates (ADR 0030).
    - It publishes only after the never-block, breakage and Consent prompt suites pass, on owned fixtures and on the online Proving ground.
    - Online, each page is judged against the same page with blocking off, so a vendor's own scoring change, which moves both, never holds the channel back; this differs from the release conformance against real vendors, which only reports (ADR 0043).
    - A failed gate publishes nothing and alerts the maintainers.
  - **Published.** It is an OCI artifact in the registry Goodfetch already publishes to, pulled by the Plugin store's code (ADR 0033).
  - **Signed.** It is signed with Sigstore to the workflow's identity and checked with `sigstore-verify` against the bundled trust root, as the Plugin index is.
  - **Versioned.** It carries a version and a 14-day expiry. When choosing the version a new run starts with, the engine refuses a set older than the last one it saw; a resumed run still uses the version it recorded (below). When the newest set it holds is past its expiry, it falls back, with a warning, to the release set or the embedded part, whichever is newer.
  - **Fetched.** Once a day per user, as a conditional manifest check (an OCI pull's anonymous token and manifest requests, with blobs fetched only when changed), with no identifier and no cookie, and never when `offline`. Operator policy can lock `clean_up.update`.
  - **Changes ADR 0043:** setting `daily` makes the channel a configured destination.
- **Changes ADRs 0043 and 0032:** the nightly job that builds the channel joins the CI jobs allowed to reach named endpoints: the upstream list URLs, autoconsent's package, the online Proving ground, and the hosts from which the challenges of the vendors protecting it, and the consent vendors it uses, load. Publishing the channel is a publishing job, outside the deny.
- **Changes ADR 0030:** every patch may refresh the Clean-up rules. What they block, hide or remove is outside semver, as stealth material is, and the release notes name the version. The Clean-up rules' manifest is a contract format with its own `major.minor`. The channel publishes a set for every format major that a supported product major reads. As with the Response cache, an engine skips a set whose minor it cannot read and uses the newest one it can.

**One version per run.**
- **Choosing.** A run uses the newest valid version available: the daily channel's, then the downloaded release set, then the embedded part. `clean_up.version` pins an exact digest instead. Under `upstream`, the lists fetched from their authors are cached by the digest the engine computes, and that digest is what the run holds to.
- **Changes ADR 0002:** a run records that version's digest in its state at start and holds to it until it ends. A resumed run uses the same digest, fetched as a pinned download if the machine lacks it. When that fails, it uses the newest version available and Provenance records the change.
- **Named in the results.** Provenance names the version and its date. The run report warns when it is older than 14 days.
- **Changes ADR 0001:** Goodfetch Cloud, which runs one engine per run, chooses the version and hands its digest to the engine through the protocol, as it hands over saved identities (ADR 0035); the daily check and the anti-rollback state are the cloud's. A local run reproduces it by setting `clean_up.version` to that digest.

**Where it runs.**
- **Browser host** (ADR 0022). Matching and live hiding run in the Browser host beside the browser, with the digest of the version the run holds to.
- **Raw CDP** (ADR 0017). A raw CDP client that switches the interception off also switches blocking off for that Session, and Provenance says so; live hiding and extraction-side removal continue.
- **Third-party CDP browsers** (ADR 0022) are treated as Chromium when the engine controls their interception, and get extraction-side removal only otherwise.
- **Lightpanda** (ADR 0020) gets extraction-side removal, and blocking where its CDP supports request interception. Provenance records which applied.
- **Changes ADR 0036:** change tracking says when a run and its Baseline used different Clean-up rules, so a difference the rules caused is not reported as a change on the site.

**Proof.**
- **The conformance suite** runs on the Proving ground (ADR 0025), on Chromium, Firefox and WebKit, with cross-origin iframes and shadow DOM. It covers:
  - ad and tracker pages with a known blocked set;
  - challenge and anti-bot pages that must keep working;
  - pages relying on redirect stand-ins;
  - pages whose hidden text must not reach extracted output.
- **Changes ADR 0025:** the matched Goodfetch rows set blocking as each competitor's default does. The benchmark publishes what the clean-up costs and saves in time and bytes, as it does for ADR 0023's controls.
