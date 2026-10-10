# Legal exposure of stealth and challenge-solving tooling

> **Not legal advice.** This is desk research by a non-lawyer, written to prepare a conversation with counsel. It maps where exposure appears to concentrate and lists the questions to ask. It does not say any option is lawful or unlawful. Several of the cases below are at the motion-to-dismiss stage or still pending. A US, French or EU lawyer may read them differently.

**Research of October 10, 2026.** Ticket [#65](https://github.com/Jbmanllr/automan/issues/65), child of map [#61](https://github.com/Jbmanllr/automan/issues/61). This note says where exposure sits for each placement of the challenge solver. It does not decide whether the solver exists: the owner has already decided that it will.

**Source tags:** **[P]** primary source (court opinion or docket, statute, regulator text, official journal). **[C]** reputable commentary, used where the primary text could not be read or would not load. **[V]** vendor self-description. **[I]** my own inference from the sources. All pages were accessed on October 10, 2026.

## Summary

- **In the US, the acting party carries more risk than the publisher, except for the solver.** Under DMCA §1201, an actor that circumvents an access control is liable, and so is anyone who "provides or otherwise traffics" in technology *primarily designed* or *marketed* for circumvention. Courts have treated CAPTCHAs and bot-detection systems as access controls. See *Ticketmaster v. Prestige* (C.D. Cal., 2018) and *Reddit v. SerpApi* (S.D.N.Y., July 31, 2026, motion-to-dismiss stage). In *Reddit*, the court sustained both the circumvention claim and the trafficking claim against a scraping-API provider. It also held that the provider's AI customer was plausibly a direct circumventor. [P]
- **Publishing the solver as open source probably counts as "trafficking" in the US.** *Universal v. Corley* (2d Cir. 2001) upheld an injunction against posting circumvention code, and the Second Circuit is the circuit of the *Reddit* court. A bundled autonomous solver is therefore the highest-exposure item for Automan as publisher, and for individual contributors. Fingerprint consistency and proxy management are dual-use and much less exposed, as long as they are not *marketed* as bypass tools. [P]+[I]
- **Running the solver in a hosted cloud concentrates exposure.** Automan Cloud would be the circumventor, the service trafficker and, in the EU, a GDPR processor or joint controller. US statutory damages apply per act of circumvention. The cloud is also the only placement where KYC, allowlists, authorization checks, kill switches and logs actually work. [P]+[I]
- **The defenses that work are narrow and fact-specific.** *Google v. SerpApi* (N.D. Cal., July 20, 2026) dismissed Google's §1201 claims for two reasons. Many search results contain no copyrighted work, and Google had not pleaded that the copyright owners authorized the measure. Google amended its complaint, and the renewed motion is set for hearing on **October 13, 2026**. Under the CFAA, scraping public pages is weak ground for a claim after *Van Buren* (2021) and *hiQ II* (2022). Evading IP blocks after an explicit revocation is still risky (*Craigslist v. 3Taps*, 2013). [P]
- **In the EU and France, exposure is real but untested for CAPTCHAs.** Selling, making available or *providing a service* designed to defeat an "effective technical measure" is an offense under French CPI L.335-3-1 II: 6 months and €30,000 for individuals, and up to five times the fine for a company. Making tools available "without legitimate reason" to commit unauthorized access is an offense under Code pénal 323-3-1, with the penalty of the underlying offense. No decision was found that treats a CAPTCHA as a technical measure. On July 9, 2026, however, the CJEU held that state-of-the-art IP geo-blocking can be an "effective" technical measure even though VPNs can defeat it (*Anne Frank Fonds*, C-788/24). [P]+[I]
- **For personal data used in AI training, EU regulators treat CAPTCHAs as a stop sign.** The CNIL lists the exclusion of sites that object to scraping through robots.txt or CAPTCHA as a *mandatory* measure (sheet of June 19, 2025; English version of January 5, 2026). The EDPB's draft Guidelines 03/2026 (adopted for consultation on July 7, 2026, open until October 30, 2026) take the same line. A managed solver used on such workflows defeats a measure the regulator calls mandatory. [P]
- **The vendors reviewed rely on contract controls, not on legal theories.** Their controls are KYC, acceptable-use policies, and bans on logged-in data, fake accounts and ticket bots (Bright Data, Oxylabs, Zyte, CapSolver). Only Browserbase ties CAPTCHA solving to authorization by the *website owner* [V]. That framing matches §1201's "without the authority of the copyright owner" element. [I]

## Scope and terms used

**Placements**

| Code | Placement | Who acts on the target site |
| --- | --- | --- |
| **A** | Published in the public Apache-2.0 framework (GitHub, crates.io, npm, PyPI) | The user. Automan is the publisher. |
| **B** | Operated in Automan Cloud (managed proxies, managed browsers, managed solving) | Automan Cloud, on behalf of a customer |
| **C** | Kept in a private component: a closed module licensed to vetted self-hosting customers (C1), or code used only inside the cloud (C2, which collapses into B) | The licensee (C1) or Automan Cloud (C2) |

**Capability families.** These follow the `stealth` catalog of [ADR-0002](../adr/0002-access-identity-stealth.md):

- **F1 Fingerprint and automation-signal masking.** Covers consistent browser profiles, TLS and HTTP/2 impersonation, patched browsers and human-like input.
- **F2 Proxy rotation.** Covers datacenter, residential and mobile proxies, sticky sessions and geolocation. The sensitive case is *rotating in response to a block*.
- **F3 Challenge and CAPTCHA handling**, split into four tiers:
  - **F3a** autonomous solver shipped by Automan;
  - **F3b** adapter to a third-party solving service, with the account provided by the customer;
  - **F3c** in-session human handoff;
  - **F3d** detection and stop.

**Levels.**

- **Low:** no statute squarely applies, or only remote secondary theories do.
- **Medium:** a plausible claim exists but is untested or depends on the facts, and mitigations can move it.
- **High:** the conduct matches facts on which a court has sustained claims, or a regulator has called the countermeasure mandatory.

`(?)` marks an untested point of law.

## Risk map

### A. Published in the open-source framework

| Family | United States | EU / France |
| --- | --- | --- |
| **F1 Fingerprint and automation-signal masking** | **Low to Medium.** §1201(a)(2) reaches technology that is *primarily designed*, has *only limited* other commercially significant use, or is *marketed* for circumvention ([17 U.S.C. §1201][usc1201]). Profile consistency has legitimate uses: privacy, testing and anti-tracking. Comparable OSS presents itself in neutral terms: fingerprint-suite says it is for "anonymizing your scrapers" [V] ([fp-suite]). Risk rises with bypass marketing: in *Reddit*, the alleged circumvention means were proxies and technologies that "mimic human behavior" ([Loeb][loeb], [C]). The publisher has no CFAA or contract exposure because it does not access the target. | **Low.** InfoSoc art. 6(2) has the same three prongs, read with recital 48 on proportionality and on not banning devices with other significant uses ([InfoSoc][infosoc]; [*Nintendo*][nintendo], 2014). CPI L.335-3-1 II requires means "conçus ou spécialement adaptés" to defeat a technical measure ([L.335-3-1][cpi-l335]). Code pénal 323-3-1 requires a tool designed to commit an offense against an information system ([Code pénal 323][cp-323]). Generic fingerprinting fits none of these well [I]. The publisher has no GDPR role [I]. |
| **F2 Proxy rotation** | **Low.** Pool management is generic networking. CFAA risk falls on whoever evades a block after revocation ([*3Taps*][3taps-aug], 2013), not on a library [I]. Caveat: *Reddit* also sued the proxy *network operators* Oxylabs and AWMProxy as traffickers. That part is pending ([Reddit's opposition, Aug. 21, 2026][reddit-oxy-opp]). | **Low.** The CJEU calls VPNs and similar services "lawful technical tools" and does not treat their provider as communicating the work ([*Anne Frank Fonds*][annefrank], July 9, 2026). The same judgment says IP-based geo-blocking is a technical measure. Features marketed as *unblocking* therefore move toward the "marketed for circumvention" prong of art. 6(2)(a) [I]. |
| **F3a Autonomous solver, bundled** | **High.** The solver is primarily designed to defeat a measure that courts have treated as an access control. *Prestige* held at the motion-to-dismiss stage that a CAPTCHA is a measure that effectively controls access, and that bots or CAPTCHA farms "avoid" it ([*Prestige*][prestige], Jan. 31, 2018). *RMG* (2007) concerned CAPTCHA tools sold to ticket brokers ([docket][rmg]). *Reddit* sustained trafficking against SerpApi ([Opinion & Order, ECF 104][reddit-104], July 31, 2026). Posting code counts as trafficking ([*Corley*][corley], 2d Cir. 2001). Remedies include injunctions and $200–$2,500 in statutory damages per device, component or offer. Criminal penalties apply if the conduct is willful and for commercial advantage (§§1203–1204) ([ch. 12][usc1201]). GitHub runs a dedicated §1201 takedown review ([GitHub policy][github-dmca]). Defense arguments: the solver is site-agnostic, and a claimant must show a copyrighted work behind a measure the copyright owner authorized ([*Google v. SerpApi*][google-order], July 20, 2026) `(?)`. | **Medium `(?)`.** CPI L.335-3-1 II 2° covers making available to the public "sous quelque forme que ce soit" means designed to defeat an effective technical measure. The penalty is 6 months and €30,000 ([L.335-3-1][cpi-l335]), with up to five times the fine for a legal person ([art. 131-38, via commentary][cp-131-38], [C]). The open question is whether a CAPTCHA is an "effective technical measure" protecting a work or database (art. 6(3) also covers the *sui generis* database right). No ruling was found. *Anne Frank Fonds* makes "yes" more plausible for IP-based controls [I]. Code pénal 323-3-1 also applies if solving counts as *accès ou maintien frauduleux* `(?)`; see *Bluetouff* ([Cass. crim., May 20, 2015][bluetouff]). The EU baseline requires direct intent for tools offenses ([Dir. 2013/40, art. 7 and recital 16][dir2013-40]); the French text uses "sans motif légitime" instead. |
| **F3b Adapter to a third-party solving service** | **Medium.** The statute covers any "component, or part thereof". An adapter whose only job is to send challenges to a solving service is plausibly *primarily designed* for circumvention [I]. It is less visible than a solver. No case against such an adapter library was found. | **Low to Medium `(?)`.** Same texts and same open questions. The adapter's design purpose is the decisive fact [I]. |
| **F3c Human handoff / F3d Detection and stop** | **Low.** Detecting a challenge and stopping is not circumvention. Handoff caveat: *Prestige* treated "CAPTCHA farms" (humans solving for bots) as avoidance ([*Prestige*][prestige]). An operator solving in their own session at human pace is the closest case to authorized use [I]. | **Low.** |

### B. Operated in Automan Cloud

| Family | United States | EU / France |
| --- | --- | --- |
| **F1 Masking, performed by the cloud** | **Medium.** The cloud is the party that accesses the site. If its masking defeats bot detection that works as an access control, it is a *direct circumventor* under §1201(a)(1)(A) and also traffics in a *service* under (a)(2). *Reddit* sustained both theories against SerpApi, which was alleged to use proxies and human mimicry, and treated the AI customer as a direct circumventor too ([ECF 104][reddit-104]; [Loeb][loeb], [C]). The claim still needs copyrighted content behind a measure the copyright owner authorized ([*Google v. SerpApi*][google-order]). Stealth has been cited as evidence of willfulness against an AI company ([NYT v. Perplexity complaint][nyt-perplexity], Dec. 5, 2025, relying on [Cloudflare][cf-perplexity], Aug. 4, 2025). CFAA risk is low for logged-off public pages ([*hiQ II*][hiq2]) and high behind login ([*hiQ* SJ][hiq-sj], fake accounts). | **Medium.** Circumvention by the cloud (InfoSoc art. 6(1)) and provision of a service (art. 6(2)). Code pénal 323-1 applies if the site's refusal is clear, by analogy with *Bluetouff* `(?)`. GDPR: when customers collect personal data, the cloud is a processor or a joint controller ([EDPB Guidelines 03/2026, ¶17–20][edpb-0326]). DPAs expect platforms to run bot detection, CAPTCHAs and IP blocking against unlawful scraping ([joint statement, 2023][joint-2023]; [concluding statement, 2024][joint-2024]). Defeating those measures weighs against data subjects' reasonable expectations ([EDPB 03/2026][edpb-0326]). |
| **F2 Managed proxies, including residential and mobile** | **Medium to High.** *Reddit* sued proxy network providers as traffickers; Oxylabs contests extraterritoriality ([Oxylabs MTD][oxy-mtd], July 21, 2026; [opposition][reddit-oxy-opp]). Pending. Evading IP blocks after a cease-and-desist was held to be access "without authorization" ([*3Taps*][3taps-aug], 2013; *Power Ventures*). FTC BOTS Act cases list IP concealment ([FTC v. Concert Specials][ftc-concert], 2021). Residential sourcing is under enforcement: the 911 S5 botnet ([DOJ, May 29, 2024][911s5]) and Google's legal and technical actions against IPIDEA ([Jan. 28, 2026][ipidea]) and NetNut ([July 2, 2026][netnut]). Google says "ethical sourcing" claims need auditable consent [V]. | **Medium to High.** Residential and mobile exit nodes run code on third-party devices, which requires consent under ePrivacy art. 5(3) ([Dir. 2002/58][eprivacy]) and raises GDPR questions for device owners' traffic [I]. If devices are enrolled without consent, Code pénal 323-1 and 323-2 risks follow [I]. Rotating to defeat IP-based access control is a technical-measure question after *Anne Frank Fonds* [I]. Systematic extraction at scale engages the *sui generis* database right ([*CV-Online*][cv-online], 2021; *Leboncoin v. Entreparticuliers*, [CA Paris, Feb. 2, 2021][leboncoin], [C]). |
| **F3a Managed solving** | **High.** This is SerpApi's posture: direct circumvention and trafficking claims sustained at the motion-to-dismiss stage ([ECF 104][reddit-104]). In the parallel case, the court found that masking queries and syndicating authorization were "sufficiently analogous" to conduct held to be circumvention, while dismissing on the copyright-nexus and authorization elements ([*Google v. SerpApi*][google-order], [C] summary of the primary text). Statutory damages scale per act ([ch. 12][usc1201]); Google alleged "billions" of acts ([complaint][google-complaint], Dec. 19, 2025). Customers are exposed too, as Perplexity was. CAPTCHA-solving services used for fraud were seized in *Microsoft v. Storm-1152* ([Microsoft, Dec. 13, 2023][storm1152]). Ticketing is covered by the BOTS Act ([15 U.S.C. §45c][bots-act]). Foreign providers can be sued in US courts ([*X Corp. v. Bright Data*, personal jurisdiction order, Feb. 5, 2024][x-bd-pj]). | **High.** CPI L.335-3-1 II 3° covers "en fournissant un service à cette fin", and I covers the cloud's own acts `(?)` ([L.335-3-1][cpi-l335]). Code pénal 323-1 and 323-3 apply where refusals are clear `(?)`. For personal data used in AI development, the CNIL makes the exclusion of sites that object through robots.txt or CAPTCHA **mandatory** ([CNIL sheet][cnil-sheet-en]), and the EDPB draft says the same ([03/2026][edpb-0326]). A managed solver on those workflows works against a measure the regulator calls mandatory. Processors are liable when they breach processor duties or act against instructions (GDPR art. 82(2)). Customers that provide general-purpose AI models and follow the Code of Practice commit not to circumvent effective technical measures (Measure 1.2) ([CoP copyright chapter][cop-copyright], July 10, 2025). |
| **F3b Customer's third-party solver, run in the cloud** | **Medium to High.** The cloud still performs or orchestrates the circumvention and offers it as a service [I]. | **Medium to High** [I]. |
| **F3c Human handoff through live view** | **Low to Medium.** Low when the customer's own operator solves. If Automan staff or contractors solve, it resembles the CAPTCHA farm in *Prestige* ([*Prestige*][prestige]). | **Low.** |

### C. Private component

| Family | United States | EU / France |
| --- | --- | --- |
| **F1, F2 in a closed module (C1)** | **Low.** Same analysis as A; less visible. | **Low.** |
| **F3a Closed solver licensed to vetted customers (C1)** | **Medium to High.** §1201(a)(2) reaches anyone who "provides or otherwise traffics", with no public-offering requirement ([ch. 12][usc1201]). Gains over A [I]: KYC, contractual purpose limits, no public bypass marketing (prong C), and no GitHub takedown exposure. Damages still run per device or copy. | **Medium `(?)`.** L.335-3-1 II 2° covers offering for sale, loan or rental and "mettre à disposition"; Code pénal 323-3-1 covers "offrir, céder". Documented purpose limits support a "motif légitime" argument `(?)` ([L.335-3-1][cpi-l335]; [Code pénal 323][cp-323]). |
| **F3a Used only inside the cloud (C2)** | Same as **B-F3a**. Keeping the code private removes trafficking *in technology*, but not trafficking in a *service* [I]. | Same as **B-F3a**. |
| **Any family, limited to targets that authorized it** (customer's own sites, QA, partners with written consent) | **Low.** Circumvention is defined as acting "without the authority of the copyright owner" (§1201(a)(3)(A)) ([ch. 12][usc1201]). *Prestige* draws the same line between authorized human users and bots ([*Prestige*][prestige]). | **Low.** No unauthorized act. L.335-3-1 III and Code pénal 323-3-1 also carve out computer-security purposes, and L.335-3-1 carves out research ([L.335-3-1][cpi-l335]; [Code pénal 323][cp-323]). |

### What the map suggests (input to the placement decision, not a decision) [I]

1. **The legal label depends more on who acts and how the code is designed and marketed than on where it lives.** Moving the solver from the public repo to a private module removes discoverability and lets Automan add gates. It does not change the "provides/traffics" or "offrir/mettre à disposition" characterization.
2. **The public framework can carry F1, F2, F3c and F3d with Low exposure, under one condition:** documentation and naming present them as consistency, privacy, testing and authorized-automation features, never as "bypass" or "undetectable". Marketing is its own prong in the US (§1201(a)(2)(C)), in the EU (InfoSoc 6(2)(a)) and in France (L.335-3-1 II 4°, advertising and inducement).
3. **The R&D proposal should be reconsidered on one point.** ADR-0002 §4 puts the third-party solver adapter (tier 3) in the public framework. This research rates that adapter **Medium** in the US, not Low. An autonomous solver bundled in the public repo is the highest-exposure option for Automan *and* for individual contributors.
4. **Automan Cloud is the highest-exposure placement for an unrestricted managed solver, and the only one where mitigations can be enforced.** Those mitigations are KYC, target allowlists, authorization attestation, purpose locks, kill switch and logs. A cloud solver restricted to authorized or allowlisted targets, and technically disabled for workflows declaring personal data with an `ai-train` purpose, rates well below an open solver [I].
5. **US exposure dominates for the solver, because §1201 has statutory damages and live precedent.** EU and French exposure is mostly untested on the technical-measure question. It is concrete on GDPR for personal data and on proxy sourcing.

## Legal sources

### United States

**DMCA §1201 (statute)** [P] ([17 U.S.C. ch. 12][usc1201])

- **§1201(a)(1)(A)** bars circumventing a measure that "effectively controls access" to a copyrighted work.
- **§1201(a)(2)** bars manufacturing, offering to the public, providing or "otherwise traffic[king]" in any technology, product, service, device, component or part that meets one of three conditions:
  - (A) it is primarily designed to circumvent;
  - (B) it has only limited commercially significant purpose other than circumvention;
  - (C) it is marketed for circumvention.
- **§1201(a)(3)** defines circumvention as avoiding or bypassing "without the authority of the copyright owner".
- **§1203** provides statutory damages of $200–$2,500 "per act of circumvention, device, product, component, offer, or performance of service".
- **§1204** provides criminal penalties for willful violations "for purposes of commercial advantage or private financial gain".

**Reddit, Inc. v. SerpApi LLC, Oxylabs UAB, AWMProxy, Perplexity AI**, No. 1:25-cv-08736-PAE (S.D.N.Y.)

- **Opinion & Order of July 31, 2026, ECF 104**, Judge Engelmayer. [P] docket text ([Justia][reddit-104]); the opinion PDF is a scan whose text could not be extracted ([court PDF][reddit-op-pdf]). The court "predominantly denies" the motions to dismiss. It sustains:
  - §1201(a)(1)(A) against both SerpApi and Perplexity;
  - §1201(a)(2) against SerpApi;
  - New York civil conspiracy against both.

  It dismisses the §1201(b) claim and the unjust-enrichment and unfair-competition claims, which are preempted.
- **Reasoning, as summarized by commentary** [C] ([Loeb & Loeb][loeb]):
  - SearchGuard, using JavaScript challenges and CAPTCHAs, is an access control even though humans can see the content, because it separates automated users from human ones.
  - The circumvention means alleged were proxies, fake user agents and techniques that "mimic human behavior".
  - Perplexity, as the customer, is plausibly a direct circumventor because it set the parameters and ran the queries.
  - The "designed or marketed" element was undisputed for SerpApi.
  - Snippets of a few sentences can be copyrightable.
- **Pending:** Oxylabs, a Lithuanian proxy provider, moved to dismiss on standing and extraterritoriality ([Oxylabs MTD, July 21, 2026][oxy-mtd]). Reddit argues the July 31 opinion already decides most issues, and that Oxylabs trafficked in technology designed and marketed to circumvent Google's and Reddit's measures ([opposition, Aug. 21, 2026][reddit-oxy-opp]). No ruling was found. **Stage: pleadings, not merits.**

**Google LLC v. SerpApi, LLC**, No. 4:25-cv-10826-YGR (N.D. Cal.)

- **Order of July 20, 2026** [P] text on FindLaw ([order][google-order]); this summary of its holdings is mine [C].
- Dismissed *without* leave to amend as to search results containing no copyrighted work.
- Dismissed *with* leave to amend because Google did not plead that copyright owners authorized SearchGuard to protect their content.
- Points the court decided against SerpApi:
  - non-owners may sue under the DMCA;
  - the measure need not be tailored to copyrighted parts;
  - *hiQ* and *Van Buren* are CFAA cases and do not control here;
  - masking queries and syndicating solved challenges are "sufficiently analogous" to circumvention.
- **Later steps:** Google filed an amended complaint on August 10, 2026, and SerpApi renewed its motion ([SerpApi MTD, Aug. 24, 2026][google-mtd2]). The hearing was continued to **October 13, 2026** ([ECF 55][google-continuance]). **Pending.** SerpApi presents the July order as a win for open access [V] ([SerpApi blog, July 21, 2026][serpapi-blog]).

**Other CAPTCHA and §1201 precedent**

- *Ticketmaster v. Prestige Entertainment* (C.D. Cal. Jan. 31, 2018) [P] ([order][prestige]): a CAPTCHA is an access control. Using "bots or CAPTCHA farms" is avoiding it, and innocent uses of the method are irrelevant.
- *Ticketmaster v. RMG Technologies* (C.D. Cal. 2007): injunction against tools that defeated CAPTCHAs ([docket][rmg]).
- *Universal City Studios v. Corley*, 273 F.3d 429 (2d Cir. 2001) [P] ([opinion, abridged][corley]): upheld an injunction against posting DeCSS code and linking to it. Code is speech, but the anti-trafficking rule survives intermediate scrutiny.
- GitHub reviews §1201 notices with technical and legal experts and offers accused developers a referral through its Developer Defense Fund [V] ([GitHub DMCA policy][github-dmca]).

**CFAA**

- *Van Buren v. United States* (U.S. June 3, 2021) [P] ([opinion][vanburen]): "gates-up-or-down" test. Footnote 8 leaves open whether only technological limits count or contractual ones too.
- *hiQ v. LinkedIn*, 31 F.4th 1180 (9th Cir. Apr. 18, 2022) [P] ([opinion][hiq2]): for public pages, "without authorization" likely does not apply.
- On remand (N.D. Cal. Nov. 4, 2022) [P] ([order][hiq-sj]): hiQ breached the user agreement through scraping and fake "turker" accounts. The case ended in a consent judgment and permanent injunction in December 2022 ([docket][hiq-consent]).
- *Craigslist v. 3Taps* (N.D. Cal. Apr. 30 and Aug. 16, 2013) [P] ([April order][3taps-apr]; [August order][3taps-aug]): continuing access through new IPs and proxies after a cease-and-desist letter and IP blocks is access "without authorization". hiQ II later distinguished the related *Power Ventures* case, where the data was behind a password.
- *Ryanair v. Booking.com* (D. Del.): a jury found a CFAA violation with exactly $5,000 in loss in July 2024. The court granted judgment as a matter of law on January 22, 2025, because loss was not proven, while holding that the CFAA applies outside the US ([opinion][ryanair-jmol] [P]; [Goldman blog][goldman-ryanair] [C]).

**Contract and terms of service**

- *Meta v. Bright Data* (N.D. Cal. Jan. 23, 2024): logged-off scraping of public data did not breach Meta's terms [C] ([Quinn Emanuel alert][meta-bd]).
- *X Corp. v. Bright Data* (N.D. Cal.):
  - Contract claims over scraping public data were dismissed on May 9, 2024, partly on Copyright Act preemption ([733 F. Supp. 3d 832][x-bd-2024]).
  - The court partly allowed a second amended complaint on November 26, 2024. That complaint alleged evasion of anti-scraping measures (IP tracking, rate limits) and server harm ([order][x-bd-sac] [P]).
  - Bright Data's antitrust counterclaims survived on April 18, 2025 ([order][x-bd-cc] [P]).
  - The parties reported a settlement in principle in June 2025 [C] ([Law360 case page][x-bd-law360]).
  - The personal jurisdiction order of February 5, 2024 kept the Israeli defendant in California because it sold scraping tools and proxies to California residents ([order][x-bd-pj] [P]).

**Sector rules and enforcement context**

- **BOTS Act**, 15 U.S.C. §45c [P] ([text][bots-act]): bars circumventing a ticket issuer's "security measure, access control system, or other technological control". It has a security-research exception. The FTC brought cases in 2021 involving CAPTCHA bypass and IP concealment ([complaint][ftc-concert]) and reminded the industry in 2025 ([FTC, Apr. 11, 2025][ftc-bots]).
- **Microsoft v. Storm-1152** (S.D.N.Y., unsealed Dec. 13, 2023): seizure of the 1stCAPTCHA, AnyCAPTCHA and NoneCAPTCHA domains, in a fraudulent-account context ([Microsoft][storm1152]; [complaint][storm1152-complaint]).
- **Residential proxy botnets:**
  - 911 S5 was dismantled and its administrator arrested ([DOJ, May 29, 2024][911s5]).
  - Google disrupted IPIDEA ([Jan. 28, 2026][ipidea]): its SDKs "surreptitiously enroll user devices", and ethical-sourcing claims are "often incorrect or overstated" [V].
  - Google also acted against NetNut ([July 2, 2026][netnut]) [V].
- **Stealth used as evidence:** Cloudflare de-listed Perplexity as a verified bot after seeing undeclared crawlers that rotated IPs and ASNs and ignored robots.txt ([Cloudflare, Aug. 4, 2025][cf-perplexity]). Publishers' complaints then cited that finding ([Britannica v. Perplexity, Sept. 10, 2025][britannica]; [NYT v. Perplexity, Dec. 5, 2025][nyt-perplexity]) [P, complaints only].

### European Union and France

**Technical measures (copyright)**

- **InfoSoc Directive 2001/29, art. 6** [P] ([EUR-Lex][infosoc]):
  - 6(1) bars knowingly circumventing an effective technical measure.
  - 6(2) bars manufacturing, distributing, possessing for commercial purposes or *providing services* that are promoted for circumvention, have limited other purpose, or are primarily designed for it.
  - 6(3) covers measures that protect copyright, related rights *and the sui generis database right*.
  - Recital 48 adds proportionality.
- **Nintendo v. PC Box** (C-355/12, Jan. 23, 2014) [P] ([EUR-Lex][nintendo]): protection is limited to measures against acts that need authorization, and is subject to proportionality.
- **Anne Frank Fonds v. Anne Frank Stichting** (C-788/24, July 9, 2026; provisional text) [P] ([EUR-Lex][annefrank]):
  - IP-based geo-blocking is a technical measure, and a "state of the art" one is *effective* even though VPNs can defeat it.
  - A VPN provider, as a provider of "lawful technical tools", is not the one communicating the work.
  - Open for a lawyer: whether this extends to IP-reputation and bot-detection controls. [I]
- **France, CPI L.331-5 and L.335-3-1** [P] ([Légifrance L.335-3-1][cpi-l335]; full text read through a [law-firm mirror][cpi-mirror] because Légifrance returned errors):
  - Knowingly defeating an effective measure for purposes other than research: €3,750 fine.
  - Knowingly procuring or offering means designed for it, whether by manufacturing, making available to the public, *providing a service*, or advertising and inducing: 6 months and €30,000.
  - Carve-out for acts done for computer-security purposes.
  - Legal persons face up to five times the fine ([commentary on art. 131-38][cp-131-38], [C]).

**Information-system offenses**

- **France, Code pénal 323-1 to 323-3-1** (in force October 2, 2026) [P] ([Légifrance][cp-323]):
  - 323-1, fraudulent access or remaining in a system: 3 years and €100,000.
  - 323-3, fraudulent extraction of data: 5 years and €150,000.
  - 323-3-1, importing, holding, offering, transferring or making available a program "conçu ou spécialement adapté" to commit these offenses, "sans motif légitime, notamment de recherche ou de sécurité informatique": the penalty of the underlying offense.
- **Bluetouff** (Cass. crim., May 20, 2015, n° 14-81.336) [P] ([Légifrance][bluetouff]): remaining in a system after noticing an access control is fraudulent, and downloading data known to be protected is theft. Open for a lawyer: whether a CAPTCHA is such an "access control". [I]
- **Directive 2013/40/EU** [P] ([EUR-Lex][dir2013-40]):
  - Art. 3: illegal access is an offense at least when committed "by infringing a security measure".
  - Art. 7: tools offense, with recital 16 requiring a *direct intent* that the tool be used for offenses.

**Database right and contracts**

- **Directive 96/9, art. 7**:
  - *CV-Online Latvia v. Melons* (C-762/19, June 3, 2021) [P] ([EUR-Lex][cv-online]): a search engine that copies and indexes a substantial part of a freely accessible database extracts and re-uses it. The maker can prohibit that when it puts the recouping of its investment at risk.
  - *Ryanair v. PR Aviation* (C-30/14, Jan. 15, 2015) ([IPPT summary][ryanair-pr], [C]): the owner of a database protected by neither copyright nor the *sui generis* right can restrict its use by contract.
  - *Leboncoin v. Entreparticuliers* (CA Paris, Feb. 2, 2021): extraction by scraping infringed the *sui generis* right ([Doctrine summary][leboncoin], [C]).

**Text and data mining**

- **CDSM Directive 2019/790** [P] ([EUR-Lex][cdsm]):
  - Art. 4(1) allows text and data mining of *lawfully accessible* works.
  - Art. 4(3) lets rightholders opt out, "such as" by machine-readable means for online content.
  - Recital 14 says lawful access includes content freely available online.
  - Art. 3(3) and recital 16 let rightholders apply proportionate security and integrity measures, "including through IP address validation".
  - France transposed it in CPI L.122-5-3 (referenced by the [CNIL][cnil-sheet-en]).
  - Open for a lawyer: whether defeating an anti-bot measure means the access was not "lawful", which would remove the TDM exception for customers. [I]
- **Kneschke v. LAION**: Hamburg Regional Court, September 27, 2024; Hanseatic Higher Regional Court, December 10, 2025, 5 U 104/24. An opt-out must be machine-readable *and* actionable, so natural-language terms were not enough at the 2021 download date ([KPW summary][olg-laion], [C]).
- **CJEU, Like Company v. Google** (C-250/25), the first TDM reference, is pending ([OJ notice][like-company]).

**Personal data**

- **CNIL web-scraping focus sheet** (FR June 19, 2025; EN January 5, 2026) [P] ([EN][cnil-sheet-en]; [FR][cnil-sheet-fr]):
  - Scope: legitimate-interest scraping of personal data for AI development.
  - **Mandatory:** exclude sites that "clearly oppose" scraping through robots.txt or CAPTCHA.
  - Exclusion of sites that object through their terms of use is recommended, not mandatory.
  - Without the robots.txt and CAPTCHA exclusion, the processing falls outside data subjects' reasonable expectations.
- **EDPB Guidelines 03/2026 on web scraping for generative AI** (adopted July 7, 2026; consultation until October 30, 2026; **draft**) [P] ([PDF][edpb-0326]; [news][edpb-news]):
  - Same exclusion measure, listing "robots authentication to view content, robots.txt or ai.txt, or CAPTCHA".
  - A scraper working on documented instructions may be a processor (¶18), or a joint controller (¶20).
- **Enforcement:** the CNIL fined KASPR €240,000 for collecting LinkedIn contact details that users had restricted ([CNIL, Dec. 19, 2024][kaspr]); the order was closed on March 4, 2026.
- **International DPA statements** ([2023][joint-2023]; [2024][joint-2024]) [P]: platforms should use bot detection, CAPTCHAs, rate limits and IP blocking against *unlawful* scraping.
- **GDPR changes under the Digital Omnibus**, including legitimate interest for AI training, are still in Parliament committee: draft report of June 22, 2026, with more than 1,750 amendments ([EP Legislative Train][omnibus-gdpr]).

**AI Act**

- Art. 53(1)(c) has applied since August 2, 2025 [P] ([AI Act Service Desk][aiact-53]). Providers of general-purpose AI models must have a copyright policy that respects art. 4(3) opt-outs "including through state-of-the-art technologies".
- The Code of Practice copyright chapter (July 10, 2025) [P] ([EC][cop-ec]; [text][cop-copyright]):
  - Measure 1.2: when crawling, do not circumvent effective technical measures (paywalls are the example given).
  - Measure 1.3: respect robots.txt (RFC 9309), including for crawlers "used on their behalf".
- The AI Omnibus, in force since July 27, 2026, mainly moves high-risk deadlines ([EP Legislative Train][ai-omnibus]). The sources reviewed show no change to art. 53(1)(c).
- Automan is not itself a model provider, but its customers may be [I].

### How comparable vendors and projects frame it [V]

| Actor | What they offer | Framing and controls |
| --- | --- | --- |
| **Bright Data** | Residential proxies; Web Unlocker "handles headers, cookies, CAPTCHAs" ([docs][bd-kyc]) | AUP bans data behind login, fake accounts and ticket bots ([AUP][bd-aup]). Human-reviewed KYC is required for residential access, scoped to the approved use case ([KYC][bd-kyc]). Trust Center with a PwC assurance report ([Trust Center][bd-trust]). Won *Meta* and settled *X* (see above). |
| **Oxylabs** | Proxy networks | AUP: no breach of target terms, only public data unless the site permits, no sensitive data ([AUP][oxy-aup]). Still sued by Reddit as a trafficker (pending). |
| **Zyte** | API with ban handling: CAPTCHA handling, fingerprint and TLS rotation, "200,000 real-world browser profiles" ([Zyte API][zyte-ban]) | AUP bans access where the customer agreed to terms prohibiting that manner of access, bans scraping LinkedIn, and requires AI Act compliance ([AUP][zyte-aup]). |
| **Browserbase** | CAPTCHA solving on by default; Advanced Stealth custom Chromium ([CAPTCHA docs][bb-captcha]; [changelog, Jan. 17, 2025][bb-stealth]); Web Bot Auth identity (see ADR-0002) | Docs: use CAPTCHA solving "only for workflows that users and website owners authorize". |
| **SerpApi** | Search-scraping API that solves challenges | Litigating; frames the July 20 Google order as a win for open access ([blog][serpapi-blog]). |
| **Camoufox** (MPL-2.0) | "Open source anti-detect browser built for webscraping & AI agents" ([README][camoufox]) | No legal positioning seen in the README excerpt reviewed. |
| **Crawlee fingerprint-suite** (Apache-2.0) | "Browser fingerprinting tools for anonymizing your scrapers" ([repo][fp-suite]) | Neutral, privacy-flavored description. |
| **2Captcha / CapSolver** | Human (2Captcha) or AI (CapSolver) solving services | "Authorized and legal purposes" only; bans illegal access ([2Captcha ToS][2captcha]). Bans fake accounts and ticket buying, and takes abuse reports, including breaches of target terms ([CapSolver][capsolver]). |

**Pattern.** The controls are contractual: KYC, AUP, indemnities, a "public data only" rule, and category bans (login, accounts, tickets, fraud). None of the texts reviewed deals with §1201 directly. Only Browserbase scopes solving to site-owner authorization. [I]

## Recognized mitigations

| Mitigation | What it addresses | Basis | Where it can be enforced |
| --- | --- | --- | --- |
| **Identified access by default**: Web Bot Auth signature, declared user agent, verified-bot programs | Shows authorized, non-deceptive access; avoids "stealth as evidence of willfulness" | [Cloudflare verified bots][cf-verified]; [Cloudflare on Perplexity][cf-perplexity]; ADR-0002 | All. Enforceable in the cloud. |
| **Scope circumvention to authorized targets**: own sites, QA, partner consent, attestation and allowlist | §1201(a)(3)(A) "without the authority of the copyright owner"; L.335-3-1 research and security carve-outs | [ch. 12][usc1201]; [*Prestige*][prestige]; [Browserbase][bb-captcha] | Cloud (allowlist); private module (contract); public framework (defaults only) |
| **Stop on challenge** (typed error, never a false success) unless an authorized scope applies | Removes the act of circumvention | ADR-0002 §4; [*Prestige*][prestige] | All |
| **Respect revocation**: a cease-and-desist or explicit block stops all access to that host for the tenant and globally; no IP rotation in response | CFAA "without authorization" after revocation | [*3Taps*][3taps-aug]; [*hiQ II*][hiq2] (distinguishing *Power Ventures*) | Cloud |
| **Logged-off public pages only by default**; no account creation | Contract and CFAA exposure; fraud context | [*Meta v. Bright Data*][meta-bd]; [*hiQ* SJ][hiq-sj]; [Bright Data AUP][bd-aup] | All (default); cloud (enforced) |
| **robots.txt (RFC 9309), TDM opt-outs (TDMRep, machine-readable), CAPTCHA as objection** for personal data or AI purposes | GDPR mandatory measure; CDSM art. 4(3); AI Act art. 53(1)(c) and CoP 1.3 | [CNIL][cnil-sheet-en]; [EDPB][edpb-0326]; [OLG Hamburg][olg-laion]; [CoP][cop-copyright]; [RFC 9309][rfc9309] | All. In the cloud, hard-disable `stealth` and solving when a workflow declares personal data and `ai-train`. |
| **KYC and use-case approval** before stealth, managed proxies or solving | Shows diligence; limits abusive tenants | [Bright Data KYC][bd-kyc] | Cloud; private module |
| **AUP with category bans**: login-gated data, fake accounts, ticketing (BOTS Act), government, health and minors' sites, fraud | Sector statutes; GDPR sensitive categories; fraud cases | [Bright Data AUP][bd-aup]; [Zyte AUP][zyte-aup]; [CapSolver][capsolver]; [BOTS Act][bots-act]; [CNIL][cnil-sheet-en] | Cloud; private module |
| **Neutral naming and marketing**: no "bypass", "undetectable" or "solve any CAPTCHA" | "Marketed for circumvention" prongs: §1201(a)(2)(C); InfoSoc 6(2)(a); L.335-3-1 II 4° | [ch. 12][usc1201]; [InfoSoc][infosoc]; [L.335-3-1][cpi-l335] | All. This is the main lever for the public framework. |
| **Auditable consent for residential and mobile exit nodes** (supplier diligence) | ePrivacy 5(3); botnet and criminal risk | [Google IPIDEA][ipidea]; [911 S5][911s5]; [ePrivacy][eprivacy] | Cloud |
| **Provenance and audit logs**: profile, options, policy decision and challenge result per attempt | Evidence of purpose and diligence; GDPR accountability | ADR-0002 §5 | Cloud; self-host |
| **Per-tenant kill switch and abuse reporting** | Fast response to notices | [CapSolver abuse process][capsolver]; [Bright Data][bd-aup] | Cloud |

## Questions for a lawyer

1. **US trafficking through open source.** Would publishing an autonomous CAPTCHA and challenge solver in an Apache-2.0 repository make Automan (a French company) and individual maintainers "traffickers" under §1201(a)(2), given *Corley* and *Reddit v. SerpApi*? Does a *site-agnostic* solver escape "primarily designed" because no specific protected work is targeted?
2. **Adapter and interface.** Is a public *interface* for a solver, with no solving logic, safe? Is a public *adapter* to a third-party solving service (customer's own account) a "component or part" under §1201(a)(2)? Should the adapter leave the public repo, as ADR-0002 tier 3 currently places it there?
3. **Human handoff.** Is an operator solving a challenge inside their own automated session "circumvention", in light of *Prestige*'s treatment of CAPTCHA farms? Does the answer change if Automan staff or contractors solve?
4. **Dual-use stealth.** Which design and documentation choices keep fingerprint consistency, TLS impersonation and automation-marker reduction outside §1201(a)(2) and InfoSoc art. 6(2)? *Reddit* treated proxies and human mimicry as alleged circumvention means.
5. **Hosted solver.** What is Automan Cloud's exposure as a direct circumventor *and* a service trafficker, including per-act statutory damages and §1204 criminal exposure? Is a solver limited to targets with an attested site-owner authorization a sound defense?
6. **Copyright nexus.** How much protection does *Google v. SerpApi* (copyrighted work required; measure authorized by the copyright owner) give in practice, and what should we watch at the **October 13, 2026** hearing? Is an interlocutory appeal in *Reddit* likely?
7. **US jurisdiction and structure.** Given *X v. Bright Data* (personal jurisdiction), the pending Oxylabs extraterritoriality motion and *Ryanair v. Booking* (CFAA extraterritorial), should managed solving sit in a separate entity, or exclude US customers or US-hosted targets? Does that really reduce exposure?
8. **CFAA and revocation.** After a cease-and-desist naming Automan Cloud, does continued access through rotated IPs create CFAA liability (*3Taps*, *Power Ventures* versus *hiQ II*)? What written revocation-handling policy is needed?
9. **Contract.** When the cloud operates logged-off, is it bound by target terms through browsewrap (*Meta*, *X v. Bright Data*)? Is there exposure for inducing customers' breaches or for tortious interference by selling tools? How far does Copyright Act preemption protect us?
10. **France, technical measures.** Is a CAPTCHA or bot-detection system an "effective technical measure" under CPI L.331-5 and InfoSoc art. 6(3), including through the *sui generis* database right, especially after *Anne Frank Fonds* (C-788/24)? Does L.335-3-1 II reach (a) public open-source availability and (b) a managed service? Do the research and security carve-outs help a general framework?
11. **France, Code pénal 323-3-1.** Is a solver or stealth module a program "conçu ou spécialement adapté" to commit 323-1 to 323-3? Is solving a CAPTCHA on a public site *accès ou maintien frauduleux* (*Bluetouff*)? Does general-purpose scraping count as a "motif légitime"? What is the personal exposure of directors and contributors, and the liability of the company?
12. **GDPR role and liability.** When customers scrape personal data, is Automan Cloud a processor or a joint controller (EDPB 03/2026)? If a customer uses managed solving on a site protected by a CAPTCHA, for AI training, against the CNIL's mandatory exclusion, what is Automan's liability (art. 28(3)(h) duty to alert, art. 82(2))? Is a technical block on personal data plus `ai-train` plus `stealth` required, or only advisable?
13. **Residential and mobile proxies.** What diligence on proxy suppliers is needed under ePrivacy art. 5(3), the GDPR and Code pénal 323-1 and 323-2, given the IPIDEA and NetNut actions? Should Automan Cloud offer residential exit nodes at all, or only customer-supplied ones?
14. **Text and data mining.** Does defeating an anti-bot measure make access not "lawful" under CDSM art. 4(1) and CPI L.122-5-3, so that customers lose the TDM exception? How should the product implement machine-readable opt-outs after *Kneschke v. LAION* (OLG Hamburg, Dec. 10, 2025) and pending *Like Company* (C-250/25)?
15. **AI Act customers.** Can providers of general-purpose AI models that signed the Code of Practice (Measure 1.2 non-circumvention; Measure 1.3 crawlers "used on their behalf") use the `stealth` profile? What representations should Automan give and require in the contract?
16. **Database right.** For repeated, systematic cloud extraction, what is the exposure under the *sui generis* right (*CV-Online*, *Leboncoin*) and under French unfair competition and parasitism? Does stealth aggravate damages or bad faith?
17. **Marketing review.** Which words in docs, READMEs, pricing pages and SDK names risk the "marketed for circumvention" prongs (§1201(a)(2)(C), InfoSoc 6(2)(a), L.335-3-1 II 4°)? Should a lawyer review the docs before release?
18. **Distribution platforms.** Does hosting on GitHub, crates.io, npm or PyPI, all US-based, add takedown or jurisdiction exposure for solver-adjacent code? Would hosting elsewhere change the legal analysis or only the operational risk?
19. **Contributors.** Do we need a CLA or DCO, or contributor indemnities and guidelines, for stealth and solver code? What is individual contributors' personal exposure, in the US and in France?
20. **Insurance and contract allocation.** Is tech E&O or media liability insurance available for circumvention claims? Which indemnities and caps should customer terms contain for `stealth` and managed solving?

## Uncertainties and gaps

- **Reddit opinion.** The PDF of *Reddit v. SerpApi* (ECF 104) is a scan, so I could not read the court's text. The holdings come from the court's docket text [P]; the reasoning comes from Loeb & Loeb [C].
- **Google order.** The details of *Google v. SerpApi* come from a model summary of FindLaw's copy of the order and need checking against the original. The renewed motion has a hearing on October 13, 2026, three days after this note.
- **Pending matters.** Reddit's claims against Oxylabs and AWMProxy, and any appeal in *Reddit*, are unresolved.
- **No EU or French ruling found** on three points: whether a CAPTCHA or bot detection is an "effective technical measure"; whether solving a CAPTCHA on a public site is *accès ou maintien frauduleux*; and whether 323-3-1 applies to a stealth or solver module. All EU and French levels for F3 are therefore marked `(?)`.
- **Drafts and secondary readings.** The EDPB Guidelines 03/2026 are a draft under consultation. The CNIL sheet is soft law, though the CNIL enforces against it. The OLG Hamburg LAION ruling is read through commentary.
- **Unknown terms.** The settlement terms in *X v. Bright Data* are not public in the sources reviewed. *Meta v. Bright Data* is read through commentary.
- **Vendor statements** are self-descriptions, not verified practice.
- **Not researched:** US state laws (for example California Penal Code §502), UK law, and the export-control or sanctions angle.

<!-- References (all accessed 2026-10-10) -->

[usc1201]: https://www.copyright.gov/title17/92chap12.html
[reddit-104]: https://law.justia.com/cases/federal/district-courts/new-york/nysdce/1:2025cv08736/651592/104
[reddit-op-pdf]: https://www.nysd.uscourts.gov/sites/default/files/2026-07/25cv8763%2C%20Reddit%20Inc.%20v.%20SerpApi%20LLC%20et%20al.%20-%20Opinion%20%26%20Order-1.pdf
[loeb]: https://www.loeb.com/en/insights/publications/2026/08/reddit-v-serpapi-llc
[reddit-oxy-opp]: https://storage.courtlistener.com/recap/gov.uscourts.nysd.651592/gov.uscourts.nysd.651592.114.0.pdf
[oxy-mtd]: https://storage.courtlistener.com/recap/gov.uscourts.nysd.651592/gov.uscourts.nysd.651592.96.1_1.pdf
[google-order]: https://caselaw.findlaw.com/court/us-dis-crt-n-d-cal/350669.html
[google-complaint]: https://s3.documentcloud.org/documents/26438560/google-v-serpapi-complaint.pdf
[google-mtd2]: https://storage.courtlistener.com/recap/gov.uscourts.cand.461513/gov.uscourts.cand.461513.49.0.pdf
[google-continuance]: https://www.courtlistener.com/docket/72059948/55/google-llc-v-serpapi-llc
[serpapi-blog]: https://serpapi.com/blog/google-v-serpapi-the-court-granted-our-motion-to-dismiss/
[prestige]: https://law.justia.com/cases/federal/district-courts/california/cacdce/2:2017cv07232/690314/32
[rmg]: https://dockets.justia.com/docket/california/cacdce/2:2007cv02534/387246
[corley]: https://cyber.harvard.edu/people/tfisher/IP/2001%20Corley%20Abridged.pdf
[github-dmca]: https://docs.github.com/en/site-policy/content-removal-policies/dmca-takedown-policy
[vanburen]: https://www.supremecourt.gov/opinions/20pdf/19-783_k53l.pdf
[hiq2]: https://caselaw.findlaw.com/court/us-9th-circuit/2169935.html
[hiq-sj]: https://caselaw.findlaw.com/court/us-dis-crt-n-d-cal/2182242.html
[hiq-consent]: https://www.docketbird.com/court-documents/HiQ-Labs-Inc-v-Linkedin-Corporation/Order-by-Judge-Edward-M-Chen-granting-405-CONSENT-JUDGMENT-AND-PERMANENT-INJUNCTION-Modified-on-12-8-2022/cand-3:2017-cv-03301-00406
[3taps-apr]: https://www.dmlp.org/sites/dmlp.org/files/2013-04-30-Order%20Granting%20in%20Part%20and%20Denying%20in%20Part%20Motions%20to%20Dismiss,%20Granting%20Motion%20to%20Bifurcate.pdf
[3taps-aug]: https://law.justia.com/cases/federal/district-courts/california/candce/3:2012cv03816/257395/101
[ryanair-jmol]: https://www.ded.uscourts.gov/sites/ded/files/opinions/20-1191_1.pdf
[goldman-ryanair]: https://blog.ericgoldman.org/archives/2025/03/court-overturns-a-bad-jury-verdict-against-scraping-ryanair-v-booking-guest-blog-post.htm
[meta-bd]: https://www.quinnemanuel.com/the-firm/news-events/client-alert-meta-v-bright-data-significant-decision-for-web-scraping-industry/
[x-bd-2024]: https://case-law.vlex.com/vid/x-corp-v-bright-1072963649
[x-bd-sac]: https://storage.courtlistener.com/recap/gov.uscourts.cand.415869/gov.uscourts.cand.415869.156.0.pdf
[x-bd-cc]: https://cases.justia.com/federal/district-courts/california/candce/3:2023cv03698/415869/263/0.pdf
[x-bd-pj]: https://docs.justia.com/cases/federal/district-courts/california/candce/3:2023cv03698/415869/67
[x-bd-law360]: https://www.law360.com/cases/64c158650e7ec102e1e49064/articles
[bots-act]: https://www.govinfo.gov/content/pkg/USCODE-2016-title15/pdf/USCODE-2016-title15-chap2-subchapI-sec45c.pdf
[ftc-concert]: https://www.justice.gov/archives/opa/press-release/file/1358896/dl
[ftc-bots]: https://www.ftc.gov/business-guidance/blog/2025/04/bots-act-compliance-time-refresher
[storm1152]: https://blogs.microsoft.com/on-the-issues/2023/12/13/cybercrime-cybersecurity-storm-1152-fraudulent-accounts/
[storm1152-complaint]: https://s3.documentcloud.org/documents/24223189/storm-1152-complaint.pdf
[911s5]: https://www.justice.gov/archives/opa/pr/911-s5-botnet-dismantled-and-its-administrator-arrested-coordinated-international-operation
[ipidea]: https://cloud.google.com/blog/topics/threat-intelligence/disrupting-largest-residential-proxy-network
[netnut]: https://cloud.google.com/blog/topics/threat-intelligence/google-continued-disruption-residential-proxy-networks
[cf-perplexity]: https://blog.cloudflare.com/perplexity-is-using-stealth-undeclared-crawlers-to-evade-website-no-crawl-directives/
[britannica]: https://www.courthousenews.com/wp-content/uploads/2025/09/perplexity-ai-complaint.pdf
[nyt-perplexity]: https://www.courthousenews.com/wp-content/uploads/2025/12/new-york-times-perplexity-ai.pdf
[infosoc]: https://eur-lex.europa.eu/eli/dir/2001/29/oj/eng
[nintendo]: https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=celex%3A62012CJ0355
[annefrank]: https://eur-lex.europa.eu/legal-content/EN/TXT/HTML?uri=CELEX%3A62024CJ0788
[cpi-l335]: https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI000006279236
[cpi-mirror]: https://www.baumann-avocats.com/CPI/article-l335-3-1-du-code-de-la-propriete-intellectuelle.php
[cp-131-38]: https://www.labase-lextenso.fr/gazette-du-palais/2018-n16/pas-d-exception-au-quintuplement-de-l-amende-pour-les-personnes-morales-GPL321w8
[cp-323]: https://www.legifrance.gouv.fr/codes/section_lc/LEGITEXT000006070719/LEGISCTA000006149839/
[bluetouff]: https://www.legifrance.gouv.fr/juri/id/JURITEXT000030635061
[dir2013-40]: https://eur-lex.europa.eu/eli/dir/2013/40/oj/eng
[cv-online]: https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:62019CJ0762
[ryanair-pr]: https://www.ippt.eu/sites/ippt/files/2015/IPPT20150115_ECJ_Ryanair_v_PR_Aviation.pdf
[leboncoin]: https://www.doctrine.fr/d/CA/Paris/2021/C0E3F30A13EA86EFBEE37
[cdsm]: https://eur-lex.europa.eu/eli/dir/2019/790/oj/eng
[olg-laion]: https://kpw.law/en/ai-training-despite-reservation-of-use
[like-company]: https://eur-lex.europa.eu/legal-content/DE/TXT/PDF?uri=OJ%3AC_202503039
[cnil-sheet-en]: https://www.cnil.fr/en/legal-basis-legitimate-interest-focus-sheet-measures-implement-case-data-collection-web-scraping
[cnil-sheet-fr]: https://www.cnil.fr/fr/focus-interet-legitime-collecte-par-moissonnage
[edpb-0326]: https://www.edpb.europa.eu/system/files/2026-07/edpb_guidelines_2020603_webscraping_v1_en_0.pdf
[edpb-news]: https://www.edpb.europa.eu/news/edpb-sheds-light-on-anonymisation-and-web-scraping-for-generative-ai-and-adopts-final-version_en
[kaspr]: https://www.cnil.fr/fr/aspiration-de-donnees-sanction-de-240-000-euros-lencontre-de-la-societe-kaspr
[joint-2023]: https://ico.org.uk/media2/migrated/4026232/joint-statement-data-scraping-202308.pdf
[joint-2024]: https://www.priv.gc.ca/en/opc-news/speeches-and-statements/2024/js-dc_20241028
[omnibus-gdpr]: https://www.europarl.europa.eu/legislative-train/theme-a-new-plan-for-europe-s-sustainable-prosperity-and-competitiveness/file-digital-package
[ai-omnibus]: https://www.europarl.europa.eu/legislative-train/theme-internal-market-and-consumer-protection-imco/file-digital-omnibus-on-ai
[aiact-53]: https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-53
[cop-copyright]: https://artificialintelligenceact.eu/en/cop-copyright/
[cop-ec]: https://digital-strategy.ec.europa.eu/en/policies/contents-code-gpai
[eprivacy]: https://eur-lex.europa.eu/eli/dir/2002/58/oj
[bd-aup]: https://brightdata.com/acceptable-use-policy
[bd-kyc]: https://docs.brightdata.com/products/residential/network-access.md
[bd-trust]: https://brightdata.com/trustcenter
[oxy-aup]: https://oxylabs.io/legal/oxylabs-acceptable-use-policy
[zyte-ban]: https://www.zyte.com/zyte-api-b
[zyte-aup]: https://www.zyte.com/terms-policies/acceptable-use-policy
[bb-captcha]: https://docs.browserbase.com/platform/identity/captcha-solving
[bb-stealth]: https://www.browserbase.com/changelog/advanced-stealth
[camoufox]: https://github.com/daijro/camoufox/blob/main/README.md
[fp-suite]: https://github.com/apify/fingerprint-suite
[2captcha]: https://2captcha.com/terms-of-service
[capsolver]: https://docs.capsolver.com/en/guide/position
[cf-verified]: https://developers.cloudflare.com/bots/concepts/bot/verified-bots/
[rfc9309]: https://www.rfc-editor.org/rfc/rfc9309.html
