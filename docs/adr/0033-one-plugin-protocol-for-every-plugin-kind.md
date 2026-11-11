# One plugin protocol for every Plugin kind; the engine accounts for every Plugin and confines process Plugins

V1 extends the engine through **Plugins**, each implementing one or more **Plugin kinds** under one contract per kind and one conformance suite. ADR 0026's pattern for Sinks becomes the rule for all of them: the public sink protocol becomes the **plugin protocol**, with a `hello` handshake that negotiates the protocol version and declares the Plugin's kinds, then one group of JSON-RPC methods per kind (`sink.*`, `fetch.*`, …). The engine accounts for every call into a Plugin, whatever its form: it counts it, classes it, journals what ADRs 0010 and 0012 require, and holds it to Limits. A process Plugin also runs confined on Linux, macOS and Windows, at the network level its manifest declares; where the host cannot provide a level, the engine refuses it rather than weaken it (unless Operator policy allows reduced confinement for a first-party Plugin), and it states plainly what a level cannot check. Decided in [Plugin and extension model](https://github.com/goodfetch/goodfetch/issues/108).

> Amended by [ADR 0132](0132-each-request-may-carry-its-callers-trace-context-across-goodfetchs-own-links-and-the-engines-spans-stay-experimental.md): every request the engine sends a process Plugin may carry `_meta`, its Trace context (`traceparent` and `tracestate`), and the engine uses it only while its own OTLP export is on.

Thirteen kinds:

| Kind | What it adds | Precedent |
| --- | --- | --- |
| Hook | code at fixed stages of requests, navigation, pages, Records and the run's lifecycle, optionally scoped by label | Crawlee `preNavigationHooks`, `postNavigationHooks`, `Router.use`, `EventManager`, browser-pool hooks |
| Fetch backend | the HTTP transport, or a remote fetcher such as an unblocker API | Crawlee `BaseHttpClient`, Spider `with_fetch_engine`, `with_remote_fetcher` |
| Browser backend | a browser to drive, given as one of three answers (below); the in-house driver and the guard stay the engine's (ADR 0019) | Crawlee browser plugins |
| Extractor | an alternative to dom-smoothie (ADR 0007) | Crawl4AI strategies |
| Sink | delivery to a destination, as ADR 0026 decided | Scrapy pipelines, Crawlee storage clients |
| Model provider | a model format beyond ADR 0012's three; ADR 0012's SDK model callback is its SDK-handler form | Stagehand's custom LLM function |
| Decision provider | a Model provider held to the decision contract the [decision models research](https://github.com/goodfetch/goodfetch/issues/92) suggested: one option from a fixed list with a confidence, or a typed abstention | none |
| Secret provider | beyond ADR 0015's built-in providers; its custom HTTP provider becomes an implementation of this kind | — |
| Challenge solver | the public interface a Private component plugs into (ADR 0018) | Crawlee through playwright-extra |
| Proxy provider | a proxy per Session | Crawlee `newUrlFunction`, Spider `with_proxy_strategy` |
| Search provider | joins ADR 0011's interchangeable web search providers | — |
| Code source | email or SMS MFA codes (ADR 0006) | — |
| Notification channel | joins ADR 0027's channels | — |

> Note, 2026-11-11, on the Extractor row (the owner, grilling of decision G-19 of the consolidated review, Q8: « Yes to all », on option (a) as put; the owner did not read this note's wording): "the existing Extractor Plugin kind names the content types it takes" (ADR 0037's matrix, RT-33).
>
> Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading): so a user's converter can take PDFs or any other type while the built-in converters keep the rest (ADR 0119's note of that day).

Every kind accepts three forms: an SDK handler (in the Client's process, over JSON-RPC), a process Plugin (any language, the plugin protocol over stdio) and a Rust trait implementation (in-process, only when Goodfetch is embedded as a Rust crate). Hooks also take a fourth: a **Script Hook**, a JavaScript function run in the Script sandbox (ADRs 0011 and 0051), with no sockets or files of its own, only the catalog calls a Hook gets (`net.fetch` and the `page.*` operations of its stage, journaled) and the run's Key-value store, and time and memory limits, so that a Recipe, the CLI or MCP can carry a Hook without an SDK or a process Plugin. First-party built-ins implement the same traits, so one suite proves built-in and third-party implementations alike.

> Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): in the SDK API, a Hook's SDK-handler form is a crawl's Hook in code, declared on its `Crawl` at the stages that change the work. A Collie program answers its page stages with its own code and the page API, and the observe-only stages reach a program through the Run's own values. A program registers its other SDK handlers on its connection by name, one option per kind, and settings choose them as they choose an installed Plugin. A handler's own exception is its kind's `<kind>_handler_failed`. A program's own implementation of a kind is this SDK-handler form with every engine, a linked one included: the Rust-trait form is the engine's seam for its built-ins, not part of the SDK API.

What the engine enforces differs by form, so every distributed Plugin takes the confined form:
- **Distributed Plugins are process Plugins.** `goodfetch plugins add` and the Plugin index accept process Plugins only. Private components ship only as process Plugins too.
- **SDK-handler and Rust-trait Plugins are the user's own code**, like a user handler (ADR 0003). They run in the user's process with the user's rights, and the engine accounts for every call into them but cannot confine them. Records produced through them say so in their Provenance. The docs state that a third-party package used in these forms runs with full rights.
  > Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): in the SDK API, the user's own code implements a kind only as an SDK handler, with every engine, a linked one included; the Rust-trait form is the engine's seam for its built-ins.
- **Process Plugins are always confined**, as below. Apart from the two named exceptions that follow, the only relaxation is reduced confinement that Operator policy may allow for a first-party Plugin on a host that cannot confine. There is no `unconfined` opt-out: a Plugin that needs more than its confinement allows has to be run as the user's own code. Confinement is not one of ADR 0016's switchable controls: no run, Recipe or setting can turn it off.
- **Two named exceptions run third-party code outside a Plugin's confinement**, each approved per digest by Operator policy: a browser a Plugin declares (below), which runs with the user's full rights as Goodfetch's own browsers do, and a Bridge Sink's container image, confined by a rootless container runtime instead.
- **Script Hooks** are held by the Script sandbox (ADR 0051).

