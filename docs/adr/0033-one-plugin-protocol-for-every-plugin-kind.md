# One plugin protocol for every Plugin kind; the engine accounts for every Plugin and confines process Plugins

V1 extends the engine through **Plugins**, each implementing one or more **Plugin kinds** under one contract per kind and one conformance suite. ADR 0026's pattern for Sinks becomes the rule for all of them: the public sink protocol becomes the **plugin protocol**, with a `hello` handshake that negotiates the protocol version and declares the Plugin's kinds, then one group of JSON-RPC methods per kind (`sink.*`, `fetch.*`, …). The engine journals, counts and limits every call into a Plugin, whatever its form; a process Plugin also runs confined on Linux, macOS and Windows, reaching the network only at the level its manifest declares and the engine enforces. Decided in [Plugin and extension model](https://github.com/Jbmanllr/automan/issues/108).

Thirteen kinds:

| Kind | What it adds | Precedent |
| --- | --- | --- |
| Hook | code at fixed stages of requests, navigation, pages, Records and the run's lifecycle, optionally scoped by label | Crawlee `preNavigationHooks`, `postNavigationHooks`, `Router.use`, `EventManager`, browser-pool hooks |
| Fetch backend | the HTTP transport, or a remote fetcher such as an unblocker API | Crawlee `BaseHttpClient`, Spider `with_fetch_engine`, `with_remote_fetcher` |
| Browser backend | a browser to drive: a launch spec the engine launches itself, or a Browser host to reach (ADR 0022); the in-house driver and the guard stay the engine's (ADR 0019) | Crawlee browser plugins |
| Extractor | an alternative to dom-smoothie (ADR 0007) | Crawl4AI strategies |
| Sink | as ADR 0026 decided | Scrapy pipelines, Crawlee storage clients |
| Model provider | a model format beyond ADR 0012's three; ADR 0012's SDK model callback is its SDK-handler form | Stagehand's custom LLM function |
| Decision provider | a Model provider held to the decision contract the [decision models research](https://github.com/Jbmanllr/automan/issues/92) suggested: one option from a fixed list with a confidence, or a typed abstention | none |
| Secret provider | beyond ADR 0015's built-in providers; its custom HTTP provider becomes a form of this kind | — |
| Challenge solver | the public interface a Private component plugs into (ADR 0018) | Crawlee through playwright-extra |
| Proxy provider | a proxy per Session | Crawlee `newUrlFunction`, Spider `with_proxy_strategy` |
| Search provider | joins ADR 0011's interchangeable web search providers | — |
| Code source | email or SMS MFA codes (ADR 0006) | — |
| Notification channel | joins ADR 0027's channels | — |

Every kind accepts three forms: an SDK handler (in the Client's process, over JSON-RPC), a process Plugin (any language, the plugin protocol over stdio) and a Rust trait implementation (in-process, only when Automan is embedded as a Rust crate). Hooks also take a fourth: a **Script Hook**, a JavaScript function run in the code-mode sandbox of ADR 0011, with no network or files, journaled catalog calls only, and time and memory limits, so that a Recipe, the CLI or MCP can carry a Hook without an SDK or a process. First-party built-ins implement the same traits, so one suite proves built-in and third-party implementations alike.

What the engine enforces differs by form. SDK-handler and Rust-trait Plugins are the user's own code, like a user handler (ADR 0003): they run in the user's process with the user's rights, and the engine accounts for every call into them but cannot confine them. Process Plugins are confined and reach the outside only as below. Script Hooks are held by the code-mode sandbox. Private components ship only as process Plugins, so they are always confined.

Changes:
- ADR 0011, whose code-mode sandbox also runs Script Hooks.
- ADR 0012, whose SDK model callback becomes the SDK-handler form of the Model provider kind.
- ADR 0015, whose custom HTTP provider becomes a form of the Secret provider kind.
- ADR 0026, whose sink protocol becomes the `sink.*` group of the plugin protocol and whose sink processes are process Plugins.
- ADR 0030, whose versioned contract formats list the plugin protocol, the Plugin manifest and the lockfile.

## Considered Options

- **A protocol per kind**: rejected. Each new kind would need its own handshake, cache, versioning under ADR 0030 and trust rules. HashiCorp's go-plugin shows the shared alternative: a child process, a handshake, a versioned protocol, many plugin types (Terraform and Packer).
- **Loading native libraries into the binary**: rejected. Rust has no stable ABI, and a crashing library takes the engine and every run with it, which ADR 0024 chose the binary to avoid.
- **WASM components**: rejected for V1, for the reasons of ADR 0026; process Plugins and Script Hooks cover the same needs.
- **Hooks only as SDK handlers**: rejected. CLI, MCP and Recipe users would get no Hooks without writing a process Plugin.
- **Storage backends as a kind** (Crawlee Python's SQL and Redis storage clients): rejected. ADR 0002 keeps one SQLite file per run so runs stay portable and resumable; a replaceable store would break it.
- **Model providers only by URL and SDK callback**, as ADR 0012 left them: rejected. A CLI or MCP user, who has no SDK, could not add a model format.
- **A browser backend returning a CDP endpoint**: rejected. ADR 0017 reaches local browsers over a pipe, never a debugging port, and a browser started by a confined Plugin would inherit the Plugin's confinement.
- **Trusting a Plugin's own report of what it did**: rejected. The engine counts calls and time itself and classes each call itself.
- **Confinement on Linux only**: rejected. The engine ships for Linux, macOS and Windows (ADR 0024).
- **A cross-platform sandbox crate**: none qualifies. `birdcage` is GPL-3.0 and archived, `gaol` is abandoned since 2019, `nono` has no native Windows support and changes fast before 1.0. Codex (`codex-rs`) and Claude Code's sandbox runtime both wrote per-OS code behind one policy, as Automan does.
- **Confining the network per host at the OS level**: not possible; no OS offers it. Codex and Claude Code's sandbox runtime both remove the network and route allowed traffic through a host-side proxy that checks domains, as Automan does through the engine.
- **No network for every process Plugin**: rejected. Drivers such as librdkafka and most database clients cannot use a proxy, so the first-party Sinks that ship as processes (Kafka, MongoDB, SQL Server, ClickHouse and others) could not run.
- **A digest pin for every Plugin on resume**: rejected for first-party Plugins, which follow the engine's version; any engine patch would otherwise stop a run from resuming, on another machine too (ADR 0002).
- **Refusing first-party Plugins on hosts that cannot confine files**: rejected. Most Sinks would disappear on RHEL 8 and Cloud Run, a regression against ADR 0026.
- **A docs page of community Plugins**: rejected for a CI-validated Plugin index that carries conformance results.

## Consequences

**Accounting for every call.**
- The engine records every call into a Plugin in Usage (calls and time, counted by the engine) and in the Run trace.
- The engine classes each call itself, from the kind's method and what the call carries (a fetch backend's `GET` is a read, a `POST` may be an Effect, a Sink delivery is an idempotent write), never from the Plugin's own claim.
- Effect-class calls go into the Effect journal under ADR 0010's rules.
- Secret provider answers are never recorded: only the Secret reference, the outcome and the time.
- Notification channels deliver through ADR 0027's ledger.
- A call made for a work item runs under its lease (ADR 0003); lifecycle Hooks and other calls outside a work item are bounded by a timeout instead.
- A Plugin that needs a page action asks the engine (`page.*`), which applies the guard and the journal.
- A Plugin that spends money outside the engine (an unblocker API, a solving service) reports raw quantities (solves, credits) in Usage. Its manifest declares a price per unit, overridable like ADR 0012's price table, so a currency Limit can hold it: the engine reserves the worst case before the call and refuses a call that would cross the Limit. These are third-party prices, so ADR 0001 holds.

**Hooks.**

| Stage | What a Hook may return |
| --- | --- |
| before request | change the URL, headers or body; skip; fail |
| after response, before parsing | rewrite the body; mark it blocked with a block cause, which feeds escalation (ADR 0021); retry; fail |
| before and after navigation, page ready | page operations through the engine (block resources, set cookies, scroll); fail |
| Record before commit | transform; drop with a reason; reject to the run's list of rejected Records |
| lifecycle (run start, checkpoint, migration, Limit reached, stop) | nothing: it observes only |

Hooks run in declared order and can be scoped by label, which replaces Crawlee's router middleware. A Hook error is a typed `hook_failed`, retried by the work item's policy, and a Hook's time counts against the lease.

**The challenge solver's contract.**
- **Detection:** the engine detects a challenge by its rules, which beat models there.
- **Call:** it then calls the solver with:
  - the Session handle
  - a page handle
  - the challenge kind and vendor (Turnstile, reCAPTCHA v2, v3 and Enterprise, hCaptcha, DataDome, Akamai, PerimeterX, AWS WAF, GeeTest, Kasada, image CAPTCHA)
  - the run's remaining Limits
- **Acting:** the solver acts on the page only through engine operations, or returns a token the engine injects.
- **Outcome:** one of these typed outcomes:
  - `solved`, with evidence the engine re-checks with its Page check (ADR 0006)
  - `unsupported`
  - `failed`, with a cause
  - `needs_human`, which hands over to a Human pause
- **Placement:** where a solver runs and who may load it belong to [Stealth and challenge-solving catalog](https://github.com/Jbmanllr/automan/issues/78).

**Conformance.**
- **Suites:** every kind has a suite in `conformance/`, run by `automan plugins test`. Results appear in the Plugin index, in `automan plugins list` and in the Console.
- **What is proved:** Sinks keep their Guarantee class. Every other kind is proved one declared capability at a time (for a fetch backend: HTTP/2, proxies, TLS impersonation).
- **Unproved capabilities:** a capability the suite has not proved is reported as unproved, and a run can require it proved.
- **Proving ground:** fetch backends, browser backends and challenge solvers are tested on it (ADR 0025): owned fixtures locally, real challenge vendors online.

**Configuration.**
- **Schema:** a Plugin's manifest declares a JSON Schema for its settings. The engine validates against it, the Console generates its form from it, and the CLI and MCP take the same fields.
- **Secrets:** a secret field accepts only a Secret reference.
- **Layers:** Plugin settings go through ADR 0028's five layers with lock and expose. Operator policy allows, denies or pins each Plugin to a digest.
- **Recipes:** a Recipe names its third-party Plugins by digest.
- **Resuming:** a run records the Plugins and digests it used in its run state.
  - First-party Plugins follow the engine's version and ADR 0030's contract versions.
  - A third-party Plugin with a different digest refuses the resume unless the caller passes `--accept-plugin-change <name>`, recorded in run state and the Audit log.

**Running a process Plugin.**
- **Instances:** one instance per engine is started on demand and shared across runs, unless the manifest says `per_run`. Each call carries its run and lease.
- **Backpressure:**
  - The manifest declares the maximum number of concurrent calls, and ADR 0024's credit-based flow control applies. Nothing is dropped; callers wait, and waiting for a credit counts against the caller's lease.
  - Calls from a Plugin back into the engine draw on a separate credit pool, so a Plugin at its limit cannot deadlock.
- **Cancellation:**
  - `$/cancel` with a deadline. A call past its deadline is abandoned and refused at the engine (ADR 0003), without touching other runs' calls.
  - A shared instance is killed only when its heartbeat stops; a `per_run` instance also when its run stops.
- **Crashes:** a crashed Plugin restarts with backoff.
  - In-flight reads and idempotent writes fail as `plugin_crashed` and are retried under their lease.
  - An in-flight Effect that has no check is reported uncertain for Reconciliation (ADR 0029), never retried blindly (ADR 0010).
  - Repeated crashes quarantine the Plugin and send a Notification.
- **Health:** a heartbeat on the same channel.
- **Where it runs:** Linux, macOS and Windows, and inside the Browser host for kinds that work beside the browser (browser backends, challenge solvers). In Automan Cloud, process Plugins run in the tenant's isolation and SDK-handler Plugins stay in the Client.
- **Handshake:** an environment-variable cookie, as in go-plugin, so that a Plugin started by hand refuses to run (not a security boundary), then `hello` as the first JSON-RPC message. The Rust ports of go-plugin are immature, so the pattern is copied rather than imported.

**Network levels for process Plugins.** The manifest declares one level, the engine enforces it, and a level the host cannot enforce is refused with a typed `confinement_unavailable`, never silently weakened.
- **Egress class by kind.** Fetch backends, browser backends, Hooks, challenge solvers and search providers reach addresses a page can reach. Sinks, Notification channels, Secret providers, Model providers, Decision providers, proxy providers and code sources reach endpoints the operator configured. Each request is checked under its class of ADR 0017, including Automan Cloud's "no private endpoints" policy.
- **`none`** (the default). The Plugin creates no socket. It reaches the outside through `net.fetch` and `net.connect` on its protocol channel, or through a proxy socket it inherits already connected at start. The engine applies the egress policy, the domains the manifest declares, Usage and the journal.
  - **Linux:** seccomp refuses every new socket and `io_uring` setup, which also closes abstract Unix sockets and paths such as `docker.sock`. `socketpair`, which language runtimes such as Python's asyncio use internally and which reaches nothing outside the process, stays allowed.
  - **macOS:** Seatbelt denies all network.
  - **Windows:** an AppContainer with no network capability, which Windows' default outbound rule blocks, as Chromium's sandbox relies on. The proxy is a named pipe (`\\.\pipe\LOCAL\…`) whose access list grants that container, the documented way out of it.
- **`proxy_tcp`**, for clients that only speak `HTTP_PROXY` or SOCKS5. The engine's proxy applies the same checks as `none`; `automan plugin-shim`, the same binary, relays TCP clients to it.
  - **Linux:** a network namespace holding only loopback, with the relay inside it, as Claude Code's sandbox runtime does. Where AppArmor restricts unprivileged user namespaces (Ubuntu 23.10 and later), the Linux packages install an AppArmor profile granting them to `automan`, as Chrome's package does.
  - **macOS:** Seatbelt allows outgoing traffic to the relay's loopback port only.
  - **Windows:** loopback between two processes of one AppContainer works only by undocumented behaviour, and a container's loopback to a process outside it needs an administrator exemption. So `proxy_tcp` on Windows needs a one-time elevated `automan plugins setup`, as Codex's Windows sandbox has. It creates a dedicated offline local user and Windows Filtering Platform filters scoped to it, which allow only the relay's loopback port. Until setup has run, `proxy_tcp` is refused with `confinement_unavailable`.
- **`direct`**, for drivers that cannot use a proxy (librdkafka, most database clients). Only kinds that reach operator-configured endpoints may declare it.
  - **Confinement:** the Plugin gets real network access but stays confined for files and processes. The engine checks each configured destination under ADR 0017 before starting the Plugin.
  - **Who gets it:** first-party sink processes have `direct` by default. A third-party Plugin gets it only when Operator policy names it, and the run report marks it.

**File and process confinement**, one policy type per Plugin, generated from its manifest.
- **Linux:**
  - Landlock (`landlock`, kernel 5.13+) limits files to those the manifest declares.
  - Without Landlock, a mount namespace exposes only those files (the bubblewrap approach).
  - seccomp (`seccompiler`) refuses new processes unless the manifest declares them.
- **macOS:**
  - A Seatbelt profile generated from the manifest is applied through `sandbox_init`, the mechanism Chrome, Codex and Claude Code still use although it is formally deprecated.
  - The FFI binding is in-house (two functions), since no crate meets the dependency bar.
- **Windows:**
  - An AppContainer with a restricted token, read access granted to the language runtime the manifest declares.
  - A job object for kill-on-close and process limits, through `windows-sys`.
- **Hosts that cannot confine files** (no Landlock and no user namespaces: RHEL 8, gVisor, WSL1, some containers):
  - A third-party Plugin is refused with `confinement_unavailable`.
  - A first-party Plugin at the engine's own digest runs with seccomp only, and the run report and Audit log say "files not confined".
- **Opting out:** a manifest asking for `unconfined` loads only if Operator policy allows that Plugin by name. The run report and the Audit log mark it.
- **The proxy socket:**
  - The engine opens one per Plugin instance, so it knows which Plugin is calling and applies that Plugin's declared domains.
  - It is created with ADR 0017's endpoint rule: owner-only (`0600`) on Linux and macOS, a DACL granting only that AppContainer on Windows, and the instance's token checked on connect.
- **CI on every OS:** under real confinement, a Plugin must be refused when it tries to open an IP socket, an abstract or filesystem Unix socket, or `io_uring`; read `$HOME`; or spawn a process.

**Finding, versioning and trusting.**
- **Packaging:** a Plugin is a package in its own registry (crates.io, npm, PyPI, a binary release or an OCI image), named `automan-plugin-*`.
- **Manifest:** it declares:
  - the Plugin's kinds and a plugin-protocol version range
  - its settings schema
  - its language runtime
  - its network level and permissions: Secret references, domains, Effects, files and processes
  - per-unit prices for spend outside the engine
- **Plugin index:** a file in the repository, validated in CI, signed with minisign (`minisign-verify` in the engine), listing community Plugins with their conformance results. `automan plugins search` reads it. A marketplace belongs to Automan Cloud's map.
- **Lockfile:** `automan plugins add` records a digest in a lockfile, like `.terraform.lock.hcl`, and the engine refuses a mismatch.
- **Provenance:** where a Plugin carries GitHub, npm or PyPI attestations, `sigstore-verify` checks them against a signer policy (repository workflow and OIDC issuer). crates.io records trusted publishing but serves no signed attestations yet, so a crates.io Plugin rests on its digest alone until it does.
- **Secrets:** a Plugin receives only the Secret references it declared.
- **Loading:** a third-party Plugin loads only when Operator policy allows it. Automan's first-party Plugins are allowed by default. A Private component is not first-party: it loads only when Operator policy allows it.

Evidence: confinement and provenance precedents gathered for the ticket:
- [Codex's sandboxes](https://github.com/openai/codex/tree/main/codex-rs) and [network proxy](https://github.com/openai/codex/blob/main/codex-rs/network-proxy/README.md)
- [Claude Code's sandbox runtime](https://github.com/anthropic-experimental/sandbox-runtime)
- [Chromium's macOS sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/sandbox/mac/README.md) and [Windows sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/design/sandbox.md)
- Windows network isolation: [Microsoft's loopback guidance](https://learn.microsoft.com/en-us/previous-versions/windows/apps/hh780593(v=win.10)), [sharing named objects with an AppContainer](https://learn.microsoft.com/en-us/windows/uwp/communication/sharing-named-objects), [Project Zero on AppContainer network access](https://googleprojectzero.blogspot.com/2021/08/understanding-network-access-windows-app.html)
- [go-plugin internals](https://github.com/hashicorp/go-plugin/blob/main/docs/internals.md)
- [Terraform's plugin protocol](https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol) and [dependency lock file](https://developer.hashicorp.com/terraform/language/files/dependency-lock)
- [Airbyte's connector metadata](https://docs.airbyte.com/platform/connector-development/connector-metadata-file)
- Crate facts taken from crates.io on 2026-10-11.
