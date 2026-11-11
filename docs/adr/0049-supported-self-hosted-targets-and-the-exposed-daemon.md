# Self-hosted targets are supported only when CI deploys there, and an Exposed Daemon runs in the foreground

A Daemon that Clients reach from other machines is an **Exposed Daemon**; it is the same engine with the same operations, so the Daemon's definition drops "on the local machine" (ADR 0024). A deployed host runs it with `goodfetch daemon run`, in the foreground, draining to resumable checkpoints on `SIGTERM`. A deployment target is **supported** only when the docs give a tested guide and CI deploys there, or into a faithful emulator, and runs a smoke run: Docker and Compose, Kubernetes through a Helm chart, a Linux VM with systemd, Google Cloud Run, AWS Lambda as a container image, and edge or serverless JavaScript runtimes as Clients only. ECS and Fargate, Azure Container Apps, Fly.io, Railway and Render are **documented** with the generic image. Chromium's sandbox is probed at startup, refused with a precise fix where one exists, and switched off automatically but recorded where the platform allows none. Decided in [Self-hosted deployment and operations](https://github.com/goodfetch/goodfetch/issues/120).

Crawlee documents AWS Lambda and Cloud Run but tests neither, and its AWS guide is stale ([crawlee#3371](https://github.com/apify/crawlee/issues/3371)). Firecrawl calls its Helm chart and manifests "starting points", bundles Redis, Postgres and RabbitMQ that users cannot replace ([firecrawl#3827](https://github.com/firecrawl/firecrawl/issues/3827)), and its k3d CI is still an open pull request ([firecrawl#2729](https://github.com/firecrawl/firecrawl/pull/2729)). Browserless has only community charts. Playwright's image runs as root, which disables Chromium's sandbox, and needs `--init` against zombie processes ([playwright#34230](https://github.com/microsoft/playwright/issues/34230), [docs](https://playwright.dev/docs/docker)); Browserless always launches with `--no-sandbox` ([browsers.cdp.ts](https://github.com/browserless/browserless/blob/main/src/browsers/browsers.cdp.ts)). Lambda's 250 MB unzipped limit is the top complaint of `@sparticuz/chromium` ([#8](https://github.com/Sparticuz/chromium/issues/8)), which Crawlee and Puppeteer point users to.

## Considered Options

- **A new term for a networked engine** ("server engine"): rejected. It would split one behaviour into two names, and the glossary already avoids *server* and *service* for the Daemon.
- **Documented targets without CI**: rejected as the meaning of "supported". Untested guides rot, as Crawlee's AWS guide did.
- **Always terminating TLS in the Daemon**: rejected. Cloud Run hands containers plain HTTP only, and meshes and ingresses terminate TLS by design.
- **A zip-based Lambda layer**: rejected. Goodfetch's signed browser builds do not fit 250 MB unzipped; container images reach 10 GB.
- **User handlers running inside edge functions**: rejected. Vercel's edge runtime must start responding within 25 s and Workers die with their request, so a crawl would stop mid-run.
- **`--no-sandbox` wherever the sandbox fails** (Browserless): rejected. It also removes Chromium's seccomp-bpf layer (`IsSeccompBPFDesired()` in [sandbox_seccomp_bpf_linux.cc](https://chromium.googlesource.com/chromium/src/+/main/sandbox/policy/linux/sandbox_seccomp_bpf_linux.cc)), and Docker or Kubernetes can be fixed with a profile.
- **SQLite run state on network filesystems**: rejected. SQLite's file locking is unreliable on them and WAL mode does not work over a network.
- **Bundling dependencies in the Helm chart** (Firecrawl): not needed. State lives in each run's file (ADR 0002).

## Consequences

**Foreground Daemon** (changes ADR 0024).
- In this mode, ADR 0024's runtime file, idle stop, one Daemon per user and per major, and Unix-socket or loopback-only listener do not apply: `goodfetch daemon run` has no idle stop and no runtime file, listens on the configured address, reads configuration from files and environment variables (ADR 0028), and logs JSON to stdout through `tracing`.
- On `SIGTERM` it drains: it stops leasing work and taking runs, checkpoints every run as resumable (ADR 0002) and exits within the grace period.
- It makes itself a child subreaper (`PR_SET_CHILD_SUBREAPER`) and reaps every browser descendant, so it runs correctly as PID 1 without `tini`.

  > Note (agent decision, accepted by the owner on 2026-10-22; [#228](https://github.com/goodfetch/goodfetch/issues/228)): how `daemon run` is built until area 4 and area 12 bring TLS, accounts and the bootstrap link.
  > - **Configuration.** Two machine settings: `daemon.address` (`host:port`, default `127.0.0.1` on port 7900 plus the major, 7600 plus the minor during `0.y` previews, beside the Console's ports) and `daemon.drain_grace` (110 s). The deployment variables `GOODFETCH_DAEMON_ADDRESS` and `GOODFETCH_DAEMON_DRAIN_GRACE` (milliseconds) override user configuration and the Operator policy for `daemon run` alone, ADR 0083's "a Daemon's own process configuration", and an Operator policy lock refuses them with `setting_locked`. Any address that is not loopback is refused with `setting_invalid` until the Daemon terminates TLS.
  > - **Clients and Console.** Clients open with the major's persistent token, `daemon.token` (`0600`, the one `daemon install` uses), whose path the first log line names. Loopback TCP is the listener, newline-delimited JSON-RPC as on the machine's Daemon. No Console is served: the Exposed Daemon's Console needs area 4's TLS and area 12's sign-in.
  > - **Drain.** `SIGTERM` or `SIGINT` (Ctrl-C on Windows), listened for before the listener opens: the engine takes no run (`run.start`, `run.resume` and Foxhound's `crawl.start` answer the new retryable Error code `daemon_draining`), a Client's leaving no longer aborts its runs, and each live run stops with the new stop reason `drain`, with a Checkpoint, resumable by its Run ID. The engine waits for them up to `daemon.drain_grace` on its `Clock`, then cancels the rest without waiting: a run whose tasks did not stop stays `running`, resumable from its run file, as after a crash. The executable exits within the grace in any case, with 1 when the grace cut it short. Nothing resumes a drained run by itself: its Client resumes it with `run.resume` on the next start.
  > - **PID 1.** On Linux, `daemon run` sets `PR_SET_CHILD_SUBREAPER`, starts the Daemon as its one child (`daemon run --reaped`, which dies with it through `PR_SET_PDEATHSIG`), forwards `SIGTERM` and `SIGINT` to it, reaps every process the kernel gives it with `waitpid(-1)`, and exits with the Daemon's code. A second process, rather than a reaper inside the Daemon, because the Daemon's Tokio runtime waits on its own children by process ID, and a reaper waiting on any child there would take those exits from it. `ps` in a container shows both processes.

**Exposed Daemon security** (changes ADRs 0017, 0027 and 0031).
- The Daemon terminates TLS itself by default, with certificate files reloaded when they change.
- A person can opt in to **TLS terminated upstream**: the Daemon then serves plain HTTP, still requires tokens or passkeys, keeps its `Host` and `Origin` checks, trusts `X-Forwarded-*` only from configured proxy addresses, records the setting in the Audit log and shows it in the Console.
- **Bootstrap.** The first operator enrolls a passkey through a one-time setup link. Its token comes from a Secret reference or, when none is configured, is printed once to the logs. It expires after 24 hours, and the Daemon refuses every other request until setup completes.

  > Note (2026-11-11, consolidated review X-03; the owner's « do as recommended », the choice being the review orchestrator's): the one-time setup link is a one-time setup code sent over a Connection, since no page exists to open a link in (ADR 0137). Its refusal is `setup_required` and its Secret reference `daemon.setup_token` ([#668](https://github.com/goodfetch/goodfetch/issues/668)); the first passkey is enrolled on it ([#1125](https://github.com/goodfetch/goodfetch/issues/1125)).
- Program tokens (with values as Secret references), OIDC configuration and role mappings can be declared in configuration, so Helm values and GitOps create them. The Console shows them as managed by configuration, not editable there.

**Deployment targets.**

| Target | Tier | CI |
|---|---|---|
| Docker and Docker Compose | Supported | The OCI image itself |
| Kubernetes (Helm chart) | Supported | A `kind` cluster: install the chart, run, drain on pod deletion |
| Linux VM with systemd | Supported | A system unit running `goodfetch daemon run`, beside the per-user unit of `goodfetch daemon install` (ADR 0031) |
| Google Cloud Run, services and jobs | Supported | One real deploy per release candidate, a job ADR 0043 allows to reach Google Cloud |
| AWS Lambda, container image | Supported | AWS's Lambda Runtime Interface Emulator |
| Cloudflare Workers, Vercel Functions, Deno Deploy, as Clients only | Supported | The SDK in `workerd` (Miniflare), Vercel's edge-runtime and Deno against a Daemon |
| ECS and Fargate, Azure Container Apps, Fly.io, Railway, Render | Documented | None |

> Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): every Supported target is supported for both Products. CI runs each Product's image on it, as its CI column says, with that Product's smoke run, and the Cloud Run deploy of each release candidate deploys both images.

**Edge and serverless Clients** (adds to ADR 0024).
- Both SDKs gain a WebSocket transport to an Exposed Daemon or Goodfetch Cloud, the carriage the Console already uses (ADR 0031), configured by `GOODFETCH_URL` and a scoped token. Page bodies stream through `page.body`.
- On an edge runtime, a Client makes request-scoped calls (fetch, extract or act on one page, start a Recipe run and poll or subscribe to it). Handler code that must outlive a request runs as a Recipe on the Daemon. Asking for in-function handlers on an edge runtime fails with a typed Error code.

  > Note (agent decision, accepted by the owner on 2026-10-22; [#256](https://github.com/goodfetch/goodfetch/issues/256)): the Daemon's side of the WebSocket transport, until area 4 brings TLS and scoped tokens and area 11 the SDKs' side and `GOODFETCH_URL`.
  > - **One address.** `daemon run` serves the WebSocket on `daemon.address` itself, beside newline-delimited JSON-RPC, so a platform that forwards one HTTP port (Cloud Run, an ingress) reaches it: a Connection whose first byte is a capital letter opens an HTTP request line and is a WebSocket handshake, at any path; any other is lines. The Daemon on the machine (its Unix socket) serves no WebSocket; the Console's own WebSocket is #817's.
  > - **Handshake.** Its `Host` must be the address the Daemon listens at, or `localhost` with that port, and its `Origin` one of those over `http`; a foreign `Host`, or a foreign or missing `Origin`, is answered `403` and logged under the refusal target with the listener `daemon_websocket` and the endpoint table's reason (`host_foreign`, `origin_foreign`, `origin_missing`). A Client outside a browser sends the Daemon's own origin. Which names an Exposed Daemon behind a proxy answers to joins with area 4's upstream TLS and `X-Forwarded-*` rule.
  > - **Messages.** Each text (or binary) message carries one JSON-RPC message, and each message the engine sends is one text message; the first must be `hello` with the token, refused as on the socket (closed unanswered), and every operation, event, credit and `page.body` chunk is the same. The token is the major's `daemon.token` until area 4's scoped tokens; a token in the URL's query string is never read. A message longer than the 16 MiB limit is answered `message_too_large` and the WebSocket then closes, since it cannot skip the message as the line framer does. tungstenite does the handshake and the framing.
  > Note on the owner's decision of 2026-11-01 (specs #1937 and #1943): a machine's Daemon also serves a loopback WebSocket on `daemon.address` when an origin is allowed or `live_view.enabled` is on, one listener for both, and none otherwise. It serves no file and sets no cookie, and an allowed origin's Connection goes through a Pairing. Goodfetch hosts no Client relay: the repository holds a sample with no release artefact, beside the protocol document. The ADR #1943's tickets write carries the rest.

**AWS Lambda.**
- Container image only, with an Attached engine per invocation, built on the `-node` or `-python` image plus AWS's runtime interface client; `goodfetch init --deploy lambda` writes it.
- Run state goes in `/tmp`, with checkpoints to object storage, and a run nearing the 15-minute limit checkpoints and stops as resumable, so another invocation resumes it.

**Google Cloud Run.**
- An Exposed Daemon as a Cloud Run service, with instance-based billing (CPU always allocated), at least one instance, at most one instance per Daemon, and checkpoints to Cloud Storage.
- Recipe runs as Cloud Run jobs: an Attached engine per Cloud Run job execution, started by Cloud Scheduler.
- The engine detects throttled CPU on Cloud Run and warns in the run's results. The guide for ECS and Fargate describes services and tasks the same way.

**Run state on deployed hosts** (changes ADR 0002).
- The engine checks the run directory's filesystem with `statfs` and refuses NFS, SMB, EFS, Azure Files, Filestore and FUSE object-storage mounts with a typed Error code that names the fix.
- Checkpoint locations accept object-storage URLs (`s3://`, `gs://`, `az://` and S3-compatible endpoints such as R2 and MinIO) with credentials as Secret references, through the object-storage clients the Sinks already ship (ADR 0026).
- On a detected ephemeral target (Lambda, Cloud Run, Fargate), periodic checkpoints are on by default every 60 seconds, settable, and a run with no checkpoint location carries a warning in its results.

**Chromium's sandbox** (changes ADRs 0016 and 0017).
- The engine probes at startup whether the namespace sandbox can start; ADR 0033's Plugin confinement reads the same probe (adds to ADR 0033).
- **A fix exists** (Docker's and containerd's default seccomp profiles, which still block `CLONE_NEWUSER`; Kubernetes `RuntimeDefault`; Ubuntu's AppArmor restriction on unprivileged user namespaces on the host): browser runs are refused with a typed Error code whose message names the fix for the detected environment. An operator can still switch the sandbox off in Operator policy, audited and reported under ADR 0016.
- **No fix exists** (Lambda, Fargate and EKS on Fargate, Azure Container Apps): the sandbox is switched off automatically, still recorded in results and the Audit log, and the docs warn against untrusted sites.
  > Note (agent decision, on a guarantee, not read by the owner), 2026-10-25, [#642](https://github.com/goodfetch/goodfetch/issues/642): the engine's `sandbox_probe` module forks a child at startup, once per process, that tries `unshare(CLONE_NEWUSER)` and then its own `uid_map`, and reads the seccomp mode, `kernel.apparmor_restrict_unprivileged_userns` and the platform's variables from the environment `main` read. A namespace made starts the sandbox anywhere. Otherwise: `AWS_LAMBDA_FUNCTION_NAME`, `AWS_EXECUTION_ENV=AWS_ECS_FARGATE` or `CONTAINER_APP_NAME` name a platform with no fix, where `security.browser_sandbox` is off by default on the host (layer `default`), which every Run reports as a switched-off control: one Audit log entry, `weakened`, one warning. A refused `unshare` under a seccomp filter is `docker_default_seccomp`, or `kubernetes_runtime_default` with `KUBERNETES_SERVICE_HOST` set, and every launch is refused with `browser_sandbox_unavailable` (`details {environment, fix}`) before any download. Under AppArmor's restriction a profile lifts it per executable, so each launch reads the label its browser started under and refuses `apparmor_userns` when it is `unconfined` or stacked with `unprivileged_userns`. Any other refusal lets the launch go on, logged. `security.browser_sandbox` is switched off only by Operator policy or user and project configuration (`Authority::Configuration`): a Recipe's or a run's value is `setting_locked`, as ticket #642 asks, which narrows ADRs 0016 and 0017's "per run". EKS on Fargate sets no variable a pod can read, so it is not yet recognized: its refusal reads as Kubernetes' until the Helm chart names it. Goodfetch's seccomp profile is not published yet, so the Docker fix names any profile that allows the namespace calls.
  > Note (agent decision, as built), 2026-11-08, on the note above: what the host allows, the forked child's answer, the seccomp mode and the AppArmor restriction, is still read once per process. Each engine judges it against its own environment map and keeps its own answer (`SandboxProbe`, in `EngineContext`), so two engines in one process, such as an SDK's or a test's, no longer share the first one's verdict, and a browser launch reads its own engine's answer rather than probing in an empty environment where none had probed yet.

  > Note (agent decision, on a guarantee, not read by the owner), 2026-10-28, step 7 review findings 564-3 and 564-4 ([#642](https://github.com/goodfetch/goodfetch/issues/642)): `security.browser_sandbox` stays configuration-only (`Authority::Configuration`), as #642's acceptance criterion says ("a Client's attempt without that policy is refused with `setting_locked`"), against the ticket's body and spec [#564](https://github.com/goodfetch/goodfetch/issues/564)'s "or run"; spec #564's line and its settings table's `security.*` row should say so for this key. A probe refusal the probe does not recognise, such as `user.max_user_namespaces` at 0, no longer lets the launch go on: every launch with the sandbox on is refused with `browser_sandbox_unavailable`, whose `details.environment` gains the value `unknown`, its message naming what the probe met and its `fix` how to check `unshare --user` or to switch the sandbox off in Operator policy, as #642's criterion "refuses to start when it failed, with the probe's error" asks.
- Goodfetch publishes its seccomp profile, which, like [Playwright's](https://github.com/microsoft/playwright/blob/main/utils/docker/seccomp_profile.json), adds only the user-namespace system calls to the default. `init`'s Compose files and the Helm chart use it.
- Cloud Run gen2's support for user namespaces is unverified; its release-candidate deploy checks it.
- The engine detects a small `/dev/shm` (Kubernetes gives 64 MB) and falls back to `--disable-dev-shm-usage` with a warning.

**OCI images** (adds to ADRs 0024 and 0032).

| Tag | Contents |
|---|---|
| `goodfetch:<v>` | The engine, Chrome for Testing (the default browser outside `stealth`), the Goodfetch Chromium (the default in `stealth`; ADR 0035) and a virtual display (ADR 0031) |
| `goodfetch:<v>-slim` | The engine only: HTTP tiers, and remote browsers through a Browser host or a CDP provider |
| `goodfetch:<v>-full` | The default plus Camoufox, and Playwright's Firefox and WebKit |
| `goodfetch:<v>-node`, `goodfetch:<v>-python` | The default plus the language runtime and SDK, the base for `init`'s Dockerfiles and for Lambda |

> Amended by [ADR 0090](0090-products-are-named-goodfetch-collie-and-goodfetch-foxhound-and-share-one-product-version.md): each Product has its image, `ghcr.io/goodfetch/collie` and `ghcr.io/goodfetch/foxhound`, under this tag scheme; which tags each Product publishes is decided in [Release, signing and testing of two products](https://github.com/goodfetch/goodfetch/issues/459), and the Browser host's image in [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/goodfetch/issues/498).
>
> Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): each Product publishes all five tags under its own image, and `nightly`, with its own executable as the image's last layer; Collie's `-slim` drives remote browsers only, since the HTTP fetch tiers are Foxhound's (ADR 0088).

- Every tag is built on Debian 13 slim (Chromium needs glibc), for amd64 and arm64, runs as the non-root UID 10001, and ships with an SBOM and Sigstore attestations.
- The Browser host runs from the same image with its own command.
  > Amended by [ADR 0096](0096-the-browser-host-runs-from-either-products-executable-and-goodfetch-cloud-runs-each-run-on-the-product-it-needs.md): the Browser host is the `browser-host` command of either Product's executable, run from Collie's image in its default and `-full` tags, which carry the browsers; `-slim` has no browser to host.

**Helm chart.**
- A `StatefulSet` of Daemons, each with its own `ReadWriteOnce` volume for run state, probes on the ops listener (ADR 0050), a `terminationGracePeriodSeconds` sized for the drain (120 by default) and a `PodDisruptionBudget`.
- `replicas` may be any number: the pods form one Daemon group whose Lead Daemon the `lead` value names, pod 0 by default. A headless `Service` gives each pod the stable name of the peer link. More than one pod needs a shared Secret provider, and either a Politeness store or `politeness_sharing: per_daemon` set in the Operator policy (ADRs 0053 and 0055).
  > Amended by [ADR 0086](0086-a-crawl-obeys-robots-txt-and-sends-one-request-at-a-time-per-domain-by-default-and-pressing-harder-is-recorded.md): the key is `politeness.sharing`.
- Optional: a `ServiceMonitor`, an `Ingress` or Gateway API `HTTPRoute`, a `NetworkPolicy` that blocks cluster-internal ranges (ADR 0017's egress classes at the network level), and a Browser host `Deployment`.
  > Amended by [ADR 0096](0096-the-browser-host-runs-from-either-products-executable-and-goodfetch-cloud-runs-each-run-on-the-product-it-needs.md): each Product's chart runs Collie's image, at the chart's Product version, for its Browser host `Deployment`.
- Operator policy as a `ConfigMap`, and secrets referenced from Kubernetes `Secret`s.
- By default it passes Kubernetes' `restricted` Pod Security level: `runAsNonRoot`, every capability dropped, `allowPrivilegeEscalation: false`, a read-only root filesystem with `emptyDir` volumes for `/tmp` and the browser cache, a memory-backed `/dev/shm` of 1 GiB, and the seccomp profile as `Localhost`, installed through an optional Security Profiles Operator `SeccompProfile` the chart can create, or by hand.
- Two documented values: `seccomp: unconfined`, needed on GKE Autopilot, which forbids custom profiles and drops to the `privileged` level; and `hostUsers: false` with `SYS_ADMIN` inside the pod's user namespace (Kubernetes 1.33 and later, GA in 1.36), which fails `baseline`.
- Published as a signed OCI artifact on GHCR at the product version, released with the other packages (adds to ADR 0030). Upgrades roll by drain, then resume from the same volume.
  > Amended by [ADR 0090](0090-products-are-named-goodfetch-collie-and-goodfetch-foxhound-and-share-one-product-version.md): a chart is named after the Product whose image it runs, under `ghcr.io/goodfetch/charts/` (`ghcr.io/goodfetch/charts/foxhound`, for instance); which Products publish one follows their images.
  >
  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): Collie's and Foxhound's charts come from one chart source that differs only by the image it runs, which a Quick check proves by rendering both; the `kind` test runs each chart, and ADR 0066's upgrade runs Foxhound's.

**Project templates** (adds to ADR 0030).
- `goodfetch init` scaffolds TypeScript, Python and Rust projects from five starters: crawl, extract, browser automation, AI agent and Recipe. The templates are embedded zstd-compressed (ADR 0057).
- Each carries a Dockerfile on the official image, a Compose file, a GitHub Actions workflow running the project's tests, the AGENTS.md index and the agent skills.
- Templates are embedded in the binary and versioned with it, so `init` works offline (ADR 0043).
- `goodfetch init --deploy <target>` adds a supported target's files: Helm values, a Lambda handler and Dockerfile, Cloud Run service or job definitions.
