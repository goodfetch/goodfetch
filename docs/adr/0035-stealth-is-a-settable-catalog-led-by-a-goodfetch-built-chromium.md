# Stealth is a settable catalog led by a Goodfetch-built Chromium, and only the solver's own logic is a Private component

The `stealth` Access mode turns on a catalog of items, each of which a run can set on or off: a Fingerprint method, generated Identity profiles, headful browsing, human-like input, HTTP fingerprints matched to the browser, saved identities and block detection. Its default Fingerprint method, `engine`, runs a Chromium that Goodfetch builds itself: fingerprint-chromium's open patches compiled on Chrome's Extended Stable channel for Linux, Windows and macOS, published reproducibly and signed. The Challenge solver has its own switch and escalates deterministic steps first, then AI, then a person. Everything is public except Goodfetch's own solving logic, which becomes a Private component behind the public Challenge solver Plugin kind (ADR 0033), because publishing it is the one item rated High legal exposure. Decided in [Stealth and challenge-solving catalog](https://github.com/Jbmanllr/automan/issues/78), on the evidence of [State of the art in stealth and challenge solving](https://github.com/Jbmanllr/automan/issues/64) and [Legal exposure of stealth and challenge-solving tooling](https://github.com/Jbmanllr/automan/issues/65).

## Considered Options

