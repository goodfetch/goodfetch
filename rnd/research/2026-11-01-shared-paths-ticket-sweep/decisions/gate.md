# Decisions: Host gate spec (#1693, absorbing #1694)

## Reversible choices

- Absorb #1694 as a ticket: #1693 lists refusal among what it asks and #1694 is that outcome's surface; alternative, leave it linked as its own spec's work.
- Leave #1670, #1671 and #1672 linked: other shapes a host speaks through (a Limit's name, a Run's notes, a retry time), and no ticket needs them; alternative, one "host surfaces" spec, which would mix unrelated work.
- A Plugin kind, as triage decided: three methods and one suite; alternative, an embedding-only interface, which leaves a Daemon host with nothing.
- Two forms, process and composed; decline the SDK-handler form: a handler lives in a Client, which is the caller being held, and on a shared Daemon a Client could register the name; alternative, triage's three forms with a rule on who may register. Departs from triage and ADR 0033.
- The composed form is a `Contribution` value, the engine's seam under ADR 0104; alternative, a public Rust SDK option, which ADR 0104 rejects.
- Hold by leaving `host_gate.admit` unanswered, with a `holding` call back naming the label; alternative, triage's `wait { label }` answer, which needs polling or a second call and breaks at most 1 gate call per hop. Departs from triage.
- A silent gate past `timeout.host_gate` (5 s) is a failure, unless it said it holds: this is what gives "does not answer" a meaning under a held call; alternative, only the request's own timeout, which never fails a dead gate closed.
- A request counts as held only once the gate says so; alternative, report any slow answer, which would report a gate's latency as a wait.
- Fail closed through a new retryable `host_gate_unavailable`, not `host_refused`: a broken gate must not mark sites refused or skip them; alternative, triage's plain refusal.
- `on_failure = admit` is a recorded weakening (`weakens`, Audit log, `weakened`); alternative, an unrecorded setting, against ADR 0028.
- Settings are `Only Operator policy`, narrower than `Configuration`: a person's user configuration has no need of it and a caller on a shared machine might hold that layer; alternative, `Configuration`.
- Setting names `host_gate.plugin`, `.on_failure`, `.address`, `.max_held`, `timeout.host_gate`: `host_gate.plugin` over `.provider`, since no third party provides anything; easily renamed.
- `host_gate.max_held` at 1,024 per Run: giving capacity back would otherwise let a many-domain Frontier hold unbounded attempts; alternative, keep capacity while held, which fails the issue's rule.
- A crawl request gives back its HTTP admission and handler concurrency while held, and keeps its Politeness Permit; alternative, asking in delivery's admission loop before the attempt, which leaves hops unsolved and needs a pending set in that loop.
- A handler's own held navigation keeps its handler's place: the handler is running and can't be paused, as for a Permit today; alternative, none that works. The issue's "no global capacity" therefore holds for crawl requests and hops, not a running handler's calls.
- The first hop asks at the attempt's start, before a browser page opens, and the first document carries the admission: no page is held open, and asks stay at 1; alternative, asking in interception, which holds a BrowserPage admission.
- A service worker's request carrying a navigation is asked about; triage left out "service workers' requests": otherwise a controlled page steps around the gate (ADR 0121). Its other requests stay out. Departs from triage.
- Add `site_file` asks (robots.txt, TDMRep, RSL, sitemaps): otherwise a refused site still receives robots.txt, against #1694's point; alternative, triage's four kinds. Departs from triage.
- A Plugin's `net.fetch` stays out, listed in Out of Scope: rarely aimed at a site and needs its own call site; alternative, gate it now.
- An outcome is told at the response's headers on every tier, not at a browser page's `load` as a Politeness Permit is released: one meaning across tiers, and it is when status and first byte are known; alternative, `load`, which has no equivalent on document tiers.
- `enhance_your_calm` carries `exact`, false on the browser tier: Chromium can't tell it apart (ADR 0118's note); alternative, a separate outcome kind for `net::ERR_HTTP2_PROTOCOL_ERROR`.
- A timeout during a hold names `host_gate` in `deadline_exceeded`'s details and backs off nothing: the site never saw the request; alternative, a new Error code, which breaks "that timeout's usual code".
- A wait counts toward `timeout.handler` too, since a handler's time includes what it awaits; alternative, excluding it as a Permit wait is excluded.
- `host_gate.address` defaults to `resolve` with a setting to turn it off, because the issue asks for the address and the engine rarely holds it at that moment (the relay and HTTP client resolve at connect); alternative, only what's already known, nearly always `null` on a browser page.
- A third-party gate gets only the origin of a logged-in Session's request unless `session_credentials` is approved (ADR 0033); alternative, an exemption for this kind.
- Report per request in the event and Run trace, and by label in the Run report (32 labels, then `other`): a report per request would be unbounded on a crawl; alternative, triage's "per-request entry in the Run result".
- Host text bounds of 80 and 280 characters, matching #1670's triage; easily changed.
- One outcome queue per Run with one task of the tracker: no spawn per outcome and no delay to a response; alternative, awaiting each outcome inline.
- A held call gives back its credit: otherwise outcomes wait behind held asks and a gate can deadlock itself; alternative, a separate credit pool for outcomes.
- Keep instances per Run (ADR 0033), with cross-Run state at the host's endpoint or a composed value; alternative, a shared instance, which ADR 0033 rejects.
- Network levels `none` and `proxy_tcp`, as Proxy provider has; alternative, adding the kind to `direct`'s list, which would leave its writes unchecked. A host needing `direct` uses the composed form.
- Rows: separate `C5.gate_*` and `F2.gate_*` rather than triage's single row, because the two paths have different units and benchmarks; all are tightenings within the contract.
- Tickets' gate links: only the tracer and #1694 use #1868, the links they already hold; other tickets take E1 (#1870), E5 (#1856), F2 (#1848) or E6 (#1858), and inherit #1868 through the tracer.
- Milestone `M4 Everything else` for all tickets: nothing earlier depends on them, and #823 and its kin are M4; alternative, earlier for the composed-form tickets, with no reason to rush.
- The prefactor ticket comes first and adds no work; alternative, fold it into the tracer, which would crowd one context.
- Spec title names the Host gate; #1693 and #1694 keep their titles.