Changes:
- ADR 0003, whose lease check on every outgoing command covers the engine's own commands, not a Plugin's streams at `proxy_tcp` or `direct`, where writes stay fenced at the destination.
- ADR 0011, whose sandbox for code mode also runs Script Hooks: the Script sandbox of ADR 0051.
- ADR 0012, whose SDK model callback becomes the SDK-handler form of the Model provider kind.
- ADR 0015, whose custom HTTP provider becomes an implementation of the Secret provider kind.
- ADR 0016, whose switchable security controls never include Plugin confinement, which each run reports.
- ADR 0017, whose two egress classes apply to Plugins per destination, and whose endpoint rule covers the Plugin proxy.
- ADR 0022, whose third-party CDP browsers can be added by a browser backend.
- ADR 0024, whose browser cache takes browsers a Plugin declares, and whose release gains a runtime cache, a Plugin store, the Linux `goodfetch-sandbox` helper and the Windows filter broker.
- ADR 0026, whose sink protocol becomes the `sink.*` group of the plugin protocol, whose sink processes are process Plugins, and whose Bridge Sinks run only as container images on a rootless runtime.
- ADR 0030, whose versioned contract formats list the plugin protocol, the Plugin manifest and the lockfile.
- ADR 0032, whose release builds, attests and publishes the helper and the broker.

## Considered Options

