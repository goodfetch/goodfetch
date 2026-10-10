# Rust CDP client options for the browser driver

Research of **October 10, 2026** for ticket #67 (map #61). Question: should the Automan browser driver build on **Chromey**, **Chromiumoxide** or an **in-house client** grown from the lab's direct WebSocket client ([`rnd/lab/adapters/cdp-rust`](../lab/adapters/cdp-rust/))? Other candidates checked: `headless_chrome`, a client generated from the official protocol, and WebDriver BiDi crates.

**Sources.** The published crate archives from crates.io, read at the repository commits they record. The crates.io and GitHub APIs for releases, contributors, issues and PRs. ChromiumDash for revision-to-milestone mapping. The `ChromeDevTools/devtools-protocol` repository at its latest roll (r1716261, October 10, 2026). For this note, nothing was compiled against or run in a browser. The behavior claims about third-party crates come from reading their source and are marked "from source". Like everything in `rnd/`, this note informs a decision; it does not make one.

---

## 1. Summary

- **The guard does not need anything these libraries provide that the lab client lacks.** The Automan guard is a set of page scripts sent with `Runtime.evaluate`, plus raw `Input.dispatch*` events. Chromey and Chromiumoxide both let you send raw commands with `Page::execute`, so the guard would bypass their high-level API anyway. Their `Element::click()` scrolls into view, takes the centre of the first content quad and clicks. It does not check uniqueness, visibility, stability or the hit target.
- **Neither crate handles out-of-process iframes (OOPIFs).** In Chromiumoxide 0.9.1 and Chromey 2.58.2, from source:
  - Both auto-attach to OOPIF targets and resume them.
  - The frame manager's `on_attached_to_target` is an empty stub, and no public API sends a command to another session.
  - The Chromiumoxide issue on this has been open since December 2025, and the PR that would fix it has been unmerged since July 2026.

  The lab client already clicks inside cross-origin iframes through the frame's own session, and passes 21 of 21 navigation, frame and file cells.
- **The protocol types are 9 to 10 Chrome versions behind.** Chrome stable is 155 and has shipped every two weeks since September 8, 2026.
  - Chromiumoxide is generated at r1566079 (Chrome 145).
  - Chromey is generated at r1581282 (Chrome 146).
  - `headless_chrome` is generated at r1510116 (Chrome 142).
  - Puppeteer 25.13.0, for comparison, pins r1696802 (Chrome 155).

  Strict typing has already broken in the past. Chrome 129 broke Chromiumoxide 0.7.0 in October 2024, and the fix only shipped in 0.8.0, in November 2025.
- **Maintenance points in opposite directions.**
  - **Chromiumoxide** is by far the most used: 2.1 million downloads over 90 days. But nothing has been merged since April 3, 2026, and 16 PRs are open, including OOPIF support, command timeouts and a protocol bump.
  - **Chromey** is very active: 243 releases since November 2025, plus 749 earlier ones under the name `spider_chrome`. But one maintainer wrote 231 of its 235 commits, its direction follows Spider's needs, and it pulls 203 crates, including mandatory fingerprint and stealth crates. The lab client pulls 56.
- **`headless_chrome` does not fit a Tokio core.** It is blocking and runs on threads. It uses the deprecated non-flat session mode. Its build dependency `auto_generate_cdp` ships the GPL-3.0 licence.
- **WebDriver BiDi in Rust is young, and on Chrome it sits on top of CDP.**
  - The most credible client is the `bidi` feature of `thirtyfour`, available since 0.37.0 (May 2026).
  - In Chrome, BiDi is implemented by the `chromium-bidi` mapper, which translates BiDi to CDP inside the browser and is loaded by ChromeDriver.
  - BiDi matters for a future Firefox lane, not for the Chromium driver.
- **Recommendation, for the owner to decide: an in-house client.** Productize the lab client and add three things:
  1. A typed layer generated only for the domains the driver uses, from a pinned `devtools-protocol` revision, with lenient deserialization.
  2. CI runs against Chrome for Testing stable and beta.
  3. The robustness the lab client still lacks (section 5).

  Reuse pieces such as the `chromiumoxide_pdl` generator or `chromiumoxide_fetcher` where they help. Do not adopt either crate's page and handler model.

