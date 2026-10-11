# One plugin protocol for every Plugin kind; the engine accounts for every Plugin and confines process Plugins

V1 extends the engine through **Plugins**, each implementing one or more **Plugin kinds** under one contract per kind and one conformance suite. ADR 0026's pattern for Sinks becomes the rule for all of them: the public sink protocol becomes the **plugin protocol**, with a `hello` handshake that negotiates the protocol version and declares the Plugin's kinds, then one group of JSON-RPC methods per kind (`sink.*`, `fetch.*`, …). The engine accounts for every call into a Plugin, whatever its form: it counts it, classes it, journals what ADRs 0010 and 0012 require, and holds it to Limits. A process Plugin also runs confined on Linux, macOS and Windows, at the network level its manifest declares; the engine enforces each level as far as the host allows and states plainly what it cannot enforce. Decided in [Plugin and extension model](https://github.com/Jbmanllr/automan/issues/108).

Thirteen kinds:

| Kind | What it adds | Precedent |
| --- | --- | --- |
| Hook | code at fixed stages of requests, navigation, pages, Records and the run's lifecycle, optionally scoped by label | Crawlee `preNavigationHooks`, `postNavigationHooks`, `Router.use`, `EventManager`, browser-pool hooks |
| Fetch backend | the HTTP transport, or a remote fetcher such as an unblocker API | Crawlee `BaseHttpClient`, Spider `with_fetch_engine`, `with_remote_fetcher` |
| Browser backend | a browser to drive, given as one of three answers (below); the in-house driver and the guard stay the engine's (ADR 0019) | Crawlee browser plugins |
| Extractor | an alternative to dom-smoothie (ADR 0007) | Crawl4AI strategies |
| Sink | delivery to a destination, as ADR 0026 decided | Scrapy pipelines, Crawlee storage clients |
| Model provider | a model format beyond ADR 0012's three; ADR 0012's SDK model callback is its SDK-handler form | Stagehand's custom LLM function |
| Decision provider | a Model provider held to the decision contract the [decision models research](https://github.com/Jbmanllr/automan/issues/92) suggested: one option from a fixed list with a confidence, or a typed abstention | none |
| Secret provider | beyond ADR 0015's built-in providers; its custom HTTP provider becomes a form of this kind | — |
| Challenge solver | the public interface a Private component plugs into (ADR 0018) | Crawlee through playwright-extra |
| Proxy provider | a proxy per Session | Crawlee `newUrlFunction`, Spider `with_proxy_strategy` |
| Search provider | joins ADR 0011's interchangeable web search providers | — |
| Code source | email or SMS MFA codes (ADR 0006) | — |
| Notification channel | joins ADR 0027's channels | — |

Every kind accepts three forms: an SDK handler (in the Client's process, over JSON-RPC), a process Plugin (any language, the plugin protocol over stdio) and a Rust trait implementation (in-process, only when Goodfetch is embedded as a Rust crate). Hooks also take a fourth: a **Script Hook**, a JavaScript function run in the code-mode sandbox of ADR 0011, with no network or files, journaled catalog calls only, and time and memory limits, so that a Recipe, the CLI or MCP can carry a Hook without an SDK or a process. First-party built-ins implement the same traits, so one suite proves built-in and third-party implementations alike.

What the engine enforces differs by form. SDK-handler and Rust-trait Plugins are the user's own code, like a user handler (ADR 0003): they run in the user's process with the user's rights, and the engine accounts for every call into them but cannot confine them. Records produced through them say so in their Provenance. Process Plugins are confined and reach the outside only as below. Script Hooks are held by the code-mode sandbox. Private components ship only as process Plugins, and Operator policy cannot allow them `unconfined`, so they are always confined.

Changes:
- ADR 0011, whose code-mode sandbox also runs Script Hooks.
- ADR 0012, whose SDK model callback becomes the SDK-handler form of the Model provider kind.
- ADR 0015, whose custom HTTP provider becomes a form of the Secret provider kind.
- ADR 0016, whose security controls gain Plugin confinement, reported per run.
- ADR 0017, whose two egress classes apply to Plugins per destination, and whose endpoint rule covers the Plugin proxy.
- ADR 0022, whose third-party CDP browsers can be added by a browser backend.
- ADR 0024, whose browser cache takes browsers a Plugin declares.
- ADR 0026, whose sink protocol becomes the `sink.*` group of the plugin protocol and whose sink processes are process Plugins.
- ADR 0030, whose versioned contract formats list the plugin protocol, the Plugin manifest and the lockfile.

## Considered Options

- **A protocol per kind**: rejected. Each new kind would need its own handshake, cache, versioning under ADR 0030 and trust rules. HashiCorp's go-plugin shows the shared alternative: a child process, a handshake, a versioned protocol, many plugin types (Terraform and Packer).
- **Loading native libraries into the binary**: rejected. Rust has no stable ABI, and a crashing library takes the engine and every run with it, which ADR 0024 chose the binary to avoid.
- **WASM components**: rejected for V1, for the reasons of ADR 0026; process Plugins and Script Hooks cover the same needs.
- **Hooks only as SDK handlers**: rejected. CLI, MCP and Recipe users would get no Hooks without writing a process Plugin.
- **Storage backends as a kind** (Crawlee Python's SQL and Redis storage clients): rejected. ADR 0002 keeps one SQLite file per run so runs stay portable and resumable; a replaceable store would break it.
- **Model providers only by URL and SDK callback**, as ADR 0012 left them: rejected. A CLI or MCP user, who has no SDK, could not add a model format.
- **A browser backend launching an executable it names**: rejected. The engine would run third-party code outside the Plugin's confinement. A local browser comes only from the engine's digest-checked browser cache.
- **A browser backend limited to local launches and Browser hosts**: rejected. No Plugin could add a remote browser vendor beside Browserbase and Bright Data, which ADR 0022 already reaches over CDP.
- **Trusting a Plugin's own account of its calls**: rejected. The engine counts calls and time itself and classes each call itself. Only spend outside the engine comes from the Plugin, bounded by its declared maximum.
- **Confinement on Linux only**: rejected. The engine ships for Linux, macOS and Windows (ADR 0024).
- **A cross-platform sandbox crate**: none qualifies. `birdcage` is GPL-3.0 and archived, `gaol` is abandoned since 2019, `nono` has no native Windows support and changes fast before 1.0. Codex (`codex-rs`) and Claude Code's sandbox runtime both wrote per-OS code behind one policy, as Goodfetch does.
- **Confining the network per host at the OS level**: not possible; no OS offers it. Codex and Claude Code's sandbox runtime both remove the network and route allowed traffic through a host-side proxy that checks domains, as Goodfetch does through the engine.
- **No network for every process Plugin**: rejected. Drivers such as librdkafka and most database clients cannot use a proxy, so the first-party Sinks that ship as processes (Kafka, MongoDB, SQL Server, ClickHouse and others) could not run.
- **Checking `direct` connections by address with a cgroup BPF filter**: rejected. It needs root and exists only on Linux.
- **A digest pin for every Plugin on resume**: rejected for first-party Plugins, which follow the engine's version; any engine patch would otherwise stop a run from resuming, on another machine too (ADR 0002).
- **Refusing first-party Plugins on hosts that cannot confine them**: rejected. Most Sinks would disappear on RHEL 8, Cloud Run and WSL1, a regression against ADR 0026.
- **A docs page of community Plugins**: rejected for a CI-validated Plugin index that carries conformance results.

## Consequences

**Accounting for every call.**
- The engine counts every call into a Plugin in Usage (calls and time) and records it in the Run trace.
- The engine classes each call itself, from the kind's method and what the call carries: a fetch backend's `GET` is a read, a `POST` may be an Effect, a Sink delivery is an idempotent write. It never takes the class from the Plugin.
- The Effect journal records, under ADRs 0010 and 0012:
  - Effect-class calls
  - every Model provider and Decision provider call
- Secret provider answers are never recorded: only the Secret reference, the outcome and the time.
- Notification channels deliver through ADR 0027's ledger.
- A call made for a work item runs under its lease (ADR 0003); lifecycle Hooks and other calls outside a work item are bounded by a timeout instead.
- A Plugin that needs a page action asks the engine (`page.*`), which applies the guard and the journal.
- A `net.connect` or proxied stream is opaque to the engine. It is counted in Usage (bytes and time), recorded in the Run trace by destination, and closed when the lease it was opened under ends (ADR 0003).
- **Spend outside the engine** (an unblocker API, a solving service):
  - The Plugin reports raw units (solves, requests, pages) in Usage.
  - Its manifest declares, for each unit, a maximum per call and a price. The operator sees them when allowing the Plugin and can override them, like ADR 0012's price table, and a currency Limit treats a unit with no price as unbounded, refusing the call.
  - Before the call, the engine reserves the maximum times the price against the currency Limit, and refuses a call that would cross it.
  - A reported quantity above the declared maximum is recorded in full, never capped, flagged in the run report, and stops the run if it crosses the Limit.
  - The currency Limit on this spend is only as accurate as the declared prices and the reported units, and the run report says the figures come from the Plugin. These are third-party prices, so ADR 0001 holds.

**Hooks.**

| Stage | What a Hook may return |
| --- | --- |
| before request | change the URL, headers or body; skip; fail |
| after response, before parsing | rewrite the body; mark it blocked with a block cause, which feeds escalation (ADR 0021); retry; fail |
| before and after navigation, page ready | page operations through the engine (block resources, set cookies, scroll); fail |
| Record before commit | transform; drop with a reason; reject into the rejection table of the run's Dataset, which the run report shows |
| lifecycle (run start, checkpoint, migration, Limit reached, stop) | nothing: it observes only |

Hooks run in declared order and can be scoped by label, which replaces Crawlee's router middleware. A Hook error is a typed `hook_failed`, retried by the work item's policy, and a Hook's time counts against the lease.

**Browser backends.** A browser backend answers with one of three:
1. **A browser from the engine's browser cache**, by name and digest. A Plugin may add a browser by declaring a download URL and digest in its manifest; the engine downloads it, checks it, and launches it itself over a pipe, like Chrome for Testing (ADR 0024). Such a browser runs as the engine's own browsers do, outside the Plugin's confinement, so Operator policy allows it per digest, it is off by default, and each allowance is recorded in the Audit log. Its launch flags come only from the engine's allowlist, which excludes flags that run commands (such as `--renderer-cmd-prefix`). The engine never launches an executable a Plugin names.
2. **A Browser host address** (ADR 0022).
3. **A third-party CDP endpoint** (as for Browserbase or Bright Data's Scraping Browser), reached as ADR 0022 reaches them, with the guard, and checked under the egress rule below.

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
- **Suites:** every kind has a suite in `conformance/`, run by `goodfetch plugins test`. Results appear in the Plugin index, in `goodfetch plugins list` and in the Console.
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
- **Instances:**
  - At network level `none`, one instance per engine and per Daemon account (ADR 0031) is started on demand and shared across that account's runs, unless the manifest says `per_run`. An instance is never shared across accounts, so one person's secrets never reach another person's calls.
  - Each call carries its run and lease.
  - At `proxy_tcp` and `direct`, every instance is per run, so all its traffic belongs to one run.
- **Backpressure:**
  - The manifest declares the maximum number of concurrent calls, and ADR 0024's credit-based flow control applies. Nothing is dropped; callers wait, and waiting for a credit counts against the caller's lease.
  - Calls from a Plugin back into the engine draw on a separate credit pool, so a Plugin at its limit cannot deadlock.
- **Cancellation:**
  - `$/cancel` with a deadline. A call past its deadline is abandoned: its credit is released, its result is refused at the engine (ADR 0003), and other runs' calls are untouched.
  - An abandoned Effect-class call is reported uncertain at once (ADR 0029), since it may still land.
  - An instance is restarted under the crash rules when its heartbeat stops or when it holds more abandoned calls than its concurrency limit.
  - A `per_run` instance stops with its run.
- **Crashes:** a crashed Plugin restarts with backoff.
  - In-flight reads and idempotent writes fail as `plugin_crashed` and are retried under their lease.
  - An in-flight Effect that has no check is reported uncertain for Reconciliation (ADR 0029), never retried blindly (ADR 0010).
  - Repeated crashes quarantine the Plugin and send a Notification.
- **Health:** a heartbeat on the same channel.
- **Where it runs:** Linux, macOS and Windows, and inside the Browser host for kinds that work beside the browser (browser backends, challenge solvers). In Goodfetch Cloud, process Plugins run in the tenant's isolation and SDK-handler Plugins stay in the Client.
- **Handshake:** an environment-variable cookie, as in go-plugin, so that a Plugin started by hand refuses to run (not a security boundary), then `hello` as the first JSON-RPC message. The Rust ports of go-plugin are immature, so the pattern is copied rather than imported.

**Egress class, per destination.** An endpoint is operator-configured under ADR 0017 only when its host matches one of the Plugin's settings or a domain its manifest declares and Operator policy allows: an unblocker's API, a solving service, a Sink's database, a vault, a SearXNG instance. The same rule applies to an endpoint a Plugin returns for the engine to reach itself, such as a browser backend's CDP URL or a proxy provider's proxy. Every other address, returned or requested, is one a page can reach, so a Plugin cannot steer the engine to a private or loopback address. Operator-configured endpoints also fall under Goodfetch Cloud's "no private endpoints" policy. Before the engine passes a target URL to a fetch backend or an unblocker, it checks that URL under the page class itself.

**Network levels for process Plugins.** The manifest declares one level. For a third-party Plugin, a level the host cannot provide is refused with a typed `confinement_unavailable`, never silently weakened; first-party Plugins follow the rule for hosts that cannot confine, below. What each refusal costs (a fetch backend with its own TLS stack in a default Docker container, for instance) is listed in the docs for each platform.
- **`none`** (the default, and the only level an Extractor may declare). The Plugin creates no network socket: it reaches the outside only through `net.fetch` and `net.connect` on its stdio channel, where the engine applies the egress class, the domains the manifest declares, Usage and the journal.
  - **Linux:** seccomp refuses every new IP or Unix-domain socket.
  - **macOS:** Seatbelt denies all network, and the Mach service allowlist below excludes the DNS service, so a Plugin cannot send data out through DNS queries.
  - **Windows:** an AppContainer with no network capability, which Windows' default outbound rule blocks, as Chromium's sandbox relies on.
- **`proxy_tcp`**, for clients that only speak `HTTP_PROXY` or SOCKS5. The engine runs a proxy for that instance, which applies the same checks as `none` and requires credentials generated for that instance, carried in the proxy URL, so no other local process can use it. `goodfetch plugin-shim`, the same binary, relays TCP clients to it where the platform needs a relay.
  - **Linux:** a network namespace holding only loopback, with the relay inside it, as Claude Code's sandbox runtime does. The relay reaches the engine through a socket it inherits already connected. seccomp allows IP sockets inside the namespace and refuses Unix-domain ones.
    - Where AppArmor restricts unprivileged user namespaces (Ubuntu 23.10 and later), an AppArmor profile must grant them to `goodfetch`, as Chrome's package does. It covers the installed binary's path and the Daemon's versioned cache copies (ADR 0024). The Linux packages install it; other installs (npm, PyPI, crates.io) get it from the elevated `goodfetch plugins setup`, which writes it for the paths actually in use.
    - In containers whose seccomp profile refuses user namespaces (Docker's default), `proxy_tcp` is refused.
  - **macOS:** Seatbelt allows outgoing traffic to the proxy's loopback port only.
  - **Windows:** loopback between two processes of one AppContainer works only by undocumented behaviour, and a container's loopback to a process outside it needs an administrator exemption.
    - So `proxy_tcp` on Windows needs a one-time elevated `goodfetch plugins setup`, as Codex's Windows sandbox has. It creates a dedicated offline local user, with file access set by access lists, and Windows Filtering Platform filters scoped to that user that allow only the proxy's loopback port.
    - Until setup has run, `proxy_tcp` is refused.
- **`direct`**, for drivers that cannot use a proxy (librdkafka, most database clients). A Plugin may declare it only when every kind it implements is one of: Sink, notification channel, Secret provider, Model provider, Decision provider or code source. A Plugin that also implements any other kind cannot get `direct`.
  - **What it gets:** real network access, while staying confined for files, processes and Unix-domain sockets.
  - **What is checked:** the engine checks every configured destination under ADR 0017 before starting the Plugin and at each Sink probe.
  - **What is not checked:** addresses the driver learns during the run (Kafka brokers, MongoDB replicas) and DNS rebinding are not checked locally. The run report and `goodfetch plugins list` say so for every Plugin at `direct`.
  - **Goodfetch Cloud:** the tenant's network firewall enforces "no private endpoints" whatever the engine sees.
  - **Who gets it:** each first-party sink process declares the lowest level it needs, and those that need `direct` have it by default. A third-party Plugin gets it only when Operator policy names it; the run report and the Audit log mark it.
  - **Bridge Sinks** (ADR 0026's Airbyte destination images and Singer targets) are third-party Plugins at `direct`: a Singer target runs under this confinement, an Airbyte image inside its container runtime with the same network rule, and either one only when Operator policy names it.
  - **Linux:** seccomp allows IP sockets and refuses Unix-domain ones.
  - **macOS:** Seatbelt allows outgoing network and denies Unix-domain sockets.
  - **Windows:**
    - An AppContainer with the `internetClient` and `privateNetworkClientServer` capabilities.
    - A loopback destination (a local PostgreSQL) needs the dedicated user that `goodfetch plugins setup` creates, with filters scoped to that user that allow the configured ports. Until setup has run, a loopback destination is refused.

**File, process and IPC confinement**, at every level, one policy type per Plugin generated from its manifest.
- **Linux:**
  - Landlock (`landlock`, kernel 5.13+) limits files to those the manifest declares; ABI 6 (kernel 6.12+) also scopes signals and abstract Unix sockets to the Plugin.
  - Without Landlock, a mount namespace exposes only those files (the bubblewrap approach).
  - On kernels before 6.12, a PID namespace isolates signals. Without Landlock ABI 6 and without user namespaces, signal isolation is unavailable, and the rule for hosts that cannot confine applies.
  - seccomp (`seccompiler`) refuses `io_uring`, `ptrace`, `process_vm_readv` and `process_vm_writev`, and new processes unless the manifest declares them. A refused call returns an error rather than killing the Plugin, so it fails visibly; the exact system call and socket-family lists belong to the spec. `socketpair`, which language runtimes such as Python's asyncio use internally and which reaches nothing outside the process, stays allowed.
- **macOS:**
  - A Seatbelt profile generated from the manifest is applied through `sandbox_init`, the mechanism Chrome, Codex and Claude Code still use although it is formally deprecated.
  - It denies signals and process inspection outside the Plugin, and limits Mach service lookups to an allowlist, so no system service can fetch over the network for the Plugin.
  - The FFI binding is in-house (two functions), since no crate meets the dependency bar.
- **Windows:**
  - An AppContainer with a restricted token, read access granted to the language runtime the manifest declares.
  - A job object for kill-on-close and process limits, through `windows-sys`.
- **Hosts that cannot confine** (no Landlock and no user namespaces, as on RHEL 8 or gVisor; no seccomp at all, as on WSL1):
  - A third-party Plugin is refused with `confinement_unavailable`.
  - A first-party Plugin, one shipped in the engine's own release, runs with whatever confinement the host offers. The run report and Audit log say which parts were not confined.
- **Opting out:** a manifest asking for `unconfined` loads only if Operator policy allows that Plugin by name. The run report and the Audit log mark it.
- **CI on every OS:** a Plugin runs under real confinement at each network level, and must be refused when it tries:
  - to open a socket its level forbids, an abstract or filesystem Unix socket, or `io_uring`
  - to read `$HOME`
  - to signal or inspect another process
  - to spawn a process

**Finding, versioning and trusting.**
- **Packaging:** a Plugin is a package in its own registry (crates.io, npm, PyPI, a binary release or an OCI image), named `goodfetch-plugin-*`.
- **Manifest:** it declares:
  - the Plugin's kinds and a plugin-protocol version range
  - its settings schema
  - its language runtime
  - its network level and permissions: Secret references, domains, Effects, files and processes
  - browsers it adds, by download URL and digest
  - for spend outside the engine, a maximum per call and a price per unit
- **Plugin index:** a file in the repository, validated in CI, signed with minisign (`minisign-verify` in the engine), listing community Plugins with their conformance results. `goodfetch plugins search` reads it. A marketplace belongs to Goodfetch Cloud's map.
- **Lockfile:** `goodfetch plugins add` records a digest in a lockfile, like `.terraform.lock.hcl`, and the engine refuses a mismatch.
- **Provenance of the package:** where a Plugin carries GitHub, npm or PyPI attestations, `sigstore-verify` checks them against a signer policy (repository workflow and OIDC issuer). crates.io records trusted publishing but serves no signed attestations yet, so a crates.io Plugin rests on its digest alone until it does.
- **Secrets:** a Plugin receives only the Secret references it declared.
- **Loading:** a third-party Plugin loads only when Operator policy allows it. Goodfetch's first-party Plugins are allowed by default. A Private component is not first-party: it loads only when Operator policy allows it.

Evidence: confinement and provenance precedents gathered for the ticket:
- [Codex's sandboxes](https://github.com/openai/codex/tree/main/codex-rs) and [network proxy](https://github.com/openai/codex/blob/main/codex-rs/network-proxy/README.md)
- [Claude Code's sandbox runtime](https://github.com/anthropic-experimental/sandbox-runtime)
- [Chromium's macOS sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/sandbox/mac/README.md) and [Windows sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/design/sandbox.md)
- Windows network isolation: [Microsoft's loopback guidance](https://learn.microsoft.com/en-us/previous-versions/windows/apps/hh780593(v=win.10)), [sharing named objects with an AppContainer](https://learn.microsoft.com/en-us/windows/uwp/communication/sharing-named-objects), [Project Zero on AppContainer network access](https://googleprojectzero.blogspot.com/2021/08/understanding-network-access-windows-app.html)
- [go-plugin internals](https://github.com/hashicorp/go-plugin/blob/main/docs/internals.md)
- [Terraform's plugin protocol](https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol) and [dependency lock file](https://developer.hashicorp.com/terraform/language/files/dependency-lock)
- [Airbyte's connector metadata](https://docs.airbyte.com/platform/connector-development/connector-metadata-file)
- Crate facts taken from crates.io on 2026-10-11.