- **A protocol per kind**: rejected. Each new kind would need its own handshake, cache, versioning under ADR 0030 and trust rules. HashiCorp's go-plugin shows the shared alternative: a child process, a handshake, a versioned protocol, many plugin types (Terraform and Packer).
- **Loading native libraries into the binary**: rejected. Rust has no stable ABI, and a crashing library takes the engine and every run with it, which ADR 0024 chose the binary to avoid.
- **WASM components**: rejected for V1, for the reasons of ADR 0026; process Plugins and Script Hooks cover the same needs.
- **Hooks only as SDK handlers**: rejected. CLI, MCP and Recipe users would get no Hooks without writing a process Plugin.
- **Storage backends as a kind** (Crawlee Python's SQL and Redis storage clients): rejected. ADR 0002 keeps one SQLite file per run so runs stay portable and resumable; a replaceable store would break it.
- **Model providers only by URL and SDK callback**, as ADR 0012 left them: rejected. A CLI or MCP user, who has no SDK, could not add a model format.
- **A browser backend launching an executable it names**: rejected. The engine would start whatever the Plugin chose. A local browser comes only from the engine's digest-checked browser cache, approved per digest.
- **A browser backend limited to local launches and Browser hosts**: rejected. No Plugin could add a remote browser vendor beside Browserbase and Bright Data, which ADR 0022 already reaches over CDP.
- **Trusting a Plugin's own account of its calls**: rejected. The engine counts calls and time itself and classes each call itself. Only spend outside the engine comes from the Plugin, reserved at its declared maximum.
- **Confinement on Linux only**: rejected. The engine ships for Linux, macOS and Windows (ADR 0024).
- **A cross-platform sandbox crate**: none qualifies. `birdcage` is GPL-3.0 and archived, `gaol` is abandoned since 2019, `nono` has no native Windows support and changes fast before 1.0. Codex (`codex-rs`) and Claude Code's sandbox runtime both wrote per-OS code behind one policy, as Goodfetch does.
- **Confining the network per host at the OS level**: not possible; no OS offers it. Codex and Claude Code's sandbox runtime both remove the network and route allowed traffic through a host-side proxy that checks domains, as Goodfetch does through the engine.
- **No network for every process Plugin**: rejected. Drivers such as librdkafka and most database clients cannot use a proxy, so the first-party Sinks that ship as processes (Kafka, MongoDB, SQL Server, ClickHouse and others) could not run.
- **Checking `direct` connections by address with a cgroup BPF filter**: rejected. It needs root and exists only on Linux.
- **A digest pin for every Plugin on resume**: rejected for first-party Plugins, which follow the engine's version; any engine patch would otherwise stop a run from resuming, on another machine too (ADR 0002).
- **Running first-party Plugins with reduced confinement by default** on hosts that cannot confine them: rejected. They are refused by default, and an operator can allow reduced confinement, so the Sinks of ADR 0026 stay available on WSL1 and in containers that refuse user namespaces on kernels before 6.12 without a silent weakening.
- **An `unconfined` opt-out for process Plugins**: rejected. One switch would undo every guarantee for exactly the Plugins that ask for it; the user's own code (an SDK handler) remains the way to run without confinement, knowingly.
- **Installing third-party SDK-handler or Rust-trait Plugins**: rejected. They would run in the user's process with full rights, which is how malicious npm and PyPI packages steal tokens.
- **Handing a Plugin the values of its Secrets**: rejected as the default. The engine inserts a Secret into an outgoing request itself, so the value never enters the Plugin; a value is handed over only per reference, with approval.
- **One dedicated sandbox user on Windows**, as Codex has: rejected. It suits one sandbox at a time; here instances, runs and Daemon accounts would reach each other, and filters set once cannot follow proxy ports chosen at run time.
- **A long-lived signing key for the Plugin index** (minisign): rejected. A stolen key would sign a malicious index; Sigstore signs it in CI, tied to the repository's workflow identity.
- **Automatic Plugin updates**: rejected. A new version can ask for new permissions or come from a hijacked repository.
- **A docs page of community Plugins**: rejected for a CI-validated Plugin index that carries conformance results.
- **Plugins as packages in their language's registry** (npm, PyPI, crates.io): rejected. Installing runs the package's own scripts with full rights before any confinement exists, the attestation covers the top package and not its dependencies, and the install lands in the user's own environment.
- **Plugin instances shared across runs**: rejected. A discarded run's data could outlive it in a live process, against ADR 0001's rule that nothing is left behind; the cost of one process start per run is small next to a run.

## Consequences

**Accounting for every call.**
- The engine counts every call into a Plugin in Usage (calls and time) and records it in the Run trace.
- The engine classes each call itself, from the kind's method and what the call carries: a fetch backend's `GET` is a read, a `POST` may be an Effect, a Sink delivery is an idempotent write, whatever HTTP method it uses. It never takes the class from the Plugin.
- The Effect journal records, under ADRs 0010 and 0012:
  - Effect-class calls
  - every Model provider and Decision provider call
- The answers of a Secret provider or a code source are never recorded: only the reference, the outcome and the time.
- **Plugin output is untrusted.** Everything a Plugin returns is treated like page content under ADR 0014, so text it returns cannot instruct a model. Three kinds differ: a Model or Decision provider's answer is the model's decision, governed by ADR 0014, and the answers of a Secret provider or a code source are secret values that never reach a model.
- Notification channels deliver through ADR 0027's ledger.
- A call made for a work item runs under its lease (ADR 0003); lifecycle Hooks and other calls outside a work item are bounded by a timeout instead.
- **Spend outside the engine** (an unblocker API, a solving service):
  - The Plugin reports raw units (solves, requests, pages) in Usage.
  - Its manifest declares, for each unit, a maximum per call and a price. The operator sees them when allowing the Plugin and can override them, like ADR 0012's price table, and a currency Limit treats a unit with no price as unbounded, refusing the call.
  - Before the call, the engine reserves the maximum times the price against the currency Limit, and refuses a call that would cross it.
  - A reported quantity above the declared maximum is recorded in full, never capped, flagged in the run report, and stops the run if it crosses the Limit.
  - The currency Limit on this spend is only as accurate as the declared prices and the reported units, and the run report says the figures come from the Plugin. These are third-party prices, so ADR 0001 holds.

**Secrets and logged-in state.** A third-party Plugin gets no Secret value and no session credential unless Operator policy approved it for that Plugin, and uses only the Secret references its manifest declares; each rule below applies that principle.
- **Secrets by reference.** A Plugin names a Secret in a dedicated field of a `net.fetch` request: a header or a query parameter, with an optional template such as `Bearer {}`, or a form field only where the operator set one for that Secret. Placeholders in free text are never expanded, so page text cannot pull a value into a request or a prompt.
  - The engine inserts the value on the way out, only toward the domains the run binds the Secret to (ADR 0015), never toward domains taken from the manifest; a reference with no binding is sent nowhere. A value inserted into a query parameter is masked in the Run trace, Usage and the journal.
  - The engine removes the value from the response before the Plugin sees it, raw and in common encodings (base64, URL, hex, JSON escapes).
  - **Limits, stated in every approval:** a bound service that stores or publishes what it receives (an issue body, a profile field) keeps the value where the Plugin's author might read it, and an encoding the scrub does not know returns it.
- **Released values.** A Plugin whose code needs a value itself (a database password over `direct`, a TLS stream through `net.connect` or `proxy_tcp`, which the engine cannot read, or an environment variable valued from a secret field) declares `needs_secret_value` for that one reference. Operator policy must allow each such reference for that Plugin, and the Audit log records every release. A released value leaves its domain binding, since the engine cannot see where the Plugin sends it; the approval says so.
- **Session credentials.** A request or Session that uses a Secret or a logged-in identity for a domain (logged in through a Secret, ADR 0015, or seeded from an import, a Profile directory or a Borrowed browser, ADR 0056) reaches a third-party Plugin only through its `session_credentials` permission, which Operator policy approves for that Plugin and domain and the Audit log records as a release. Ordinary cookies on an anonymous crawl need no approval. Without it:
  - a browser a third-party Plugin supplies (a Browser host or a third-party CDP endpoint, which sees every cookie and typed value) is not used for that Session;
  - a third-party fetch backend does not carry those requests: the engine carries them itself, since a backend with its own TLS stack would hold the cookies and inserted Secrets in clear, and an unblocker would pass them to its vendor;
  - a third-party proxy provider's proxy carries those requests only when they use TLS (HTTPS or WSS), inside a tunnel where it sees the host and not the content; a plain-HTTP or WS request is refused with a typed error rather than sent through it or around it;
  - a Plugin's `net.fetch` never carries the Session's cookies, and a page-stage Hook cannot set cookies on that domain, so no Plugin can switch the Session into another account;
  - the requests, page content and responses of a logged-in Session, which can show tokens, are not passed to the Plugin, Hooks included, and a Hook cannot change those requests; on other requests a Hook sees credentials (`Cookie`, `Authorization`, `Set-Cookie`) and Secret values masked;
  - a Plugin's `page.*` operations cannot run script in the page or read form values, storage or cookies; its separate `page_script` permission, approved per domain, lifts this.
- **Secret providers own their schemes.** A third-party Secret provider declares its reference scheme in its manifest. It may not claim the scheme of a built-in provider or of another installed one, so it never answers for references meant for another provider.
- **No certificate override.** No Plugin answer can turn off certificate checks or add a trusted certificate authority for a Session, as some unblocker proxies ask, since that would hand the proxy every byte of the Session, typed Secrets included.

**What a Plugin may do through the engine.**
- **Calls back into the engine** are possible only during a call the engine made to the Plugin, and carry that call's ID. They act within that call's run, lease and Session and the Plugin's declared permissions, and never outside them. A Hook, and a Script Hook alike, gets `net.fetch` (never `net.connect`) and the `page.*` operations of its stage, never the Run index, settings, the Operator policy or accounts. A Script Hook, which keeps no state between calls, also gets its run's Key-value store (ADR 0051).
- **Page actions** go through the engine (`page.*`), which applies the guard and the journal.
- **Effects need a permission.** Toward the run's sites, a `page.*` action that changes the page (click, fill, submit, key press), a navigation while the Session is logged in, and a `net.fetch` with a method other than `GET` or `HEAD` are Effects. A Plugin performs them only on the domains its `effects` permission names, approved by Operator policy, and they go through the Effect journal (ADR 0010). Calls to a Plugin's own operator-configured endpoints (its vendor's API, a vault, a webhook, a Sink's destination) are not Effects on the run's sites: they are classed by the kind, a Sink delivery as an idempotent write. For a third-party Plugin, a default it ships, and a Recipe or run value that Operator policy has not allowed, is not operator-configured: it is reached through `net.fetch` only and follows the Effects rule. At `direct`, where a Plugin opens its own sockets, the engine cannot hold its writes to this rule; the approval for `direct` states it.
- **Opaque streams are narrow.** `net.connect` reaches only the Plugin's operator-configured endpoints and, for a fetch backend, the target the engine handed to that call; any other stream is refused, since the engine cannot see what it carries. A third-party fetch backend that opens its own streams (for its own TLS stack) can send any request to the targets it is handed, outside the `effects` check; its approval states this, and a run that needs every write journaled uses the engine's own fetch. Through an unblocker, the vendor can likewise send any request to any public address under the user's vendor account and key; the approval states that too.
- **Navigation.** A Plugin's navigations stay within the registrable domain of the page it was called for and, only while the Session is not logged in, the domains it declares. A Plugin's declared domains only narrow what it may reach: they never widen the run's domain policy (ADR 0014), and every request a Plugin causes toward the run's sites is also held to that policy; its operator-configured endpoints are checked under **Egress class** instead. While the Session is logged in, every navigation and every approved page script is journaled as a possible Effect, since a `GET` can change state.
- **Script Hooks** declare their permissions inline, in the Recipe or settings that carry them, approved by Operator policy like a Plugin's. A Script Hook supplied by the run itself (layer 5 of ADR 0028, which an agent through MCP or a workflow tool can fill in) gets none of `effects`, `page_script`, `session_credentials` or Secret references unless Operator policy allows them for it.
- **Challenge solvers** act on the challenge's own widget (the frame or elements the engine's detection identified) within their kind and need no `effects` permission there; acting anywhere else on the page, a form's submit button included, needs it.

