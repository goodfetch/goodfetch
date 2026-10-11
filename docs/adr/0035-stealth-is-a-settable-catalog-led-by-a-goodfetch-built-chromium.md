# Stealth is a settable catalog led by a Goodfetch-built Chromium, and only the solver's own logic is a Private component

The `stealth` Access mode turns on a catalog of items, each of which a run can set on or off: a Fingerprint method, generated Identity profiles, headful browsing, human-like input and HTTP fingerprints matched to the browser. Its default Fingerprint method, `engine`, runs a Chromium that Goodfetch builds itself: fingerprint-chromium's open patches compiled on every Chrome Stable major for Linux, Windows and macOS, published reproducibly and signed, as ADR 0030 requires of the shipped browser. Saved identities, block detection and trap-link avoidance serve every Access mode. The Challenge solver has its own switch and escalates deterministic steps first, then AI, then a person. Everything is public except Goodfetch's own solving logic, which becomes a Private component behind the public Challenge solver Plugin kind (ADR 0033), because publishing it is the one item rated High legal exposure. Decided in [Stealth and challenge-solving catalog](https://github.com/Jbmanllr/automan/issues/78), on the evidence of [State of the art in stealth and challenge solving](https://github.com/Jbmanllr/automan/issues/64) and [Legal exposure of stealth and challenge-solving tooling](https://github.com/Jbmanllr/automan/issues/65).

## Considered Options

- **Injecting fingerprints from page JavaScript as the default** (Crawlee with fingerprint-suite, Crawlee Python with browserforge, Spider with `spider_fingerprint`, Steel's open-source image): rejected as the default. Overridden properties are detectable, and detectors now judge consistency across layers ([stealth research](https://github.com/Jbmanllr/automan/blob/research/stealth-state-of-the-art/rnd/research/2026-10-10-stealth-state-of-the-art.md)). It stays selectable as the `js-injection` method, with its warning.
- **Shipping fingerprint-chromium or Clearcote binaries as the default** (both BSD-3, at Chromium 150 on 2026-10-11, when Stable was 155 and 156 was in early release, per [chromiumdash](https://chromiumdash.appspot.com/schedule)): rejected as the default. Version 150 misses about 14 weeks of Chrome security fixes, including CVE-2026-85046, exploited in the wild and fixed in 152.0.7977.82 on 2026-09-03 ([report](https://esecurityplanet.com/threats/news-google-chrome-cve-2026-85046-zero-day)), and their builds cannot be reproduced from public sources. Research found no anti-bot vendor that flags an old version by itself, only versions that contradict the engine ([Akamai](https://techdocs.akamai.com/cloud-security/docs/detection-methods), [Castle](https://blog.castle.io/how-dare-you-trust-the-user-agent-for-detection/)), so staleness costs security and a thinner crowd. They stay selectable, under the security switch below.
- **Closed Chromium builds** (CloakBrowser, BotBrowser) or closed antidetect cores (Multilogin Mimic, GoLogin Orbita, AdsPower SunBrowser, Kameleo Chroma, Dolphin Anty, Octo): rejected as shipped defaults. Their licences forbid redistribution or hosting, they live inside their vendors' apps, and BotBrowser's terms forbid scraping use. A user with a licence runs one through a Browser backend Plugin (ADR 0033).
- **A Goodfetch build on Chrome's Extended Stable channel**, which ships a major every eight weeks with security fixes back-ported ([Chrome blog](https://developer.chrome.com/blog/chrome-two-week-release)): rejected. ADR 0030 already carries every Chrome major in a patch release so that the shipped browser and the stealth material stay current, and the only gain was fewer builds.
- **A Goodfetch build for Linux only**: rejected. On a Windows or macOS desktop the real fingerprint is human, but account rotation runs many identities from one machine, and only an engine-level build gives each its own graphics, canvas, audio and fonts.
- **No Goodfetch build, only Chrome's debugging protocol on a stock Chrome** (close to Kernel, Patchright and nodriver): kept as the `basic` method. The [Emulation domain](https://chromedevtools.github.io/devtools-protocol/tot/Emulation/) sets user agent, client hints, time zone, locale, geolocation, screen and core count, but not graphics, canvas, audio or fonts, so Sessions on one machine stay linkable.
- **Camoufox as the main stealth browser** (Crawlee's template; [Scrapling 0.3.13](https://github.com/D4Vinci/Scrapling/releases) replaced it with patchright): rejected as the main engine. Firefox is about 2.8% of traffic against about 78% for the Chromium family ([StatCounter](https://gs.statcounter.com/browser-market-share), September 2026), Firefox no longer speaks CDP ([Mozilla](https://fxdx.dev/cdp-retirement-in-firefox/)), and some vendors test the JavaScript engine itself ([Camoufox](https://github.com/daijro/camoufox)), so a Firefox can only claim to be Firefox. It stays the Firefox stealth engine.
- **Stealth WebKit or Safari identities**: rejected. No stealth WebKit build exists, Playwright's Linux WebKit is not Safari, and vendors that offer Safari profiles fake them with Chromium.
- **Each stealth option switched on one by one** (the R&D's draft ADR-002): rejected. Opting into `stealth` turns the whole catalog on; each item can then be switched off.
- **Moving every Medium-risk item out of the public framework**: rejected. Only High-risk items leave. Rotate-on-block and the token-service adapters are rated Medium, and Crawlee and Spider publish them; Spider (MIT) even publishes an autonomous solver for Turnstile, reCAPTCHA, GeeTest, Imperva and hCaptcha (`src/features/solvers.rs`, 2.53.9).
- **Renaming `stealth` to a neutral word**: rejected. Spider, Browserbase and Browser Use use the same word; the wording policy below carries the legal weight.

## Consequences

### Fingerprint methods

| Method | Browser | How the fingerprint changes | Warning |
| --- | --- | --- | --- |
| `none` (default outside `stealth`) | Chrome for Testing (ADR 0024) | no change beyond the language and time zone every Access mode matches to the proxy (ADR 0021) | the machine's real fingerprint shows, and Sessions on one machine are linkable |
| `js-injection` | Chrome for Testing | page JavaScript | detectors can see the injected values |
| `basic` | Chrome for Testing | the CDP Emulation domain and launch flags | graphics, canvas, audio and fonts stay the machine's own |
| `engine` (default in `stealth`) | the Goodfetch build | inside the engine | none while the build matches the current Chrome Stable major |
| `engine` with fingerprint-chromium or Clearcote binaries | theirs | inside the engine | behind on security fixes and not reproducible; allowed only with the security switch below off |
| `engine` with Camoufox | Camoufox, a separate download (ADR 0018) | inside the engine, Firefox identities only | a smaller crowd |
| `engine` with the user's own browser (a licensed CloakBrowser, another Chromium build) | through a Browser backend Plugin: a declared download with a digest per platform, allowed per digest by Operator policy and off by default, or a CDP endpoint (ADR 0033) | whatever it does | Goodfetch cannot vouch for the build or its licence |

- The warning appears in the CLI, the SDK results, the Console and the run's report, and the choice is recorded in Provenance and the Audit log (ADRs 0016 and 0023).
- Choosing any method other than `none` makes the Session a `stealth` Session; the items the run set keep their values, and `stealth`'s defaults fill the rest. `identified` refuses every masking item and the Challenge solver: a signed agent that masks spends its key's reputation (ADR 0021).
- **Security switch.** "Only current, reproducible browser builds" joins ADR 0016's security controls. It is on by default; switching it off allows the fingerprint-chromium and Clearcote binaries, is recorded in the Audit log and marks the results, as ADR 0023 requires.

### The Goodfetch build

- It reuses ungoogled-chromium's build pipeline, applies fingerprint-chromium's patch set and sends fixes upstream.
- **Cadence.** It is rebuilt for every Chrome Stable major and Google's security updates, and ships in the patch release that carries that major (ADR 0030). One measured first build on a paid runner, approved by the owner, sets the cost before the cadence starts.
- **Source and distribution.** The repository holds only the patch files and the build scripts (ADR 0032); the pipeline fetches Chromium's source at the tagged version, so Chromium's code is never copied in. The build runs as its own release job on paid large runners. Each build is a signed, attested release asset (ADR 0030), never part of the npm, PyPI or crates.io packages, and reaches users through the digest-checked browser cache (ADR 0024). A patch that no longer applies fails CI, as a broken Private component build does (ADR 0032).
- It always claims its true version, in the user agent, client hints and features; claiming a newer version than the engine is the contradiction detectors look for.
- **Version check.** Before a Session uses a browser build, the engine compares it with Google's published Chrome releases and caches the answer; offline, it uses the release dates shipped with Goodfetch and marks the check unverified. The lookup can be switched off, and that is recorded (ADR 0023). Levels, with the exact gap in versions and weeks: current shows nothing; 1 version behind Stable is info; 2 or 3 behind is a notice; 4 or more behind, or a version Google no longer patches, is a warning that calls the build outdated. Nothing blocks the run. It matters for users who do not update and for every browser Goodfetch does not build.
- HTTP-only `stealth` Sessions send through wreq the TLS ClientHello, HTTP/2 settings, header order and client hints captured from the Goodfetch build by the lab's relay, so HTTP and the browser claim the same version (ADR 0020). The version check grades those profiles too.

### Engines

- Chromium is the main stealth engine, driven over Goodfetch's CDP driver (ADR 0019), with Chrome, Edge and Android Chrome identities.
- Firefox stealth is Camoufox, reached through Playwright's driver and its Juggler protocol, with Firefox identities only. A native WebDriver BiDi path would not replace Juggler for Camoufox, whose isolation depends on it.
- WebKit runs without a stealth mode and never claims to be Safari.

### The rest of the catalog

- **Headful.** `stealth` Sessions show a window by default, on a virtual display on Linux; Browser Use measured 2% of protected sites passed headless and 50% headful on the same IP ([its own benchmark](https://browser-use.com/posts/stealth-benchmark)).
- **Generated identities.** Drawn from a population model per engine (the open data of fpgen, fingerprint-suite and BrowserForge, Apache-2.0), matched to the proxy exit for language, time zone, geolocation and WebRTC address, with the OS following real traffic shares rather than the host's, and applied only inside the engine.
- **Human-like input.** Cursory's recorded mouse movements (LGPL-3.0-or-later) are downloaded at runtime into the browser cache, never compiled in, kept replaceable and listed in the notice inventory; Goodfetch writes the reshaping step from Cursory's published description, not from its code. The Boğaziçi mouse recordings may widen the set once their CC BY 4.0 licence is confirmed. A Rust port of ghost-cursor (MIT) is the fallback when the recordings are absent. Typing with neighbouring-key slips, scrolling and press-and-hold use Goodfetch's own model. Input goes through native CDP events, inside the guard's deadlines (ADR 0019).

### What serves every Access mode

- **Saved identities.** A Session's identity can be saved under a name and reused by later runs: a seed that regenerates the Identity profile exactly, the Session's proxy binding (the same sticky exit where the provider supports it, else the same country and proxy kind), and its cookies and storage. Saved identities live in a store per install, outside any run's state, because a run's key is destroyed with the run (ADR 0015). The store's secret fields are encrypted under a key reached through a Secret reference, and one run at a time may write a saved identity, under an install-wide lock; other runs read it or copy it. A saved identity expires by age and use count, by default Crawlee's 3,000 s and 50 uses. An optional warm-up browses declared pages before work. Reusing it under an inconsistent proxy is refused.
- **Block detection.** Rules seeded from Spider-rs (MIT), wafw00f (BSD-3), identYwaf and Nuclei's `waf-detect` (MIT) record their source and licence per rule. webappanalyzer (GPL-3.0) and Scrapfly's Antibot-Detector (NPOSL-3.0) are watchlists only: a scheduled job opens an issue when they list a vendor Goodfetch lacks, and the rule is written from vendor documentation and Goodfetch's own captures. A rule fires on evidence of a challenge, never on a vendor's mere presence, and is tested against captured pages and the Proving ground. Causes are `challenge`, `captcha`, `rate_limit`, `ip_block` and `decoy`.
- **Trap links.** In the browser tier, the Frontier skips links a person cannot see and invisible `nofollow` links, and a page reached through one, or marked `noindex` and failing its Page check, is classed `decoy`; Cloudflare documents both signals for [AI Labyrinth](https://developers.cloudflare.com/bots/additional-configurations/ai-labyrinth/). HTTP-tier crawls flag such links instead. This avoids traps rather than honouring Site preferences, so ADR 0016 is unaffected. It can be switched off; that is recorded and the results say so (ADR 0023).

### Challenge solver and escalation

`solve_challenges` is off by default. The solver runs in `standard` and `stealth` Sessions, never in `identified` ones, and does not itself require `stealth`. The ladder below completes ADR 0021's escalation and is learned per domain, so later pages start at the step that worked:
1. A challenge page on HTTP moves to the browser tier, in the same Access mode (ADR 0020).
2. The Session waits, with a timeout, for an invisible challenge to clear in the genuine browser.
3. If the run allows `stealth`, a new `stealth` Session opens.
4. If `solve_challenges` is on, deterministic solving: scripted interactive steps (checkbox, slider, press-and-hold), then the configured token services (CapSolver, 2Captcha, CapMonster) and the services that compute the anti-bot sensor data a site expects (such as Hyper Solutions), whose accounts the user brings. Goodfetch never reverse-engineers vendor scripts itself.
5. If `solve_challenges` is on and the run allows AI escalation, vision models solve puzzles, and the Decision provider classifies pages the rules cannot (ADR 0021).
6. A Human pause, if a human channel is configured (ADR 0006).
7. Otherwise a typed stop, `E_CHALLENGE` with the block cause.

- Every step is an entry in the Effect journal and counts in Usage, so money Limits cap paid solves and model calls. Only the Page check declares a challenge passed (ADR 0013).
- Every solver in steps 4 and 5, first-party, adapter or Plugin, follows ADR 0033's Challenge solver contract: it acts only through engine operations or returns a token the engine injects, and its `needs_human` outcome leads to step 6.
- A solver that needs a vision model asks the engine through an operation of the Operation catalog (ADR 0011). The engine makes the call with the run's Model provider and Limits (ADR 0012), so a solver Plugin never holds model credentials.

### Placement

| Item | Legal level if public | Lives in |
| --- | --- | --- |
| Fingerprint methods, the Goodfetch build, identities, input, headful, saved identities | Low with neutral wording | public framework |
| Block detection, trap links, typed stop, the timed wait for invisible challenges | Low | public framework |
| Proxy pools, provider adapters, retiring a blocked Session and opening a new one at the next tier | Medium | public framework, worded as Session health |
| Token-service and sensor-data adapters | Medium | public framework |
| Human pause on a challenge, and the Challenge solver Plugin kind | Low | public framework |
| Goodfetch's own solving logic: scripted interactive steps and vision-model puzzle solving | High (US §1201 trafficking; posting code counts) | Private component |
| Managed solving, managed proxies, hosted stealth browsers | operations | Goodfetch Cloud, designed in its own map |

- **Who loads the Private component.** It ships only as a confined process Plugin signed by Goodfetch (ADR 0033). Goodfetch Cloud loads it in its own fleet; a self-hoster loads it only under a licence granted to a vetted customer (ADR 0018) and an Operator policy that allows it.
- A self-hoster without it keeps the whole ladder except Goodfetch's own solving logic: steps 4 and 5 run through their token-service account, their own Challenge solver Plugin, or a licensed copy of the Private component.
- On this one row Goodfetch publishes less than Spider. Parity holds, because the capability ships in V1 through the Private component and Goodfetch Cloud.
- **Wording policy.** Docs, READMEs, the CLI, the Console, error messages and the agent skills never say "bypass", "undetectable" or "solve any CAPTCHA"; they describe consistency, privacy, testing and authorized automation. Marketing is its own legal prong in the US (§1201(a)(2)(C)), the EU (InfoSoc 6(2)(a)) and France (L.335-3-1 II 4°).
- **Counsel review.** Before V1 is released, counsel reviews the public stealth documentation and the Private component's licence terms, starting from questions 1, 2, 4, 17 and 19 of the legal note. Counsel shapes the wording and the terms; it does not remove the solver.
- **Recheck triggers.** A ruling on SerpApi's renewed motion to dismiss in *Google v. SerpApi* (N.D. Cal. 4:25-cv-10826, undecided on 2026-10-11 after the October 13 hearing was vacated, ECF 60), or on Oxylabs' motion in *Reddit v. SerpApi* (S.D.N.Y. 1:25-cv-08736), reopens the placement table. At the pleading stage, *Reddit* (ECF 104) held masking (proxies, IP shifting, fake user agents, human mimicry) to be circumvention and cited an automatic CAPTCHA bypass as circumvention, and *Google*'s July 20 order found masking and passing on a solved challenge's authorization close to circumvention; neither has ruled on the merits.

### Measurement

Stealth quality is the content of the benchmark's Access family (ADR 0025), on two rungs:
1. Hermetic fixtures in CI, on the Proving ground's local copy: JA4, HTTP/2, header order and client hints kept consistent from HTTP to the browser; self-hosted probes such as CreepJS and BotD; CDP side-effect probes; the version check; and the block-detection corpus.
2. Real vendor protections in front of the Proving ground's online copy, on a domain Goodfetch owns, read back through the vendor's own verdict: Turnstile with Siteverify, the Cloudflare bot score, AWS WAF Bot Control labels, reCAPTCHA scores, Vercel BotID, hCaptcha, and proof-of-work walls (Anubis, ALTCHA, Cap). DataDome, Akamai, HUMAN and Kasada need trials the owner approves one by one.

Every result is reported per Fingerprint method, engine, Access mode and build age, against stock headless and stock headful Chrome as controls. Outside the benchmark, dated live campaigns on targets that authorized them may tune the rules; they are never published or merged with the benchmark, which ADR 0025 limits to the Proving ground.