- **Injecting fingerprints from page JavaScript as the default** (Crawlee with fingerprint-suite, Crawlee Python with browserforge, Spider with `spider_fingerprint`, Steel's open-source image): rejected as the default. Overridden properties are detectable, and detectors now judge consistency across layers ([stealth research](https://github.com/Jbmanllr/automan/blob/research/stealth-state-of-the-art/rnd/research/2026-10-10-stealth-state-of-the-art.md)). It stays selectable as the `js-injection` method, with its warning.
- **Shipping fingerprint-chromium or Clearcote binaries as the default** (both BSD-3, at Chromium 150 while Stable is 155): rejected as the default. Version 150 is about 14 weeks behind Chrome's security fixes, including a flaw exploited in the wild that Chrome 152 fixed, and their builds cannot be reproduced from public sources. Research found no anti-bot vendor that flags an old version by itself, only versions that contradict the engine, so staleness is a security cost and a thinner crowd, not a detection rule. They stay selectable, recorded as a security switch-off (ADR 0016).
- **Closed Chromium builds** (CloakBrowser, BotBrowser) or closed antidetect cores (Multilogin Mimic, GoLogin Orbita, AdsPower SunBrowser, Kameleo Chroma, Dolphin Anty, Octo): rejected as shipped defaults. Their licences forbid redistribution or hosting, they live inside their vendors' apps, and BotBrowser's terms forbid scraping use. A user with a licence runs one through a Browser backend Plugin (ADR 0033).
- **A Goodfetch build that follows Chrome Stable within days**: rejected for Extended Stable. Stable now ships every two weeks; Extended Stable ships every eight with Google's security fixes back-ported, and its versions stay inside the mainstream crowd. Both keep the browser honest about its version.
- **A Goodfetch build for Linux only**: rejected. On a Windows or macOS desktop the real fingerprint is human, but account rotation runs many identities from one machine, and only an engine-level build gives each its own graphics, canvas, audio and fonts.
- **No Goodfetch build, only Chrome's debugging protocol on a stock Chrome** (close to Kernel, Patchright and nodriver): kept as the `basic` method. The Emulation domain sets user agent, client hints, time zone, locale, geolocation, screen and core count, but not graphics, canvas, audio or fonts, so Sessions on one machine stay linkable.
- **Camoufox as the main stealth browser** (Crawlee's template; Scrapling dropped it in 0.3.13 for patchright): rejected as the main engine. Firefox is about 2.8% of traffic against about 78% for the Chromium family (StatCounter, September 2026), Firefox no longer speaks CDP, and some vendors test the JavaScript engine itself, so a Firefox can only claim to be Firefox. It stays the Firefox stealth engine.
- **Stealth WebKit or Safari identities**: rejected. No stealth WebKit build exists, Playwright's Linux WebKit is not Safari, and vendors that offer Safari profiles fake them with Chromium.
- **Each stealth option switched on one by one** (the R&D's draft ADR-002): rejected. Opting into `stealth` turns the whole catalog on; each item can then be switched off.
- **Moving every Medium-risk item out of the public framework**: rejected. Only High-risk items leave. Rotate-on-block and the token-service adapters are rated Medium, and Crawlee and Spider publish them; Spider (MIT) even publishes an autonomous solver for Turnstile, reCAPTCHA, GeeTest, Imperva and hCaptcha (`src/features/solvers.rs`, 2.53.9).
- **Renaming `stealth` to a neutral word**: rejected. Spider, Browserbase and Browser Use use the same word; the wording policy below carries the legal weight.

## Consequences

### Fingerprint methods

| Method | Browser | How the fingerprint changes | Warning |
| --- | --- | --- | --- |
| `none` (default outside `stealth`) | Chrome for Testing (ADR 0024) | not at all | the machine's real fingerprint shows, and Sessions on one machine are linkable |
| `js-injection` | Chrome for Testing | page JavaScript | detectors can see the injected values |
| `basic` | Chrome for Testing | the CDP Emulation domain and launch flags | graphics, canvas, audio and fonts stay the machine's own |
| `engine` (default in `stealth`) | the Goodfetch build | inside the engine | none while the build is current |
| `engine` with fingerprint-chromium or Clearcote binaries | theirs | inside the engine | behind on security fixes, not verifiable; recorded as a security switch-off |
| `engine` with Camoufox | Camoufox, a separate download (ADR 0018) | inside the engine, Firefox identities only | a smaller crowd |
| `engine` with the user's own binary | through a Browser backend Plugin | whatever it does | Goodfetch cannot verify the binary or its licence |

- The warning appears in the CLI, the SDK results, the Console and the run's report, and the choice is recorded in Provenance and the Audit log (ADRs 0016 and 0023).
- Choosing any method other than `none` makes the Session a `stealth` Session. `identified` refuses every masking item and the Challenge solver: a signed agent that masks spends its key's reputation (ADR 0021).

### The Goodfetch build

- It reuses ungoogled-chromium's build pipeline, applies fingerprint-chromium's patch set, sends fixes upstream, and is published to the digest-checked browser cache (ADR 0024).
- It rebuilds for each Extended Stable major and each of Google's security point releases on that channel. One measured first build on a paid runner, approved by the owner, sets the cost before the cadence is committed.
- It always claims its true version, in the user agent, client hints and features; claiming a newer version than the engine is the contradiction detectors look for.
- **Version check.** Before a Session uses a browser build, the engine compares it with Google's published Chrome releases and caches the answer; offline, it uses the release dates shipped with Goodfetch and marks the check unverified. The lookup can be switched off, and that is recorded (ADR 0023). Levels, with the exact gap in versions and weeks: current shows nothing; 1 version behind Stable is info; 2 or 3 behind is a notice; 4 or more behind, or a version Google no longer patches, is a warning that calls the build outdated. Nothing blocks the run.
- HTTP-only `stealth` Sessions send through wreq the TLS ClientHello, HTTP/2 settings, header order and client hints captured from the Goodfetch build by the lab's relay, so HTTP and the browser claim the same version (ADR 0020). The version check grades those profiles too.

### Engines

- Chromium is the main stealth engine, driven over Goodfetch's CDP driver (ADR 0019), with Chrome, Edge and Android Chrome identities.
- Firefox stealth is Camoufox, reached through Playwright's driver and its Juggler protocol, with Firefox identities only. A native WebDriver BiDi path would not replace Juggler for Camoufox, whose isolation depends on it.
- WebKit runs without a stealth mode and never claims to be Safari.

### The rest of the catalog

- **Headful.** `stealth` Sessions show a window by default, on a virtual display on Linux; Browser Use measured 2% of protected sites passed headless and 50% headful on the same IP (its own benchmark).
- **Generated identities.** Drawn from a population model per engine (the open data of fpgen, fingerprint-suite and BrowserForge, Apache-2.0), matched to the proxy exit for language, time zone, geolocation and WebRTC address, with the OS following real traffic shares rather than the host's, and applied only inside the engine.
- **Human-like input.** Cursory's recorded mouse movements (LGPL-3.0-or-later) are downloaded at runtime into the browser cache, never compiled in, kept replaceable and listed in the notice inventory; Goodfetch writes the reshaping step from Cursory's published description, not from its code. The CC BY 4.0 Boğaziçi recordings may widen the set. A Rust port of ghost-cursor (MIT) is the fallback when the recordings are absent. Typing with neighbouring-key slips, scrolling and press-and-hold use Goodfetch's own model. Input goes through native CDP events, inside the guard's deadlines (ADR 0019).
- **Saved identities.** A saved Identity profile holds a seed that regenerates its fingerprint exactly, its proxy binding (the same sticky exit where the provider supports it, else the same country and proxy kind), and its cookies and storage, encrypted under ADR 0015. One run writes it at a time, under a lease (ADR 0003); others read it or copy it. It expires by age and use count, by default Crawlee's 3,000 s and 50 uses. An optional warm-up browses declared pages before work. Reusing it under an inconsistent proxy is refused.
- **Block detection.** Rules seeded from Spider-rs (MIT), wafw00f (BSD-3), identYwaf and Nuclei's `waf-detect` (MIT) record their source and licence per rule. webappanalyzer (GPL-3.0) and Scrapfly's Antibot-Detector (NPOSL-3.0) are watchlists only: a scheduled job opens an issue when they list a vendor Goodfetch lacks, and the rule is written from vendor documentation and Goodfetch's own captures. A rule fires on evidence of a challenge, never on a vendor's mere presence, and is tested against captured pages and the Proving ground. Causes are `challenge`, `captcha`, `rate_limit`, `ip_block` and `decoy`.
- **Trap links.** In the browser tier, the Frontier skips links a person cannot see and invisible `nofollow` links, and a page reached through one, or marked `noindex` and failing its Page check, is classed `decoy`; Cloudflare documents both signals for AI Labyrinth. HTTP-tier crawls flag such links instead. This avoids traps rather than honouring Site preferences, so ADR 0016 is unaffected, and it can be switched off (ADR 0023).

### Challenge solver and escalation

`solve_challenges` is off by default. The ladder below completes ADR 0021's escalation and is learned per domain, so later pages start at the step that worked:
1. A challenge page on HTTP moves to the browser tier, in the same Access mode (ADR 0020).
2. The Session waits, with a timeout, for an invisible challenge to clear in the genuine browser.
3. If the run allows `stealth`, a new `stealth` Session opens.
4. If `solve_challenges` is on, deterministic solving: scripted interactive steps (checkbox, slider, press-and-hold), then the configured token-service adapters (CapSolver, 2Captcha, CapMonster) and payload-generation adapters (the Hyper Solutions class), whose accounts the user brings. Goodfetch never reverse-engineers vendor scripts itself.
5. If the run allows AI escalation, vision models solve puzzles through the engine's model calls (ADR 0012), and the Decision provider classifies pages the rules cannot (ADR 0021).
6. A Human pause, if a human channel is configured (ADR 0006).
7. Otherwise a typed stop, `E_CHALLENGE` with the block cause.

Every step is an entry in the Effect journal and counts in Usage, so money Limits cap paid solves and model calls. Only the Page check declares a challenge passed (ADR 0013).

### Placement

| Item | Legal level if public | Lives in |
| --- | --- | --- |
| Fingerprint methods, the Goodfetch build, identities, input, headful, saved identities | Low with neutral wording | public framework |
| Block detection, typed stop, the timed wait for invisible challenges | Low | public framework |
| Proxy pools, provider adapters, retiring a blocked Session and opening a new one at the next tier | Medium | public framework, worded as Session health |
| Token-service and payload-generation adapters | Medium | public framework |
| Human pause on a challenge, and the Challenge solver Plugin kind | Low | public framework |
| Goodfetch's own solving logic: scripted interactive steps and vision-model puzzle solving | High (US §1201 trafficking; posting code counts) | Private component, shipped as a confined process Plugin (ADR 0033) |
| Managed solving, managed proxies, hosted stealth browsers | operations | Goodfetch Cloud, designed in its own map |

- A self-hoster without the Private component keeps the whole ladder except Goodfetch's own solving logic: steps 4 and 5 run through their token-service account, their own Challenge solver Plugin, or a licensed copy of the Private component.
- On this one row Goodfetch publishes less than Spider. Parity holds, because the capability ships in V1 through the Private component and Goodfetch Cloud.
- **Wording policy.** Docs, READMEs, the CLI, the Console, error messages and the agent skills never say "bypass", "undetectable" or "solve any CAPTCHA"; they describe consistency, privacy, testing and authorized automation. Marketing is its own legal prong in the US (§1201(a)(2)(C)), the EU (InfoSoc 6(2)(a)) and France (L.335-3-1 II 4°).
- **Counsel review.** Before V1 is released, counsel reviews the public stealth documentation and the Private component's licence terms, starting from questions 1, 2, 4, 17 and 19 of the legal note. Counsel shapes the wording and the terms; it does not remove the solver.
- **Recheck triggers.** A ruling on SerpApi's renewed motion to dismiss in *Google v. SerpApi* (N.D. Cal. 4:25-cv-10826, undecided on 2026-10-11 after the October 13 hearing was vacated, ECF 60), or on Oxylabs' motion in *Reddit v. SerpApi* (S.D.N.Y. 1:25-cv-08736), reopens the placement table. Both courts tied circumvention, as pleaded, to masking (proxies, IP shifting, fake user agents, human mimicry) rather than to CAPTCHA solving.

### Measurement

Stealth quality is measured on three rungs, whose pass rates and protocol belong to [V1 benchmark protocol](https://github.com/Jbmanllr/automan/issues/79):
1. Hermetic fixtures in CI: JA4, HTTP/2, header order and client hints kept consistent from HTTP to the browser; self-hosted probes such as CreepJS and BotD; CDP side-effect probes; the version check.
2. Real vendor protections on domains Goodfetch owns, read back through the vendor's own verdict: Turnstile with Siteverify, the Cloudflare bot score, AWS WAF Bot Control labels, reCAPTCHA scores, Vercel BotID, hCaptcha, and proof-of-work walls (Anubis, ALTCHA, Cap). DataDome, Akamai, HUMAN and Kasada need trials the owner approves one by one. The Proving ground hosts the pages these protections guard.
3. Dated live campaigns on third-party sites, kept separate and never merged with rungs 1 and 2.