**Streams and leases.**
- A `net.connect` or proxied stream is opaque to the engine. It is counted in Usage (bytes and time), recorded in the Run trace by destination, and a `net.connect` stream is closed when the lease it was opened under ends (ADR 0003).
- At `proxy_tcp`, the engine cannot tell which call a proxied stream belongs to, so when a call is abandoned while the instance has a proxied stream open, the instance is restarted under the crash rules.
- At `proxy_tcp` and `direct`, the engine cannot refuse a stale call's traffic as ADR 0003 does for its own commands. Writes stay fenced at the destination, by the run epoch and the Sink's Guarantee class (ADRs 0003 and 0026).

**Hooks.**

| Stage | What a Hook may return |
| --- | --- |
| before request | change the URL, headers or body; skip; fail |
| after response, before parsing | rewrite the body; mark it blocked with a block cause, which feeds escalation (ADR 0021); retry; fail |
| before and after navigation, page ready | page operations through the engine (block resources, set cookies, scroll); fail |
| Record before commit | transform; drop with a reason; reject into the rejection table of the run's Dataset, which the run report shows |
| lifecycle (run start, checkpoint, migration, Limit reached, stop) | nothing: it observes only |

> Amended by [ADR 0103](0103-a-handler-pushes-records-staged-with-its-attempt-into-datasets-its-crawl-declares-and-a-reader-never-slows-the-crawl.md): the "Record before commit" stage runs once the handler returns, only on the Records of the attempt about to commit, and a Hook that fails fails that attempt; a Record a Hook transformed is checked against its Dataset's schema again and rejected into that Dataset's rejection table when it no longer fits; the request still ends `succeeded`, its outcome counting the Records dropped and rejected; a program reads the table with `run.rejectedRecords(dataset)`, and dropped Records are counted by reason, never stored.
> Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): before request changes the URL and headers, never the body, and runs on every attempt with the previous attempt's error. An observe-only `request_ended` stage receives each crawl request's outcome. A Hook before a Record's commit may also be scoped by Dataset.

A before-request Hook may rewrite a URL within the registrable domain of the original request. A rewrite that leaves it must stay within the domains the Hook declares and the run's domain policy, and never gets the run's private-range allowlist or a switched-off resolved-IP check. Changing a request that is an Effect needs the Hook's `effects` permission for that domain, and changing one that carries a Secret or a logged-in identity needs `session_credentials`. A Hook cannot set `Host`, `Cookie`, `Authorization`, `Proxy-*` or method-override headers; the engine inserts Secrets and cookies after the last Hook, and the Effect journal records the request as sent. A Hook's retry of an Effect goes through the journal. What a Hook sees of credentials and Secrets follows **Secrets and logged-in state**, above.

> Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): every Hook's rewrite, whatever its form, is also checked against the crawl's scope and the run's domain policy, and robots.txt and Site preferences read the URL as rewritten. A Plugin's or a Script Hook's keeps this rule on top; a Hook in code, the program's own code, declares no domains and may rewrite to any host that check admits.

Hooks run in declared order and can be scoped by label, which replaces Crawlee's router middleware. A Hook error is a typed `hook_failed`, retried by the work item's policy, and a Hook's time counts against the lease.

> Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): a Hook's retry and a retryable `hook_failed` draw on the crawl request's `crawl.retry.max`, the same budget as its handler's.
> Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): Hooks named in settings run first, layer by layer from Operator policy to the run layer, then a crawl's Hooks in code in the order declared. A Hook's own exception is retried only when `crawl.retry.handler_failed` is on; a Goodfetch code raised inside, when its retryable flag says so.

**Browser backends.** A browser backend answers with one of three:
1. **A browser from the engine's browser cache**, by name and digest. A Plugin may add a browser by declaring a download URL and a digest for each platform in its manifest; the engine downloads it over HTTPS, checking the URL under the page class so a manifest cannot point it at an internal address, verifies the digest at download and again at every launch, and launches it itself over a pipe, like Chrome for Testing (ADR 0024). Such a browser runs as the engine's own browsers do, outside the Plugin's confinement and with the user's full rights, so Operator policy allows it per digest, saying so, it is off by default, and each allowance is recorded in the Audit log. Its launch flags come only from the engine's allowlist, which excludes flags that run commands (such as `--renderer-cmd-prefix`). The engine never launches an executable a Plugin names.
2. **A Browser host address** (ADR 0022).
3. **A third-party CDP endpoint** (as for Browserbase or Bright Data's Scraping Browser), reached as ADR 0022 reaches them, with the guard, and checked under the egress rule below.

**The Challenge solver's contract.**
- **Detection:** the engine detects a challenge by its rules, which beat models there.
- **Call:** it then calls the solver with:
  - the Session handle
  - a page handle
  - the challenge kind and vendor (Turnstile, reCAPTCHA v2, v3 and Enterprise, hCaptcha, DataDome, Akamai, PerimeterX, AWS WAF, GeeTest, Kasada, image CAPTCHA)
  - the run's remaining Limits
- **Acting:** the solver acts on the page only through engine operations, or returns a token the engine injects. When it needs a vision model, it asks through an Operation catalog operation, and the engine makes the call with the run's Model provider and Limits, so the solver never holds model credentials (ADR 0035).
- **Outcome:** one of these typed outcomes:
  - `solved`, with evidence the engine re-checks with its Page check (ADR 0006)
  - `unsupported`
  - `failed`, with a cause
  - `needs_human`, which hands over to a Human pause once the engine has checked that the page is still the challenged one, so a solver cannot steer the person to a look-alike login page
- **Placement:** where a solver runs and who may load it belong to ADR 0035.

**Conformance.**
- **Suites:** every kind has a suite in `conformance/`, run by `goodfetch plugins test`. Results appear in the Plugin index, in `goodfetch plugins list` and in the Console.
- **Harness:** the suites in `conformance/` run in `goodfetch-conformance`, a Rust crate under `crates/` that depends only on `goodfetch-protocol` and is linked into the binary, with the suites' data embedded. `goodfetch plugins test` fetches the Proving ground binary and seed as a digest-pinned download on first use (ADR 0043) and runs it locally (settled on the gaps the [Proving ground and conformance spec](https://github.com/goodfetch/goodfetch/issues/152) listed, as the [owner settled them](https://github.com/goodfetch/goodfetch/issues/152#issuecomment-6115242029)).
- **What is proved:** Sinks keep their Guarantee class. Every other kind is proved one declared capability at a time (for a fetch backend: HTTP/2, proxies, TLS impersonation).
- **Unproved capabilities:** a capability the suite has not proved is reported as unproved, and a run can require it proved.
- **Proving ground:** fetch backends, browser backends and Challenge solvers are tested on it (ADR 0025): owned fixtures locally, real challenge vendors online, in the job ADR 0043 names for it.

**Configuration.**
- **Schema:** a Plugin's manifest declares a JSON Schema for its settings. The engine validates against it, the Console generates its form from it, and the CLI and MCP take the same fields.
- **Secrets:** a secret field accepts only a Secret reference.
- **Layers:** Plugin settings go through ADR 0028's five layers with lock and expose. Operator policy allows, denies or pins each Plugin to a digest.
- **Digests:** every pin (Operator policy, the lockfile, Recipes, run state) is the digest of the Plugin's multi-platform OCI image index, under which each platform's artifact is checked, so a pinned Recipe or a checkpointed run moves between platforms (ADR 0002).
- **Recipes:** a Recipe names its third-party Plugins by digest.
- **Resuming:** a run records the Plugins and digests it used in its run state.
  - First-party Plugins follow the engine's version and ADR 0030's contract versions.
  - A third-party Plugin with a different digest refuses the resume unless the caller passes `--accept-plugin-change <name>`, recorded in run state and the Audit log.

**Running a process Plugin.**
- **Instances:**
  - Every instance belongs to one run: it starts with the run, or when the run first needs it, and is killed when the run stops, finishes or is discarded, so nothing from that run survives in a Plugin process.
  - Each call carries its lease, and all of an instance's traffic belongs to its run.
  - Work outside a run's lifetime (a Sink probe, `goodfetch plugins test`, delivering a finished run's backlog, a Notification sent after the run ends) gets its own instance, scoped to that one task and its run's Secrets and killed after it.
