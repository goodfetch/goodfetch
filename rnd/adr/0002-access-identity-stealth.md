# ADR-002: site access, agent identity and stealth options

**Status: principle decided by the project owner on October 8, 2026; terms proposed, R&D draft.**
**Date: October 8, 2026.**
**Final decision: user / project owner.**

## Context

Proxies, consistent browser fingerprints and stealth options are part of the standard offering of data collection services and hosted browsers. The dossier already provides for them: consistent proxy/fingerprint configuration, proxy pool with sticky sessions, health, cooldown and rotation ([architecture §4.1](../proposals/architecture.md:144)), transports specialized in fingerprinting with a consistent profile ([architecture §5.1](../proposals/architecture.md:177)).

In parallel, identified access is becoming a recognized access path:

- Cloudflare defines a **Verified** bot or agent by deterministic identification — Web Bot Auth signature, published IP list or reverse DNS — and non-abusive behavior, including respect for `robots.txt` and crawl directives. Since July 1, 2026, signed agents are classified there as **Direct** or **Intermediary**; a service can be removed from the allowlist if its traffic does not match the declared use. [Verified bots, updated July 1, 2026](https://developers.cloudflare.com/bots/concepts/bot/verified-bots/).
- Web Bot Auth has been an IETF working group document since September 1, 2026 (authors Cloudflare and Google): RFC 9421 HTTP signatures, `Signature-Agent` header, JWKS key directory at a well-known URI. The text remains an Internet-Draft. [draft-ietf-webbotauth-httpsig-protocol-00](https://datatracker.ietf.org/doc/draft-ietf-webbotauth-httpsig-protocol/).
- Browserbase offers both its browser access options and a signed identity in beta, and now presents this work as **identity continuity** across sessions rather than one-off evasion. [Web Bot Auth at Browserbase](https://docs.browserbase.com/platform/identity/web-bot-auth), [post of April 6, 2026](https://www.browserbase.com/blog/identity).

Two legal developments change the risk of certain options:

- **Reddit v. SerpApi (SDNY, July 31, 2026)**: at the motion to dismiss stage, the court considers that CAPTCHA and bot detection can constitute technological measures controlling access within the meaning of DMCA §1201, even if the content is visible to humans. The circumvention (§1201(a)(1)(A)) and *trafficking* (§1201(a)(2)) claims survive against the provider of a scraping tool described as using proxies and techniques imitating human behavior. This is not a judgment on the merits. [Loeb & Loeb summary](https://www.loeb.com/en/insights/publications/2026/08/reddit-v-serpapi-llc).
- **CNIL, web scraping focus sheet**: for a collection of **personal data intended to build AI training datasets on the basis of legitimate interest**, excluding sites that object to it via `robots.txt` or CAPTCHA is one of the mandatory measures. The scope is this purpose, not all scraping. [Sheet of June 19, 2025](https://www.cnil.fr/fr/focus-interet-legitime-collecte-par-moissonnage), [English version of January 5, 2026](https://www.cnil.fr/en/legal-basis-legitimate-interest-focus-sheet-measures-implement-case-data-collection-web-scraping).

These elements are not legal advice. The question settled here is therefore not *whether* stealth options are needed, but **how to deliver them**: profiles, defaults, open source/cloud split, CAPTCHA modality, safeguards and consequences for the session model.

## Decision

### 1. Three access profiles, exclusive within a session

| Profile | Content | Default |
| --- | --- | --- |
| `identified` | Declared agent: User-Agent with product token and contact URL, Web Bot Auth signature, site preference policy applied | **Yes** |
| `standard` | Generic browsing and HTTP, without signature or masking; preference policy applied | No |
| `stealth` | All the options in the catalog below, enabled individually | No, explicit opt-in |

A session has a single profile, set when it is opened. Signing a request and then masking automation in the same session is contradictory: it exposes the key and the reputation of the signed identity, and contradicts the declared use required by verification programs. Changing profile is an explicit transition that opens a new session, like a backend change ([architecture §5.2](../proposals/architecture.md:183)).

### 2. Compliant default (`identified`)

- Web Bot Auth signature with installation-specific keys locally, **per-tenant keys** in the cloud; published key directory, rotation and revocation.
- Registration of the cloud service as a verified agent (Cloudflare Verified *Intermediary*, then other anti-bot providers that verify Web Bot Auth).
- Policy before each request: `robots.txt` (RFC 9309) including crawl-delay, Content-Signal, TDMRep, meta `noai`, then the IETF aipref vocabulary once it is stabilized.
- Declared purpose per workflow (search, ai-input, ai-train, agent, data-collection, transact), aligned with Cloudflare's behavior categories.
- Per-host politeness and rate, adapted to `429`/`Retry-After` responses.
- `401`, `403`, `402` and challenge pages are **stop or licensing signals**, never a success.

### 3. `stealth` catalog: all the options

| Family | Options | Open source | Cloud |
| --- | --- | --- | --- |
| Proxies | Datacenter, residential, mobile, ISP; BYO provider; sticky sessions, geolocation, reasoned rotation, health, cooldown, measured cost per GB | Pool management and provider adapters (BYO) | Managed pools, isolated per tenant |
| Network fingerprint | TLS and HTTP/2 profiles consistent with the announced browser, header order and case, client hints | Impersonation transports (wreq/Impit class) and base profiles | Maintained and dated profile catalog |
| Browser fingerprint | Consistent profiles: User-Agent, platform, screen, fonts, WebGL/canvas, language, time zone and geolocation aligned with the proxy exit; persistent profiles; headful mode | Consistency engine and base profiles | Maintained profiles, maintained patched browsers |
| Automation signals | Reduction of driver and browser automation markers; patched browsers as plugins (Patchright/Camoufox class) | Interfaces and plugins | Maintained builds |
| Behavior | Realistic typing rhythms and pointer trajectories, optional per action | Yes | Yes |
| Continuity | Session warm-up, cookie/storage persistence, same identity on return | Yes | Yes |
| CAPTCHA | See §4 | Detection, handoff, solver interface | Managed solving |
| Block detection | Classifier for challenge pages, interstitials and misleading `200`s; profile escalation according to policy | Yes | Success statistics per site and profile |

Each option is declared in the workflow's plan or policy, visible in the log and counted in the attempt budget. Proxy rotation, session rotation and repair consume the same total budget ([architecture §3.4](../proposals/architecture.md:125)).

### 4. CAPTCHA: all modalities, delivered in tiers

1. Systematic **detection**, with a typed error; never a false success.
2. In-session **human handoff**: the operator solves, the run resumes. Open source.
3. **Third-party solver adapter**: open interface, account and key provided by the customer. Open source.
4. **Managed solving** in the cloud: contractual offering, verified workspace.

Recommendation: ship 1 to 3 with the framework and open tier 4 after legal review, particularly for exposure in the United States. The *trafficking* risk raised in Reddit v. SerpApi targets the tool provider; it therefore concerns Automan as publisher, not only its customers.

### 5. Safeguards

- **Workspace policy**: an administrator can allow or forbid `stealth` and each option family.
- **Provenance**: each attempt and each record carry the access profile, the signing identity if any, and the state of the site's preferences at the time of the request (hash of `robots.txt`/signals).
- **Cloud**: customer verification and acceptable use policy for `stealth`; IP pools and reputation isolated per tenant; per-tenant kill switch; exportable audit logs.
- **Personal data**: a workflow declaring personal data and an `ai-train` purpose displays the CNIL obligation recalled above; enabling `stealth` on this combination requires an explicit, logged confirmation.
- **Self-host**: redirects to private addresses refused by default (SSRF protection), with an explicit allowlist for tests; the lab observed this native behavior in Spider.

## Open source / cloud split

The Apache-2.0 framework contains all the mechanisms: profiles, identity consistency, BYO proxy management, impersonation transports, block detection, handoff and solver interface, Web Bot Auth signing with self-managed keys, policy engine. It works without a cloud key.

The cloud sells what requires **continuous maintenance**: managed proxy pools, an up-to-date catalog of network and browser profiles, maintained patched browsers, managed CAPTCHA solving, a hosted key directory and registrations with verifiers, reputation isolation and per-site success statistics. Profiles published in a public repository are quickly identifiable by anti-bot vendors; the value sold is keeping them up to date, not their existence.

## Architectural consequences

- The `Session` carries a **complete identity profile**: signature or absence of signature, proxy exit, network profile, browser profile, cookies and storage. These elements are consistent with one another and with the proxy's geolocation.
- This profile is **preserved when a request moves from HTTP to the browser** within the same session. An engine escalation that changes fingerprint itself creates the bot signal.
- The errors of [the API](../proposals/api.md) gain distinct codes for a detected challenge, a refusal by site policy and a payment request (`402`). The exact term remains to be set in the API review.
- Network and browser profiles are versioned and dated artifacts, with the version of the browser they imitate; an outdated profile is flagged.
- For each attempt, the log records: profile, active options, policy decision and block detection result.

## Options considered

| Option | Advantages | Limitations | Status |
| --- | --- | --- | --- |
| Stealth only, like most services | Maximum access rate on protected sites | No verified access path for enterprise customers; shared reputation exposed | Rejected |
| Identified access only | Minimal legal and reputational risk | Insufficient success rate on protected sites; rejected by the owner | Rejected |
| **Exclusive profiles, `identified` by default, full `stealth` as opt-in** | Covers the market and verified access; explicit and traceable control | Two families to maintain; session transition rules | **Adopted** |

## Risks

- **Shared reputation**: an abusive tenant can degrade the intermediary identity or the shared IPs. Response: per-tenant keys and pools, customer verification, kill switch.
- **Legal**: managed CAPTCHA and circumvention tooling sold by the publisher (§1201 in the United States); customers' GDPR obligations. Response: CAPTCHA tiers, legal review before tier 4, provenance, explicit confirmation on personal data.
- **Permanent race**: profiles age. Response: dated profiles, success measurements per site and profile, maintenance funded by the cloud.
- **Contradictory identity**: mixing signing and masking. Response: exclusive profiles per session.

## Qualification actions

- [ ] **Identity consistency**: local fixture that records TLS fingerprint (JA4), HTTP/2 parameters, header order, User-Agent and client hints, then checks their stability when the same session moves from HTTP to the browser.
- [ ] **Web Bot Auth**: fixture that verifies the RFC 9421 signature, the key directory, `created`/`expires`, rotation and refusal of a revoked key.
- [ ] **Policy**: `robots.txt`, Content-Signal and TDMRep fixtures; logged refusal; a challenge page served as `200` must never produce a verified result.
- [ ] **Proxies**: local test proxy for sticky sessions, rotation, health, cooldown and attempt budget.
- [ ] **Transitions**: profile change refused in an active session, explicit new session accepted.
- [ ] **Live**: separate campaign, outside CI, on authorized sites; its results are never aggregated with the lab's hermetic scores.

The promotion gates remain those of the [R&D plan](../plan/rd-plan.md#7-promotion-gates-without-invented-figures): compliant behavior, visible limits, no success announced for an unverified effect or access.
