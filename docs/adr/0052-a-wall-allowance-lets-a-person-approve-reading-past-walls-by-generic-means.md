# A Wall allowance lets a person approve reading past Walls, by generic means only

A **Wall** is an overlay a site draws over its page as the condition for reading the content: a paywall, a consent wall or an ad-blocker wall. ADRs 0045, 0047 and 0048 never get past one. V1 adds an opt-in, the **Wall allowance**: `wall_domains`, a list of registrable domains that only a person approves, as with `pay_domains` and `consent_domains`. On those domains, the engine reads past Walls instead of stopping at them, using only generic means:
- it reads the text the server sent and hides what covers it;
- it lets the Filter lists block a Wall's script;
- it starts one fresh Session per page when a paywall leaves the served page without its main content.

Consent walls are read past without consenting; accepting still needs `consent_domains`. A demand for payment (`payment_required`) is not a Wall and stays as ADR 0045 has it.

Goodfetch's own code never names a paywall vendor, a search crawler or an archive as a way past a Wall. Posing as Googlebot, spoofing a referrer and fetching archived copies stay reachable only through the user's own Hooks, Plugins and Filter lists. The official Plugin index does not list Plugins presented that way. Decided in [Opt-in paywall and consent-wall circumvention](https://github.com/Jbmanllr/automan/issues/155), on the evidence of [Legal exposure of paywall and consent-wall circumvention](https://github.com/Jbmanllr/automan/issues/154) ([findings](https://github.com/Jbmanllr/automan/blob/research/paywall-consent-circumvention/rnd/research/2026-10-12-paywall-consent-circumvention-legal.md)).

**Why generic, and why public.** The legal research found that risk follows naming, not capability. Every takedown it found hit a tool presented as a way past paywalls:
- one §1201 notice disabled 3,879 GitHub repositories of Bypass Paywalls Clean ([notice](https://github.com/github/dmca/blob/master/2024/08/2024-08-09-news-media-alliance.md));
- 12ft.io's host locked it;
- smry.ai, which used to pose as Googlebot and fetch archives, retreated and now tells users not to use it on hard paywalls.

Generic means rate Low at every level in the public framework and in a user's own Plugin (US, EU and France). Clearing one's own cookies rates Low too, as long as it is not presented as a meter reset; whether doing it systematically at scale changes that is one of the research's open questions for counsel. Named means (a vendor list for blocking Wall scripts, a Googlebot or meter-reset preset, an archive fallback) rate Medium to High. A Private component lowers visibility, not exposure: it still provides the tool, and in the EU and France it loses the "other commercial uses" argument (*Nintendo v. PC Box*).

**Competitors.**
- No mainstream scraping framework ships a named feature for blocking Wall scripts, disguising identity or fetching archives.
- Firecrawl treats "paywalls block the actual content" as a successful scrape and does not escalate ([engpicker.ts](https://github.com/firecrawl/firecrawl/blob/main/apps/api/src/lib/engpicker.ts)).
- One third-party Apify actor offers `useGoogleBotHeaders` ([actor](https://apify.com/lukaskrivka/article-extractor-smart)).
- uBlock Origin's uAssets ([readme](https://github.com/uBlockOrigin/uAssets)) and AdGuard ([filter policy](https://adguard.com/kb/general/ad-filtering/filter-policy/)) refuse rules that get past paywalls.
- Accepting consent automatically is common (Apify, Browser Use, Crawl4AI; ADR 0048).

No parity row and none of the 24 ranked pains asks for getting past a paywall (ADR 0037).

## Considered Options

- **Named presets as a Private component** (ADR 0018's usual route for exposed capabilities): rejected. The research rates a Private component no lower than the public framework for blocking Wall scripts (Medium to High), identity disguise (High) and archives (Medium), and it is a second codebase to sign and test. ADR 0018's rule decides where a capability lives once it is built; here the named presets are not built at all, because the generic means already reach every level.
- **Named presets in the public framework**: rejected. This is Bypass Paywalls Clean's profile, and a single notice to GitHub could disable the repository and every fork of it.
- **A per-run switch, like ADR 0016's compliance controls**: rejected. An agent could switch it on for itself, and a published Recipe could ship with it on. A Wall is a condition the site set for access, which is the reason ADRs 0045 and 0048 made their approvals person-only.
- **Letting the allowance cover `payment_required` too**, so the engine escalates, rotates or resets past a 402 or a paywall redirect: rejected. That is the masking the *Reddit* case rests on ([Legal exposure of stealth and challenge-solving tooling](https://github.com/Jbmanllr/automan/issues/65)), and a site that asks to be paid already has an answer, the Paid fetch (ADR 0045).
- **The allowance implying acceptance of consent walls**: rejected. Reading past a consent wall gives no consent and loads no trackers. Accepting gives consent in the user's name, which ADR 0048 keeps as a separate approval (`consent_domains`).
- **Unlimited fresh-Session resets, in any Session**: rejected. Resetting a logged-in Session logs the account out and runs against its terms. Resetting an `identified` Session contradicts the identity it signs (ADR 0045). Unlimited resets can loop.
- **Naming it `bypass_domains` or `circumvention_domains`**: rejected. The first is the "named" profile the research rates higher; the second is the legal term of art and reads as an admission. The docs describe what the allowance does in plain words.
- **Listing third-party Plugins presented as a way past Walls in the Plugin index when they pass conformance**: rejected. The index is a signed file in Goodfetch's own repository, so listing a Plugin would make Goodfetch its distributor. That reopens, through third parties, the exposure that keeping named means out of Goodfetch's code avoids.

## Consequences

**Who allows it.**
- `wall_domains` lists registrable domains, matched as in ADR 0004. `*` is accepted only when written explicitly. It is empty by default.
- It is taken only from Operator policy, user and project configuration, or a person's answer in the Console or an interactive CLI prompt, as for `pay_domains` (ADR 0045).
  - On a Daemon with accounts, only the operator role widens it, and it is stored in Operator policy.
  - **Changes ADR 0028:** the Recipe and run layers can only narrow it.
  - **Changes ADR 0011:** no MCP tool, code-mode function, SDK method or `--json` CLI operation widens it.
  - **Changes ADR 0031:** on a Daemon with accounts, widening it goes through step-up.
  - Operator policy can lock it empty.
- **A warning at each widening.** The Console, the interactive CLI prompt, and the first configuration load that adds a domain show the person, in plain words, what they take on: copyright in content behind a paywall, the site's terms, and GDPR where consent is later accepted. The warning goes to the Audit log with the approval. There is no confirmation loop.
- **`on_wall`.**
  - `report` (default): a Wall on a domain outside `wall_domains` is handled as ADRs 0047 and 0048 handle it today.
  - `ask`: raises a Human pause with a Notification. The person answers "for this run" or "always for this domain"; the second writes `wall_domains` in user configuration, or, on a Daemon with accounts, in Operator policy, and only a person with the operator role can give either answer there.
  - When a consent wall would raise both this pause and ADR 0048's `on_consent_wall = ask`, the engine raises one Human pause offering both answers: read past it (`wall_domains`) or accept it (`consent_domains`).
  - The Console's "Needs you" view lists the domains that showed a Wall and offers to add them to `wall_domains`.
- **Changes ADR 0016:** never getting past a Wall is still not one of the compliance controls a run switches. The Wall allowance is its only exception, and only a person grants it, for the domains that person lists.

**What the engine does on an allowed domain.** All of it is generic: it acts on any Wall the detection rules of ADRs 0047 and 0048 find, and no part of it is chosen by vendor name.
- **Reading what was served.**
  - The engine keeps the text the server sent.
  - It hides, on the live page, the element that covers the main content, judged by the covering test of ADR 0048's wall test. On Chromium it uses ADR 0047's invisible style sheet; on Firefox and WebKit it hides on demand.
  - It removes that element from extracted content in every tier, and lifts a scroll lock the Wall left, from the isolated world, as ADR 0048 does after a refusal.
  - In the HTTP tier, which has no layout, the route's browser sample decides what covers the main content and whether the served page holds it, as it decides the wall test (ADR 0048).
  - **Changes ADR 0047:** the "never hidden" rule for paywalls, consent walls and ad-blocker walls, and the protection of elements covering a section marked `isAccessibleForFree: false`, do not apply on these domains. Sections the page declares paid are still never removed, since they are the content.
  - **Changes ADR 0045:** on these domains, its rule that no cosmetic rule hides a paywall vendor's overlay or an element covering a declared paid section does not apply, nor does its shielding of paywall vendors' overlay scripts through the never-block list.
- **Letting the Filter lists block a Wall's script.**
  - **Changes ADR 0047:** on these domains, the never-block list's entries for paywall vendors' overlay scripts do not apply. The Filter lists then block them as they do in any browser running uBlock Origin; Fanboy Annoyance already blocks `||piano.io^` and `||tinypass.com^`.
  - **Changes ADRs 0020 and 0047:** the breakage rules that turn blocking off because a Wall appears only with blocking off, or that retry with blocking off because an ad-blocker wall appears, are skipped. An ad-blocker wall is hidden instead.
  - **Changes ADRs 0020 and 0047:** ADR 0047's comparison of the route's sample with blocking off still applies, except that differences caused by requests to scripts of a Wall the rules detected are ignored, so blocking a Wall's script never turns blocking off for the route. Entries for challenge providers, anti-bot sensors, and anti-fraud and payment scripts still win.
  - The consent vendors' prompt scripts keep loading, so a prompt with a refusal is still refused (below).
- **One fresh Session per page.**
  - When a paywall leaves the served page without its main content, the engine retires the Session through its Session pool and fetches the page once more in a new one, with a fresh cookie jar and storage.
  - It never does this on a logged-in Session or in an `identified` Session (ADR 0021). There, the page stays as served, with the reason recorded.
  - It happens at most once per page. It never counts as a block for ADR 0021's escalation, so it never moves the domain up a proxy tier.
  - The Site access spec sets the test for "withholds the content", from the paywall detector and the extracted main content.
  - A consent wall or an ad-blocker wall never triggers it: either one would greet a new Session the same way.

**Consent walls.**
- A prompt that offers a refusal is still refused (ADR 0048).
- On a domain in `wall_domains`, a consent wall still goes through the wall test. It is then read past as above, without consenting: no acceptance is clicked, and the vendor's trackers stay blocked as for any refused prompt.
- When the consent wall withholds the content, the engine accepts only if the domain is also in `consent_domains`. Otherwise the page stays `consent_required`.
- **Changes ADRs 0047 and 0048:** on a domain in both lists, the engine reads past first and accepts only when the content is withheld, rather than accepting every wall. On a route whose browser sample showed a consent wall, pages on a domain in `wall_domains` are read as served in every tier rather than `consent_required`, unless the sample showed the content withheld.
- **Changes ADR 0048:** `consent_domains` accepts `*` when a person writes it explicitly, as `pay_domains` does. ADR 0048's rule for an Agent's own "accept" click (its change to ADR 0014) still applies: the Agent asks for confirmation on a domain outside `consent_domains`, so under an explicit `*` it no longer asks anywhere. Writing `*` in `consent_domains` therefore shows a plain-language warning saying so, recorded in the Audit log, as widening `wall_domains` does.

**What the allowance never covers.**
- **`payment_required`.** A 402 or a paywall redirect (ADR 0045) is not a Wall. On every domain, the engine never escalates, rotates, resets or reads past it, and a Paid fetch stays the only way through.
- **Named techniques.** Goodfetch ships no list of Wall scripts to block, no Googlebot, crawler or referrer preset, no meter-reset preset, and no archive or cached-copy fallback. A user's own Hooks, Plugins and Filter lists can do these things through the generic means ADRs 0033 and 0047 already provide. The engine cannot judge a Hook's or Plugin's intent, so these are not limited to `wall_domains`; a user's Filter list, though, gets past a Wall's script only on `wall_domains`: elsewhere the never-block list wins over it for paywall vendors' overlay scripts, and for an in-house paywall or ad-blocker-wall script ADR 0047's breakage rules turn blocking off instead. Consent vendors' prompt scripts are never blocked, on `wall_domains` included. Provenance records what they did as that user's configuration, as ADR 0047 records the scriptlets of a user's own lists, never as the engine's own act.
- **Changes ADR 0033:** the official Plugin index does not list, and delists once found, any Plugin whose name, description or docs present it as a way past Walls, crawler verification or paywalls, or as a source of archived copies of protected pages. Delisting is not revocation: the Plugin's digest and signer do not join the revocation list, which stays for security, so installing it by exact digest, where Operator policy allows it, is unaffected.

**How results are marked.**
- **Provenance** records, for each page:
  - the Wall's kind (`paywall`, `consent_wall`, `ad_blocker_wall`) and its detected vendor;
  - each action taken (`read_served`, `hidden`, `script_blocked`, `new_session`, `accepted`, `left`);
  - the `wall_domains` entry that allowed it.
- **Records** carry a `wall` marker that Dataset queries and every export format can filter on.
- **Completeness.** A page read past its Wall counts as reached. A page whose Wall still withheld the content keeps today's handling: a consent wall left unaccepted stays `consent_required` and makes the run `unverified` (ADR 0048).
- **The Audit log** records every widening of `wall_domains` with the warning shown, and one entry per run and domain where the allowance was used, not one per page.
- **Changes ADR 0046:** Response cache entries fetched under the allowance are tagged with their domain and served only to runs whose `wall_domains` covers that domain. Saving an identity drops them from its partition unless the saving run's `wall_domains` covers them, as ADR 0048 does for acceptances.
- **Changes ADR 0036:** change tracking says when a run and its Baseline used different allowances for a domain, so a difference the allowance caused is not reported as a change on the site.

**Goodfetch Cloud** runs the same contract and can lock `wall_domains` empty through Operator policy (ADR 0001). The research rates every level higher for a hosted service, so whether the cloud offers the allowance at all belongs to the cloud map.

**Proof.** The conformance suite runs on the Proving ground (ADR 0025) on Chromium, Firefox and WebKit. It covers each Wall kind ADRs 0047 and 0048 test: each paywall vendor, "accept or pay" walls, ad-blocker walls, plus a metered paywall. It checks two things:
- **Outside `wall_domains`:** the behaviour of ADRs 0045, 0047 and 0048 is unchanged.
- **Inside `wall_domains`:**
  - the served content is read and every action is recorded;
  - challenge, anti-bot and payment scripts still load;
  - no reset happens on a logged-in or `identified` Session, for a consent or ad-blocker wall, or twice for a page;
  - consent walls are never accepted without `consent_domains`;
  - a 402 is never got around;
  - cache entries never reach a run without the allowance.