- **Backpressure:**
  - The manifest declares the maximum number of concurrent calls, and ADR 0024's credit-based flow control applies. Nothing is dropped; callers wait, and waiting for a credit counts against the caller's lease.
  - Calls from a Plugin back into the engine draw on a separate credit pool, so a Plugin at its limit cannot deadlock.
- **Cancellation:**
  - `$/cancel` with a deadline. A call past its deadline is abandoned: its credit is released, its result is refused at the engine (ADR 0003), and the instance's other calls are untouched unless the instance is restarted (below).
  - An abandoned Effect-class call is reported uncertain at once (ADR 0029), since it may still land.
  - An instance is restarted under the crash rules when its heartbeat stops, when it holds more abandoned calls than its concurrency limit, or when a call is abandoned while it has a proxied stream open at `proxy_tcp` (**Streams and leases**).
- **Crashes:** a crashed Plugin restarts with backoff.
  - In-flight reads and idempotent writes fail as `plugin_crashed` and are retried under their lease.
  - An in-flight Effect that has no check is reported uncertain for Reconciliation (ADR 0029), never retried blindly (ADR 0010).
  - Repeated crashes quarantine the Plugin and send a Notification.
- **Health:** a heartbeat on the same channel.
- **Where it runs:** Linux, macOS and Windows, and inside the Browser host for kinds that work beside the browser (browser backends, Challenge solvers). In Goodfetch Cloud, process Plugins run in the tenant's isolation and SDK-handler Plugins stay in the Client.
- **Start:** every process Plugin starts, on every OS:
  - with an empty environment holding only the variables its manifest declares, valued from its settings and never inherited from the shell (a manifest cannot declare loader variables such as `LD_PRELOAD`, `DYLD_*` or `NODE_OPTIONS`), so tokens such as `AWS_SECRET_ACCESS_KEY` or `GITHUB_TOKEN` never leak in
  - with no inherited handles other than stdio and its own proxy socket
  - with a private temporary directory, deleted when the instance stops
- **Resource limits:** memory, CPU time, process count and open files, from its manifest and capped by Operator policy.
  - **Linux:** cgroup v2 where the user's cgroup is delegated (systemd user sessions), rlimits otherwise.
  - **macOS:** rlimits. macOS enforces no per-process memory limit an unprivileged process can set and offers no event for one, so the engine samples the instance's footprint.
  - **Windows:** job object limits.
  - Exceeding a limit kills the instance under the crash rules, with a typed `plugin_limit_exceeded`.
- **Handshake:** an environment-variable cookie, as in go-plugin, so that a Plugin started by hand refuses to run (not a security boundary), then `hello` as the first JSON-RPC message, which must match the approved manifest (kinds, protocol range, network level) or the instance is stopped. The Rust ports of go-plugin are immature, so the pattern is copied rather than imported.