## 2. Comparison table

State on October 10, 2026. "From source" means read in the published archive, not executed. Download counts are crates.io's 90-day `recent_downloads`.

| Criterion | Chromey 2.58.2 | Chromiumoxide 0.9.1 | In-house (lab `cdp-rust`) | headless_chrome 1.0.22 |
| --- | --- | --- | --- | --- |
| Last release | 2026-09-12; `main` has 2.58.3 (2026-09-29) | 2026-02-25; last merge 2026-04-03 | n/a (lab code) | 2026-06-11 |
| Release cadence | 243 releases since 2025-11-01; 749 earlier as `spider_chrome` (2024-10 → 2026-03) | 0.7.0 (2024-08) → 0.8.0 (2025-11) → 0.9.0/0.9.1 (2026-02) | owned by Automan | 6 releases since 2025-02 (1.0.16 → 1.0.22) |
| Maintainers | j-mendez: 231 of 235 commits | mattsse (author, 257 commits); Sytten (94, current releases) | Automan | community |
| 90-day downloads | 42,212 (+173,648 for `spider_chrome`) | 2,100,838 | — | 789,305 |
| License | MIT OR Apache-2.0 | MIT OR Apache-2.0 | Automan's | MIT; GPL-3.0 build-dependency |
| How protocol types are made | `spider_chromiumoxide_cdp` 0.7.9, generated from PDL and checked in | `chromiumoxide_cdp` 0.9.1, generated from PDL by `chromiumoxide_pdl` and checked in (111k lines) | none: untyped `serde_json::Value` | generated at build time from the JSON at a pinned `devtools-protocol` commit |
| Protocol revision | r1581282 → Chrome 146 | r1566079 → Chrome 145 | whatever the browser speaks; 20 commands and 5 events used | r1510116 → Chrome 142 |
| Tolerance to protocol drift | struct-level `#[serde(default)]` (1,354 sites); no catch-all enum variant | invalid messages skipped since 0.8.0; missing fields still fail a response (issue #335); no catch-all enum variant | additive changes invisible by construction; only the used fields matter | strict generated types |
| Session multiplexing | flat sessions on one WebSocket; opt-in `parallel-handler` routes per session | flat sessions on one WebSocket; one `Handler` task drives all pages | flat sessions on one WebSocket; one reader task, one broadcast channel for events | non-flat: each command wrapped in deprecated `Target.sendMessageToTarget` |
| OOPIF | auto-attached and resumed, not modeled; no public per-session send (from source) | same (from source); issue #280 open, PR #331 unmerged | frame session via `Target.setAutoAttach`; script and input sent to that session; one level only | no auto-attach found in source |
| Isolated world | utility world per frame, with universal access | `__chromiumoxide_utility_world__` per frame, with universal access; exposed as `secondary_execution_context` | in-process iframes only; the main-page guard runs in the main world | not inspected |
| Connection loss | transport errors surfaced; reset-without-handshake disposes the handler; target crash fails in-flight commands at once | transport error ends the handler stream; on a clean close, in-flight commands wait for the 30 s timeout (from source); no crash handling | reader end fails all pending commands at once; no per-command timeout; no crash handling | `ConnectionClosed` flag; blocking wait polled every 5 ms |
| Reconnect | none | none | none | none |
| Async runtime | Tokio required | Tokio only (async-std removed in 0.9.0) | Tokio | blocking std threads |
| Commands to open a page (default) | same staged sequential chain as Chromiumoxide | 15, sent one after another (count from source) | 4 | — |
| Resolved crates (default features, all targets) | 203 | 136 | 56 | 101 |
| MSRV | 1.70 | 1.85 | 1.85 (edition 2024) | 1.85 |
| Fit with the guard | only via raw `Page::execute`; high-level click unguarded | same | proven on 78 Chromium cells (section 4) | poor |

**Revision-to-milestone mapping.** A CDP revision is a Chromium `main` commit position. It ships in the first milestone that branched after it. Branch positions from [ChromiumDash](https://chromiumdash.appspot.com/fetch_milestones):
- M141: 1509326; M142: 1522585.
- M144: 1552494; M145: 1568190; M146: 1582197.
- M154: 1689415; M155: 1697595; M156: 1706539.

Chrome 155 reached stable on October 6, 2026 (155.0.8059.39). Chrome 156 is due on October 20, 2026 ([schedule](https://chromiumdash.appspot.com/fetch_milestone_schedule?mstone=156)).

**How the dependency counts were made.** Each crate, pinned exactly (`=x.y.z`), was resolved in an empty scratch project with `cargo tree -e normal --target all`, then unique `name vX` lines were counted, including the root. The lab client was counted with `--locked` against its own `Cargo.lock`. Nothing was compiled.

## 3. Context: Chrome's stance on CDP and its release pace

- **CDP is not a stable API.** Chrome's protocol site calls it "not a public or supported API for Chrome": there is no backward-compatibility guarantee, and syntax and semantics can change without notice ([CDP home](https://chromedevtools.github.io/devtools-protocol/)). The tip-of-tree protocol changes often and can break at any time. The "stable 1.3" protocol is only a historical snapshot from Chrome 64, with fewer capabilities.
- **Every option carries the same risk.** What differs between them is how fast a breakage can be fixed and released.
- **Flat sessions are the intended mode.** The `Target` domain's notes on `flatten` say Chrome plans to make it the default, deprecate non-flattened mode and eventually retire it (crbug.com/991325). `Target.sendMessageToTarget` is already marked `deprecated` ([Target.pdl at r1716261](https://github.com/ChromeDevTools/devtools-protocol/blob/c3ed21b573/pdl/domains/Target.pdl)). Several things Automan relies on are still marked experimental:
  - the `flatten` parameter of `setAutoAttach`;
  - the `Target.attachedToTarget` event;
  - `Input.insertText`;
  - `Browser.setDownloadBehavior` and `Browser.downloadProgress`.
- **Chrome now ships every two weeks.** The cadence started with Chrome 153 on September 8, 2026. The Extended Stable channel keeps an eight-week cycle ([announcement, March 3, 2026](https://developer.chrome.com/blog/chrome-two-week-release); [launch, September 8, 2026](https://developer.chrome.com/blog/chrome-two-week-start)). Protocol drift now reaches users twice as often.
- **The protocol itself moves several times a week.** It rolled on October 3, 8 and 10, 2026 ([commits](https://github.com/ChromeDevTools/devtools-protocol/commits)).
- **Puppeteer keeps pace.** Each release maps to one Chrome version: 25.13.0 maps to Chrome for Testing 155.0.8059.39 ([supported browsers](https://pptr.dev/supported-browsers)). Its `puppeteer-core` 25.13.0 pins `devtools-protocol` 0.0.1696802 (npm registry). Those are TypeScript types, erased at runtime, so messages stay plain JSON. That is the tolerance the lab client already has.

## 4. Per-candidate notes

### Chromiumoxide 0.9.1 (`mattsse/chromiumoxide`)

**Release history.** Changes per release, from the [CHANGELOG](https://github.com/mattsse/chromiumoxide/blob/main/CHANGELOG.md):
- **0.8.0, November 28, 2025.** PDL moved to r1519099 (Chromium 142). Tokio became the default runtime. Message parsing became more resilient, so unknown events no longer crash the handler.
- **0.9.0, February 20, 2026.** CDP moved to r1566079. async-std support was removed. `add_init_script` and `click_with` were added.
- **0.9.1, February 25, 2026.** A fix to the zip import.

Each release catches the protocol up to the stable Chrome of its day. Releases are sparse, though. The protocol was bumped in March 2023 (r1045629), in October 2024 (r1359167, then rolled back to r1354347), in November 2025 and in February 2026 ([lib.rs history](https://github.com/mattsse/chromiumoxide/commits/main/chromiumoxide_cdp/src/lib.rs)).

**Maintenance.** The last merges are dated April 3, 2026 ([commits](https://github.com/mattsse/chromiumoxide/commits/main)). On October 10, 2026, the repository had 45 open issues and 16 open PRs, 10 of them opened after that last merge. The unmerged PRs include:
- [#331](https://github.com/mattsse/chromiumoxide/pull/331), session-aware cross-origin iframe support;
- #332 and #336, making page commands honour `request_timeout`;
- #305, a protocol bump to r1596832.

**Compatibility history.** In [#243](https://github.com/mattsse/chromiumoxide/issues/243), Chrome 129.0.6668.90 broke 0.7.0 in October 2024. The handler hit a deserialization error on an unknown message and stopped. The maintainer linked it to a pending "skip invalid messages" change. The fix only shipped in 0.8.0, 13 months later. [#335](https://github.com/mattsse/chromiumoxide/issues/335), from September 2026 and still open, shows the remaining failure mode. When a response omits experimental `Page.Frame` fields, `Page.getFrameTree` does not deserialize, frame initialization stops, and navigation never starts. Skipping invalid messages (`ignore_invalid_events` defaults to `true`) turns a crash into a silent wait, because a skipped response leaves its command to the 30 s eviction.

**Protocol generation.** The `chromiumoxide_pdl` crate parses PDL (a Rust rewrite of Chromium's `pdl.py`) and generates code into `chromiumoxide_cdp/src/cdp.rs`: 111,037 lines, checked in. A test re-downloads the PDL at `CURRENT_REVISION` and fails if the generated code is stale ([generate.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/chromiumoxide_cdp/tests/generate.rs)). Experimental items are included; deprecated ones are excluded by default. A source comment recommends staying behind tip-of-tree so older browsers keep working ([lib.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/chromiumoxide_cdp/src/lib.rs)). Generated enums have no catch-all variant (0 `#[serde(other)]`).

**Frames and OOPIFs.** From source:
- Pages auto-attach with `flatten: true` and `waitForDebuggerOnStart: true`, and the handler sends `Runtime.runIfWaitingForDebugger` to new sessions ([target.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/target.rs)).
- `TargetType` has no `iframe` variant, so OOPIF targets become `Unknown("iframe")`. `Target::poll` only advances pages.
- `FrameManager::on_attached_to_target` is an empty stub ([frame.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/frame.rs)).
- The messages that would target another session are `pub(crate)`.

Issue [#280](https://github.com/mattsse/chromiumoxide/issues/280), open since December 2025, confirms this. A user reports that cross-origin frames are missing from `page.frames()` and can only be reached with raw JSON. The maintainer replied that this is beyond his knowledge and invited a PR.

**Isolated worlds.** Each frame gets a utility world, `__chromiumoxide_utility_world__`, created with `grantUniveralAccess: true`. The CDP doc calls that option powerful and says to use it with caution ([Page.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/c3ed21b573/pdl/domains/Page.pdl)). The world is reachable through `secondary_execution_context()` and `frame_secondary_execution_context()`.

**Page initialization.** It runs as a chain of commands, each waiting for the previous response ([cmd.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/cmd.rs)). With the default config, from source, that is 15 sequential commands, plus a queued `Network.setCacheDisabled`:
- `createTarget` and `attachToTarget`;
- `Page.enable`, `getFrameTree`, `setLifecycleEventsEnabled` and `Runtime.enable`;
- `addScriptToEvaluateOnNewDocument` and `createIsolatedWorld`;
- `Network.enable` and `Security.setIgnoreCertificateErrors` (`ignore_https_errors` defaults to `true`);
- `setAutoAttach`, `Performance.enable` and `Log.enable`;
- `setDeviceMetricsOverride` and `setTouchEmulationEnabled` (default viewport 800×600).

The lab client opens a page with 4. With a remote browser, each extra command costs one round trip.

**Connection loss.** From source:
- A WebSocket error makes the `Handler` stream yield `Err`, and the README's polling loop then stops.
- On a clean close, the connection stream ends, but the handler returns `Pending` instead of terminating. In-flight commands then fail with `Timeout` once `REQUEST_TIMEOUT` (30 s) evicts them ([handler/mod.rs](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/src/handler/mod.rs)).
- There is no `targetCrashed` handling and no reconnect. `Browser::connect` can re-attach from scratch.

**Runtime.** The README says the crate only supports Tokio. `tokio` is a non-optional dependency with `rt-multi-thread`, `process` and `fs` ([manifest](https://github.com/mattsse/chromiumoxide/blob/a7e2bb835b9643410f9e3dc044f0d947e96cbfa4/Cargo.toml)).

**Reusable parts.** These are published as separate MIT OR Apache-2.0 crates:
- `chromiumoxide_pdl`: parser and generator;
- `chromiumoxide_types`;
- `chromiumoxide_fetcher`: Chrome for Testing download.

### Chromey 2.58.2 (`spider-rs/chromey`)

**Lineage.** Chromey is a fork of Chromiumoxide maintained for Spider. It keeps the library name `chromiumoxide` and publishes the package `chromey` ([README](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/README.md)). Before the crate `chromey` appeared on November 1, 2025, it was published as `spider_chrome`: 749 versions from October 2024, the last in March 2026 ([crates.io API](https://crates.io/api/v1/crates/spider_chrome)).

**Releases.** Chromey published 243 versions between November 1, 2025 and September 12, 2026. Activity comes in bursts: 75 releases in December 2025, 69 in April 2026, then 0 to 7 per month since May (none in August). Nearly all commits are by one author, j-mendez (231 of 235 by GitHub's contributor count). The repository has 52 stars ([repo API](https://api.github.com/repos/spider-rs/chromey)). Its recent work follows Spider's needs:
- a navigation deadline;
- caller headers on the WebSocket upgrade;
- a remote cache;
- vendor domains (`Content.getMarkdown`, `WebMCP`).

**Protocol.** Types come from `spider_chromiumoxide_cdp` 0.7.9, at r1581282. Since the repository moved in June 2025, the protocol has been bumped once, on February 8, 2026 ([cdp history](https://github.com/spider-rs/chromey/commits/main/cdp/src/lib.rs)). On March 2, 2026, Chromey added `Default` and `#[serde(default)]` to every type "for old Chrome compat" ([commit 321ec919](https://github.com/spider-rs/chromey/commit/321ec91912a3ea7c31dc0a6a2d407f130c5ab975)). As a result, missing fields no longer break deserialization, which is exactly the failure Chromiumoxide still has in #335. Unknown enum values have no catch-all variant.

**Frames, sessions and isolated worlds.** These have the same structure as Chromiumoxide (from source):
- auto-attach with flat sessions;
- the same `on_attached_to_target` stub, with no `iframe` handling;
- a utility world with universal access ([frame.rs](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/src/handler/frame.rs)).

Chromey's frame init does not call `Runtime.enable`; the line is commented out. An opt-in `parallel-handler` feature demultiplexes messages by session onto one task per page ([router.rs](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/src/handler/parallel/router.rs)).

**Robustness.** Chromey goes further than upstream, from source ([handler/mod.rs](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/src/handler/mod.rs)):
- `Target.targetCrashed` fails that target's in-flight commands at once.
- `AlreadyClosed` and reset-without-closing-handshake errors dispose the handler.
- The WebSocket reader decodes off the handler, offloads large frames to `spawn_blocking`, and applies bounded back-pressure ([conn.rs](https://github.com/spider-rs/chromey/blob/623c0dff613842a54783609d221bd38130857303/src/conn.rs)).

**Weight.** By default, Chromey depends on `reqwest`, `spider_fingerprint`, `spider_network_blocker`, `aho-corasick`, `phf`, `rand` and `sonic-rs` (the default `simd` feature). It resolves to 203 crates. Stealth is opt-in (`enable_stealth_mode`), but the fingerprint crate is always compiled in. That matters given ADR-002, which makes `identified` the default profile and keeps stealth out of scope ([ADR-002](../adr/0002-access-identity-stealth.md)).

**Other.**
- License: MIT OR Apache-2.0 in Cargo; GitHub displays Apache-2.0.
- MSRV: 1.70.
- Tokio is required, with `rt-multi-thread`, `fs`, `process` and `net`.

### In-house: what the lab client already proves

**Size.** The client is 917 lines of Rust in 5 files ([`cdp.rs`](../lab/adapters/cdp-rust/src/cdp.rs) 371, `task.rs` 308, `main.rs` 106, `browser.rs` 72, `scripts.rs` 60). It has 4 direct dependencies: `tokio` 1.53.2, `tokio-tungstenite` 0.29.0, `futures-util` 0.3.34 and `serde_json` 1.0.151, which resolve to 56 crates. It embeds the shared page scripts from [`adapters/browser-scripts`](../lab/adapters/browser-scripts/) (3.7 KB of JS).

**Protocol coverage.** It uses 20 commands and 5 events across 8 domains:
- Target: `createTarget`, `attachToTarget` (flat), `setAutoAttach`, `setDiscoverTargets`, `closeTarget`;
- Page: `enable`, `navigate`, `createIsolatedWorld`, `getNavigationHistory`, `navigateToHistoryEntry`;
- Runtime: `evaluate`;
- Input: `dispatchMouseEvent`, `dispatchKeyEvent`, `insertText`;
- DOM: `getDocument`, `querySelector`, `describeNode`, `setFileInputFiles`;
- Emulation: `setDeviceMetricsOverride`;
- Browser: `setDownloadBehavior`;
- Security: `setIgnoreCertificateErrors`.

The events are `Page.domContentEventFired`, `Page.frameNavigated`, `Target.targetCreated`, `Target.attachedToTarget` and `Browser.downloadProgress`.

**Correctness, measured.** These results are class-A correctness results, not timings ([driver contracts](../lab/reports/browser-driver-contracts-2026-10-08.md)):
- Extended actionability, guarded: 27 of 27 cells.
- Primitives, guarded: 30 of 30.
- Navigation, frames and files: 21 of 21, natively and guarded. That covers SPA route, history back, popup, same-origin and cross-origin iframe, upload and download.

The cross-origin case taught one lesson. Routed through the page, input to a freshly created OOPIF can land on the parent `<iframe>` element, with any driver. The client therefore sends that input to the frame's own session, in frame coordinates. The same command sequence is shared with the Node CDP driver, and an integration test requires both to send the same number of CDP commands ([lab README](../lab/README.md)).

**Performance, measured.** Relative to native Playwright, guarded Rust CDP takes 0.80 / 0.84 / 0.73 of the time at 20 ms RTT and 0.68 / 0.72 / 0.80 at 300 ms (form, clicks, JS catalog). Node CDP is within ±0.02, so the protocol language makes no difference; the command sequence does ([quiet host](../lab/reports/quiet-host-2026-10-10.md)). A library choice therefore pays off only if it lets Automan control every round trip. Chromiumoxide's default page open, 15 sequential commands, is the opposite.

**Protocol drift.** The client parses everything as `serde_json::Value`. Fields added by Chrome are invisible to it. Only a change to the 20 commands it uses can break it, and it would fail where the field is used, for example `missing targetId`.

**What it does not prove yet** (from source):
- **Isolated world.** On the main page, the guard runs in the **main world**: `Runtime.evaluate` without `contextId`, with its state in `window.__automanGuard`. Only in-process iframes get an isolated world (`automan`). On an OOPIF, the guard runs in that session's default world. Page code can therefore see or tamper with the guard's globals and DOM prototypes.
- **OOPIFs.** Attachment is one level only, waits for the matching `attachedToTarget`, and is not recursive. It does not follow a frame that changes process.
- **Timeouts.** `Connection::send` has no per-command timeout; only waits on events have a deadline. A command to a crashed renderer could hang. No `targetCrashed` or `detachedFromTarget` is handled.
- **Events.** All events go through one `broadcast` channel of 4096. A lagged receiver skips events, so a missed event becomes a timeout. The lab never enables `Network`, so event volume stayed low.
- **Reconnect.** There is none. On socket end, all pending commands fail with "CDP connection closed".
- **Chrome versions.** Only one browser was tested, Chrome for Testing 153.0.8010.12 ([lab README](../lab/README.md)). No long sessions, no many-page concurrency beyond the declared `maxConcurrency` of 16.

### headless_chrome 1.0.22 (`rust-headless-chrome`)

- **It is blocking.** It uses `tungstenite` and `std::thread` workers. Each command waits on a polling loop with 5 ms sleeps up to `idle_browser_timeout` ([transport/mod.rs](https://github.com/rust-headless-chrome/rust-headless-chrome/blob/0a5c307a85debc450378a1f19e4dac1838d7b22d/src/browser/transport/mod.rs)).
- **It uses the deprecated session mode.** Tabs attach with `flatten: None`, and each target command is wrapped in the deprecated `Target.sendMessageToTarget` ([tab/mod.rs](https://github.com/rust-headless-chrome/rust-headless-chrome/blob/0a5c307a85debc450378a1f19e4dac1838d7b22d/src/browser/tab/mod.rs)).
- **Its protocol is old, and generated with a GPL tool.** The protocol is generated at build time by `auto_generate_cdp` 0.4.6, from `devtools-protocol` commit `bb9208f5` (r1510116, September 3, 2025). The JSON is bundled under the default `offline` feature. Without that feature, the build fetches it from GitHub. `auto_generate_cdp` ships a GPL-3.0 `LICENSE.txt`, and its code templates are emitted into the generated module. That would need a licence review.
- **Activity.** It has 2,952 stars and 144 open issues; the last release was on June 11, 2026.
- **Verdict.** It does not fit a Tokio core or flat sessions.

### A client generated from the official protocol

The official definitions are PDL and JSON in [`ChromeDevTools/devtools-protocol`](https://github.com/ChromeDevTools/devtools-protocol). They roll several times a week, and npm `devtools-protocol` has 1,521 published versions. In Rust there are three ways to generate a client from them:
- **Reuse `chromiumoxide_pdl`.** It is MIT OR Apache-2.0, and Chromiumoxide and Chromey both use it. Its output is the whole protocol (about 111k lines) with strict serde.
- **Generate only the domains and commands Automan uses, with lenient serde.** That means `#[serde(default)]` on structs, a catch-all variant on enums and `#[non_exhaustive]`, pinned to a revision a CI job advances. This keeps the lab's tolerance and adds type checking.
- **Use another generated crate,** such as `cdp-protocol` 0.3.2 (MIT, July 2026, 1,603 downloads over 90 days). It has little adoption, and its revision and tolerance were not inspected.

### WebDriver BiDi crates

- **thirtyfour 0.37.7** (October 8, 2026) has a `bidi` feature since 0.37.0 (May 3, 2026). Its module, about 6.7k lines, covers the BiDi spec modules. It supports `script` sandboxes (named isolated realms), `input`, `network` and `browsingContext`. BiDi is negotiated with `webSocketUrl: true` on a WebDriver session, so a ChromeDriver or GeckoDriver process is required ([bidi/mod.rs](https://docs.rs/crate/thirtyfour/0.37.7/source/src/bidi/mod.rs)).
- **Other crates are under a year old or little used:** `webdriverbidi` 0.2.2 (July 2025), `webdriver-bidi` 0.1.0 (October 2026), `rustenium` 1.1.14 (6.2k downloads over 90 days).
- **On Chrome, BiDi is CDP underneath.** BiDi is `chromium-bidi`, a JavaScript layer translating between BiDi and CDP. ChromeDriver uploads it into the browser as `mapper.js` ([chromium-bidi](https://github.com/GoogleChromeLabs/chromium-bidi), [bidimapper README.chromium](https://chromium.googlesource.com/chromium/src/+/HEAD/third_party/bidimapper/README.chromium)). On Chromium, BiDi therefore adds a hop above CDP rather than replacing it.
- **Firefox has moved to BiDi.** It stopped enabling CDP by default from Firefox 129 and is moving to WebDriver BiDi ([Selenium, February 2025](https://www.selenium.dev/blog/2025/remove-cdp-firefox)).
- **What this means for Automan.** BiDi is relevant to a native Firefox lane, today covered by Playwright with the same guard ([cross-engine](../lab/reports/cross-engine-2026-10-09.md)). It does not change the Chromium choice.

## 5. Fit with the Automan actionability guard

The guard needs the following from the protocol client ([actionability](../lab/docs/browser-actionability.md)):
1. Evaluate a promise-returning script in a chosen frame and context, with `awaitPromise` and `returnByValue`.
2. Arm a capture-phase listener before the gesture and read it back afterwards.
3. Dispatch trusted input to the right session, in the right coordinates.
4. Repeat until a deadline.

All of this is raw `Runtime.evaluate` and `Input.dispatch*`. Every candidate can send these through an escape hatch: `Page::execute`, or `send` in the lab client. What separates them:

- **Isolated world.** Today the guard runs in the main world on the main page. That is the gap to close, whichever option wins. CDP offers two ways:
  - `Page.createIsolatedWorld`, which is not experimental and returns an `executionContextId` per call;
  - `Page.addScriptToEvaluateOnNewDocument` with `worldName` and `runImmediately`, both experimental, plus tracking the context IDs.

  For evaluation, `Runtime.evaluate` accepts `uniqueContextId` (experimental), which the protocol describes as more reliable in a multi-process environment ([js_protocol.pdl](https://github.com/ChromeDevTools/devtools-protocol/blob/c3ed21b573/pdl/js_protocol.pdl)).

  Chromiumoxide and Chromey already create one utility world per frame. But they grant it universal access, and only for frames in the page's own session. The guard's capture listener and `preventDefault` work on the shared DOM events, but moving them into an isolated world must be re-qualified on the lab matrices before it is relied on.
- **OOPIF.** The guard has to run inside the OOPIF's session, and input has to go to that session. The lab client does this. The crates cannot do it through their public API today.
- **Round trips.** The guard costs several commands per action: guard, arm, gesture, disarm. That is why it wins as latency grows. Hidden library round trips, such as page init or `find_element`'s `DOM.getDocument` / `querySelector` / `describeNode`, would eat into that margin.
- **The guard's own contract.** The libraries' `click()` and `find_element()` give none of the guard's guarantees, as already noted on October 7 ([crawl engines](2026-10-07-crawl-engines.md)). Adopting a crate would therefore mean adopting its connection and session management only.

## 6. Implications and open questions for Automan

1. **Proposed answer for D-browser-driver ("Rust CDP library"):** in-house client, with the robustness gaps of section 4 closed:
   - per-command timeouts;
   - fast failure on crash and detach;
   - recursive OOPIF auto-attach, with a map from frame to session that follows process swaps;
   - an isolated world on the main page;
   - lossless per-session event queues;
   - a reconnect strategy: new socket, re-attach, re-create worlds.

   Chromey's handler (crash fast-fail, bounded reader, per-session router) is a useful reference for the robustness part. Its code is MIT OR Apache-2.0, so ideas and code can be reused with attribution.
2. **Protocol pinning as a process.** Pin one `devtools-protocol` revision, generate types for the used domains only, and advance them with a CI job. Run the lab's 78 Chromium cells against Chrome for Testing stable, beta and (for enterprise users) extended stable. With two-week releases, this job is what replaces a library's release cadence.
3. **Re-qualification still owed.** The README line "Qualify Chromey and Chromiumoxide against the guard" ([R&D report §5](../README.md)) can be closed in two ways: by this source review, or by running the three matrices through each crate's `Page::execute`. A run would show whether their page init and event handling change any cell or timing. This note expects no gain in correctness, and some cost in latency on a remote browser.
4. **Owner decisions needed:**
   - whether owning protocol maintenance (about 20 commands now, more as the driver grows) is acceptable versus depending on a single-maintainer fork or a stalled upstream;
   - whether Firefox should later move from Playwright to direct BiDi.
5. **Where to watch:**
   - Chromiumoxide PR #331 (OOPIF) and its first release after April 2026;
   - whether Chromey keeps its protocol current (last bump February 2026) and its maintainer base widens;
   - Chrome's plan to retire non-flat sessions (crbug.com/991325).

## 7. Limitations

- No crate was compiled or run in a browser for this note. The claims about connection loss, OOPIF stubs, init sequences and timeouts come from reading the published 0.9.1, 2.58.2 and 1.0.22 sources, and could differ in behavior.
- Download counts and stars measure use, not correctness.
- The Exa search service was out of credits during this research. Searches used other web tools and, mostly, the primary sources directly: crates.io and GitHub APIs, crate archives, ChromiumDash, `devtools-protocol`.
