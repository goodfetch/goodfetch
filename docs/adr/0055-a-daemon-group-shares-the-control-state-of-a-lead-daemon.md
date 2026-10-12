# A Daemon group shares the control state of a Lead Daemon, and the Helm chart runs any number of them

ADR 0049's Helm chart ran one Daemon, and several Exposed Daemons serving one team shared nothing. Much of a Daemon's state lives outside its runs: the Usage ledger, accounts and passkeys, named tokens, the Console key, the Operator policy and user configuration, Schedules, the Audit log, the Run index, certificate pins and saved identities. Kept per Daemon, a $50 Limit on a person became $50 per pod, a token worked on one pod and not the next, and a Schedule vanished with its pod. A **Daemon group** is a set of Daemons, its **Group members**, that serve one team as one: one of them, the **Lead Daemon**, holds the group's shared control state as its single writer, and the others reach it over an authenticated peer link. The chart now runs any number of replicas as one Daemon group. Decided in [Scaling past one engine](https://github.com/Jbmanllr/automan/issues/116).

How comparable self-hosted tools run several instances:
- **An external database:** Firecrawl's queue (NuQ on PostgreSQL, [self-hosting](https://github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md)), n8n's queue mode and Airflow's metadata database with one scheduler.
- **Nothing shared:** Browserless runs independent containers behind a load balancer, each with its own `CONCURRENT` and `QUEUED` limits ([config](https://docs.browserless.io/enterprise/docker/config)).
- **One leader with cached followers:** Consul and Nomad, joined through bootstrap tokens and mutual TLS, as kubeadm and k3s join nodes.

## Considered Options

- **Islands, each pod its own Daemon**: rejected. Limits multiply by the number of pods, which defeats ADR 0041, and accounts and tokens differ from pod to pod.
- **An external PostgreSQL for the shared state**, as Goodfetch Cloud keeps its ledger on Aurora: rejected for self-hosting. A second required dependency beside the Politeness store, and a second code path for every store below, buys high availability the control plane does not need: runs go on while the Lead Daemon restarts.
- **Consensus among the Daemons** (Raft through `openraft`, or dqlite): rejected. It is the most complex option, dqlite is LGPL C code, and it buys the same availability.
- **A lead chosen by election**: rejected. The Lead Daemon's state sits on its own disk, so an elected Daemon would have nothing to lead with; the lead is fixed by configuration and replaced from a Group backup.
- **A shared group token**: rejected for the peer link. One leaked secret would let anyone join, and no single Group member could be revoked.
- **Pinning each run to the Group member that created it**: rejected. Scaling down would strand its runs.

## Consequences

**Forming a group.**
- `goodfetch group init` makes a Daemon a group's Lead Daemon and creates the group's certificate authority.
- **Joining.** On VMs, `goodfetch group join` takes a short-lived, single-use join token minted by `goodfetch group token`. In the Helm chart, a pod joins with its projected Kubernetes ServiceAccount token, which the Lead Daemon checks through the `TokenReview` API, together with the pod's name and its `StatefulSet`; no join token is stored in the chart. Either way, the Lead Daemon signs a certificate for the new Group member, which renews it automatically.
- **Leaving.** `goodfetch group leave` and revocation in the Console remove one Group member. The peer link is mutual TLS and is separate from the endpoints Clients use (ADR 0017).
- **Versions.** Group members share one product major (ADR 0030). During a rolling upgrade their minors may differ: the peer protocol joins ADR 0030's contract formats, versioned on its own and checked by a handshake like the JSON-RPC protocol's (changes ADR 0030). The Lead Daemon is upgraded last.
- **Which Daemon leads** is fixed by configuration: the chart's `lead` value, pod 0 by default, or the Daemon where `group init` ran.

**What the Lead Daemon holds** (changes ADRs 0031, 0035, 0036 and 0041).
- **Single writer of:** the Usage ledger; accounts, passkeys, roles and OIDC configuration; named tokens; the Console key; the Exposed Daemon setup token and the HTTP listener's token (ADR 0027); the Operator policy and user configuration (ADR 0028), including `pay_domains`, `consent_domains`, `wall_domains`, `trusted_cas`, Model provider and Secret provider configuration and Notification channels; certificate pins (ADR 0042); the Schedule store; the active-major marker; saved identities; the Plugin index's last-seen version and revocation list (ADR 0033) and the Clean-up rules channel's last-seen set (ADR 0047); the group Run index; the Audit log.
- **Read-mostly stores** are cached on every Group member. The Lead Daemon pushes each change over the peer link and each copy carries a version, so a Group member never applies an older one and nothing polls.
- **Usage.** Group members take ADR 0041's slices from the Lead Daemon's ledger; ADR 0041's minimum slice keeps the number of calls bounded. Limits on a person, a named token, the Daemon or the install now hold across the group, and a Partitioned crawl adds its own Limit scope (ADR 0054). A slice is reclaimed by the Group member that owns or adopts its run, which checks the run's writer lock and reads its state as ADR 0041 does, then reports the charge to the Lead Daemon.
- **Saved identities** are fetched from the Lead Daemon when a run starts and written back when it ends. ADR 0035's install-wide lock becomes a lease held at the Lead Daemon.
- **Schedules** fire only on the Lead Daemon; that role replaces ADR 0036's OS lock on the Schedule store. Their runs are placed like any other.
- **The Audit log** is one hash chain kept by the Lead Daemon, which appends the events Group members send. A Group member that cannot reach the Lead Daemon chains its events locally from the last lead hash it saw, and the Lead Daemon splices that branch in when it returns, so the chain stays verifiable end to end.
- **The group Run index** is fed by Group members, and each row names the owning Group member. `goodfetch runs reindex` on a Group member re-reports that Group member's runs.
- **Group events.** Schedule and Limit Notifications are deduplicated at the Lead Daemon. Run events stay with their run.

**What stays on each Group member.** Run state on its own volume (ADR 0049 refuses network filesystems), the run lock files, browser and Plugin caches and unpacked Plugins, TLS files, governor measurements, crash reports, the ops listener's token and the Response cache (ADR 0046). Erasure, discarding a run and deleting a saved identity are broadcast by the Lead Daemon to every Group member's Response cache, and each one acknowledges. Machine-specific settings (`local_addresses` of ADR 0044, TLS files, browser paths) take a new *group member* scope in ADR 0028's layers, applied only by that Group member.

**Secrets** (changes ADR 0015).
- A Daemon group needs a shared Secret provider that every Group member reaches (Vault or OpenBao, a cloud KMS or secrets manager, or a Kubernetes `Secret`), for the key-encryption key that wraps run data keys and for every credential that rotates, such as OAuth refresh tokens (ADR 0026).
- A run handed to another Group member carries its data key still wrapped, so no key crosses the peer link in the clear.
- The OS keychain and the encrypted local file are refused for those two uses in a group, with the Error code `secret_provider_not_shared`, which names the fix.

**Routing a Client.**
- A Client connects to any Group member. A unary call on a run another Group member owns is forwarded over the peer link, after a lookup in the group Run index that the receiving Group member caches.
- Heavy streams (live frames, screencast, WebRTC and the event stream) are redirected when the owner has an `advertise_url` the Client can reach, such as a per-pod Ingress or a VM address. The SDKs and the Console follow the redirect; without an `advertise_url`, those streams are forwarded too.
- After a run moves, the former owner answers with the Error code `run_moved`; the receiving Group member refreshes its cache and retries once.

**Placing a run.**
- The Lead Daemon places new runs. Group members push their governor headroom (ADR 0040) when it changes noticeably, and what they can run: installed browsers and Plugins, local addresses (ADR 0044), local model servers.
- The Lead Daemon keeps the Group members that can run the work, then picks the one with the most headroom. A `group_member` pin in `runs.start` is honoured, and Partitions go to distinct Group members (ADR 0054).

**Leaving a group: scale-down and drain** (adds to ADR 0049).
- A draining Group member tells a restart from a removal: in the chart by reading its own `StatefulSet` through a read-only ServiceAccount (its pod index at or above the desired replicas means removal), on VMs through `goodfetch group leave`.
- On a restart, it resumes the runs the group Run index still assigns to it.
- On a removal, each run's Checkpoint (ADR 0002) goes over the peer link to the Group member the Lead Daemon places it on, which resumes it and becomes its owner in the group Run index; Partitions hand their domains off instead (ADR 0054).
- What the grace period does not cover stays checkpointed on the kept volume, still assigned to the removed Group member. `goodfetch runs adopt` brings it into another Group member: the chart runs it as a `Job` that mounts the kept `ReadWriteOnce` volume. A pod index added back later resumes only what the group Run index still assigns to it, so a run already adopted never runs twice.
- The chart documents autoscaling on the governor's saturation metric (ADRs 0040 and 0050) through KEDA or the Prometheus adapter.

**When the Lead Daemon is unreachable.** Runs go on, and every Group member stays ready on `/readyz` (ADR 0050). A metric on the ops listener and an event on the event stream (ADR 0024) report it.

| Store | Behaviour | Why |
|---|---|---|
| Usage | Group members spend the slices they hold; a run that cannot take a new slice stops at its next safe point, resumable (ADR 0041) | A Limit is never exceeded |
| Tokens and sign-ins | A cached token check stays valid for what remains of it, at most 10 minutes; new sign-ins fail with the Error code `lead_unreachable` | A revoked token stops working within 10 minutes |
| Policy and configuration | The last pushed version applies | Never older than the Group member's last known version |
| Saved identity write-back | Kept encrypted on the Group member and retried until accepted; the run still finishes | No lost logins |
| Schedules | Nothing fires; on return, ADR 0036's rule catches up the latest missed occurrence once | One firing, never two |
| Placement and handoffs | New runs start on the Group member that received them; handoffs wait | Work keeps moving |

**Group backups and losing the Lead Daemon.**
- A *Group backup* is a consistent copy of the Lead Daemon's stores, written the way ADR 0002 writes a Checkpoint, to a configured destination: every 15 minutes by default, and on demand with `goodfetch group backup`. The group's certificate authority key is in it, wrapped by the shared Secret provider's key-encryption key.
- `goodfetch group promote --from-backup` makes another Group member the Lead Daemon; in the chart, the `lead` value then names that pod. Every peer message carries the group's *lead epoch*, so a former Lead Daemon that comes back is fenced off, as ADR 0003 fences a run.

**The Helm chart** (changes ADR 0049).
- `replicas` may be any number, and the `lead` value names the Lead Daemon's pod, pod 0 by default.
- A `Service` reaches any Group member for new connections; a headless `Service` gives each pod the stable name the peer link uses.
- With more than one replica, the chart refuses to render unless a Politeness store is set or `politeness.sharing: per_daemon` is chosen explicitly (ADR 0053), and unless a shared Secret provider is set.
- **Goodfetch Cloud** keeps its own gateway, ledger and index per Tenant (ADRs 0001 and 0031); a Daemon group is the self-hosted form of the same operations.
