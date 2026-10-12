# Consent prompts are refused by default, and a consent wall is the Block cause `consent_required`

Every live browser Session answers each Consent prompt by refusing. A prompt that offers no refusal but leaves the content readable is hidden instead. The engine never accepts unless a person approved the domain. A prompt that offers no refusal and covers the content, or asks for payment instead, is a **consent wall**: it gets the Block cause `consent_required` and is never evaded.

DuckDuckGo's autoconsent recognises the prompts, driven through Goodfetch's own driver in an isolated world, with trusted input, in every frame. Its rules that call a consent vendor's own API run fixed snippets in the page's main world, the engine's only action there by default. A prompt no rule recognises goes to the Decision provider when the run has one.

A consent answer is Session state, not an Effect. Decided in [Built-in page clean-up](https://github.com/Jbmanllr/automan/issues/119) (parity row SA-28). Ads, trackers and the Clean-up rules that carry autoconsent are ADR 0047's.

**Why handle prompts at all.** A prompt left alone is sometimes the only text returned ([firecrawl#78](https://github.com/firecrawl/firecrawl/issues/78), [jina-ai/reader#56](https://github.com/jina-ai/reader/issues/56)). It covers screenshots ([firecrawl#647](https://github.com/firecrawl/firecrawl/issues/647)) and click targets, which the guard of ADR 0019 refuses to click through.

**What the alternatives do to the page.**
- **Hiding without answering** stores nothing, so the prompt returns in every new Session. EasyList documents broken scrolling and dead clicks ([COOKIE_CONSENTS.md](https://github.com/easylist/easylist/blob/master/COOKIE_CONSENTS.md)).
- **Accepting** loads the trackers, and the sites that insist most on it, those with consent walls, set 6.4 times more third-party cookies than sites with ordinary banners ([Rasaii et al., IMC 2023](https://arxiv.org/abs/2310.01108)). It is also not consent in GDPR's sense, which is the data subject's own indication of their wishes ([Art. 4(11)](https://gdpr-info.eu/art-4-gdpr/)).
- **Refusing** has a basis: Art. 21(5) lets a person object "by automated means using technical specifications" ([Art. 21](https://gdpr-info.eu/art-21-gdpr/)).
- **Agents left to themselves accept.** Four of eight browser agents chose "accept all" even when an equal "deny" button was there ([Ukani et al.](https://arxiv.org/html/2512.07725v1)).

**Competitors.**
- **On by default:**
  - Apify's Website Content Crawler (`removeCookieWarnings`) and Browser Use, both through "I don't care about cookies" variants, which accept where they have to.
  - Firecrawl (`blockAds`), whose mechanism is closed.
- **Opt-in:**
  - Crawl4AI's `remove_consent_popups`, which clicks "Accept All" first ([async_crawler_strategy.py](https://github.com/unclecode/crawl4ai/blob/main/crawl4ai/async_crawler_strategy.py)).
  - Crawlee 3's `closeCookieModals`. Crawlee 4 removed it ([crawlee#4073](https://github.com/apify/crawlee/pull/4073)), its issue calling the library it wrapped "largely unmaintained" with "licensing problems" ([crawlee#3987](https://github.com/apify/crawlee/issues/3987)), and its guide now points to autoconsent ([guide](https://github.com/apify/crawlee/blob/master/docs/guides/cookie-modals.mdx)).
- **Refuse:** DuckDuckGo's browsers, which then hide ([help](https://duckduckgo.com/duckduckgo-help-pages/company/how-does-duckduckgo-protect-privacy)), and Ghostery's Never-Consent ([autoconsent.js](https://github.com/ghostery/ghostery-extension/blob/main/src/background/never-consent/autoconsent.js)).
- **Nothing:** Stagehand, Browserbase, ScrapingBee and Spider.

## Considered Options

- **Hiding by default** (Brave, [blog](https://brave.com/privacy-updates/21-blocking-cookie-notices/)): rejected as the default, for the reasons above. It stays the fallback for prompts that offer no refusal and leave the content readable.
- **Accepting, or accepting when needed** ("I don't care about cookies" in Apify and Browser Use, Crawl4AI): rejected. No person consents, and agents that do this are the documented privacy failure.
- **Leaving prompts alone** (Stagehand, Browserbase, ScrapingBee, Spider): rejected. It leaves the prompt in the output and in front of the guard.
- **autoconsent's standalone bundle**, as Crawlee 4's guide injects it: rejected.
  - It runs in the main world and writes page globals ([standalone/content.ts](https://github.com/duckduckgo/autoconsent/blob/main/standalone/content.ts)).
  - It uses the `tier2` heuristic, which clicks the only "accept" when there is no refusal.
  - It clicks with `element.click()`, whose events are `isTrusted: false` ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Event/isTrusted)).
- **"I don't care about cookies" or "I still don't care about cookies"**: rejected. They are GPL-3.0 and accept where they have to.
- **Consent-O-Matic** (MIT, [repository](https://github.com/cavi-au/Consent-O-Matic)) as a second rule source: rejected as over-engineering. A second rule format needs a second runner, for rules that largely overlap autoconsent's larger and more active set, and the Decision provider covers what neither knows. A coverage gap measured by the conformance suite reopens this.
- **The rules of Firefox's Cookie Banner Reduction**: unavailable. The feature shipped only in private windows in Germany ([Firefox 120](https://www.firefox.com/en-US/firefox/120.0/releasenotes/)) and was later turned off everywhere ([bug 1940275](https://bugzilla.mozilla.org/show_bug.cgi?id=1940275)), and its list is archived ([repository](https://github.com/mozilla/cookie-banner-rules-list)).
- **Hiding a consent wall to read what is behind it**: rejected. It gets around the condition the site set for access, and for an "accept or pay" wall it is the paywall ADR 0045 forbids getting around.
- **Treating a consent answer as an Effect**: rejected. Repeating the same answer changes nothing. Journaling it would put a Target fingerprint check (ADR 0009) and possible Reconciliation (ADR 0029) on every page for no protection.

## Consequences

**Handling a prompt.**
- **`consent_handling`** is on by default, in every live browser Session and every Access mode, and can be switched off per run. Provenance then records the prompts it left.
- **The order:**
  1. autoconsent's vendor rules;
  2. its `reject` heuristics, in about ten languages ([heuristic-patterns.ts](https://github.com/duckduckgo/autoconsent/blob/main/lib/heuristic-patterns.ts));
  3. the Decision provider (below);
  4. then either hiding (a prompt with no refusal that leaves the content readable) or `consent_required` (a wall).
- **Hiding** means ADR 0047's: always removed from extracted content, hidden on the live page through the invisible style sheet on Chromium, and on Firefox and WebKit only when the prompt covers an action's target.
- **The Filter lists wait.** While `consent_handling` is on, the rules of the Filter lists aimed at Consent prompts never block, hide or run scriptlets on the live page, and the consent vendors' hosts are never blocked, so the runner always sees the prompt, and the wall test always sees the wall (ADR 0047).
- **Never delaying a page with no prompt.** The engine waits on a page only once a vendor is detected or a pre-hide rule matches. A prompt that appears later is answered when it appears, and ADR 0047 already keeps its text out of extracted content.
- **In the HTTP tier,** which has no layout, the wall test runs on the route's browser sample (ADR 0020).
  - **Changes ADR 0020:** a route whose browser sample showed a consent wall never moves to a tier that skips it. Its pages are `consent_required` in every tier, unless the domain is in `consent_domains`.
  - On a route whose sample showed a refusable prompt or a notice, ADR 0047 removes the prompt from extracted content.
  - A route whose tier the run forced (ADR 0023) still takes one browser sample for the wall test when a browser can start. When none can, its pages record "no wall test" in Provenance, and their prompts are left in extracted content rather than removed.
  - A server-side consent page, such as Google's `consent.google.com` or Yahoo's `guce`:
    - **with a refusal path,** is answered once through the Session's browser. Browser-fetch keeps the Session's identity (ADR 0020), and later HTTP fetches carry its cookies.
    - **without one,** or when no browser can be started for the Session, is `consent_required`.

**The runner.**
- **Where it comes from.** autoconsent (MPL-2.0, [repository](https://github.com/duckduckgo/autoconsent)) ships its library and rules inside the Clean-up rules (ADR 0047) and is never fetched from npm at run time.
- **Where it runs.**
  - On Chromium, in an isolated world in every frame, cross-origin iframes included, through ADR 0019's out-of-process iframe tracking.
  - On Firefox and WebKit, in Playwright's utility world.
  - It is set up again on every new document and every route change of a single-page app.
  - It runs wherever the Session's browser is: in the Browser host for a remote browser (ADR 0022), and on a third-party CDP browser whose isolated worlds and input the engine controls. Lightpanda has no layout for the wall test, but ADR 0020 picks it only when its sample matches the browser's, so a walled route never uses it; on its pages prompts are only removed from extracted content.
- **How it reports to the engine.** Through `Runtime.addBinding`, named with the isolated world's own unique name (`executionContextName`; the `executionContextId` form is deprecated) and added again on every new isolated world, since with `Runtime` off V8 does not carry named bindings to new contexts. Its `bindingCalled` events are sent without `Runtime` being enabled ([v8-runtime-agent-impl.cc](https://chromium.googlesource.com/v8/v8/+/main/src/inspector/v8-runtime-agent-impl.cc)), so nothing polls and nothing appears in the page's own world.
- **How it clicks.**
  - Clicks and keys go through the guard's trusted input on all three browsers, never as untrusted `element.click()` events.
  - autoconsent takes its DOM actions from its public `domActions` field ([web.ts](https://github.com/duckduckgo/autoconsent/blob/main/lib/web.ts)), typed as its `DomActions` class. Goodfetch supplies a subclass that sends clicks to the guard, in its own code under the framework's licence, and autoconsent's files stay unmodified (ADR 0018).
  - Five coded vendors (Admiral, Conversant, Sourcepoint's frame, TikTok and UniConsent) call `element.click()` directly. An isolated world has its own JavaScript prototypes, so the runner replaces `HTMLElement.prototype.click` there, invisibly to the page, and those calls reach the guard too. On Firefox and WebKit, where that cannot be confirmed for Playwright's utility world, those vendors are switched off through autoconsent's `disabledCmps`.
  - Seven of the main-world snippets click from the page's own world (`EVAL_COOKIE_POPUP_CLOSE`, `EVAL_COOKIEFIRST_1`, `EVAL_ETSY_1`, `EVAL_IUBENDA_0`, `EVAL_MICROSOFT_0`, `EVAL_MICROSOFT_1`, `EVAL_WAITROSE_0`). The rules that use them are switched off through `disabledCmps`, and their prompts go to the heuristics and the Decision provider. Goodfetch contributes upstream a version that clicks through `domActions`, and the conformance suite measures what the switched-off rules cost in coverage.
- **Heuristics.** `reject` only, never `tier1` or `tier2`. A notice with no choice is hidden, never acknowledged.
- **Pre-hiding** (hiding a known prompt before it flashes) uses ADR 0047's live style sheet on Chromium, and is undone before the wall test, so a wall is always judged as the site shows it.
- **The main world.**
  - **Which rules.** About 50 call the consent vendor's own JavaScript API, which an isolated world cannot reach: 45 rule files with `eval` steps, and five vendors coded in TypeScript (OneTrust, Cookiebot, consentmanager, Klaro and TrustArc). Each makes a few calls, up to seven, of the fixed snippets in autoconsent's `eval-snippets.ts`.
  - **How on Chromium.** `DOM.resolveNode`, given no context, resolves in the frame's main world ([resolve_node.cc](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/inspector/resolve_node.cc)), and `Runtime.callFunctionOn` then runs the snippet there. V8 refuses only `compileScript`, `runScript` and two settings calls while `Runtime` is off, never `callFunctionOn`, so `Runtime` stays off, and the snippets leave no global behind.
  - **How on Firefox and WebKit.** Through Playwright's main-world evaluation.
  - **When the call can't be made,** the prompt goes to the wall test: hidden when the content is readable, a wall otherwise.
  - **Changes ADR 0019:** by default, these calls and the TCF read below are the engine's only actions in a page's main world. The opt-in exceptions are ADR 0047's page scriptlets, ADR 0035's `js-injection` Fingerprint method, and the user's own `evaluate` and init scripts, each marked in Provenance. ADR 0047's on-demand hiding on Firefox and WebKit adds a visible `<style>` element, which is not main-world code but does stay in the page, and Provenance marks it too. **Changes ADR 0031:** its rejection of an in-page recorder stands. This exception covers calls into the consent vendor's own API, not code that stays in the page.

**Prompts no rule recognises.**
- **Which prompts.** Prompts the heuristics detect but no rule can answer.
- **Who is asked.** When the run has a Decision provider, it is asked about them, as about the soft blocks of ADR 0021. Without one, the wall test below decides: a prompt that leaves the content readable is hidden, and any other is a wall.
- **What it may choose.**
  - The engine offers only the prompt's buttons, leaving out any that match autoconsent's accept patterns.
  - The model picks the refusal, "close without answering", or abstains.
  - The prompt's text is wrapped as untrusted page content (ADR 0014).
- **Checking the outcome.**
  - Where the page exposes a TCF consent string, the engine reads it through `__tcfapi` after the click.
  - A consent given by mistake is reported, and the Session's consent cookies are cleared.
  - Elsewhere, Provenance marks the answer `unverified`, since a model alone never judges an outcome (ADR 0013).
- **Accounting.** The call is journaled and counted against Limits (ADR 0012).

**Consent walls.**
- **The test.** A prompt is a wall when it offers no refusal and either covers the main content or offers payment or a subscription instead.
- **Who decides.** Rules decide. The Decision provider is asked only when they cannot. When it abstains, the prompt is treated as a wall: a notice wrongly blocked costs a page, while a wall wrongly hidden gets around an access condition.
- **Changes ADR 0035:** `consent_required` joins the Block causes. The quoted price of an "accept or pay" wall, when the page gives one, is recorded in Provenance.
- **Changes ADR 0021:** it never moves a domain up a proxy tier, never retires a Session to rotate, and is never retried. On a domain in `consent_domains`, the engine accepts instead of blocking.
- **Changes ADR 0005:** a finished run that left pages in scope at `consent_required` reports Completeness `unverified` and lists them, as for `payment_required`.
- **Changes ADR 0046:** these responses are never stored in the Response cache, like every Block cause.
- **`consent_domains`** lists the registrable domains (ADR 0004) on which the engine accepts.
  - **Who widens it.** Only Operator policy, user and project configuration, or a person's answer in the Console or an interactive CLI prompt. On a Daemon with accounts, only the operator role, in Operator policy, as for `pay_domains` (ADR 0045).
  - **Changes ADR 0028:** the Recipe and run layers can only narrow it.
  - **Changes ADR 0011:** no agent surface widens it.
  - **Changes ADR 0031:** widening it goes through step-up.
- **`on_consent_wall`.**
  - `report` (default) leaves the page at `consent_required`.
  - `ask` raises a Human pause with a Notification. The person answers for this run, or "always for this domain", which writes `consent_domains` in user configuration. On a Daemon with accounts, only a person with the operator role can give either answer, and "always" is stored in Operator policy, as ADR 0045 does for `pay_domains`.
  - The Console's "Needs you" view lists the domains that showed a wall and offers to add them to `consent_domains`.
- **Changes ADR 0014:** `consent_domains` is the only allowance for "accepting terms" on a Consent prompt; a run's general allowance for accepting terms never covers one.
  - An Agent that clicks "accept" on a domain outside it asks for confirmation.
  - Agents rarely meet a prompt, because the engine answers prompts before building an Agent's observation.
- **Audit log.** Every acceptance goes to it, naming the domain and the approval it came from.

**A consent answer is Session state.**
- **Where it lives.** With the Session's cookies and storage, so a saved identity carries it (ADR 0035). A new or resumed Session simply answers again.
- **Acceptances stay in their scope.** The engine records which cookies and storage each answer set. Saving an identity drops those of acceptances that the saving run's `consent_domains` does not cover, such as a person's own click or an "accept for this run" answer, so a later run never inherits them.
- **Provenance per page** records:
  - the action: refused, hidden, accepted, accepted by a person, left, or none;
  - the rule that matched;
  - whether the outcome was verified.
- **Changes ADR 0009:**
  - The engine answers a prompt before each Plan step and its Target fingerprint check, so consent never becomes a step and never causes Drift.
  - The Console recorder leaves out every click or key activation (Tab, Enter, Space) on a prompt a rule recognises, and keeps the person's choice as Session state.
  - When a recording is saved, the recorder lists the steps that dismissed an overlay no rule recognised, and the person keeps or drops each one.
- **Changes ADR 0031:**
  - The runner's clicks stop at "Take control" and resume at "Hand back". These are events, so nothing polls.
  - Blocking, live hiding and extraction-side removal (ADR 0047) keep running.
  - Whatever the person answers, an acceptance included, is recorded in Provenance and the Audit log as theirs, for that Session only, and never widens `consent_domains`. On a Daemon with accounts, any member holding control may answer this way, since it is their own act in the page; widening `consent_domains` stays the operator's. A holder of an embedded Live view grant, who is not a Goodfetch person, is recorded under the grant (ADR 0031).
- **Raw CDP** (ADR 0017). The runner's clicks also stop while a raw CDP client is attached, since the engine cannot tell what that client is doing; ADR 0047 says what blocking and hiding do then.

**Global Privacy Control.**
- **The setting.** `gpc` is `auto` by default, which follows the Identity profile. The `Sec-GPC` header and `navigator.globalPrivacyControl` are sent only when the browser the Session presents would send them.
- **Which browsers send it.**
  - Brave and DuckDuckGo send them by default ([GPC](https://globalprivacycontrol.org/)).
  - Firefox sends them in private windows ([Firefox 120](https://www.firefox.com/en-US/firefox/120.0/releasenotes/)).
  - Chrome, Edge and Safari never send them ([browser-compat-data](https://github.com/mdn/browser-compat-data/blob/main/http/headers/Sec-GPC.json)).

  So `auto` never sends them for a stock Chrome identity.
- **Forcing it.** `gpc = on` sends them anyway, with a warning that a signal the presented browser never sends breaks the consistency `standard` keeps (ADR 0021).
  - **Firefox and Camoufox** use their own preference.
  - **Chromium** gets the header through the interception, and the warning also says that the JavaScript property is absent.
  - **The HTTP tier** sends `Sec-GPC` exactly when the Session's browser would, since it presents the same identity.

**Proof.**
- **The conformance suite** runs on the Proving ground (ADR 0025) on all three browsers. Its prompts are modelled on OneTrust, Cookiebot, Didomi, Usercentrics, Sourcepoint, Quantcast, TrustArc, Google's consent page and Axeptio in shadow DOM. It covers:
  - "accept or pay" walls;
  - notices, which must be hidden, against walls, which must be blocked;
  - prompts that appear late or on a route change;
  - prompts in cross-origin iframes;
  - prompts in more than ten languages;
  - server-side consent pages in the HTTP tier;
  - the Live view and recorder scenarios above.
- **Real vendors.** The online Proving ground uses real consent vendors' free tiers where their terms allow. The owner approves any paid one.
