# Can the Filter lists block with no pause per request, with the same verdicts? (2026-11-12)

The research of [#2436](https://github.com/goodfetch/goodfetch/issues/2436), asked with the owner's answer to Q4 of the grilling of decision G-05 (2026-11-10), which kept the Filter lists' network blocking on by default ([ADR 0047](../../docs/adr/0047-filter-lists-block-ads-and-trackers-by-default-from-clean-up-rules-each-run-pins.md)'s note of that day). Once [#1895](https://github.com/goodfetch/goodfetch/issues/1895) lands, the lists are what keeps a default Session's requests paused. The question: which way, if any, applies the embedded lists' network rules with no pause per request in the engine, and gives every verdict the engine's match gives today?

**Method.** Read-only, no code, nothing run but a count of the embedded lists' rules (a script over the text files). Sources:
- Chromium at tag `156.0.8078.4`, the Chrome for Testing build Collie pins (`xtask/generated/test_facts.rs:6`), read on chromium.googlesource.com.
- The DevTools Protocol's `.pdl` files at that tag, and Chrome's `declarativeNetRequest` reference.
- `adblock` 0.13.3, the crate the engine matches with (`Cargo.toml`), read from its published crate.
- uBlock Origin at `gorhill/uBlock` `2bc679d` (2026-11-10), uBO Lite's release `2026.1110.1816` (`uBlockOrigin/uBOL-home`), and Brave's `brave-core` for how Brave applies the same matcher.
- Goodfetch's own code, read at `4f9678a04`.

Each claim names its source. **[I]** marks an inference from code that was read but not run. Nothing was measured: the measurement window was on, so every cost below is in commands and work counted from code, not in time.

**Limits.**
- No candidate was built or run. In particular, how fast Chromium matches about 100,000 rules per request in each candidate was not measured.
- The rule counts come from a line classifier written for this note. They are approximate to a few percent, and say how much of each list a candidate can carry, not which verdicts change.

## Answer

**No exact way exists.** None of the four candidates gives every verdict the engine gives today. Each one loses rules.

The closest is **an extension's declarative rules** (`declarativeNetRequest`, DNR), compiled at build time from the same embedded lists by Goodfetch's own converter:
- It can express resource types, third party, exceptions, the never-block list (as allow rules of the highest priority), `$important` (as priority), `$badfilter` (applied when converting) and stand-ins (as redirects to the extension's files).
- It matches in the browser process, with no CDP message per request.
- Provenance can still count each block: the request fails with `net::ERR_BLOCKED_BY_CLIENT`, and `Network.loadingFailed` reports it with `blockedReason: "other"`.

What it loses:
- **Third party is judged differently.** The engine judges third party, `$domain` and the per-domain learning against the request's `Referer`, or else the page's document (`crates/goodfetch-page/src/interception/pipeline.rs:814-824`). DNR judges them against the frame that sent the request.
- **Stand-ins follow other rules.** `adblock` serves a stand-in only when the request is blocked, and lets exceptions apply to it. DNR ranks allow, block and redirect by priority, so the converter has to reproduce that with priorities. It was not checked rule by rule.
- **Not everything is checked.** The 42 regex rules fit under DNR's cap of 1,000, but whether each compiles in DNR's RE2 under its 2 KB limit was not checked.
- **A site can see the stand-ins.** They are served from a `chrome-extension://` URL, which the page can read where today the engine answers on the script's own URL.
- **It does not exist in every browser.** It needs an extension in the browser: a Borrowed browser and an installed branded Chrome cannot load one by switch (Chrome 137 removed `--load-extension` there).
- **A Session's context is off the record.** DNR rules apply there only to an extension enabled in incognito, which only `Extensions.loadUnpacked` with `enableInIncognito: true` sets; `--load-extension` cannot.

The other three are further off:
- **`Network.setBlockedURLs`** matches URLs only: no resource type, no third party, no stand-in. Each renderer scans its patterns one by one for every request.
- **The subresource filter** never gets its rules: Goodfetch turns the component updater off (ADR 0043), and no switch loads a ruleset. It has no regex, no redirect, no `$important`, and never filters the main frame.
- **Host matching in the relay** carries only the rules with neither path nor option: about 85% of EasyList's and 77% of EasyPrivacy's network rules. It loses every typed, third-party, path and stand-in rule, and it counts per connection, not per request or per page.

**Recommendation.** Keep the pause as the way that gives the same verdicts. If the owner's rule of 2026-11-10 brings the pause back as a cost to cut, the candidate worth building and measuring is a DNR extension of Goodfetch's own, under three conditions:
1. Its rulesets are compiled from the lists the run pins: at build time for the embedded part, as the `adblock` engine is (ADR 0047), and at the run's start for a downloaded version or `filter_lists.extra`.
2. It is a setting whose off state keeps today's pause.
3. The engine matches each request against the lists after the fact, from `Network.requestWillBeSent`, which pauses nothing. This gives Provenance its per-list count and measures how often DNR and `adblock` disagree on real pages before any default changes.

uBO Lite itself is not a candidate: it is GPL-3.0 (ADR 0047 rejects it for that), and it converts uBO's lists, not the pinned ones.

## What the engine does today

- **Pausing.** Every page, every out-of-process frame and every worker pauses every request at the request stage, and a document's response (`crates/goodfetch-page/src/interception.rs:1069-1096#pause_requests`, called at `:552` and `:731` with no condition).
- **Per request.** Each paused request costs one `Fetch.requestPaused` event and one command back: `continueRequest`, `failRequest` or `fulfillRequest`. A document pays twice, once at each stage.
- **Per target.** Each target pays one `Fetch.enable` when it opens.
- **Judging.** The engine judges a paused request in `send_on` (`crates/goodfetch-page/src/interception/steps.rs:113-183`): the egress check, then `matching::judge` (`crates/goodfetch-clean-up-rules/src/matching.rs:166-218`). That judge runs:
  1. **The never-block list**: 72 entries, all plain `adblock` rules with no option (`rules/never-block.toml`), compiled into an engine of its own, so no exception or `$important` rule of any list reaches it (`never_block.rs:1-6`).
  2. **Each list that is on.** Each list is one `adblock` engine carrying Brave's unbreak list beside it (`filter_set.rs:9-14`). A block whose `redirect` is a stand-in is fulfilled with that body; any other block fails as `BlockedByClient`. `filter_lists.first_party_scripts` lets a script of the document's own site through every list.
  3. **`browser.block_resources`.**
- **The matcher's input.** `adblock` gets the URL, the source URL, the resource type and the method. The source is the request's own URL for a navigation, else its `Referer`, else the page's document (`pipeline.rs:814-824`). `adblock` derives third party from the two registrable domains (adblock 0.13.3 `src/request.rs:213-238`).
- **Engine work.** The never-block match, then one `adblock` match per list that is on, per paused request. A match took 13.2 to 19.4 µs on the reference server, measured in ADR 0047's note of 2026-10-23 (#713).
- **Counting.** Each page counts the requests the lists blocked (`interception.rs:286-287`, `:427-430`). `goto` compares the count before and after a navigation to decide whether the lists "touched" a challenge page (`crates/goodfetch-page/src/pages.rs:1102`, `:1143`).
- **Retry and learning.** ADR 0047's retry turns the lists off for one page while it loads once more. The per-domain learning turns them off for every request whose source is on a learned domain. Both go through `lists_off` (`interception.rs:423-425`), per request; the never-block list and `block_resources` stay.

### The embedded lists, counted

At the manifest of 2026-10-22 (`crates/goodfetch-clean-up-rules/rules/`). "Host only" is a block rule `||host^` (or `||host`) with no path and no option.

| | EasyList | EasyPrivacy | Fanboy Annoyance | Brave unbreak |
|---|---:|---:|---:|---:|
| Network rules | 60,307 | 56,362 | 2,099 | 442 |
| Host-only blocks | 51,199 (85%) | 43,327 (77%) | 55 | 8 |
| Exceptions (`@@`) | 749 | 852 | 126 | 213 |
| With a resource type | 4,173 | 1,107 | 219 | 127 |
| `$third-party` / `$first-party` | 2,162 | 4,305 | 781 | 135 |
| `$domain=` | 540 | 662 | 99 | 140 |
| Regex | 22 | 7 | 0 | 13 |
| `$redirect` / `$redirect-rule` | 7 | 31 | 0 | 23 |
| `$important` | 0 | 10 | 0 | 3 |
| `$badfilter` | 0 | 0 | 0 | 13 |
| `$document`, `$generichide` and similar | 760 | 1 | 11 | 83 |
| `$csp`, `$removeparam` | 2 | 0 | 0 | 2 |

The engine applies no `$csp` and no `$removeparam`: `judge` reads only `should_block` and `redirect` (`matching.rs:200-216`). So no candidate has to give those to match today's verdicts.

## The four candidates

### A. The browser's blocking by URL pattern (`Network.setBlockedURLs`)

- **What it is.** In 156 it takes `urlPatterns`, a list of `{urlPattern, block}`. A pattern is a URLPattern constructor string, and `block: false` makes an exception. The older `urls` field (with `*` wildcards) is deprecated, and both are experimental (`third_party/blink/public/devtools_protocol/domains/Network.pdl:1140-1156`).
- **Rule kinds.**
  - URL only: Blink's probe receives nothing but the URL (`core/probe/core_probes.pidl:211`, `ShouldBlockRequest(const KURL&)`), so there is no resource type, no third party and no `$domain`.
  - Exceptions work by order: the first pattern that matches wins (`core/inspector/inspector_network_agent.cc:1283-1305`, `:2510-2548`).
  - The never-block list can go first as `block: false` patterns.
  - There are no stand-ins.
  - **Exact:** no. It loses every typed, third-party and `$domain` rule, and every stand-in (stand-ins become blocks).
- **Can a site see it?** As today: the request fails with `net::ERR_BLOCKED_BY_CLIENT`.
- **Counting.** Yes. `Network.loadingFailed` carries `blockedReason: "inspector"` (`inspector_network_agent.cc:455`, `:500-512`). It cannot tell which list blocked.
- **Retry and learning.**
  - The retry sends an empty list to the page's targets, then the full list again after: 2 commands per target.
  - Per-domain learning cannot be expressed, since the pattern sees no source. The page would have to drop the lists entirely while it shows a learned domain **[I]**.
- **Cost.**
  - **Per target:** 1 `Network.enable`, which pages already send, and 1 `setBlockedURLs` carrying about 95,000 patterns. Each out-of-process frame and each worker needs its own, since the agent belongs to the target's session (`inspector_network_agent.cc:856-866`, `:2416-2429`).
  - **Per request:** 0 CDP messages. In the renderer, a linear scan over every pattern, each a `SimpleUrlPatternMatcher` of RE2 parts (`components/url_pattern/simple_url_pattern_matcher.h:20-40`). That is likely far more than the engine's one indexed match **[I, not measured]**.
- **Coverage.** It runs from Blink's fetch contexts for frames and workers (`core/loader/frame_fetch_context.cc:1134-1141`, `worker_fetch_context.cc:89-93`). The browser-side `network_handler.cc` does not implement it, and neither the frame loader nor `navigation_request.cc` asks the inspector. So a main-frame or iframe navigation is not blocked **[I]**: the engine's `||host^` verdicts on documents would be lost, unless a `Fetch` pattern on documents kept them paused.

### B. The browser's subresource filter (`Page.setAdBlockingEnabled`)

- **What it is.** "Enable Chrome's experimental ad filter on all sites" (`Page.pdl:1053-1057`). It forces activation on the tab whatever the site (`components/subresource_filter/content/browser/devtools_interaction_tracker.cc:17-23`, `safe_browsing_page_activation_throttle.cc:212-220`).
- **Where its rules come from.** Only the component updater, which indexes a ruleset of format 1 (`chrome/browser/component_updater/subresource_filter_component_installer.cc:80-100`). No switch loads a ruleset of one's own.
  - Goodfetch launches every browser with `--disable-component-update` and points the updater at `components.invalid` (ADR 0043 and its note of 2026-10-21, #639), so the filter has no rules.
  - Seeding the component's folder in the profile might load a converted list, through `ruleset_converter --input_format filter-list` (`components/subresource_filter/tools/ruleset_converter/main.cc:26-55`). That is unsupported, and Chrome would replace it on its next update **[I, not tried]**.
- **Rule kinds** (`components/url_pattern_index/proto/rules.proto`, `tools/rule_parser/rule_parser.cc:143-151`):
  - Supported: resource types, first and third party, request and initiator domains with exclusions, and exceptions, an exception always beating a block (`core/common/indexed_ruleset.cc:198-203`).
  - Not supported:
    - regex rules ("TODO… Implement REGEXP rules matching", `components/url_pattern_index/url_pattern_index.cc:386-389`);
    - `$redirect`, so no stand-ins;
    - `$important`, so the never-block list cannot outrank a list's block except as an ordinary exception, which any list's block cannot outrank either.
  - The main frame is never filtered, where the engine blocks a top-level visit to a host a `||host^` rule lists (adblock 0.13.3 `src/filters/network.rs:854-872`).
  - **Exact:** no.
- **Can a site see it?** As today (`ERR_BLOCKED_BY_CLIENT`) **[I]**.
- **Counting.** Yes: `blockedReason: "subresource-filter"` (`inspector_network_agent.cc:457`).
- **Retry and learning.**
  - The retry sends `setAdBlockingEnabled(false)` on the page, then `true`: 2 commands per page.
  - Per-domain learning means rebuilding the ruleset, which only the updater does **[I]**.
- **Cost.** 1 `Page.setAdBlockingEnabled` per page. Matching is an n-gram index over a flatbuffer, per frame and per request, in the browser and renderer **[I]**. 0 CDP messages per request.

### C. An extension's declarative rules (`declarativeNetRequest`)

- **What it is.** An MV3 extension carries static rulesets; Chrome indexes them and matches every request in the browser process ([DNR reference](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest); `extensions/browser/api/declarative_net_request/ruleset_manager.cc:466-502`).
- **Loading it.**
  - Chrome for Testing keeps `--load-extension`, which only branded Chrome ignores (`chrome/browser/extensions/extension_service.cc:420-440`; [the extensions group's notice](https://groups.google.com/a/chromium.org/g/chromium-extensions/c/1-g8EFx2BBY)).
  - `Extensions.loadUnpacked` loads one over CDP on the browser target (`chrome/browser/devtools/protocol/extensions_handler.cc:200-220`).
  - Collie launches new headless (`crates/goodfetch-page/src/launch/arguments.rs:143`), which is the full browser.
  - **Sessions are off the record.** A Session is a browser context made with `Target.createBrowserContext` (`crates/goodfetch-page/src/driver.rs:818-826`), which Chrome makes as an off-the-record profile (`chrome/browser/devtools/devtools_browser_context_manager.cc:79-92`).
    - DNR skips a request in an off-the-record context unless the extension is enabled in incognito (`extensions/browser/api/declarative_net_request/ruleset_manager.cc`, `ShouldEvaluateRulesetForRequest`; `extensions/browser/extension_util.cc:85-151`). The manifest's `incognito` key does not enable it; a stored preference does.
    - `Extensions.loadUnpacked(path, enableInIncognito)` sets that preference (`Extensions.pdl:28-36`, `chrome/browser/devtools/protocol/extensions_handler.cc:224-236`, `extensions/browser/unpacked_installer.cc:445-453`). `--load-extension` never does (`unpacked_installer.cc:149-198`).
    - So the extension has to be loaded over CDP. Whether the browser needs `--enable-unsafe-extension-debugging` for that was not found in the source, and is left to a run. Whether Chrome treats DevTools' unique off-the-record profile as it treats an incognito window was not checked **[I]**.
- **Rule kinds.**
  - Conditions: `urlFilter` (with `*`, `|`, `||` and `^`), `regexFilter` (RE2, at most 1,000 regex rules), `resourceTypes`, `domainType` (`firstParty` or `thirdParty`), `initiatorDomains`, `requestDomains` and their exclusions.
  - Actions: `block`, `allow`, `allowAllRequests` and `redirect`, ordered by `priority`.
  - Limits: 30,000 guaranteed static rules per extension; uBO Lite folds 42,597 EasyList hosts into one `requestDomains` rule (uBOL-home `chromium/log.txt:116-141`), so the host rules fit.
  - Chromium has its own converter from filter lists to DNR, but it rejects regex rules and every activation option but `$document` (`extensions/browser/api/declarative_net_request/filter_list_converter/converter.cc:117-160`, `:183`). uBO Lite's converter goes further (`gorhill/uBlock` `src/js/static-net-filtering.js:4930-5075`):
    - `$important` becomes +30 priority;
    - `$badfilter` is applied at conversion;
    - redirects point at the extension's web-accessible files.
- **Mapping each kind:**
  - **Resource type, third party, `$domain`:** yes. DNR judges third party and initiator domains against the frame that sent the request. The engine judges them against the `Referer`, or else the page's document, so they differ:
    - when the `Referer` is withheld (`no-referrer`) in a third-party frame;
    - when the `Referer` names another page than the sender.
  - **A top-level document:** yes, when the converter says so. `adblock` applies a `||host^` rule with no type option to the main document too (adblock 0.13.3 `src/filters/network.rs:854-872`), but a DNR rule with no `resourceTypes` matches every type except `main_frame` ([DNR reference](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest)). The converter has to add `main_frame` to those rules.
  - **Exceptions:** yes, as `allow` rules.
  - **`$important`:** yes, as priority.
  - **The never-block list:** yes, as `allow` rules above every list's priority, as uBO Lite's trusted-site rules are.
  - **`filter_lists.first_party_scripts`:** yes, as `allow` with `resourceTypes: ["script"]` and `domainType: "firstParty"`.
  - **`browser.block_resources`:** yes, as `block` by type above the lists' allows, under the never-block allow.
  - **Stand-ins:** yes, with differences:
    - `adblock` serves a stand-in only when the request is blocked, and lets exceptions apply to it (adblock 0.13.3 `src/blocker.rs:47-55`, `:221-276`). DNR orders allow, block and redirect by priority.
    - The page sees the stand-in's `chrome-extension://` URL.
  - **Regex:** the 42 regex rules fit under 1,000. Chromium's converter drops them; uBO Lite keeps most.
  - **Exact:** no. Close, with the losses above.
- **Can a site see it?**
  - A block looks as today (`ERR_BLOCKED_BY_CLIENT`).
  - A stand-in is served from the extension's web-accessible files. uBO Lite marks them `use_dynamic_url`, so their URL changes and no fixed URL probes the extension (`gorhill/uBlock` `platform/mv3/make-rulesets.js:1033`, `:1259-1268`). The page can still read a `chrome-extension://` URL in `currentScript.src` or Resource Timing **[I]**.
  - Today a stand-in answers on the script's own URL. Under `stealth`, this is a signal today's engine does not give.
- **Counting.**
  - Yes. A DNR block fails with the extension's reason, which DevTools reports as `blockedReason: "other"` (`chrome/common/chrome_resource_request_blocked_reason.h:11-15`, `content/browser/devtools/protocol/network_handler.cc:3103-3116`).
  - It cannot tell which list blocked. `getMatchedRules` is limited to 20 calls per 10 minutes, and `onRuleMatchedDebug` only works for unpacked extensions with `declarativeNetRequestFeedback` ([DNR reference](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest)).
  - The engine's after-the-fact match (Recommendation) would fill that in.
- **Retry and learning.**
  - **Per-domain learning:** a dynamic `allow` rule with `initiatorDomains: [domain]` above the lists, below `block_resources`. This keys on the frame, as close to the engine's `Referer` key as DNR gets.
  - **The retry:** a session rule `allowAllRequests` with `tabIds: [tab]` for the page's one load, then removed. The engine has to map a CDP target to the extension's tab ID, a mapping CDP does not give **[I]**.
  - Each change is a call into the extension's service worker, over CDP (`Target.attachToTarget` once, then `Runtime.evaluate`): 1 command per change.
- **Cost.**
  - **Per browser started:**
    - 1 `Extensions.loadUnpacked`.
    - An unpacked extension's rulesets are all parsed and indexed in the browser process each time it loads (`unpacked_installer.cc:308-349`, `install_index_helper.cc:224-246`), and one invalid rule fails the whole load. Browserless documents about 69,000 rules compiled on every fresh launch (ADR 0047).
    - uBO Lite's folding of hosts into `requestDomains` rules brings EasyList and EasyPrivacy to about 13,000 DNR rules (uBOL-home `chromium/log.txt`).
    - A packed, pre-indexed extension would avoid this, but it is not loaded by switch or over CDP **[I]**.
  - **Per Session:** 0 commands, once the extension is enabled in incognito: the Session's context inherits it from the browser's profile **[I]**.
  - **Per page:** 0 commands.
  - **Per request:** 0 CDP messages and 0 engine work. Once any extension uses DNR or webRequest, Chrome sends every request through `WebRequestProxyingURLLoaderFactory` in the browser process (`extensions/browser/api/web_request/web_request_api.cc:182-189`, `:1013-1023`). That is one more hop per request, against a CDP round trip today **[I, not measured]**.
- **Coverage.**
  - DNR applies to a service worker's `fetch()`, but not to a response the worker makes itself or serves from CacheStorage ([DNR reference](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest)). Today the engine pauses the service worker's own requests the same way.
  - It works only in a browser that loads Goodfetch's extension. A Borrowed browser and an installed branded Chrome would keep the pause.

### D. Matching by host in the egress relay

- **What it is.** The relay sees a `CONNECT` host name, not the path, the resource type or the page (#2436). For `http://` it sees the URL too, which is rare.
- **Rule kinds.**
  - Only the host-only blocks: 51,199 of EasyList's 60,307 network rules and 43,327 of EasyPrivacy's 56,362 (table above).
  - Lost: types, third party, `$domain`, paths, path-scoped exceptions and stand-ins. A host-anchored exception scoped by path or option cannot lift a host block. uBO Lite's build log finds the same split (42,597 and 43,382 hosts folded into one rule each, uBOL-home `chromium/log.txt:116-141`).
  - A top-level visit to a listed host is blocked, as the engine blocks it today: `adblock` applies a `||host^` rule with no type option to documents too, as uBlock Origin does (adblock 0.13.3 `src/filters/network.rs:854-872`).
  - The never-block list fits only where an entry is a whole host: 45 of the 72 are. The other 27 are scoped by a path (`/cdn-cgi/challenge-platform/`, `||www.google.com/recaptcha/`), and the relay can keep them only by not blocking their whole host.
  - **Exact:** no. It is the furthest.
- **Can a site see it?** The request fails with `net::ERR_TUNNEL_CONNECTION_FAILED` (`net/http/http_proxy_client_socket.cc:499-530`), not `ERR_BLOCKED_BY_CLIENT`. A site that tells the two apart learns a proxy refused it **[I]**.
- **Counting.**
  - The relay counts refused tunnels per Session. CDP reports them with no `blockedReason`, so the page cannot tell them from a network failure.
  - A tunnel serves many requests, and a refused host is refused per connection attempt, so the count is not per request and not per page **[I]**.
- **Retry and learning.**
  - The relay holds no page, so the retry with blocking off cannot be per page: it can only be per Session.
  - The learning keys on the document's domain, which the relay never sees.
- **Cost.** 0 CDP messages. One hash lookup per `CONNECT`, not per request. It exists only where the relay enforces egress (ADR 0140), not on a Borrowed browser.

## What else keeps a request paused

This research changes the Filter lists' pause only. A request still pauses for:
- a route;
- the Response cache;
- a navigation gate;
- a placeholder;
- the egress check where the relay does not enforce it (ADR 0140 C5).

#2337's test found that the relay's credentials can be given once per Session from a page of the engine's own, so a program's pages need no pause for them (ADR 0140's note of 2026-11-12). So whatever replaces the lists' pause removes the pause only for a Session at its defaults, on a browser Goodfetch launched behind its relay.

## What to measure after the window, if the owner's rule brings the pause back

1. C's cost per request against today's pause, on B2's cells (ADR 0140), with the extension carrying the embedded lists: the page's time to load and the browser process's CPU per request.
2. C's cost per browser start: the time to load and index the extension's rulesets.
3. How often C's verdict differs from `adblock`'s on the Proving ground and a sample of live pages, from the after-the-fact match.

## Sources

Chromium 156.0.8078.4, under `https://chromium.googlesource.com/chromium/src/+/refs/tags/156.0.8078.4/`:
- the protocol: `third_party/blink/public/devtools_protocol/domains/{Network,Page,Fetch,Extensions}.pdl`;
- Blink: `third_party/blink/renderer/core/inspector/inspector_network_agent.cc`, `core/loader/{base_fetch_context,frame_fetch_context,worker_fetch_context}.cc`, `core/probe/core_probes.pidl`;
- `components/url_pattern/simple_url_pattern_matcher.h`;
- `content/browser/devtools/protocol/network_handler.cc`;
- the subresource filter: `components/url_pattern_index/{proto/rules.proto,url_pattern_index.cc}`, `components/subresource_filter/content/browser/{devtools_interaction_tracker,safe_browsing_page_activation_throttle,ruleset_service}.cc`, `components/subresource_filter/core/common/indexed_ruleset.cc`, `components/subresource_filter/tools/{rule_parser/rule_parser.cc,ruleset_converter/main.cc}`, `chrome/browser/component_updater/subresource_filter_component_installer.cc`;
- Chrome's DevTools and extensions: `chrome/browser/devtools/protocol/{page_handler,extensions_handler}.cc`, `chrome/browser/extensions/extension_service.cc`, `chrome/common/chrome_resource_request_blocked_reason.h`;
- the extension APIs: `extensions/browser/api/declarative_net_request/{README.md,ruleset_manager.cc,filter_list_converter/converter.cc}`, `extensions/browser/api/web_request/web_request_api.cc`;
- `net/http/http_proxy_client_socket.cc`.

Other primary sources:
- [chrome.declarativeNetRequest](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest).
- [The `--load-extension` notice](https://groups.google.com/a/chromium.org/g/chromium-extensions/c/1-g8EFx2BBY), Chromium extensions group.
- `adblock` 0.13.3: [crate](https://static.crates.io/crates/adblock/adblock-0.13.3.crate) (`src/request.rs`, `src/blocker.rs`, `src/content_blocking.rs`) and [docs.rs](https://docs.rs/adblock/0.13.3). Its only exporter is to Apple's content-blocker format; it has none to DNR.
- Brave matches with `adblock-rust` in the browser process, not through DNR: [brave_ad_block_tp_network_delegate_helper.cc](https://github.com/brave/brave-core/blob/master/browser/net/brave_ad_block_tp_network_delegate_helper.cc).
- [gorhill/uBlock](https://github.com/gorhill/uBlock) at `2bc679d`: `platform/mv3/make-rulesets.js`, `src/js/static-net-filtering.js`.
- [uBlockOrigin/uBOL-home](https://github.com/uBlockOrigin/uBOL-home) at `2026.1110.1816`: `chromium/log.txt`, `chromium/rulesets/ruleset-details.json`, and the wiki's FAQ.

Goodfetch, at `4f9678a04`: the files cited above, ADRs 0043, 0047 and 0140.