**Egress class, per destination.**
- **Operator-configured endpoints** are values set in Operator policy or in user and project configuration (layers 2 and 3 of ADR 0028): an unblocker's API, a solving service, a Sink's database, a vault, a SearXNG instance. For a third-party Plugin, a value from a Recipe or from the run itself (layers 4 and 5, which an agent or a workflow tool may fill in) is operator-configured only when Operator policy allows that host. A first-party Plugin's endpoints count as operator-configured from any layer, as a built-in Sink's do under ADR 0017. Only these, and a fetch backend's target where allowed (below), may resolve to private or loopback ranges locally, under ADR 0017, and they fall under Goodfetch Cloud's "no private endpoints" policy.
- **Every other address is checked as one a page can reach**, including domains a third-party Plugin's manifest declares and defaults it ships, since its author controls their DNS.
- **Traffic the engine carries for a Plugin never inherits the run's exceptions for pages.** A run's allowlist of private ranges and a switched-off resolved-IP check (ADRs 0016 and 0017) never apply to a Plugin's destinations, as ADR 0022 already rules for the Browser host. The one exception is a fetch backend's connection to the target the engine handed it: always for a first-party backend, and for a third-party one only when Operator policy allows it; otherwise the engine fetches that target itself.
- **Link-local and cloud-metadata ranges** are refused for every destination the engine carries for a Plugin, configured or not. A first-party Plugin that needs cloud credentials gets them from the engine as Secrets: the engine itself holds the instance role and resolves them, so the Plugin needs no access to the metadata address (at `direct` on Linux and macOS it is not prevented from reaching it; see below).
- **Returned endpoints** follow the same rules: a browser backend's CDP URL or a proxy provider's proxy is operator-configured only when it matches an operator-configured value.
- **Targets:** before the engine passes a target URL to a fetch backend, it checks it under the page class and the run's domain policy. The backend's `net.connect` for that call reaches only that target and its own operator-configured endpoints (an unblocker's API); it asks for each redirect as a new target, checked the same way.
So, except at `direct` (below), a Plugin cannot steer itself or the engine to `127.0.0.1`, the local network or a cloud metadata address.

**Network levels for process Plugins.** The manifest declares one level. For a third-party Plugin, a level the host cannot provide is refused with a typed `confinement_unavailable`, never silently weakened; first-party Plugins follow the rule for hosts that cannot confine, below. Every "refused" in this section carries that one qualification for first-party Plugins. What each refusal costs (a fetch backend with its own TLS stack in a default Docker container, for instance) is listed in the docs for each platform.
- **`none`** (the default). The Plugin creates no network socket: it reaches the outside only through `net.fetch` and `net.connect` on its stdio channel, under **Egress class**, the Effects and stream rules above, and Usage. Hooks and Extractors may declare only this level.
  - **Linux:** seccomp allows no new socket of any family (`socketpair` excepted, below).
  - **macOS:** Seatbelt denies all network, and the Mach service allowlist for this level excludes the name-resolution service, so a Plugin cannot send data out through DNS queries.
  - **Windows:** an AppContainer with no network capability, which Windows' default outbound rule blocks, as Chromium's sandbox relies on.
- **`proxy_tcp`**, for clients that only speak `HTTP_PROXY` or SOCKS5. The engine runs a proxy for that instance, which reaches only what `net.connect` may reach (operator-configured endpoints and the targets of the instance's open fetch calls), under the same checks, and requires credentials generated for that instance, carried in the proxy URL, so no other local process can use it. `goodfetch plugin-shim`, the same binary, relays TCP clients to it where the platform needs a relay.
  - **Linux:** a network namespace holding only loopback, with the relay inside it, as Claude Code's sandbox runtime does. The relay starts outside the Plugin's filter and reaches the engine through a socket it inherits already connected. The Plugin's seccomp filter allows IP sockets inside the namespace and refuses Unix-domain ones.
    - Where AppArmor restricts unprivileged user namespaces (Ubuntu 23.10 and later), an AppArmor profile grants them only to `goodfetch-sandbox`, a small root-owned helper that `goodfetch plugins setup` installs only once its attestation verifies (ADR 0039), never to a path the user can write. The helper is not setuid. Its boundary is that it confines whatever it starts: it creates the namespaces, drops every capability, sets `no_new_privs`, and applies its own baseline seccomp filter (no further namespaces, no netlink) whatever its caller passes, then the Plugin's filter, before starting it, so a program that calls it gets a user namespace only already confined. It also starts only entries of the engine's Plugin store or runtime cache, which is hygiene rather than a boundary, since the store is writable by the user. ADR 0024's channels (npm, PyPI, crates.io, GitHub Releases, Homebrew, the installers, the OCI image) ship no root-owned files, so the elevated `goodfetch plugins setup` installs the helper and the profile.
    - In containers whose seccomp profile refuses user namespaces (Docker's default), `proxy_tcp` is refused.
  - **macOS:** Seatbelt allows outgoing traffic to the proxy's loopback port only.
  - **Windows:** loopback between two processes of one AppContainer works only by undocumented behaviour, and a container's loopback to a process outside it needs an administrator exemption. So the filter broker (below) adds a Windows Filtering Platform filter scoped to that instance's AppContainer that allows only the proxy's loopback port. Until `goodfetch plugins setup` has installed the broker, `proxy_tcp` is refused.
- **`direct`**, for drivers that cannot use a proxy (librdkafka, most database clients). A Plugin may declare it only when every kind it implements is one of: Sink, notification channel, Secret provider, Model provider, Decision provider or code source. A Plugin that also implements any other kind cannot get `direct`.
  - **What it gets:** outgoing network access to any address, while staying confined for files, processes and Unix-domain sockets, and with no incoming connections.
  - **What is checked:** the engine checks every configured destination under ADR 0017 before starting the Plugin and at each Sink probe.
  - **What is not checked:** any connection beyond those destinations, including addresses the driver learns during the run (Kafka brokers, MongoDB replicas), DNS rebinding, loopback, the local network and the link-local cloud-metadata address (instance-role credentials), is not checked locally on Linux and macOS; IMDSv2's hop limit does not help, since the Plugin runs on the host itself. Writes the Plugin sends to the run's sites over its own sockets are not held to the `effects` permission either. On Windows, the broker adds filters refusing link-local, metadata and other private addresses for each `direct` instance that runs in an AppContainer. Locally, only a host firewall rule the operator sets protects the metadata address at `direct` on Linux and macOS and for Bridge Sinks on Windows, and the docs give one that keeps the engine's own access to the instance role: on Linux by matching the Plugin's own cgroup, where the user's cgroup is delegated (for a Bridge Sink, the cgroup of the rootless runtime's network helper, such as pasta or slirp4netns); on macOS with a Network Extension content filter (as LuLu or Little Snitch use), since `pf` cannot tell the engine and a Plugin of the same user apart; and for Bridge Sinks on Windows with a Windows Firewall rule on Docker Desktop's `com.docker.backend.exe`, through which its containers' traffic leaves, or a Hyper-V firewall rule for the WSL virtual machine that runs Podman machine. Where none of these is possible, the docs say the rule cannot spare the engine. In Goodfetch Cloud, the tenant's firewall protects it.
  - **Goodfetch Cloud:** the tenant's network firewall enforces "no private endpoints" whatever the engine sees.
  - **Who gets it:** each first-party sink process declares the lowest level it needs, and those that need `direct` have it by default. A third-party Plugin gets it only when Operator policy names it; the run report and the Audit log mark it.
  - **Bridge Sinks** (ADR 0026's Airbyte destination images and Singer targets) are third-party Plugins at `direct`, each only when Operator policy names it by image digest. Both run as container images (a Singer target in the image its project or Meltano publishes; one with no image does not run), only under a container runtime that does not run as root on the host (rootless Podman or Docker on Linux; Docker Desktop or Podman machine on macOS and Windows, whose containers run in a virtual machine), with a read-only root filesystem, all capabilities dropped, no privilege escalation, the runtime's default seccomp profile and only `direct`'s outgoing network; with only a rootful runtime they are refused. On every OS they get none of the broker's filters, since those apply to AppContainers only; the container runtime's own network settings and any host firewall rule the operator sets are all that hold them, and their approval says so. They read every credential from a configuration file, so each credential is a value released under `needs_secret_value`.
  - **Linux:** seccomp allows outgoing IP sockets and refuses `listen`, `accept`, binding a chosen port (so no UDP socket waits for incoming datagrams) and Unix-domain sockets.
  - **macOS:** Seatbelt allows outgoing network and the name-resolution service, and denies incoming connections and other Unix-domain sockets.
  - **Windows:**
    - An AppContainer with the `internetClient` capability only, which allows no incoming connections. On a network with the Public profile, `internetClient` also reaches the local network, so `direct` on Windows always needs the broker. It adds, for each instance, filters refusing link-local, metadata and private ranges, and allowing the operator-configured private or loopback destinations, instead of `privateNetworkClientServer`, which would also allow incoming connections.
    - Until the broker is installed, `direct` is refused on Windows.

**File, process and IPC confinement**, at every level, one policy type per Plugin generated from its manifest.
- **Linux:**
  - Landlock (`landlock`, kernel 5.13+) limits files to those the manifest declares; ABI 6 (kernel 6.12+) also scopes signals and abstract Unix sockets to the Plugin.
  - Without Landlock, a mount namespace exposes only those files (the bubblewrap approach).
  - On kernels before 6.12, a PID namespace isolates signals. Without Landlock ABI 6 and without user namespaces, signal isolation is unavailable, and the rule for hosts that cannot confine applies.
  - seccomp (`seccompiler`) works from allowlists: socket families per network level, and system calls. It refuses, among others, `io_uring`, `ptrace`, `process_vm_readv` and `process_vm_writev`, the kernel keyring (`keyctl`), creating namespaces, System V IPC, `listen` and `accept`, and new processes unless the manifest declares them. A refused call returns an error rather than killing the Plugin, so it fails visibly; the exact system call and socket-family lists belong to the spec. `socketpair`, which language runtimes such as Python's asyncio use internally and which reaches nothing outside the process, stays allowed.
- **macOS:**
  - A Seatbelt profile generated from the manifest is applied through `sandbox_init`, the mechanism Chrome, Codex and Claude Code still use although it is formally deprecated.
  - It denies signals and process inspection outside the Plugin, and limits Mach service lookups to an allowlist per network level, so no system service can fetch over the network for the Plugin.
  - The FFI binding is in-house (two functions), since no crate meets the dependency bar.
- **Windows:**
  - Every instance runs in its own AppContainer, created for that instance without administrator rights and deleted after it, so instances, runs and Daemon accounts never reach each other's files, pipes or ports.
  - A restricted token, with read access granted to that container for the Plugin's bundle and the runtime it declares, both in the engine's caches.
  - A job object for kill-on-close, the resource limits and every UI restriction (no clipboard, global atoms, desktop switching or system settings), through `windows-sys`, and an alternate desktop, as Chromium's sandbox uses, so a Plugin cannot read or drive other windows.
  - **The filter broker.** An elevated `goodfetch plugins setup`, run once, verifies its attestation (ADR 0039) and installs a small Windows service whose only job is to add and remove Windows Filtering Platform filters, each scoped to one AppContainer.
    - Its boundary is what it can do: it can only allow a loopback port or an address the engine names, or refuse addresses, for one live AppContainer. Code running as the same user could pass the other checks, by injecting into a genuine engine, so they are hygiene, not the boundary: a named pipe that only the installing user can open, and a caller whose executable matches a Goodfetch release attestation, checked against a Sigstore trust root bundled with the broker, so engine updates need no new elevation.
    - It adds only filters scoped to a live AppContainer that engine created: allowing a proxy's loopback port or a `direct` instance's operator-configured private or loopback destinations, or refusing link-local, metadata and other private addresses.
    - It logs every change in the Windows event log. Its filters are persistent, never tied to its own session, so they outlive a crash of the broker. Refusing filters are added before an instance starts. When an instance stops, the broker terminates the instance's job object and removes that instance's filters only once the job has no process left. When an engine exits, the broker terminates the job objects of that engine's instances itself, since a handle it holds keeps kill-on-close from firing, and removes that engine's filters once those job objects are empty (each Daemon and Attached engine separately). If the service stops, the engine first stops every instance that depends on it, under the crash rules; when the service starts again, it removes every filter whose AppContainer no longer has a live instance, and instances start again. Uninstalling the broker removes all its filters.
    - It is a separate, minimal binary with no network access, built, attested, Platform-signed and released with the engine (ADR 0039).
- **Hosts that cannot confine**: no seccomp at all (WSL1), or neither user namespaces nor Landlock ABI 6 (a container under Docker's default seccomp profile on a kernel before 6.12), or a host that cannot provide the network level a Plugin declares:
  - A third-party Plugin is refused with `confinement_unavailable`.
  - A first-party Plugin, one shipped in the engine's own release, is refused by default too. Operator policy can allow it to run with the confinement the host offers; the run report and the Audit log then say which parts were not confined.
- **CI on every OS**, with real confinement and a real engine, nothing mocked:
  > Amended by [ADR 0067](0067-security-is-tested-per-trust-boundary-with-secret-canaries-and-private-fuzzing.md): the OS-confinement cases are generated from each policy, check the exact error, pair with a positive control and assert the mechanism in use.
  - **OS confinement**, at each network level: a Plugin is refused when it opens a socket its level forbids, an abstract or filesystem Unix socket, `io_uring`, `listen` or `accept`; reads `$HOME`, a path on the fixed deny list, a variable from the engine's environment or the kernel keyring; signals or inspects another process; spawns a process; exceeds each resource limit; on Windows, uses the clipboard or another desktop, or resolves a name through the DNS Client service at `none`.
  - **Engine refusals:** a declared domain that resolves to loopback or a metadata address; a run's private-range allowlist reaching a Plugin; a Hook rewriting a URL to a site outside its declared domains and the run's domain policy; a stream to a third-party Plugin's shipped default or to an unallowed Recipe or run value; a `net.fetch` with a method other than `GET` or `HEAD` to the run's site through such a value without `effects`; a `page.*` navigation outside its scope; script without `page_script`; an Effect without `effects`; a Secret value reaching the Plugin through field insertion, a missed scrub or a Hook; a logged-in Session refused a third-party fetch backend or Browser host; a UDP bind at `direct`; a `hello` that differs from the manifest; a revoked Plugin.
  - **Reset gate:** ADR 0001's canaries, planted through a Plugin, are found nowhere after a run is discarded.

**Finding, versioning and trusting.**
- **Packaging:** a Plugin is published as one OCI artifact per platform, in any OCI registry such as ghcr.io, holding the Plugin and every dependency it runs, built and attested by its own CI, as Steampipe distributes its plugins. Its name is `goodfetch-plugin-*`.
- **Installing runs nothing.** `goodfetch plugins add` downloads the artifact, verifies it and unpacks it into the engine's Plugin store, refusing path traversal, links that leave the bundle and device files. It never runs npm, pip, a build or any install script, and never touches the user's own environment (`node_modules`, a virtualenv). The spec checks the Rust OCI client library against the dependency bar.
- **Manifest:** it declares:
  - the Plugin's kinds and a plugin-protocol version range
  - its settings schema
  - its language runtime (a Node or Python version), which the engine provides from a runtime cache by digest, as it does browsers (ADR 0024), so a Plugin never relies on what happens to be installed
  - its network level and permissions: Secret references and `needs_secret_value`, domains, `effects`, `page_script`, `session_credentials`, environment variables, files and processes
  - browsers it adds, by download URL and digest
  - for spend outside the engine, a maximum per call and a price per unit
- **Plugin index:** a file in the repository, validated in CI and signed there with Sigstore, tied to the repository's workflow identity. The engine checks it with `sigstore-verify` against a trust root bundled with each release, so the check also works offline. It lists community Plugins with their signer identity and conformance results, and carries a version, an expiry and a revocation list of digests and signers, as TUF metadata does: the engine refuses an index older than the last one it saw (in a Daemon group, the last one the Lead Daemon saw, ADR 0055) or past its expiry, and a revoked Plugin is refused at load and its running instances are stopped. An expired index blocks loading third-party Plugins, since revocation can no longer be checked; an air-gapped host imports a fresh signed index (`goodfetch plugins index import`). The spec picks the library (AWS's `tough`, a Rust TUF client, is the first candidate). `goodfetch plugins search` reads it. A marketplace belongs to Goodfetch Cloud's map.
- **Lockfile:** `goodfetch plugins add` records a digest in a lockfile, like `.terraform.lock.hcl`, and the engine refuses a mismatch.
- **Provenance:** a third-party Plugin's artifact must carry a provenance attestation (a GitHub artifact attestation or a Sigstore bundle from its CI), which `sigstore-verify` checks against the signer the index records (repository workflow and OIDC issuer). Since the artifact holds every dependency, the attestation covers every byte the Plugin ships; code it fetches at run time stays confined but is not attested. An unattested Plugin loads only if Operator policy allows its exact digest.
- **Updates:** never automatic. `goodfetch plugins update` shows the new digest, the signer and a permission diff, and any new permission or wider value (anything the manifest declares under permissions, a network level, a default endpoint, a browser, a spend maximum or price) needs explicit approval, as a browser asks when an extension wants more. A change of signer identity between versions is refused until the operator approves it again. That catches a release from another repository or workflow, not a compromise of the project's own release workflow (the Ultralytics releases on PyPI in 2024 carried valid attestations), which only revocation in the index covers.
- **Look-alike names:** `goodfetch plugins add` refuses a name close to an index entry (typosquatting) unless it is given by exact digest.
- **Files:** a Plugin reads its own unpacked bundle and runtime, and writes only to directories that belong to its instance, deleted when the instance stops. A Plugin keeps nothing across runs: what it needs lives in its bundle or its settings. Any other path it declares is read-only, approved by Operator policy, and never covers the fixed deny list: the engine's own files (settings, Operator policy, lockfiles, the browser and runtime caches, the Plugin store, run files, the Run index, the Audit log, the Daemon's runtime and token files, Console keys) and every other Plugin's directories; credential files (`~/.ssh`, `~/.aws`, `~/.kube`, `~/.docker/config.json`, `.env`, `.netrc`, `.git-credentials`, the GitHub CLI's token, keychains, browser profiles), maintained as a list in the spec; shell startup files; and version-control hooks.
- **Loading:** a third-party Plugin loads only when Operator policy allows it. Goodfetch's first-party Plugins are allowed by default. A Private component is not first-party: it loads only when Operator policy allows it, and the index lists its signer identity so that its expiry and revocation apply to it (ADR 0035).

Evidence: confinement and provenance precedents gathered for the ticket:
- [Codex's sandboxes](https://github.com/openai/codex/tree/main/codex-rs) and [network proxy](https://github.com/openai/codex/blob/main/codex-rs/network-proxy/README.md)
- [Claude Code's sandbox runtime](https://github.com/anthropic-experimental/sandbox-runtime)
- [Chromium's macOS sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/sandbox/mac/README.md) and [Windows sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/design/sandbox.md)
- Windows network isolation: [Microsoft's loopback guidance](https://learn.microsoft.com/en-us/previous-versions/windows/apps/hh780593(v=win.10)), [sharing named objects with an AppContainer](https://learn.microsoft.com/en-us/windows/uwp/communication/sharing-named-objects), [Project Zero on AppContainer network access](https://googleprojectzero.blogspot.com/2021/08/understanding-network-access-windows-app.html)
- [go-plugin internals](https://github.com/hashicorp/go-plugin/blob/main/docs/internals.md)
- [Terraform's plugin protocol](https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol) and [dependency lock file](https://developer.hashicorp.com/terraform/language/files/dependency-lock)
- [Airbyte's connector metadata](https://docs.airbyte.com/platform/connector-development/connector-metadata-file)
- Crate facts taken from crates.io on 2026-10-11.
- `goodfetch plugins add` also fetches the Plugin's language runtime, so an engine with the `offline` setting never downloads one, and a third-party Plugin's approved manifest domains and network level count as configured destinations (ADR 0043).
- ADR 0051 confines the Script sandbox's worker as a Plugin at network level `none`, with these resource limits (on the Linux rlimit fallback, `RLIMIT_DATA`, never `RLIMIT_AS`) and these crash rules under its own `script_*` Error codes, except that only crashes count toward quarantine, a quarantine lasts for the run and answers `script_quarantined`, and the engine, not the worker, carries out every Effect; a failed Script Hook's `hook_failed` carries the `retryable` flag of the `script_*` code it names and is retried only when that flag is set. ADR 0051 also has the Rust crate run `plugin-shim` from the `goodfetch` binary of its exact version.