## OWNER

None. No money, legal or terms risk, or public commitment arises, and no Cost contract is raised. The two-form kind contradicts ADR 0033's wording, but nothing is published, so it is reversible; it is recorded below for the owner's review.

## CLOSE

None. #1693 and #1694 stay open as tickets. #1670, #1671 and #1672 are not moot.

## ADRs and glossary edits the spec needs (not written here)

- **New ADR, "A host's gate":** the Host gate kind and its interface, and that it isn't Politeness and holds a program's own navigation.
- **Amends ADR 0033:** the kinds list gains the Host gate, and this kind takes two forms. A catalog kind declares its forms.
- **Amends ADR 0079:** `host_refused` in `RefusedError`, `host_gate_unavailable` in `UnavailableError`, and the skip reason `host_refused`.
- **Amends ADR 0080:** a Host gate's wait counts toward the request's timeouts and `timeout.handler`, unlike a navigation gate's Permit wait; `timeout.host_gate` joins the family.
- **Notes on ADRs 0086 and 0087:** the Host gate sits beside the page's value, and a program's page may be held by it, while "a page a program opens has no navigation gate" stays true.
- **Note on ADR 0118:** outcomes reuse the `ENHANCE_YOUR_CALM` reading and its browser-tier limit.
- **Note on ADR 0121:** a service worker's request carrying a navigation takes the Host gate as it takes the navigation gate.
- **Note on ADR 0140:** C5's and F2's Cost contract text gains "with a Host gate: at most 1 call, 1 outcome and 1 lookup per request or hop". The register gains the `C5.gate_*` and `F2.gate_*` rows through the tickets' pull requests.
- **`GLOSSARY.md`:** **Host gate**: what a host supplies, through an Operator policy or the engine's composition, for the engine to ask before each request it sends to a site, after Politeness; it admits, holds or refuses the request and learns its outcome. _Avoid_: rate limiter, throttle, gatekeeper. The Navigation gate's entry stays; it may add "not a Host gate".
