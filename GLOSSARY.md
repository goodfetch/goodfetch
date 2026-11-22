# Goodfetch

Goodfetch is the organization behind two open-source Products: Collie, for browser automation, and Foxhound, for crawling and collecting web data, both with AI-assisted workflows. Goodfetch Cloud runs their engines as a hosted service.

## Products and the engine

**Goodfetch**:
The organization and umbrella brand behind Collie, Foxhound and Goodfetch Cloud; not the name of a Product or of anything a person installs to get one.
_Avoid_: framework, library, platform, Automan (its former working name)

**Product**:
One thing Goodfetch ships for people to install, Collie or Foxhound: an executable with its SDKs, CLI, MCP server and agent skills. Goodfetch Cloud is a hosted service, not a Product.
_Avoid_: edition, flavour, distribution

**Collie**:
The Product that drives one browser page at a time: the CDP driver and its guard, the page API, AI actions, and Extraction from the page it is on.
_Avoid_: browser module, agent

**Foxhound**:
The Product that discovers and collects many pages: the Frontier, crawl scope, Politeness toward sites, the HTTP fetch tiers, Datasets and Sinks. It contains Collie, which it uses to render pages and act on them.
_Avoid_: spider, scraper

**Product version**:
The one version that Collie, Foxhound and the packages people install carry, released together; a product major is its major.
_Avoid_: lockstep version, release train

**Engine**:
The runtime inside each Product that executes every run, whichever surface starts it; Foxhound's engine includes Collie's.
_Avoid_: framework, core, backend

**Engine instance**:
One process of an engine, named anew each time it starts and given in its `hello`: the positions of its events count within it, so a Client that meets a new one, as after a Daemon restart, resumes from an `events.resync`.
_Avoid_: engine ID, boot ID, session (a Session is a browsing identity)

**Goodfetch Cloud**:
The hosted service that runs the engine for customers on Goodfetch's infrastructure.
_Avoid_: platform, Goodfetch (for the hosted service)

**Kennel**:
Goodfetch's local tool for watching and controlling the runs of a machine's Daemon, published apart from the Products under its own version; part of no Product.
_Avoid_: dashboard, tray app, monitor

