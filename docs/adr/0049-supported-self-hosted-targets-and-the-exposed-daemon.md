# Self-hosted targets are supported only when CI deploys there, and an Exposed Daemon runs in the foreground

A Daemon that Clients reach from other machines is an **Exposed Daemon**; it is the same engine with the same operations, so the Daemon's definition drops "on the local machine" (ADR 0024). A deployed host runs it with `goodfetch daemon run`, in the foreground, draining to resumable checkpoints on `SIGTERM`. A deployment target is **supported** only when the docs give a tested guide and CI deploys there, or into a faithful emulator, and runs a smoke run: Docker and Compose, Kubernetes through a Helm chart, a Linux VM with systemd, Google Cloud Run, AWS Lambda as a container image, and edge or serverless JavaScript runtimes as Clients only. ECS and Fargate, Azure Container Apps, Fly.io, Railway and Render are **documented** with the generic image. Chromium's sandbox is probed at startup, refused with a precise fix where one exists, and switched off automatically but recorded where the platform allows none. Decided in [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120).

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

**Exposed Daemon security** (changes ADRs 0017, 0027 and 0031).
- The Daemon terminates TLS itself by default, with certificate files reloaded when they change.
- A person can opt in to **TLS terminated upstream**: the Daemon then serves plain HTTP, still requires tokens or passkeys, keeps its `Host` and `Origin` checks, trusts `X-Forwarded-*` only from configured proxy addresses, records the setting in the Audit log and shows it in the Console.
- **Bootstrap.** The first operator enrolls a passkey through a one-time setup link. Its token comes from a Secret reference or, when none is configured, is printed once to the logs. It expires after 24 hours, and the Daemon refuses every other request until setup completes.
- Program tokens (with values as Secret references), OIDC configuration and role mappings can be declared in configuration, so Helm values and GitOps create them. The Console shows them as managed by configuration, not editable there.

**Deployment targets.**

| Target | Tier | CI |
|---|---|---|
| Docker and Docker Compose | Supported | The OCI image itself |
| Kubernetes (Helm chart) | Supported | A `kind` cluster: install the chart, run, drain on pod deletion |
| Linux VM with systemd | Supported | A system unit running `goodfetch daemon run`, beside the per-user unit of `goodfetch daemon install` (ADR 0031) |
| Google Cloud Run, services and jobs | Supported | One real deploy per release candidate |
| AWS Lambda, container image | Supported | AWS's Lambda Runtime Interface Emulator |
| Cloudflare Workers, Vercel Functions, Deno Deploy, as Clients only | Supported | The SDK in `workerd` (Miniflare), Vercel's edge-runtime and Deno against a Daemon |
| ECS and Fargate, Azure Container Apps, Fly.io, Railway, Render | Documented | None |

**Edge and serverless Clients** (adds to ADR 0024).
- Both SDKs gain a WebSocket transport to an Exposed Daemon or Goodfetch Cloud, the carriage the Console already uses (ADR 0031), configured by `GOODFETCH_URL` and a scoped token. Page bodies stream through `page.body`.
- On an edge runtime, a Client makes request-scoped calls (fetch, extract or act on one page, start a Recipe run and poll or subscribe to it). Handler code that must outlive a request runs as a Recipe on the Daemon. Asking for in-function handlers on an edge runtime fails with a typed Error code.

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

- Every tag is built on Debian 13 slim (Chromium needs glibc), for amd64 and arm64, runs as the non-root UID 10001, and ships with an SBOM and Sigstore attestations.
- The Browser host runs from the same image with its own command.

**Helm chart.**
- A `StatefulSet` of Daemons, each with its own `ReadWriteOnce` volume for run state, probes on the ops listener (ADR 0050), a `terminationGracePeriodSeconds` sized for the drain (120 by default) and a `PodDisruptionBudget`.
- `replicas` may be any number: the pods form one Daemon group whose Lead Daemon the `lead` value names, pod 0 by default. A headless `Service` gives each pod the stable name of the peer link. More than one replica needs a Politeness store, or `politeness_sharing: per_daemon` set in the Operator policy, and a shared Secret provider (ADRs 0053 and 0055).
- Optional: a `ServiceMonitor`, an `Ingress` or Gateway API `HTTPRoute`, a `NetworkPolicy` that blocks cluster-internal ranges (ADR 0017's egress classes at the network level), and a Browser host `Deployment`.
- Operator policy as a `ConfigMap`, and secrets referenced from Kubernetes `Secret`s.
- By default it passes Kubernetes' `restricted` Pod Security level: `runAsNonRoot`, every capability dropped, `allowPrivilegeEscalation: false`, a read-only root filesystem with `emptyDir` volumes for `/tmp` and the browser cache, a memory-backed `/dev/shm` of 1 GiB, and the seccomp profile as `Localhost`, installed through an optional Security Profiles Operator `SeccompProfile` the chart can create, or by hand.
- Two documented values: `seccomp: unconfined`, needed on GKE Autopilot, which forbids custom profiles and drops to the `privileged` level; and `hostUsers: false` with `SYS_ADMIN` inside the pod's user namespace (Kubernetes 1.33 and later, GA in 1.36), which fails `baseline`.
- Published as a signed OCI artifact on GHCR at the product version, released with the other packages (adds to ADR 0030). Upgrades roll by drain, then resume from the same volume.

**Project templates** (adds to ADR 0030).
- `goodfetch init` scaffolds TypeScript, Python and Rust projects from five starters: crawl, extract, browser automation, AI agent and Recipe.
- Each carries a Dockerfile on the official image, a Compose file, a GitHub Actions workflow running the project's tests, the AGENTS.md index and the agent skills.
- Templates are embedded in the binary and versioned with it, so `init` works offline (ADR 0043).
- `goodfetch init --deploy <target>` adds a supported target's files: Helm values, a Lambda handler and Dockerfile, Cloud Run service or job definitions.
