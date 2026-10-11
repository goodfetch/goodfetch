# One plugin protocol for every Plugin kind, with the engine between the run and every Plugin

V1 extends the engine through **Plugins**, each implementing one or more **Plugin kinds** under one contract per kind and one conformance suite. ADR 0026's pattern for Sinks becomes the rule for all of them: the public sink protocol becomes the **plugin protocol**, with a `hello` handshake that negotiates the protocol version and declares the Plugin's kinds, then one group of JSON-RPC methods per kind (`sink.*`, `fetch.*`, …). The engine stands between the run and every Plugin, so what a Plugin does is journaled, counted in Usage and held to Limits like the engine's own work, and a process Plugin runs confined on Linux, macOS and Windows with no network of its own. Decided in [Plugin and extension model](https://github.com/Jbmanllr/automan/issues/108).

Twelve kinds:

| Kind | What it adds | Precedent |
| --- | --- | --- |
| Hook | code at fixed stages of requests, navigation, pages, Records and the run's lifecycle; Router middleware is Hooks scoped by label | Crawlee `preNavigationHooks`, `postNavigationHooks`, `Router.use`, `EventManager`, browser-pool hooks |
| Fetch backend | the HTTP transport, or a remote fetcher such as an unblocker API | Crawlee `BaseHttpClient`, Spider `with_fetch_engine`, `with_remote_fetcher` |
| Browser backend | a CDP endpoint, launched or connected; the in-house driver and the guard stay the engine's (ADR 0019) | Crawlee browser plugins |
| Extractor | an alternative to dom-smoothie (ADR 0007) | Crawl4AI strategies |
| Sink | as ADR 0026 decided | Scrapy pipelines, Crawlee storage clients |
| Model provider | a model format beyond ADR 0012's three; ADR 0012's SDK model callback is its SDK-handler form | Stagehand's custom LLM function |
| Decision provider | the decision-model interface the [decision models research](https://github.com/Jbmanllr/automan/issues/92) suggested | none |
| Secret provider | beyond ADR 0015's built-in providers; its custom HTTP provider becomes a form of this kind | — |
| Challenge solver | the public interface a Private component plugs into (ADR 0018) | Crawlee through playwright-extra |
| Proxy provider | a proxy per Session | Crawlee `newUrlFunction`, Spider `with_proxy_strategy` |
| Search provider | joins ADR 0011's interchangeable web search providers | — |
| Code source | email or SMS MFA codes (ADR 0006) | — |
| Notification channel | joins ADR 0027's channels | — |

Every kind accepts three forms: an SDK handler (in the Client's process, over JSON-RPC), a process Plugin (any language, the plugin protocol over stdio) and a Rust trait implementation (in-process, only when Automan is embedded as a Rust crate). Hooks also take a fourth: a **Script Hook**, a JavaScript function run in the code-mode sandbox of ADR 0011, with no network or files, journaled catalog calls only, and time and memory limits, so that a Recipe, the CLI or MCP can carry a Hook without an SDK or a process. First-party built-ins implement the same traits, so one suite proves built-in and third-party implementations alike.

Changes ADR 0026, whose sink protocol becomes the `sink.*` group of the plugin protocol, and ADR 0030, whose versioned contract formats list the plugin protocol in its place.

## Considered Options

- **A protocol per kind**: rejected. Each new kind would need its own handshake, cache, versioning under ADR 0030 and trust rules. HashiCorp's go-plugin shows the shared alternative: a child process, a handshake, a versioned protocol, many plugin types (Terraform and Packer).
- **Loading native libraries into the binary**: rejected. Rust has no stable ABI, and a crashing library takes the engine and every run with it, which ADR 0024 chose the binary to avoid.
- **WASM components**: rejected for V1, for the reasons of ADR 0026; process Plugins and Script Hooks cover the same needs.
- **Hooks only as SDK handlers**: rejected. CLI, MCP and Recipe users would get no Hooks, and Automan Cloud could not run a Recipe's Hooks without the author's script.
- **Storage backends as a kind** (Crawlee Python's SQL and Redis storage clients): rejected. ADR 0002 keeps one SQLite file per run so runs stay portable and resumable; a replaceable store would break it.
- **Model providers only by URL and SDK callback**, as ADR 0012 left them: rejected. A CLI or MCP user, who has no SDK, could not add a model format.
- **Trusting a Plugin's own report of what it did**: rejected. The engine counts calls and time itself; only money a Plugin spends outside the engine is reported by the Plugin, and it is reserved first.
- **Confinement on Linux only**: rejected. The engine ships for Linux, macOS and Windows (ADR 0024).
- **A cross-platform sandbox crate**: none qualifies. `birdcage` is GPL-3.0 and archived, `gaol` is abandoned since 2019, `nono` has no native Windows support and changes fast before 1.0. Codex (`codex-rs`) and Claude Code's sandbox runtime both wrote per-OS code behind one policy, as Automan does.
- **Confining the network per domain at the OS level**: not possible; no OS offers it. Codex and Claude Code's sandbox runtime both remove the network and route allowed traffic through a host-side proxy that checks domains, as Automan does through the engine.
- **A separate network namespace on Linux**: rejected as the mechanism. Ubuntu 23.10 and later restrict unprivileged user namespaces through AppArmor, and Landlock's network rules need kernel 6.7; seccomp refusing IP sockets works everywhere.
- **A docs page of community Plugins**: rejected for a CI-validated Plugin index that carries conformance results.

## Consequences

**The engine between the run and every Plugin.**
- Every call into a Plugin is recorded in the Effect journal under the work item's lease (ADR 0003), and the engine counts its calls and time in Usage.
- A Plugin call's Operation class (read, idempotent write or Effect) comes from its kind, never from the Plugin.
- A Plugin that needs a page action or a network request asks the engine through the protocol (`page.*`, `net.fetch`, `net.connect`). That passes through the guard, ADR 0017's egress policy, the domains its manifest declares, and the journal.
- A Plugin that spends money outside the engine (an unblocker API, a solving service) declares a worst case per call, which the engine reserves against the run's Limit before the call, as ADR 0012 does for model calls. The Plugin then reports the actual cost, shown in Usage as reported by the Plugin.

**Hooks.**

| Stage | What a Hook may return |
| --- | --- |
| before request | change the URL, headers or body; skip; fail |
| after response, before parsing | rewrite the body; mark it blocked with a block cause, which feeds escalation (ADR 0021); retry; fail |
| before and after navigation, page ready | page operations through the engine (block resources, set cookies, scroll); fail |
| Record before commit | transform; drop with a reason; reject to a dead-letter list |
| lifecycle (run start, checkpoint, migration, Limit reached, stop) | observe only |

Hooks run in declared order and can be scoped by label. A Hook error is a typed `hook_failed`, retried by the work item's policy, and a Hook's time counts against the lease.

**The challenge solver's contract.**
- **Detection:** the engine detects a challenge by its rules, which beat models there.
- **Call:** it then calls the solver with:
  - the Session handle
  - a page handle
  - the challenge kind and vendor (Turnstile, reCAPTCHA v2, v3 and Enterprise, hCaptcha, DataDome, Akamai, PerimeterX, AWS WAF, GeeTest, Kasada, image CAPTCHA)
  - the remaining Limit budget
- **Acting:** the solver acts on the page only through engine operations, or returns a token the engine injects.
- **Outcome:** one of these typed outcomes:
  - `solved`, with evidence the engine re-checks with its Page check (ADR 0006)
  - `unsupported`
  - `failed`, with a cause
  - `needs_human`, which hands over to a Human pause
- **Placement:** where a solver runs and who may load it belong to [Stealth and challenge-solving catalog](https://github.com/Jbmanllr/automan/issues/78). A Private component ships only as a process Plugin, so closed code never enters the public binary or crates.

**Conformance.**
- **Suites:** every kind has a suite in `conformance/`, run by `automan plugins test`. Results appear in the Plugin index, in `automan plugins list` and in the Console.
- **What is proved:** Sinks keep their Guarantee class. Every other kind is proved one declared capability at a time (for a fetch backend: HTTP/2, proxies, TLS impersonation).
- **Unproved capabilities:** a capability the suite has not proved is reported as unproved, and a run can require it proved.
- **Proving ground:** fetch backends, browser backends and challenge solvers are tested on it (ADR 0025): owned fixtures locally, real challenge vendors online.

**Configuration.**
- **Schema:** a Plugin's manifest declares a JSON Schema for its settings. The engine validates against it, the Console generates its form from it, and the CLI and MCP take the same fields.
- **Secrets:** a secret field accepts only a Secret reference.
- **Layers:** Plugin settings go through ADR 0028's five layers with lock and expose. Operator policy allows, denies or pins each Plugin to a digest.
- **Recipes and runs:** a Recipe names its Plugins by digest. A run records the Plugins and digests it used in its run state, and a resumed run refuses a different digest.

**Running a process Plugin.**
- **Instances:** one instance per engine is started on demand and shared across runs, unless the manifest says `per_run`. Each call carries its run and lease.
- **Backpressure:** the manifest declares the maximum number of concurrent calls, and ADR 0024's credit-based flow control applies. Nothing is dropped; callers wait.
- **Cancellation:** `$/cancel` with a deadline, after which the process is killed. A call past its lease is refused at the engine.
- **Crashes:** a crashed Plugin restarts with backoff, and its in-flight calls fail as `plugin_crashed` and are retried under their lease. Repeated crashes quarantine the Plugin and send a Notification.
- **Health:** a heartbeat on the same channel.
- **Where it runs:** Linux, macOS and Windows, and inside the Browser host for kinds that work beside the browser (browser backends, challenge solvers). In Automan Cloud, process Plugins run in the tenant's isolation and SDK-handler Plugins stay in the Client.
- **Handshake:** an environment-variable cookie, as in go-plugin, so that a Plugin started by hand refuses to run (not a security boundary), then `hello` as the first JSON-RPC message. The Rust ports of go-plugin are immature, so the pattern is copied rather than imported.

**Confinement, one policy type per Plugin, generated from its manifest.**
- **No network of its own:** a Plugin reaches the outside only through the engine: `net.fetch`, `net.connect`, or the engine's proxy.
  - The proxy listens on a Unix socket on Linux and macOS and a named pipe on Windows.
  - For clients that speak only TCP, `automan plugin-shim` (the same binary) runs inside the Plugin's confinement, exposes `HTTP_PROXY` and SOCKS5, and relays to that socket. Windows therefore needs no admin step for a loopback exemption.
- **Linux:**
  - seccomp (`seccompiler`) refuses to create `AF_INET` and `AF_INET6` sockets, and refuses new processes unless the manifest declares them. This blocks the network on any kernel without user namespaces.
  - Landlock (`landlock`, kernel 5.13+) limits files to those the manifest declares.
  - Without Landlock, the Plugin fails to start with a typed `confinement_unavailable`.
- **macOS:**
  - A Seatbelt profile generated from the manifest is applied through `sandbox_init`, the mechanism Chrome, Codex and Claude Code still use although it is formally deprecated.
  - The FFI binding is in-house (two functions), since no crate meets the dependency bar.
- **Windows:**
  - An AppContainer with no network capability and a restricted token.
  - A job object for kill-on-close and process limits, through `windows-sys`.
- **Opting out:** a manifest asking for `unconfined` loads only if Operator policy allows that Plugin by name. The run report and the Audit log mark it.
- **CI on every OS:** a Plugin tries to open a socket, read `$HOME` and spawn a process, and all three must be refused under real confinement.

**Finding, versioning and trusting.**
- **Packaging:** a Plugin is a package in its own registry (crates.io, npm, PyPI, a binary release or an OCI image), named `automan-plugin-*`.
- **Manifest:** it declares the Plugin's kinds, a plugin-protocol version range, its settings schema and its permissions: Secret references, domains, Effects, files and processes.
- **Plugin index:** a file in the repository, validated in CI, signed with minisign (`minisign-verify` in the engine), listing community Plugins with their conformance results. `automan plugins search` reads it. A marketplace belongs to Automan Cloud's map.
- **Lockfile:** `automan plugins add` records a digest in a lockfile, like `.terraform.lock.hcl`, and the engine refuses a mismatch.
- **Provenance:** where a Plugin carries GitHub, npm or PyPI attestations, `sigstore-verify` checks them against a signer policy (repository workflow and OIDC issuer). crates.io records trusted publishing but serves no signed attestations yet, so a crates.io Plugin rests on its digest alone until it does.
- **Secrets:** a Plugin receives only the Secret references it declared.
- **Loading:** a third-party Plugin loads only when Operator policy allows it; Automan's first-party Plugins are allowed by default.

Evidence: confinement and provenance precedents gathered for the ticket:
- [Codex's sandboxes](https://github.com/openai/codex/tree/main/codex-rs) and [network proxy](https://github.com/openai/codex/blob/main/codex-rs/network-proxy/README.md)
- [Claude Code's sandbox runtime](https://github.com/anthropic-experimental/sandbox-runtime)
- [Chromium's macOS sandbox](https://chromium.googlesource.com/chromium/src/+/HEAD/sandbox/mac/README.md)
- [go-plugin internals](https://github.com/hashicorp/go-plugin/blob/main/docs/internals.md)
- [Terraform's plugin protocol](https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol) and [dependency lock file](https://developer.hashicorp.com/terraform/language/files/dependency-lock)
- [Airbyte's connector metadata](https://docs.airbyte.com/platform/connector-development/connector-metadata-file)
- Crate facts taken from crates.io on 2026-10-11.