**Desktop notice**:
A message the Kennel shows through the operating system's notifications, only while it runs, when a Run or the Daemon changes to a state that needs a person; not a Notification, which the engine sends whether or not a Kennel runs.
_Avoid_: alert, notification (for the Kennel's)

**Needs you**:
The engine's one list of everything waiting for a person across every run in the Run index and the stores beside it, which a Client reads, each item of a kind whose source, a domain crate's value, fills it, and gone once resolved or dismissed.
_Avoid_: inbox, to-do list, attention list

**Client**:
Any program that drives the engine through its operations: an SDK, the CLI, the MCP server or Goodfetch Cloud. Speaking CDP to a browser does not make a program a Client.
_Avoid_: frontend, binding, consumer

**Connection**:
A Client's open link to one engine, through which a program reaches its Runs, and the scope of the settings and Limits set where it opens. Closing it never stops a Daemon.
_Avoid_: client (for the Connection), engine (for the Connection), session (a Session is a browsing identity)

**SDK API**:
What a developer writes and calls in a Goodfetch SDK: its objects, methods, types and errors, generated in part from the Operation catalog.
_Avoid_: consumer API, public surface

**Handle**:
A value of the SDK API through which a program acts on one thing the engine runs or holds, such as a Run, a Session, a Page or a Locator: each of its methods sends an operation of that thing, the raw catalog call included. A Handle on a Locator stays that lazy Locator and is never bound to an element.
_Avoid_: proxy, stub or wrapper (for a Handle), remote object, element handle and JS handle (Playwright's values bound to one element or one page value, which Collie does not have)

**CLI**:
The command-line interface of each Product's executable, through which a person or a script runs that Product's commands; Foxhound's keeps every command of Collie's.
_Avoid_: shell, terminal, command-line tool

**Attached engine**:
An engine started by one Client for its own use, which stops when that Client goes away.
_Avoid_: sidecar, child engine

**Daemon**:
A long-lived engine that several Clients share and that outlives each of them.
_Avoid_: server, service, background engine

**Exposed Daemon**:
A Daemon that Clients reach from other machines.
_Avoid_: server, remote engine, hosted engine

**Daemon group**:
Daemons that serve one team as one, sharing the control state of a Lead Daemon.
_Avoid_: cluster, fleet

**Lead Daemon**:
The Group member of a Daemon group that holds the group's shared control state.
_Avoid_: master, primary, leader

**Group member**:
Any Daemon of a Daemon group, the Lead Daemon included. Not the *member* role of a person on a shared Daemon.
_Avoid_: member (alone), node, worker, replica

**Group backup**:
A consistent copy of a Lead Daemon's stores from which another Group member can become the Lead Daemon.
_Avoid_: snapshot, Checkpoint (which is a run's)

**Error code**:
The stable name a Client receives when an operation fails, such as `page_check_failed`, which programs can match on and which keeps its meaning across releases of a major version.
_Avoid_: error name, error kind, status code

**Trace context**:
The W3C values a request carries so that its caller's OpenTelemetry trace continues in the process that receives it.
_Avoid_: trace (alone), span context, correlation ID

**Private component**:
A closed component kept outside the open-source Products, used by Goodfetch Cloud and licensed under contract to vetted customers, that holds a capability whose public release would itself create legal exposure.
_Avoid_: enterprise edition, premium module, closed core

**Browser host**:
The process, started from either Product's executable, that sits beside a browser and carries out a Session's operations for an engine running elsewhere.
_Avoid_: worker, agent, sidecar

**Borrowed browser**:
A browser the engine did not launch and never closes: the person's running browser, one their own code launched, or a third-party cloud browser reached by its address.
_Avoid_: external browser, attached browser

**GUI session**:
The graphical login session of the operating-system user an engine runs as, such as a logged-in macOS desktop, which the full browser build needs to start and the headless shell does not; unrelated to a Session.
_Avoid_: desktop session, user session (a Session is a browsing identity)

**Lent page**:
A page of a Session that a program hands, for a time, to another automation library such as Playwright or Puppeteer, through a CDP endpoint that shows only that page's Session; the reverse of a Borrowed browser.
_Avoid_: shared page, exposed page, remote page

**CDP**:
The Chrome DevTools Protocol, through which the engine drives a Chromium-family browser over the pipe it started it with, and through which a program's CDP session or a Lent page's library reaches a page.
_Avoid_: DevTools protocol, debugger protocol

**Plugin**:
A package that adds implementations of one or more Plugin kinds to the engine, such as delivery to a Sink, a fetch backend or a Secret provider; the reverse of an Integration, which puts a Product inside another tool.
_Avoid_: extension (a browser extension is something else), add-on, connector, module

**Plugin kind**:
One of the fixed places where the engine accepts a Plugin, each with its own contract and conformance suite.
_Avoid_: extension point, hook (a Hook is one kind), slot

**Hook**:
The Plugin kind whose implementations run at a fixed stage of a request, a navigation, a page, a Record or a run's lifecycle, and may change, skip, retry or fail the work at that stage, or only observe it.
_Avoid_: middleware (for a Hook scoped by a Label), pipeline (for a Hook before a Record's commit), callback, interceptor

**Script Hook**:
A Hook written as a short script that the engine runs in the Script sandbox, so it can travel inside a Recipe or settings without an SDK or a process Plugin.
_Avoid_: inline hook, user script

**Script sandbox**:
The confined place where the engine runs Code mode programs and Script Hooks, with only the catalog calls it allows and with time and memory limits.
_Avoid_: code-mode sandbox, VM, isolate, JS runtime

**SDK handler**:
An implementation of a Plugin kind written in a program's own code and given to its SDK, which the engine calls over the program's connection; it runs with the program's rights and is never confined.
_Avoid_: injectable component, provider (for the form), callback

**Plugin index**:
The signed list of community Plugins that people search to find one, with each Plugin's conformance results.
_Avoid_: registry (for the index; Plugins live in OCI registries), marketplace, hub

**Experimental**:
The status of an operation, setting, SDK method, metric or part of the engine's trace output released outside semver's compatibility promise until it is promoted.
_Avoid_: unstable, nursery

## Pillars

**Extraction**:
Turning a page or a response into structured data or clean content.
_Avoid_: scraping, scraper

**Main content**:
The part of a page's document a person reads it for, without its navigation, asides, banner and footer; a page with only those has none, a value and never an error.
_Avoid_: article body, boilerplate-free text

**Extractor**:
What finds a document's Main content: dom-smoothie by default, trafilatura, or an Extractor Plugin; the step every page format reads the Main content through.
_Avoid_: content extractor

**Page formats**:
What `page.formats` answers from one serialization of a page's document: each format the request names, such as its Main content as Markdown, and none it does not.
_Avoid_: scrape formats, outputs

**Crawl**:
Discovering and visiting many pages from seeds, within a declared scope.
_Avoid_: scraping, spidering, scraper

**Automation**:
Driving a browser session through actions toward an objective.
_Avoid_: scraping, scraper, bot

## Execution

**Run**:
One piece of work the engine carries out from start to finish, with its own inputs, state, limits and usage; a Continuous crawl's finish comes only when it is told to end. The unit of isolation, resumption and counting.
_Avoid_: job, execution, session (for a whole run)

**Run ID**:
The name that identifies one Run in the Run index for good, given by its program or generated by the engine; a start under an existing Run ID reaches that Run, never a new one.
_Avoid_: run name, label (for a Run ID), job ID

**Work item**:
One unit of a Run's work that the engine hands to one handler at a time and that ends in exactly one outcome; a crawl's work items are its Crawl requests.
_Avoid_: task, job, message, item (alone)

**Lease**:
The right of one attempt to process a Work item for a bounded time, numbered by an epoch that only goes up; only an attempt whose lease still holds commits what it produced, so a hung or late attempt commits nothing.
_Avoid_: lock (for a Lease), claim, visibility timeout

**Run epoch**:
The number an engine takes when it becomes a Run's writer, higher than any before it; a write that carries an older one is refused, so a copy of the Run that lost its writer role commits nothing.
_Avoid_: generation, term, fencing token (for the Run's)

**Safe point**:
The moment a Run asked to stop can stop without losing or repeating work: it admits nothing new, every Effect in flight finishes or is reported uncertain, and every attempt commits or is abandoned under its Lease. A Limit, a Run's timeout, an abort and a drain all stop a Run at its next one.
_Avoid_: checkpoint (for the moment), graceful stop, barrier

**Subscription**:
One Client's stream of an engine's events, each at a position that only goes up within its Engine instance, sent no faster than the credits the Client grants; one whose queue of lossless events is full falls behind to an `events.resync` of the current state, so no producer and no other Subscription waits for it, and a telemetry event replaces the unsent one of its kind, counting it as dropped. A value's `on` returns one in TypeScript and Python (ADR 0081): a page's listener's events come on the `on` call itself, under the same credits, and a Session's on the Connection's own Subscription, which its listeners share, each through its own bounded queue, sent before the first `on` and cancelled once the last leaves; its `close()` cancels that call.
_Avoid_: listener (for the engine's side), feed, channel

**Body handle**:
The name, path and size of a page body stored as a file in its run's folder, which a Client on the same machine reads directly and any other through `page.body`.
_Avoid_: blob, attachment, body reference

**Partitioned crawl**:
One crawl carried out as several runs on several Group members of a Daemon group, or on several engines of Goodfetch Cloud, each owning part of the domains.
_Avoid_: distributed crawl, sharded crawl

**Partition**:
One run of a Partitioned crawl, owning the domains assigned to it.
_Avoid_: shard

**Goodfetch home**:
The folder holding one user's Goodfetch data, caches and configuration on a machine, shared by both Products: the operating system's usual folders unless the user names another.
_Avoid_: data dir, app folder, install folder

**Resource governor**:
The one part of each engine process that admits HTTP fetches, browser pages and model calls for all its runs, each kind between its floor and its ceiling, sized from the memory, CPU and Tokio saturation it measures and shared fairly between the runs.
_Avoid_: autoscaled pool, concurrency manager

**Memory allowance**:
The most memory an engine and its children may use, the tightest of its own cgroup's limit, its Windows Job object's, `governor.memory_allowance` and half of physical memory, against which its Resource governor judges pressure.
_Avoid_: memory budget, memory limit (alone)

**Run report**:
What a run states about itself when it ends, and a Continuous crawl also while it goes on: its Lifecycle, Verdict and Completeness, its Usage, every safeguard it ran without and, for a crawl, how its requests ended and, for a Continuous crawl, its Freshness.
_Avoid_: run result, summary, statistics, stats

**Browser cache**:
The per-user folder, shared by both Products and every major, of pinned browser builds, among them those Plugins declare, the executable copies a Daemon runs from and the engine's other pinned downloads, each checked by digest. Plugin runtimes have their own runtime cache beside it.
_Avoid_: browser registry, browsers folder

**Pinned artifact**:
A download a Product version names by URL and digest, such as a browser build, Playwright's driver or a full Clean-up rules set, which the Browser cache holds and the engine fetches with a plain HTTPS GET, the only connection nobody configures. A Chrome for Testing build is fetched instead below `browser.mirror` when configuration sets one, an Operator-configured endpoint, and its bytes still have the pinned digest.
_Avoid_: asset (for a pinned download)

**Run index**:
The catalog of every run of one user and one product major, whichever Product ran it, on a machine or in a Daemon group, pointing at each run's own state, from which it can be rebuilt, except the rows of runs discarded on purpose.
_Avoid_: run registry, run database, history

**Usage ledger**:
The per-user record, on one machine or in a Daemon group, of the Usage reserved and consumed by every run of that user, which stores the Limits on the install, a Daemon, a person, a named token, a Connection or a Partitioned crawl and against which they are held.
_Avoid_: billing, meter, spend log

**Slice**:
A part of a Limit one run takes from the Usage ledger in one transaction, the smaller of 1 % of the Limit and what is left over twice the live runs, against which it charges the Usage it measures; it gives back what is left when it ends, and the next engine to open the ledger reclaims the Slices of a run whose engine died, from what the run journaled.
_Avoid_: lease, reservation (both for a Slice), quota, chunk

**Reservation**:
The worst case of a cost a run is about to pay, such as a model call or a paid solve, which it takes from its Slices and journals before the cost is sent, and settles at what it cost once known; one still open when the run ends, or when a reclaimer finds it, stays charged at its worst case.
_Avoid_: pre-authorization, escrow, Slice (for a Reservation)

**Effect**:
An action of a run that changes something on a website or third-party service, such as submitting a form or placing an order; a program's own actions are Effects only when it declares them. A resumed run checks whether an effect already happened instead of repeating it, and reports it as uncertain when it cannot tell.
_Avoid_: side effect, mutation, write

**Effect key**:
The name a program gives an Effect it declares, unique within its Run or within one crawl request, by which a retried or resumed run finds that Effect in the Effect journal.
_Avoid_: step ID, effect name, idempotency key (the opaque key a site receives)

**Reconciliation**:
A person's recorded answer about an uncertain Effect: it happened, it did not happen, or it stays uncertain. An agent can ask for one but never give it.
_Avoid_: resolution, manual fix, override (for this answer)

**Verdict**:
A run's or an AI task's statement of whether its Success checks, Page checks and Grounding prove what a run committed or what an AI task returned: verified, rejected by a failed Success check, or needing a person's review.
_Avoid_: status, success, result (for the verdict)

**Recipe**:
A portable package of user code, described by a manifest, that runs the same locally or on Goodfetch Cloud and can be published.
_Avoid_: actor, app, pack, script

**Setting**:
A named value that shapes how a Product works, such as a timeout, a retry count or a Sink's guarantee, resolved through layers from its built-in default to the run itself, where a lower layer can cap or lock it.
_Avoid_: config key, knob, preference, option (for a setting)

**Preset**:
A named set of default Settings for one use case, laid over the built-in defaults so that each value it sets can still be changed.
_Avoid_: profile (a Session's Identity profile), template (a Recipe to copy), mode

**Deployment variable**:
One of the few environment variables a Product reads for its own configuration, each holding a value that differs by machine rather than by program, such as the Goodfetch home or the address of an Exposed Daemon.
_Avoid_: env setting, config variable

**Operator policy**:
The settings an operator fixes for every run on a machine, a Daemon or a Daemon group, including the values runs may not exceed or change.
_Avoid_: admin config, global settings, defaults

**Usage**:
The raw quantities a run consumed, such as requests, browser time or LLM tokens, without any price.
_Avoid_: cost, consumption, credits

**Checkpoint**:
A consistent, self-contained copy of a run's state from which the run can resume, on the same machine or another one.
_Avoid_: snapshot, savepoint, backup

**Durability level**:
What a run's state survives, chosen per run with `run_state.durability`, each value a preset of the batch settings (`run_state.batch.*` and `run_state.commit_interval`), which a person may also set one by one: a crash or a power cut loses at most the last Run-state batch at `batch`, the default; nothing but the attempts in flight at `full`; nothing to a process crash, and recent commits to a power cut, at `fast`; nothing to a process crash, and at most one `run_state.commit_interval` to a power cut, at `periodic`; at `off` a run file a crash left is discarded. A clean stop loses nothing at any level.
_Avoid_: persistence mode, sync mode, fsync mode

**Run state**:
What a run keeps to go on and to resume. A crawl's lives in memory, owned by one actor: its Frontier, the Leases of its attempts in flight and the Permits it alone serves; its run file holds it as of the last Run-state batch.
_Avoid_: crawl state, job state

**Run-state batch**:
The changes to a run's state since the last batch, written to its run file in one transaction.
_Avoid_: flush, journal (an Effect journal is another thing), snapshot

**Delegation**:
The right of one run, or of one engine's runs together, to serve a Politeness key's Permits from memory while no other engine uses the key; a recall writes the key's state back to the per-user Politeness file or the Politeness store.
_Avoid_: lease (a Lease is a work item's), lock, oplock, Slice

**Limit**:
A cap on the Usage of one run, or of every run counted against a scope (a Schedule, a Partitioned crawl, the install or a Daemon, a person, a named token or a Connection), over a period or a Connection's lifetime; reaching it stops runs cleanly and refuses new ones until the period ends or a person raises it.
_Avoid_: budget, quota, spending cap

**Model allowance**:
The most model Usage one run, or one Partitioned crawl, may spend, past which its model calls are refused while it goes on; part of its own task, not a Limit.
_Avoid_: model budget, model Limit, AI quota

**Path budget**:
An entry of `crawl.path_budgets`: a glob matched against a URL's path and query, and the most requests a crawl admits whose URL it matches first, past which one is not added with `path_budget` and the Run ends `incomplete` (CO-08); a bound on a section of a site, not a Limit on Usage.
_Avoid_: path limit, section quota

**Price table**:
The list prices of third parties' Usage, per model and per metered proxy, with fixed rates between currencies, shipped with each release and changed only in configuration or, for a Plugin's, its manifest, through which a currency Limit reserves and a price is reported; an estimate, never a bill.
_Avoid_: price list, cost map, rate card

**Schedule**:
A standing instruction, kept by the Daemon or Goodfetch Cloud, that starts runs of one target when its time comes or its trigger fires.
_Avoid_: cron job, periodic job, monitor, trigger (for the whole Schedule)

**Watch**:
A small run that checks whether chosen pages or values have changed, so that a Schedule starts its target only on change.
_Avoid_: monitor, probe, poll

**Tenant**:
A customer of Goodfetch Cloud. The engine never knows tenants.
_Avoid_: account, organization, user (for a customer)

## Collection

**Fetch tier**:
One way a crawl request's document is fetched, from the cheapest up: an HTTP request, the site's own API, a request sent from a browser page, a browser without layout, or the full browser page; a crawl uses the cheapest whose results match the browser's, or the one its settings force.
_Avoid_: fetch mode, fetcher, backend (for a tier)

**Frontier**:
The URLs a run has discovered but not yet visited, with the list of those already visited, when each is due again in a Continuous crawl, and the rules that decide when two URLs are the same page.
_Avoid_: queue, request list, to-do list

**Page signature**:
The SHA-256 of a fetched page's title and the text of its main content, white space collapsed, by which the Frontier tells whether two URLs serve one page.
_Avoid_: fingerprint, content hash, signature (alone)

**Learned rule**:
A rule the Frontier learns on one host from the pages a crawl fetched and the redirects it followed (a query parameter with no effect, an equivalent form such as `www.` or a trailing slash, a trap path or cursor pagination preferred to offset), kept with its evidence and every URL it merged, and listed in the Run report; a parameter or form a contradicting pair of pages later shows wrong stops merging and gives back the requests it had ended, and the report says so.
_Avoid_: canonicalization rule, URL filter

**Label**:
The name a crawl gives one kind of request, which picks the handler that processes it, the shape of its Payload and the Hooks scoped to it.
_Avoid_: route (for a Label), page type (for a Label), callback (for the name)

**Payload**:
The data a program attaches to a request for its handler, owned by the program and kept apart from the engine's own fields.
_Avoid_: userData, user_data, meta, cb_kwargs, context

**Crawl request**:
One request a crawl's Frontier admitted (its method, URL and body), with its Label and Payload: the work item one handler processes, which ends in exactly one outcome.
_Avoid_: `Request` (the type or identifier is a page's network request), item, task, job

**Continuous crawl**:
A crawl whose run does not end when its Frontier empties: it revisits each URL when its Revisit policy says and reads its sources again, until it is told to end.
_Avoid_: recrawl, standing crawl, monitor, never-ending run

**Revisit**:
A Continuous crawl's next visit to a URL it already visited, which first checks whether the page changed.
_Avoid_: recrawl, refetch, re-visit, retry (for a revisit)

**Revisit policy**:
The rule, set per Label, that decides when a Continuous crawl visits a URL again: never, at a fixed interval, adapting to the changes seen, from an estimate of how often it changes, or by a source's rate of new entries.
_Avoid_: schedule, fetch interval, refresh rate

**Freshness**:
How far a Continuous crawl's revisits keep up with their due dates, overall and per domain; never part of Completeness.
_Avoid_: staleness, lag (for the whole measure), recency

**Syndication feed**:
An RSS or Atom document listing a site's recent entries, which a crawl reads as a source of URLs.
_Avoid_: feed (alone, a Subscription's word), news feed, RSS (for Atom too)

**Politeness**:
The per-domain and per-proxy limits on how hard a user's runs press one site or one proxy, shared by those runs.
_Avoid_: rate limiting, throttling

**Politeness store**:
The optional store through which engines on several machines share Politeness.
_Avoid_: rate limiter, Redis

**Permit**:
One request in flight under a Politeness key, such as a domain key, that an engine takes from the per-user Politeness file before the request starts and gives back once it ended, recording the run and run epoch that hold it, so the count in flight is every sharing run's together.
_Avoid_: token, slot, lease (a Lease is a work item's)

**Completeness**:
A run's statement of whether its results cover everything in scope: complete, incomplete (ended early) or unverified (finished, but the engine cannot vouch that nothing was missed).
_Avoid_: success, done, finished (for coverage)

**Session**:
A reusable browsing identity of a run: its cookies, login state, Access mode, Identity profile and proxy, shared across the pages fetched under it.
_Avoid_: login, profile, context

**Page**:
One browser tab of a Session, which a program, an Agent or a crawl handler drives: it navigates, is read and receives gestures, and belongs to its Session for its whole life.
_Avoid_: tab, target (for a page)

**Spare page**:
A Page its Session keeps open between two of a crawl's requests, which the Session's next request navigates instead of opening a new Page.
_Avoid_: idle tab, pooled page

**URL pattern**:
A glob or a regular expression that a wait, a route or a kept response matches URLs against, in the engine, so that it matches the same URLs from every SDK; a predicate is no URL pattern, since it runs in the program.
_Avoid_: URL filter, URL match

**Route**:
A program's handler for the requests of a page, or of every page of a Session, whose URL its URL pattern matches: it answers each by sending it on, answering it with a response of its own, failing it, or passing it to the route armed before; one that answers nothing passes it on (ADR 0085).
_Avoid_: interceptor, request handler

**Interception**:
The engine's one pipeline that every request of a page passes, in one written order: routes, the egress check and the domain policy, the Filter lists, the Response cache, the navigation gate and lease check, then the engine's own additions, such as the Session's cookies, which no route sees (ADR 0085).
_Avoid_: middleware, request interception (for Playwright's routing)

**Navigation gate**:
What Foxhound sets on a crawl handler's page to admit, at the Interception's step 6, each main-frame navigation and hop, each `page.request` and each service worker's request carrying a navigation, holding one with a permit or refusing it for a site's preferences, and to judge a document once its response arrives; a page a program opens has none (ADRs 0087 and 0093).
_Avoid_: politeness hook, admission hook, gatekeeper

**Lease check**:
What a crawl handler's page asks before each command it sends: whether its work item's Lease still holds, so a page past its Lease sends nothing more and raises the error Foxhound gives, and which call encloses its calls (ADRs 0003 and 0087).
_Avoid_: lease guard, lease hook

**Enclosing call**:
The call whose timeout bounds the calls made inside it, such as a crawl handler's call for its page's calls: a call inside ends at that call's end if its own comes later, and its `deadline_exceeded` then names that call as `ended_by` (ADR 0080).
_Avoid_: parent call, outer deadline

**Response observer**:
What Foxhound registers on a crawl handler's Session to receive the responses its pages observe whose URLs its URL patterns match, each body up to a byte cap, those a service worker answered included (ADR 0110).
_Avoid_: response hook, response listener, sniffer

**Encoding**:
The character encoding a document's or a response body's bytes are decoded with, a WHATWG Encoding, found in ADR 0119's order: a BOM, a forced one, the `Content-Type` charset, the declaration in its own bytes as Chrome reads it, then UTF-8.
_Avoid_: character set, code page

**Access mode**:
How a Session presents itself to sites, fixed when it opens: standard (an ordinary browser), identified (a declared, signed agent) or stealth (masked).
_Avoid_: profile, access profile, stealth level

**Identity profile**:
The consistent set of values a Session presents to sites, such as its user agent, network and browser fingerprints, signing key, language and time zone, fixed for the Session's life.
_Avoid_: fingerprint (alone), persona, profile (alone)

**HTTP profile**:
The part of an Identity profile a Session's HTTP-only requests present: the TLS ClientHello, HTTP/2 settings, header order and client hints of the browser it claims.
_Avoid_: impersonation profile, TLS profile

**Fingerprint method**:
How a Session's browser comes to present its Identity profile: not changed at all, changed by page scripts, changed through the browser's debugging protocol, or changed inside the browser engine itself.
_Avoid_: stealth engine, spoofing mode, fingerprinting

**Goodfetch Chromium**:
The Chromium that Goodfetch builds with its fingerprint patches on each Chrome Stable version, and ships as the browser of the engine-level Fingerprint method.
_Avoid_: Goodfetch build (alone), patched Chrome, stealth browser, fingerprint-chromium (for Goodfetch's own build)

**Challenge**:
A check a site's bot protection puts between a client and the content, such as an interstitial page, an invisible browser test or a CAPTCHA.
_Avoid_: block (for a challenge), captcha (for every kind), wall (for a challenge)

**Challenge solver**:
What completes a Challenge on a Session's behalf, whether Goodfetch's own logic, a third-party solving service or a Plugin.
_Avoid_: unblocker, bypass, captcha solver (for every kind)

**Session pool**:
The Sessions a crawl keeps, scoring, rotating and retiring them as they fail; a Session a program or an Agent opens itself is never in it.
_Avoid_: session manager, account pool

**Proxy tier**:
A rung of the ladder of proxy kinds a Session's proxy is drawn from, cheapest first: none, datacenter, ISP, residential, mobile; a block that escalates moves its domain one tier up, where its Session's replacement opens.
_Avoid_: proxy level, proxy class

**Saved identity**:
A Session's identity, cookies and storage, kept under a name so that later runs can reuse them.
_Avoid_: profile, context, login

**Browser data folder**:
The folder a browser the engine launched for a Run keeps its data in while it runs, in the Run's folder, one per browser of the Run's browser pool: Chrome's data directory (`--user-data-dir`), fresh for each launch and removed once the browser exited, never a person's Profile directory.
_Avoid_: profile (alone), user data dir

**Profile directory**:
A browser's own folder of a person's browsing data, on which the engine can launch that browser in place; for the Chromium family, the data directory (Chrome's `--user-data-dir`), not the profile folder inside it that Chrome's `--profile-directory` names.
_Avoid_: user data dir, profile (alone)

**Session source**:
Where a Session's login comes from: a Saved identity, a file or a cookie header, a browser installed on the machine, a Profile directory or a Borrowed browser.
_Avoid_: login source, auth source, profile

**Block cause**:
The reason the rules give for a site refusing a page, such as a rate limit, a challenge, an IP block or a demand for payment, which decides how the run responds.
_Avoid_: block type, ban, error (for a block)

**Paid fetch**:
A fetch for which a run pays the site, from the user's own account or wallet, after the site asked for payment.
_Avoid_: purchase, paid request, micropayment

**Response cache**:
The per-user store of responses that later runs may reuse instead of fetching the page again.
_Avoid_: HTTP cache, page cache, crawl cache

**Response recording**:
The responses one run received, kept in a WARC file the program names, from which a later run is answered instead of the network.
_Avoid_: cassette, HAR (for a recording), fixture (alone), Response cache (for a recording)

**Filter list**:
A list of rules, published or the user's own, naming the ads, trackers and page annoyances that a run blocks, hides, or removes from what it extracts.
_Avoid_: blocklist, adlist, ruleset

**Consent prompt**:
A site's request that the visitor accept or refuse cookies or tracking, shown as a banner, a modal or a wall that withholds the content.
_Avoid_: cookie banner, cookie modal, CMP (the vendor software behind it), consent popup

**Wall**:
An overlay a site draws over its page as the condition for reading the content: a paywall, a consent wall or an ad-blocker wall.
_Avoid_: gate, barrier, paywall (for every kind), wall (for a Challenge or a demand for payment)

**Wall allowance**:
A person's approval for a domain on which the engine reads past Walls instead of stopping at them.
_Avoid_: bypass, unblock, circumvention

**Clean-up rules**:
The dated collection of Filter lists, the rules that recognise and answer Consent prompts, the rules that recognise paywall and ad-blocker overlays, and never-block entries, which a run holds to for its whole life and its results name.
_Avoid_: rule set, list bundle

**Live view**:
A Session's browser shown to a person while it works, which that person can take control of during a Human pause.
_Avoid_: screencast, stream, remote desktop

**Run trace**:
The recording of what a run's Sessions did, such as their screens, page snapshots and network traffic, kept so a person can replay it.
_Avoid_: session recording, replay (for the recording), video

**Gesture**:
A call that sends input to a page or navigates it, such as a click, a key press or a navigation; the engine runs one page's gestures one at a time.
_Avoid_: interaction, command (for a gesture)

**Locator**:
What a page action looks for on a page, such as a CSS selector, resolved again at each use and never bound to one element; an action needs it to match exactly one element. A program holds one through a Handle.
_Avoid_: element handle (a value bound to one element, which Collie does not have), query, selector (for the value)

**Guard**:
The engine's check of each locator action: it waits until the target is unique and actionable (visible, enabled, stable and receiving the event, as the action needs), cancels a gesture that lands on another element and replays it, until the action's timeout.
_Avoid_: actionability check, auto-wait

**Page function**:
JavaScript source a program runs in a page with one explicit JSON argument, its result crossing back as JSON, in the page's main world by default or in an isolated world; an init script is one run before each document's own scripts.
_Avoid_: injected code, user script

**Page check**:
A declaration by a run's code of what an expected page and a login page look like, against which every page fetched in a session is judged.
_Avoid_: contract, validator, assertion

**Page analysis**:
The one step in which a fetched page is parsed once, or read in one streaming pass when no reader needs a tree, and read by every reader its crawl or call needs, leaving one result that every later step shares.
_Avoid_: parse step, pipeline stage, page processing

**Dataset**:
The append-only collection of a run's Records, named by the crawl that fills it, which may give it a schema and the fields that identify one Record.
_Avoid_: results table, output, items

**Record**:
One unit of structured data in a Dataset, committed once with the work that produced it.
_Avoid_: row, document, item (an item is a unit of work)

**SQL tab**:
The read-only SQL query over the runs a person selects, each run's tables a table function that reads its run file, with no write, attach, install or outside read and with time, memory and row limits (`dataset.query`).
_Avoid_: SQL console, query editor, DuckDB tab

**Key-value store**:
A run's named storage for files, blobs and the user's own state.
_Avoid_: KVS, blob store, cache

**Export**:
A file the engine writes from a run's results in a chosen format.
_Avoid_: dump, download

**Sink**:
A system outside the engine that receives a run's committed Records, as rows or messages or as Export files, while the run is in progress or afterwards.
_Avoid_: output, pipeline, connector

**Guarantee class**:
How strongly a Sink keeps a delivery from being duplicated or overwritten by a stale copy of the run, probed for each deployment: Fenced, Record-fenced, Keyed or At-least-once.
_Avoid_: delivery semantics, QoS, consistency level

**Notification**:
A message about the progress or outcome of a run or a Schedule, sent to a person or a system, that carries no Records.
_Avoid_: alert, event (for the message), webhook (for every channel)

**Integration**:
A package that puts a Product inside another tool, such as a workflow tool node or a document loader; one that drives the engine is a Client.
_Avoid_: connector, plugin, app

**Baseline**:
A previous run, or Partitioned crawl, that a run's results are compared with to report what changed.
_Avoid_: snapshot, reference run

## AI and agents

**Plan**:
The concrete steps of a task, produced by an AI or recorded from a person, each with a target and its Target fingerprint, kept as a versioned file and replayed without a model.
_Avoid_: cache, script, prompt

**Target fingerprint**:
What identifies a Plan step's target to a person, checked before any Effect to tell the same element from a look-alike.
_Avoid_: fingerprint (alone, it means a browser or TLS identity), selector, locator

**Drift**:
The state of a Plan step whose target no longer matches its Target fingerprint, whose page text no longer parses under the Value transform the step keeps, or whose element is no longer in the language kept for that transform.
_Avoid_: breakage, flake

**Repair**:
One bounded recompilation of a Plan after Drift, checked in full before it acts and kept only after a verified run.
_Avoid_: self-healing, auto-fix

**Effect journal**:
The record of the Effects and model calls of a run, consulted so that a retried, corrected or resumed step never performs an Effect twice.
_Avoid_: action log, history

**Operation catalog**:
The single list of engine operations, each classed read, idempotent write or Effect and marked with the smallest Product that carries it; every surface of a Product exposes that Product's operations to people and agents.
_Avoid_: tool list, API surface

**Answer**:
An operation's whole result as an SDK hands it to a program: its typed view, which reads only the fields that SDK knows, and the JSON it came from, which keeps the others (ADR 0082). A member that answers one field of its result or opens a Handle on what it started answers that value alone.
_Avoid_: response (an HTTP Response), reply (the JSON-RPC message), envelope

**Code mode**:
Driving the engine through a short program written by a model that calls only the Operation catalog.
_Avoid_: scripting, eval

**Framework adapter**:
A package that offers a Product's operations as tools of an agent framework, such as the Vercel AI SDK, LangChain or Mastra.
_Avoid_: wrapper, toolkit, Integration

**MCP host**:
An application that connects a model to MCP servers, such as Claude Code, VS Code or ChatGPT, and so reaches a Product through its MCP server.
_Avoid_: MCP client (for the application), agent app

**Success check**:
A condition declared by the user that the engine evaluates live to decide whether a step or a task succeeded. A Page check judges every page of a Session; a Success check judges an outcome.
_Avoid_: check (alone), assertion, judge, validator

**Agent**:
A model-driven loop over the Operation catalog toward an objective, whether Goodfetch's built-in one or an outside one; a single AI extraction is not an Agent.
_Avoid_: bot, assistant

**Agent tool**:
A function of the user's program that Collie's built-in Agent may call as an action during a task, such as a lookup in the program's own data.
_Avoid_: function tool, custom action, skill, Plugin

**Model provider**:
Whatever answers a run's model calls: a model company reached directly, a gateway, a local model server, or a model run inside the engine.
_Avoid_: LLM vendor, backend

**Model reference**:
The name by which a program chooses a model: a Model provider's name, a slash, and that provider's own identifier for the model, such as `anthropic/claude-sonnet-4-5`.
_Avoid_: model name, model string, model ID (for the whole reference)

**Decision provider**:
A Model provider whose model picks one option from a fixed list with a confidence, or abstains.
_Avoid_: classifier, router

**Grounding**:
The requirement that every extracted value appears in the page it was read from (for an Agent's task, any page the task observed), or in the program's own data that an Agent tool returned, or follows from text found there by a Value transform the engine runs.
_Avoid_: verification, fact-check

**Value transform**:
One of the engine's closed list of deterministic rules by which an extracted value follows from the page text it was read from, such as a number read under the locale `de-DE` or a date-time in a stated time zone, with every parameter stated and recorded in Provenance.
_Avoid_: normalizer, converter, cast

**Element reference**:
A short name the engine gives an element in an AI accessibility snapshot of a page, such as `e12`, by which a model or a program acts on that element for as long as it stays in the page.
_Avoid_: handle (a Handle is a value of the SDK API, such as a Page or a Locator), element ID, node ID

**Human pause**:
The state of a Session's work while it waits for a person to complete a login, MFA, a CAPTCHA, a confirmation, a review or another step a program asks for.
_Avoid_: manual step, HITL

## Security and compliance

**Secret reference**:
The name by which a run points to a credential, such as a password, a TOTP seed or an API key, without ever holding its value.
_Avoid_: secret (for the reference), credential ID, variable

**Secret placeholder**:
The name, such as `%password%`, by which a Plan, a prompt or a program asks for a secret to be typed or sent, and which a run binds to a Secret reference and to the domains allowed to receive it.
_Avoid_: variable, alias

**Secret provider**:
Whatever turns a Secret reference into its value at the moment of use: the environment, the OS keychain, an encrypted local file, the caller, or an external vault.
_Avoid_: vault (for every provider), secret store, credential manager

**Provenance**:
The record of how a Record, or any other result of a run, was obtained: where and when it was fetched, under which Session and signing identity, by which extractor, Plan or model, and under which Site preferences.
_Avoid_: lineage, metadata, source (alone)

**Purpose**:
What a run declares its results are for (search, AI input, AI training, or plain collection), against which Site preferences are judged.
_Avoid_: use case, intent, mode

**Site preferences**:
What a site states about how it may be crawled and used, such as its robots.txt and its AI-use signals.
_Avoid_: robots (for all of them), policy, rules

**Audit log**:
The tamper-evident record of the security and compliance events of a run, a Daemon or a Daemon group, such as secret uses, confirmations and overrides. The Effect journal prevents repeated Effects; the Audit log answers who did what.
_Avoid_: journal (alone), event log, history

**Secret canary**:
A recognisable fake secret, cookie or token planted in a test; finding it anywhere that must not hold secrets proves a leak.
_Avoid_: honeytoken, dummy secret, fake credential

**Egress canary**:
A forbidden connection that a CI job attempts once its outbound deny is in place, and that must be refused.
_Avoid_: canary connection, network probe

**Platform signature**:
The signature macOS or Windows checks before running a Goodfetch executable (an Apple Developer ID signature with notarization, or an Authenticode signature), naming the legal entity that publishes it. The release's Sigstore signature and attestation, which say which workflow built it, are what Goodfetch itself checks.
_Avoid_: code signing (alone), signature (alone), certificate

## Measurement

**Flaky test**:
A test that passes and fails with the same pinned inputs. Unlike Drift, the fault is in the test or the product, never in the site.
_Avoid_: flake, intermittent test, unstable test

**Earned passes**:
The clean passes a red stress run leaves a crate's tests toward their counts, which the next gate run counts while the crate's inputs, the host and the base stay the same, so each test runs only what its count still owes.
_Avoid_: credit, credited passes

**Live check**:
A check of Goodfetch against something it does not pin, such as a vendor, a real SaaS account, a live model or a moving Chrome channel, run outside the test suites and ending in pass, fail or inconclusive.
_Avoid_: monitor, flaky test, informing job, canary (for a Live check)

**Proving ground**:
The website Goodfetch owns and generates, whose every page has a known expected result, against which the benchmark, the Evaluations, the conformance suites and the Journeys run.
_Avoid_: fixtures site, test site, sandbox, demo site

**Simulation**:
A test that runs several Goodfetch engines or Daemons in one process over a simulated network and simulated time, all drawn from one seed, so that a failure replays exactly.
_Avoid_: chaos test, emulation, sim (alone)

**Fault plan**:
The explicit list of faults, such as crashes, power cuts and partitions, and of Client operations that one test applies at named moments.
_Avoid_: nemesis, fault schedule, chaos script

**Observation log**:
The ordered list of every operation, answer and fault of one Simulation or fault test, against which its invariants are checked. It is a test's record, never a run's.
_Avoid_: history, trace (alone), event log

**Evaluation**:
A measurement of an AI capability with live models on Proving ground tasks, repeated over trials and graded by the Proving ground's oracle, never by a model.
_Avoid_: eval test, AI test, benchmark (alone: the benchmark is what is published)

**Fake model**:
A Model provider the Proving ground serves, which answers from the task and the page it is shown, the same way every time, and counts the tokens of every call itself.
_Avoid_: mock LLM, stub model, fake LLM

**Scripted model**:
A Model provider used only in tests, which gives a fixed sequence of replies and fails on any call beyond it or any reply left unused.
_Avoid_: mock model, fake (alone)

**Model recording**:
A real exchange with a Model provider, recorded once with its secrets removed, that tests replay in place of the provider.
_Avoid_: cassette, fixture (alone), snapshot

**Proving ground task**:
One thing the Proving ground asks a tool to do on one page family, with its starting URL and the expected result its oracle judges against. Called a task where the Proving ground is already the subject.
_Avoid_: fixture, test case

**Standing**:
What a comparison report says of the Product's figure against a competitor's on one metric: ahead when every trial of the Product's is better than every trial of the competitor's, behind when every one is worse, overlapping when their ranges overlap.
_Avoid_: verdict, winner

**Result document**:
The one JSON document a conformance suite or a Journey variant writes each time it is run, of kind `conformance` or `journey`, whose outcomes are derived from what it records and recomputed by every reader, never set by the test.
_Avoid_: test report, run result, results file

**Case log**:
The JSON Lines a runner writes as it runs Client conformance cases, one entry per case run with its checks and evidence and no outcome, which the conformance harness turns into a Result document.
_Avoid_: test log, results log

**Journey**:
A goal a person reaches through a Product's public surfaces, written as steps from a starting state to an outcome checked from outside the engine, kept under a stable ID and run on each surface that offers it.
_Avoid_: scenario, user story, critical user journey, flow, end-to-end test (for the Journey itself)

**Golden scenario**:
A complete program for a task developers commonly write, in every SDK language beside the leader's program for the same task, by which the SDK API's shape is judged. Unlike a Journey, it is judged against a competitor's program, not by the Proving ground's oracle.
_Avoid_: hero scenario, champion scenario, sample, Journey (for a golden scenario)

**Feature**:
One behaviour Goodfetch promises, listed in the Feature inventory under a kind and an ID, such as a matrix row, a user story, an operation or an Error code. A Cargo feature is a different thing and keeps its full name.
_Avoid_: requirement, test target, capability (for an inventory entry)

**Feature inventory**:
The list of every Feature of Goodfetch, generated from the V1 scope matrix, the specs' user stories and the contracts the code defines, against which "every feature is tested" is checked.
_Avoid_: feature list, traceability matrix, requirements list

**Chromium build gate**:
The tests that run where the Goodfetch Chromium is built, on every platform, before its bytes reach a suite or a user, because they need the build's own source tree.
_Avoid_: browser roll check, Chromium suite, smoke build

**Chromium pull request**:
A pull request that changes what the Goodfetch Chromium is built or tested from, such as its Chromium version, its patches, its expected results, or the launch arguments the engine gives it.
_Avoid_: browser roll, Chromium bump (for the pull request)

**Control build**:
The Goodfetch Chromium's source tree at the same revision, built with the same arguments but without Goodfetch's patches, against which the Chromium build gate tells a patch's failure from an upstream one.
_Avoid_: baseline build, vanilla build, reference build

**Time budget**:
The longest median (p50) duration a suite, a CI job or a whole "all green" run may take over its last 20 runs in its tier, as `TESTING.md`'s table gives them; a breach opens an issue (`xtask budgets`), and nothing fails.
_Avoid_: budget (alone), timeout (a job's hard stop)

**Size budget**:
The largest size each released file of a Product may reach on each platform, and the growth a single change may add without an explicit, recorded acceptance.
_Avoid_: size limit (alone), quota, bloat check

**Compatibility corpus**:
The committed files of each contract format: those its current version writes, and one frozen set per released version, which every later engine of that major must read as the release wrote them.
_Avoid_: golden files, legacy fixtures, test data (for these files)

**Scripted engine**:
A stand-in for the engine used only in tests, which plays one script of protocol messages to a Client and fails on any message it did not expect or any step left unused.
_Avoid_: fake engine, mock engine, mock server

**Recording gate**:
A stand-in, used only in the engine's own tests, for what Foxhound sets on a crawl handler's page: a navigation gate, lease check and response observer that record each call the page makes to them and refuse, hold or end as the test sets, so the page's call sites are tested without Foxhound.
_Avoid_: mock gate, fake gate, spy

**Testing kit**:
The public module of each Product's SDK for testing a program written against it, holding a Scripted model and a test engine that reaches no site unless a test records.
_Avoid_: test utilities, test helpers, mock library

**Reference screenshot**:
A committed image of a screen or component in one state, which a visual test compares the current rendering with, pixel for pixel.
_Avoid_: baseline, golden image, snapshot (alone)

**Performance budget**:
The counted costs of Goodfetch's code (instructions, heap allocations, protocol messages and bytes, renders, bundle bytes) that a single change may grow only by a set margin without an explicit, recorded acceptance, and the absolute ceilings that those costs and resident memory may never pass.
_Avoid_: perf test, speed limit, benchmark (for a budget)

**Performance target**:
An absolute figure of time or resident memory that Goodfetch's engine is meant to meet, measured on the owner's machine and reported at each owner test, never failing a single change.
_Avoid_: SLO, goal, budget (for a target)

**Performance series**:
The timed measurements of Goodfetch's latest code taken every night on one controlled machine, read for lasting shifts and never used to fail a single change.
_Avoid_: nightly benchmark, perf dashboard

**Shared path**:
The route every unit of one kind takes through Goodfetch, such as a fetched page, a CDP command, a protocol message or an Audit entry, which most features add work to and one module owns.
_Avoid_: hot path, pipeline, code path

**Counted work**:
Work whose amount repeats exactly from run to run, such as a parse, a compilation, a statement prepared, a file opened, a transaction, a sync or a configuration read, so that a count of it can fail a change where a duration cannot.
_Avoid_: overhead, cost (alone), system calls (for counted work)

**Cost contract**:
The most Counted work one unit of a Shared path may cost, which is part of the interface of the module that owns the path.
_Avoid_: performance budget (for a contract), budget (alone), SLA

**Quality register**:
The written record of Goodfetch's quality bar: the rules every change keeps, the limits that fail a merge, the figures that may only improve and the exceptions that expire.
_Avoid_: constraints file, quality gate (for the record), CONSTRAINTS.md

**Quick suite**:
The tests and checks of Goodfetch that run in-process, on what a change touches or on everything, run after every edit.
_Avoid_: unit suite, smoke tests, fast tests

**Pre-merge suite**:
The Quick suite plus the integration, conformance, sink and SDK tests of what a change touches, on one platform, which every pull request must pass.
_Avoid_: PR checks, CI suite

**Release suite**:
The Pre-merge suite for every package across every supported platform, runtime and browser, which every merge (once the merge queue exists) and every release must pass.
_Avoid_: full suite, nightly suite, matrix (alone)

**Stub gateway**:
A test-only stand-in for Goodfetch Cloud's gateway, kept in the public repository, that serves a Client the same operations across per-run engines, each run on its Product's engine, so the cloud-readiness conformance can run without Goodfetch Cloud.
_Avoid_: mock cloud, fake gateway, cloud emulator
