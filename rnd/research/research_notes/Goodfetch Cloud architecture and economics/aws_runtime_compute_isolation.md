# AWS runtime for Goodfetch Cloud: compute, isolation and orchestration (as of 2026-10-12)

*Scope: compute, isolation and orchestration for the five workloads: (a) HTTP crawl and extraction runs, (b) hosted browser sessions, (c) untrusted Recipe or actor code, (d) AI agent runs and (e) scheduled runs. Prices are for us-east-1 and appear only where they decide a trade-off. "Computed" marks my own arithmetic from the cited unit prices. "Unverified" marks a claim I could not confirm against a primary source. Midway through, the shared web-search budget for this turn ran out ("limit: 200 WebSearch calls per turn, shared by every agent in it"). Everything after that point comes from direct fetches of official URLs that were already known, so some gaps below could not be searched.*

## Q1. How do Lambda, Fargate, ECS on EC2, EKS with Karpenter or Auto Mode, bare-metal Firecracker, nested virtualization, Batch, Step Functions and EventBridge Scheduler compare for the five workloads?

### Takeaway
No single AWS runtime fits all five workloads. Here is how each option fits:
- **Lambda (default).** Fits only short, bursty request/response work: it is capped at 15 minutes and cannot accept inbound connections.
- **Lambda MicroVMs (new).** Fits sessions of up to 8 hours with an inbound WebSocket endpoint, Firecracker isolation and suspend/resume. A running MicroVM costs about 3.2 times Fargate ARM per hour.
- **Fargate.** The simplest isolated container per run, with Graviton Spot. It starts in about 30 s or more, the smallest task is 0.25 vCPU / 0.5 GB, and each task gets its own network interface.
- **EC2 under ECS or EKS with Karpenter, on Graviton Spot.** Gives the best density and the lowest unit cost, at the highest operational burden.
- **Nested virtualization.** Launched on 2026-02-16, on Intel 7i/8i families only, so Firecracker on Graviton still needs `.metal`.
- **Batch, Step Functions and EventBridge Scheduler.** These are orchestrators, not runtimes for these workloads.

### Cited Findings

**Lambda (default compute type)**
- Limits:
  - Timeout is 900 s (15 minutes).
  - Memory is 128–10,240 MB; "At 1,769 MB, a function has the equivalent of one vCPU".
  - A container image can be up to 10 GB uncompressed; `/tmp` is 512–10,240 MB.
  - File descriptors and processes/threads are capped at 1,024 each.
  - Network bandwidth is 625 Mbps per execution environment.
  - Source: [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- Scaling: the default concurrency is 1,000, raisable to "tens of thousands". The scaling limit is "For each function, 1,000 execution environments every 10 seconds". New accounts get reduced concurrency and memory quotas — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- Response streaming: up to 200 MB per streamed response. Bandwidth is uncapped for the first 6 MB, then "2MBps for the remainder of the response" — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html). Streaming has existed since 2023-04-06 — [Lambda document history](https://docs.aws.amazon.com/lambda/latest/dg/lambda-releases.html)
- Isolation: the default compute type "is multi-tenant, making use of Firecracker MicroVM technology to provide isolation between execution environments" — [Lambda Managed Instances docs](https://docs.aws.amazon.com/lambda/latest/dg/lambda-managed-instances.html)
- Prices:
  - Duration on x86 is $0.0000166667 per GB-second; requests cost $0.20 per million.
  - The rate in the pricing page's durable-functions compute example is $0.0000133334 per GB-second. That is Lambda's Arm rate; the Arm table itself did not load.
  - Source: [Lambda pricing](https://aws.amazon.com/lambda/pricing/)
- Warm start: provisioned concurrency "keeps functions initialized and ready to respond in double-digit milliseconds" — [Lambda SnapStart docs](https://docs.aws.amazon.com/lambda/latest/dg/snapstart.html)
- 2022 benchmark (older scaling model): with default limits Lambda reached "~3,000 instantly, then +500/min"; with raised limits "~12,000 instantly" — [Vlad Ionescu, Scaling containers on AWS in 2022](https://www.vladionescu.me/posts/scaling-containers-on-aws-in-2022/)

**Lambda SnapStart**
- Runtimes: Java 11+, Python 3.12+ and .NET 8+, in ZIP and container-image formats. Container images "built on AWS Lambda base images or custom base images" use runtime hooks.
- Not supported: "Other managed runtimes (such as `nodejs24.x` …) and OS-only runtimes", provisioned concurrency, EFS, and "ephemeral storage greater than 512 MB".
- Billing: SnapStart charges caching (3-hour minimum per version) plus a restore charge per GB.
- Source: [SnapStart docs](https://docs.aws.amazon.com/lambda/latest/dg/snapstart.html); prices of $0.0000015046 per GB-second cached and $0.0001397998 per GB restored from [Lambda pricing](https://aws.amazon.com/lambda/pricing/)

**Lambda tenant isolation mode (announced 2025-11-19)**
- Callers pass a tenant ID, and "execution environments are never reused across different tenants". There is a limit of "2,500 tenant-isolated execution environments … for every 1,000 concurrent executions"; it is not offered in Asia Pacific (New Zealand), GovCloud or China — [Lambda tenant isolation docs](https://docs.aws.amazon.com/lambda/latest/dg/tenant-isolation.html), [What's New](https://aws.amazon.com/about-aws/whats-new/2025/11/aws-lambda-tenant-isolation-mode)
- Charge: $0.000167 per tenant-isolated execution environment created, scaled by memory in GB — [Lambda pricing](https://aws.amazon.com/lambda/pricing/)

**Lambda Managed Instances (launched 2025-11-30)**
- Launch date: [Lambda document history](https://docs.aws.amazon.com/lambda/latest/dg/lambda-releases.html)
- Model: functions run on EC2 instances in your account, including Graviton4.
  - Concurrency: one execution environment serves several invocations at once ("multi-concurrent invocations").
  - Isolation: "Managed Instances use containers running on EC2 Nitro instances to provide isolation rather than Firecracker. Capacity providers serve as the security boundary".
  - Price: "EC2-based pricing with a 15% management fee on top of the EC2 instance cost". Discounts apply to the EC2 part only.
  - Scaling: it "Scales asynchronously based on CPU resource utilization only, without cold starts", does not scale to zero, launches three instances by default, and absorbs "traffic doubling within five minutes by default".
  - Source: [Lambda Managed Instances docs](https://docs.aws.amazon.com/lambda/latest/dg/lambda-managed-instances.html)
- Timeout: 15 minutes for synchronous calls, and up to 5,400 s (90 minutes) "for functions … invoked asynchronously or through any event source mapping except Amazon MQ and Amazon DocumentDB". The file descriptor limit is 4,096 — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- Runtimes: Java 21+, Python 3.13+, Node.js 22+ and .NET 8+. Rust is "Supported using the OS-only runtime `provided.al2023`", as "a single process with async tasks powered by Tokio" — [Lambda Managed Instances runtimes](https://docs.aws.amazon.com/lambda/latest/dg/lambda-managed-instances-runtimes.html)
- Regions: a third-party post says availability reached all commercial Regions except four on 2026-06-08. Unverified against AWS — [hidekazu-konishi guide](https://hidekazu-konishi.com/entry/aws_lambda_managed_instances_design_guide.html)

**Lambda durable functions (generally available 2025-12-02, US East Ohio)**
- What it does: "automatically checkpoint[s] progress", and waits "pause execution without incurring compute charges", suspending "up to one year". At launch it supported Python 3.13/3.14 and Node.js 22/24 — [What's New 2025-12](https://aws.amazon.com/about-aws/whats-new/2025/12/lambda-durable-multi-step-applications-ai-workflows/)
- Region expansion: 14 more Regions on 2025-12-18 — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/12/lambda-durable-functions-14-additional-regions/). Then 16 more Regions plus Java 17+ on 2026-04-22 — [What's New](https://aws.amazon.com/about-aws/whats-new/2026/04/lambda-durable-functions-16-new-regions/)
- Hard quotas per execution: 3,000 durable operations and 100 MB of storage written. New executions can start at 300 per second — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- Prices: $8.00 per million operations, $0.25 per GB written, $0.15 per GB-month retained. Waits count as operations, and "on-demand functions pay no duration charges while suspended" — [Lambda pricing](https://aws.amazon.com/lambda/pricing/)

**Lambda MicroVMs (new product; launch date unverified)**
- What it is:
  - Purpose: "serverless compute environments that provide VM-level isolation with full operating system capabilities, snapshot-based rapid startup speeds, and fine-grained control over ingress networking (port access, support for HTTP/2, gRPC, WebSockets)". It is built "through Firecracker virtualization".
  - Suspend: MicroVMs "can be suspended when idle, preserving memory and disk state".
  - Target uses include "AI code execution sandboxes", "Multi-tenant CI/CD" and "Game servers".
  - Source: [Lambda MicroVMs guide](https://docs.aws.amazon.com/lambda/latest/dg/lambda-microvms-guide.html)
- Images: you upload a zip holding a `Dockerfile`, built on "a Lambda-published managed base image". Lambda runs the app, waits for an optional `/ready` hook, and then "captures a snapshot of the disk and memory state, including all running processes". Each MicroVM runs Amazon Linux 2023 — [MicroVMs core concepts](https://docs.aws.amazon.com/lambda/latest/dg/microvms-how-it-works.html)
- Lifecycle: PENDING → RUNNING → SUSPENDED → TERMINATED. While suspended, "No compute charges accrue". `--maximum-duration-in-seconds` ranges from 1 to 28,800 s (8 hours) "including both running and suspended time". `maxIdleDurationSeconds` has a maximum of 28,800 s — [MicroVMs core concepts](https://docs.aws.amazon.com/lambda/latest/dg/microvms-how-it-works.html), [Running MicroVMs](https://docs.aws.amazon.com/lambda/latest/dg/microvms-launching.html)
- Quotas:
  - Memory allocated across all MicroVMs defaults to 400 GB per account per Region, and 1,024 GB in us-east-1, us-west-2, us-east-2 and Tokyo. It is "Burstable up to four times this quota".
  - The 8-hour maximum is not adjustable.
  - Concurrent connections per MicroVM: "8 (1 vCPU) … 128 (16 vCPU)".
  - Requests per second per MicroVM: "40 (4 vCPU / 8 GB), 160 (16 vCPU / 32 GB)".
  - `RunMicrovm` is limited to 5 TPS, adjustable.
  - The docs say "Lambda MicroVMs support the ARM64 (AWS Graviton) architecture". I found no x86 option.
  - Source: [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- Bandwidth through the endpoint scales with size: from 0.5 GB / 0.25 vCPU at 1 MB/s (8 Mbps) up to 8 GB / 4 vCPU at 16 MB/s (128 Mbps) — [MicroVMs networking](https://docs.aws.amazon.com/lambda/latest/dg/microvms-networking.html)
- Vertical scaling: a MicroVM can "vertically scale to 4x of configured baseline during peak activity. Pay the baseline rate while your MicroVM is running and only pay for active use above the baseline" — [Lambda MicroVMs guide](https://docs.aws.amazon.com/lambda/latest/dg/lambda-microvms-guide.html)
- Prices (Arm): $0.0000276944 per vCPU-second and $0.0000036667 per GB-second. Snapshot writes on suspend cost $0.0038 per GB, reads on resume or launch cost $0.00155 per GB, and storage is $0.08 per GB-month — [Lambda pricing](https://aws.amazon.com/lambda/pricing/)
- Start latency: "snapshot-based sub-second startup", and "Resume from Firecracker snapshot in sub-second to single-digit seconds" — [MicroVMs as sandbox for Claude Managed Agents](https://docs.aws.amazon.com/lambda/latest/dg/microvms-integrations-claude-managed-agents.html)
- Launch date: not found in the Lambda document history. The earliest dated related entry is a managed policy for the "Lambda Network Connector", dated 2026-06-19 — [Lambda document history](https://docs.aws.amazon.com/lambda/latest/dg/lambda-releases.html). The hook interface's OpenAPI version is "2025-12-03" — [Running MicroVMs](https://docs.aws.amazon.com/lambda/latest/dg/microvms-launching.html)

**Fargate (ECS)**
- Task sizes run from 0.25 vCPU (512 MiB–2 GB) to 16 vCPU (32–120 GB), and now 32 vCPU (60, 120 or 244 GB) — [Fargate task definition differences](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-tasks-services.html). The 32 vCPU size does not appear in the ECS document history, so its date is unverified.
- Restrictions:
  - Always `awsvpc`, so each task gets its own network interface.
  - Not valid on Fargate: `privileged`, `dockerSecurityOptions` and `ipcMode`.
  - In `linuxParameters`, "the only capability you can add is `CAP_SYS_PTRACE`. The `devices`, `sharedMemorySize`, and `tmpfs` parameters are not supported".
  - Source: [Fargate task definition differences](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-tasks-services.html)
- Ephemeral storage is 20 GiB per task by default, raisable "up to a maximum of 200 GiB" — [Fargate security best practices](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/security-fargate.html)
- SOCI lazy loading: "containers only spend a few seconds on the image pull before they can start". AWS recommends it for images over 250 MiB compressed; only gzip or uncompressed images work (no zstd) — [Fargate task definition differences](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-tasks-services.html). New users must use SOCI manifest v2 since 2025-07-02 — [ECS document history](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/document_history.html)
- Prices:
  - ARM: $0.0000089944 per vCPU-second and $0.0000009889 per GB-second.
  - x86: $0.000011244 per vCPU-second and $0.000001235 per GB-second.
  - Billing is "per second with a 1-minute minimum"; Spot offers "up to a 70% discount".
  - Source: [Fargate pricing](https://aws.amazon.com/fargate/pricing/)
- Quotas:
  - On-Demand and Spot each start at 6 concurrent vCPUs (adjustable; "Fargate … automatically increases the quotas based on your usage").
  - Launch rate is 20 tasks/s sustained and 100 burst in most Regions.
  - A service launches at most 500 tasks per minute on Fargate.
  - RunTask launches at most 10 tasks per call, and a service holds at most 5,000 tasks.
  - Source: [ECS and Fargate quotas](https://docs.aws.amazon.com/general/latest/gr/ecs-service.html)
- Firecracker: "We have deployed Firecracker in two publically-available serverless compute services at AWS (Lambda and Fargate)" (2020) — [Firecracker NSDI'20 paper](https://www.usenix.org/conference/nsdi20/presentation/agache)
- 2022 benchmark (us-east-1, 1 vCPU / 2 GB tasks, 380 MB image):
  - ECS on Fargate started scaling at ~30 s and reached ~3,500 tasks in "a little over 5 minutes", and 10,000 in about 10 minutes.
  - ARM was about 20 s faster than x86.
  - The author concluded "Fargate is now faster than EC2 in these tests".
  - Source: [Vlad Ionescu 2022](https://www.vladionescu.me/posts/scaling-containers-on-aws-in-2022/)

**ECS on EC2 and ECS Managed Instances**
- ECS Managed Instances launched on 2025-09-30, gained more Regions on 2025-10-27, and had its maximum instance lifetime extended "from 14 days to 21 days" on 2025-12-11 — [ECS document history](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/document_history.html)
- Spot support arrived on 2025-12-18 via `capacityOptionType=Spot`. A management charge applies on top of Spot EC2 costs (paraphrased from a search summary of the announcement) — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/12/amazon-ecs-managed-instances-ec2-spot-instances). A third-party post says an existing capacity provider cannot be switched between On-Demand and Spot; unverified — [Classmethod](https://dev.classmethod.jp/articles/amazon-ecs-managed-instances-ec2-spot-instances/)
- "Amazon ECS Managed Instances follow the Amazon EC2 quotas" — [ECS service quotas](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/service-quotas.html)
- Capacity provider strategies:
  - A strategy holds at most 20 capacity providers.
  - It "can only contain either Fargate or Auto Scaling group capacity providers, but not both".
  - Only one provider can define a base.
  - Source: [Fargate capacity providers](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-capacity-providers.html)
- On EC2, a service launches at most 500 tasks per minute. A cluster holds at most 5,000 container instances and 500 tasks in PROVISIONING — [ECS and Fargate quotas](https://docs.aws.amazon.com/general/latest/gr/ecs-service.html)
- ECS tasks can run IPv6-only since 2025-09-29 — [ECS document history](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/document_history.html)
- 2022 benchmark: ECS on EC2 took ~30 s to start scaling at a 30% capacity target and ~2.5 min at 80%, reaching 3,500 containers in roughly 6–10 minutes — [Vlad Ionescu 2022](https://www.vladionescu.me/posts/scaling-containers-on-aws-in-2022/)

**EKS with Karpenter, and EKS Auto Mode**
- Karpenter disruption:
  - `WhenEmptyOrUnderutilized` is the default consolidation policy.
  - Spot-to-Spot replacement needs the `SpotToSpotConsolidation` feature flag and "a minimum instance type flexibility of 15 instance types".
  - Interruptions are handled "by watching an SQS queue", with a 2-minute Spot notice.
  - "Karpenter does not currently support taint, drain, and terminate logic for Spot Rebalance Recommendations".
  - `karpenter.sh/do-not-disrupt` excludes a node from consolidation but not from forceful disruption.
  - `expireAfter` defaults to 720h.
  - Source: [Karpenter disruption docs](https://karpenter.sh/docs/concepts/disruption/)
- EKS Auto Mode:
  - It runs on Karpenter.
  - Nodes use Bottlerocket-based immutable AMIs with SELinux enforcing and a read-only root, and "disallowing SSH or SSM access".
  - Nodes "have a maximum lifetime of 21 days (which you can reduce)".
  - It "Automatically handles EC2 Spot Instance interruption notices".
  - Source: [EKS Auto Mode docs](https://docs.aws.amazon.com/eks/latest/userguide/automode.html)
- Auto Mode fee: billed per second on top of EC2. The worked example's fees (for example $0.03672/h on c6a.2xlarge) equal 12% of the instance price; that ratio is derived, not stated. The control plane costs $0.10 per cluster-hour on standard support — [EKS pricing](https://aws.amazon.com/eks/pricing/)
- 2022 benchmark: EKS on EC2 with Karpenter (c6g.4xlarge Spot, Bottlerocket) took ~2.5 min to start scaling and reached ~3,500 pods in ~7 minutes. Karpenter beat cluster-autoscaler — [Vlad Ionescu 2022](https://www.vladionescu.me/posts/scaling-containers-on-aws-in-2022/)

**Bare metal with Firecracker (and Kata)**
- Firecracker specification: "<= 125 ms" from InstanceStart to guest `/sbin/init`, and "<= 5 MiB" of VMM memory overhead (1 vCPU, 128 MiB guest). Network reaches "up to 14.5 Gbps" at ≤80% of a host core. Measured on m5d.metal and m6g.metal (Graviton2) — [Firecracker SPECIFICATION.md](https://github.com/firecracker-microvm/firecracker/blob/main/SPECIFICATION.md)
- Kata Containers on EKS (AWS Containers blog, 2024-05-16):
  - It used `i3.metal`, because "standard EC2 instances don't allow nested virtualization". That statement predates the February 2026 launch.
  - The `kata-fc` RuntimeClass adds a fixed overhead of 130Mi memory and 250m CPU per pod, and Firecracker needs a devmapper snapshotter.
  - The authors "recommend using managed solutions" for most users.
  - Source: [AWS Containers blog](https://aws.amazon.com/blogs/containers/enhancing-kubernetes-workload-isolation-and-security-using-kata-containers/)

**Nested virtualization on regular EC2 instances (launched 2026-02-16)**
- The What's New post is dated 2026-02-16 — [What's New](https://aws.amazon.com/about-aws/whats-new/2026/02/amazon-ec2-nested-virtualization-on-virtual)
- Supported families today: M7i, M7i-flex, M8i, M8id, M8i-flex, C7i, C7i-flex, C8i, C8id, C8i-flex, R7i, R7iz, R8i, R8id, R8i-flex, X8i, I7i and I7ie. No Graviton or AMD family is listed.
- "KVM and Hyper-V are the supported L1 hypervisors"; there is "no additional cost"; it is enabled with `--cpu-options "NestedVirtualization=enabled"`.
- For performance-sensitive or latency-strict work, AWS recommends customers "evaluate bare metal instances".
- Source: [EC2 nested virtualization docs](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-nested-virtualization.html)

**AWS Batch, Step Functions and EventBridge Scheduler**
- AWS Batch supports Graviton Fargate Spot since August 2025 — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/08/aws-batch-graviton-based-spot-compute-fargate). It supports EKS access entries since 2026-10-05 — [What's New](https://aws.amazon.com/about-aws/whats-new/2026/10/aws-batch-access-entries/)
- Step Functions limits:
  - Standard workflows run up to 1 year; Express workflows up to 5 minutes.
  - An execution history holds 25,000 events.
  - A Distributed Map runs up to 10,000 parallel child executions.
  - Standard `StartExecution` refills at 300 per second in us-east-1, us-west-2 and eu-west-1.
  - An HTTP Task is capped at 60 s.
  - Source: [Step Functions quotas](https://docs.aws.amazon.com/step-functions/latest/dg/service-quotas.html)
- EventBridge Scheduler limits:
  - 10,000,000 schedules per Region by default, "adjustable to billions".
  - 1,000 invocations per second and 5,000 `CreateSchedule` TPS in the primary Regions (both adjustable).
  - A 256 KB target input and a per-schedule limit of 10 TPS.
  - Source: [EventBridge Scheduler quotas](https://docs.aws.amazon.com/scheduler/latest/UserGuide/scheduler-quotas.html)

**Lab baselines (Goodfetch repository)**
- Container pool: a fresh container per Session takes 180 ms to acquire, with 350 ms for the full Session. A warm pool takes ≈0 ms to acquire, with 188 ms for the full Session and "≈ 39 MiB per container". The Firecracker snapshot lane was `unsupported` because Docker Desktop exposes no `/dev/kvm` — [rnd/lab/reports/sinks-topology-2026-10-09.md](../../../lab/reports/sinks-topology-2026-10-09.md)
- HTTP: "reqwest and wreq require about 1.5 ms of CPU per task, less than 11 MiB of memory, and start in about ten milliseconds". A task there is 8 local pages served at 3 ms each — [rnd/lab/reports/http-transport-2026-10-09.md](../../../lab/reports/http-transport-2026-10-09.md)
- Browser: Chrome with the `rust-cdp` driver used 1,243 ms of CPU per task, with a maximum RSS of 2,860 MiB for driver plus browser — [rnd/lab/reports/light-engine-paired-2026-10-09.md](../../../lab/reports/light-engine-paired-2026-10-09.md)

### Inferences

**(a) HTTP crawl and extraction runs.** CPU per page is negligible, so cost is driven by the run's memory residency and by the overhead each isolated run carries.
- Dense packing of one engine process per run on Graviton EC2 (EKS with Karpenter, or ECS capacity providers), mostly on Spot, is cheapest. The engine fits in ~11 MiB, against Fargate's 0.5 GB floor.
- Lambda fits short (<15 min), spiky, single-URL API requests that should not pay for idle capacity. Longer crawls would need checkpoint/resume chaining every <15 minutes (ADR 0002 makes that possible but adds overhead).
- Lambda's 1,024-descriptor limit caps concurrent sockets per run.

**Lambda Managed Instances is a poor fit for ADR 0001.**
- Multi-concurrency means several invocations, and hence several tenants' runs, would share one container and process. That conflicts with "each cloud run gets its own engine process".
- Its isolation is containers, not Firecracker.
- It remains a candidate for Goodfetch's own trusted control-plane services (it supports Rust and Tokio).

**(b) Hosted browser sessions.**
- Lambda (default) is unsuitable: it has no inbound WebSocket and a 15-minute cap.
- Fargate works but has no warm pool: each session start costs a ~30 s+ task start, unless running tasks are kept idle and paid for.
- Lambda MicroVMs is the first managed AWS option with an inbound WebSocket per VM, Firecracker isolation, snapshot start and an 8-hour cap. It is ARM64-only and priced at a premium.
- Self-run EC2 hosts with pre-started microVM or container pools give sub-second start and the lowest cost.

**(c) Untrusted Recipe code.** This needs a VM-class boundary. The options, in order of rising operational burden and falling price:
1. Lambda MicroVMs.
2. Fargate.
3. Kata or Firecracker on `.metal`, or on nested-virtualization C8i/M8i.

See Q2.

**(d) AI agent runs.** These mostly wait on model calls.
- The cheapest home is a densely packed, CPU-overcommitted EC2 pool.
- Managed options that bill only active CPU (AgentCore Runtime) or suspend idle VMs (Lambda MicroVMs) win when a team will not run a fleet.
- Durable functions do not fit. The engine is a Rust binary, but the durable SDK covers Python, Node.js and Java. A model call in flight inside the engine is an ordinary HTTP wait, billed as duration, not a durable `wait`. A run is also capped at 3,000 operations per execution.

**(e) Scheduled runs.**
- EventBridge Scheduler's 10M schedules and 1,000 invocations/s are enough as the time source.
- The Schedule semantics of ADR 0036 (overlap, Watch, catch-up) stay in Goodfetch's own scheduler, with targets fed through a queue to the same pools.
- Step Functions Standard is warranted only for multi-step pipelines.
- Batch's queueing adds latency without adding isolation.

**Operational burden, lowest to highest:**
1. Lambda and Lambda MicroVMs.
2. Fargate.
3. ECS Managed Instances and EKS Auto Mode.
4. ECS or EKS on self-managed EC2 with Karpenter.
5. Kata on EKS bare metal.
6. A home-grown Firecracker orchestrator (E2B-like).

Auto Mode's immutable, no-SSH nodes probably also rule out installing custom runtimes such as `runsc` or Kata. That is inferred from "immutable" AMIs and unverified.

### Gaps
- No Lambda container-image cold-start study was retrieved, for Chromium images or in general. The search budget ran out before it.
- No recent (2024–2026) Fargate per-task start latency distribution was found; the only data is Vlad Ionescu's 2022 aggregate curves.
- I could not confirm the launch date, GA or preview status, or Region list of Lambda MicroVMs.
- The ECS Managed Instances management fee rate and the Fargate 32 vCPU launch date were not found.
- Launch times for `.metal` instances were not found.

## Q2. Which strong-isolation options fit untrusted user code on AWS, and how do they trade density, start time and cost?

### Takeaway
On AWS the established boundary for untrusted code is one VM per tenant session:
- **Managed Firecracker:** Lambda, Lambda tenant isolation, Lambda MicroVMs and AgentCore Runtime.
- **Fargate:** runs on Firecracker per the 2020 paper.
- **Self-run:** Firecracker or Kata on `.metal`, or since February 2026 on Intel 7i/8i instances with nested virtualization.

gVisor is the no-KVM alternative: its `systrap` platform is the one recommended inside VMs.

Lambda Managed Instances and plain ECS/EKS containers share a kernel. They are not adequate for Recipe code.

### Cited Findings
- **Lambda.** The default compute type uses Firecracker per execution environment. Lambda Managed Instances instead "use containers running on EC2 Nitro instances to provide isolation rather than Firecracker. Capacity providers serve as the security boundary" — [Lambda Managed Instances docs](https://docs.aws.amazon.com/lambda/latest/dg/lambda-managed-instances.html)
- **Lambda tenant isolation.** Each tenant gets dedicated Firecracker environments, never reused across tenants — [tenant isolation docs](https://docs.aws.amazon.com/lambda/latest/dg/tenant-isolation.html)
- **Lambda MicroVMs.** "A MicroVM is a resource that represents an isolated compute environment for a single tenant, user session, or job". The Claude Managed Agents integration describes a "Hardware-virtualized boundary per session" and says "Sessions never share state" — [MicroVMs core concepts](https://docs.aws.amazon.com/lambda/latest/dg/microvms-how-it-works.html), [Claude Managed Agents integration](https://docs.aws.amazon.com/lambda/latest/dg/microvms-integrations-claude-managed-agents.html)
- **MicroVM networking.**
  - Egress defaults to the public internet. A customer-managed VPC egress connector routes traffic through your VPC, where "outbound traffic is subject to security group rules and network ACLs".
  - Network connectors "cannot be changed while a MicroVM is running".
  - Ingress needs a JWE token scoped to one MicroVM, a set of ports and an expiry; "There is no unauthenticated access option".
  - Source: [MicroVMs networking](https://docs.aws.amazon.com/lambda/latest/dg/microvms-networking.html), [Running MicroVMs](https://docs.aws.amazon.com/lambda/latest/dg/microvms-launching.html)
- **MicroVM snapshots and uniqueness.** "If your code generates unique content during image version build phase (unique IDs, secrets, or entropy for pseudorandomness), that content is shared across all MicroVMs run from the same image version". AWS advises regenerating it in the `/run` hook and using its patched OpenSSL (`openssl-snapsafe-libs`) — [MicroVM snapshots](https://docs.aws.amazon.com/lambda/latest/dg/microvms-images-snapshots.html)
- **AgentCore Runtime.**
  - "Each session in AgentCore Runtime receives its own dedicated microVM with isolated compute, memory, and filesystem resources … After session completion, the entire microVM is terminated and memory is sanitized".
  - "Sessions last up to 8 hours for each lifecycle on microVMs, or up to 14 days on Instances".
  - Source: [AgentCore Runtime sessions](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-sessions.html)
- **Fargate.** Firecracker was "deployed … in … Lambda and Fargate" as of 2020 — [NSDI'20](https://www.usenix.org/conference/nsdi20/presentation/agache). The current Fargate security page I fetched does not describe the isolation mechanism. Fargate allows adding only `CAP_SYS_PTRACE` — [Fargate security best practices](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/security-fargate.html)
- **Kata on EKS.** It needs bare metal (or, since 2026, nested virtualization), and `kata-fc` adds 130Mi memory and 250m CPU per pod. AWS warns that operating hypervisors at scale "requires a deep understanding of their implications on the entire stack" — [AWS Containers blog](https://aws.amazon.com/blogs/containers/enhancing-kubernetes-workload-isolation-and-security-using-kata-containers/)
- **gVisor.**
  - "`systrap` replaced `ptrace` as the default gVisor platform in mid-2023".
  - "The `systrap` platform is a better choice when running inside a VM", while "The KVM platform runs best on bare-metal setups".
  - In nested VMs, "the `systrap` platform will often provide better performance in such a setup, due to the overhead of nested virtualization".
  - Source: [gVisor platforms](https://gvisor.dev/docs/architecture_guide/platforms/)
- **Firecracker overhead.** At most 5 MiB of VMM memory per microVM and ≤125 ms boot — [Firecracker SPECIFICATION.md](https://github.com/firecracker-microvm/firecracker/blob/main/SPECIFICATION.md)
- **Firecracker snapshots.** "we consider resuming execution from the same state more than once insecure". Unique identifiers and "cached random numbers, cryptographic tokens" are replicated across clones. VMGenID only reseeds the guest kernel's PRNG — [Firecracker snapshot support](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md)
- **E2B.**
  - Each sandbox is one Firecracker microVM "in its own cgroup and network namespace", with "a per-sandbox nftables egress firewall with domain allow/deny lists".
  - Templates "are pre-booted VMs (memory, disk, and machine state) stored in object storage"; memory is restored lazily via userfaultfd.
  - "The runtime needs Linux with KVM".
  - It ships "as Terraform for GCP, AWS, and Azure".
  - Source: [e2b-dev/infra README](https://github.com/e2b-dev/infra)
  - AWS maturity is contested. A third-party guide calls AWS "explicitly Beta" while GCP is production-ready (unverified) — [agenticwire](https://www.agenticwire.news/article/e2b-self-hosted-guide)
- **Vercel Sandbox.** Per a search summary of its docs, each sandbox runs in a Firecracker microVM "with its own filesystem, network, and dedicated kernel", on Amazon Linux 2023, only in `iad1`. Whether it runs on AWS EC2 is unverified — [Vercel Sandbox docs](https://vercel.com/docs/sandbox)
- **Nested virtualization.** "Customers are responsible for 'security in the cloud,' which includes securing the operating system, hypervisor, nested virtual machines" — [EC2 nested virtualization docs](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-nested-virtualization.html)
- **Lab.** "a pool reused without reset leaks one tenant's cookie to the next 30 times out of 30" — [ADR 0001](../../../../docs/adr/0001-single-tenant-engine-cloud-as-protocol-client.md), [rnd/lab/reports/sinks-topology-2026-10-09.md](../../../lab/reports/sinks-topology-2026-10-09.md)

### Inferences

**Trade-offs per option.** In approximate order of operational burden and start latency:

| Option | Start | Density | Cost and constraints |
|---|---|---|---|
| Lambda MicroVMs | Sub-second from snapshot | Billed per VM | Premium price (Q7); 8-hour cap; ARM64 only; account memory quota of 400 GB by default |
| Fargate | ~30 s+ | 0.25 vCPU / 0.5 GB minimum task | No snapshot; Graviton Spot available |
| Self-run Firecracker or Kata on Graviton `.metal` | Snapshot restore | Highest: ≤5 MiB VMM plus a minimal guest | Cheapest per tenant at scale. You build an orchestrator, image pipeline, snapshot store, per-VM egress firewall and patching, and `.metal` capacity comes in large, coarse units |
| Firecracker or Kata on C8i/M8i/R8i with nested virtualization | Not measured | Finer-grained instances than `.metal` | Intel only, at AWS's warned performance cost |
| gVisor (`runsc`) on ordinary Graviton or Intel nodes | Container-like | Highest density | Needs no KVM; a weaker boundary than a VM, and its system-call and network overheads matter for Chromium (unmeasured here) |

**Running the boundary per run fits ADR 0001's reset gate.** Discarding the VM is the reset, so run state never outlives the VM; only the checkpoint leaves it (ADR 0002).

**Egress policy must sit outside the sandbox.** ADR 0017 and ADR 0022 need policy enforcement that untrusted code cannot change, for example:
- E2B-style host nftables rules per VM;
- per-ENI security groups (Fargate, or a MicroVM's VPC egress connector);
- a host-side relay.

Lambda MicroVMs' default internet egress connector cannot express per-domain policy. The Browser host's egress relay would then have to run inside the VM, where Recipe code shares the VM. So untrusted Recipe runs should get a VPC egress connector with security groups, or run where the host enforces policy.

**Snapshot clones conflict with the Goodfetch-built Chromium's per-process fingerprint seed.** ADR 0040 says that seed is a launch flag. A browser launched before the snapshot would carry the same seed into every clone. So either:
- Chromium starts after restore, in the `/run` hook, paying Chromium's own start time; or
- the seed becomes settable at runtime, before the first navigation.

TLS and other random state in Chromium (BoringSSL) also needs a check for snapshot safety. This is unverified.

### Gaps
- No measurements were found of gVisor overhead on Chromium or network-heavy crawling, or of Cloud Hypervisor on AWS.
- How Fly.io and Modal isolate workloads was not retrieved (search budget). From memory, which I have not verified: Fly.io runs Firecracker on its own hardware rather than AWS, and Modal uses gVisor.
- I did not confirm whether AgentCore Runtime requires ARM64 containers, or whether AgentCore Browser accepts a custom Chromium build.
- No current AWS statement on Fargate's isolation mechanism was found.

## Q3. How should Chromium run on AWS (arm64 builds, /dev/shm and sandbox on Fargate and Lambda, fonts, memory per browser, sessions per vCPU, instance families)?

### Takeaway
Linux arm64 Chromium is now mainstream, and the brief's premise is out of date: Chrome for Testing publishes `linux-arm64` for Stable 155, Beta, Dev and Canary. Sparticuz has shipped arm64 since Chromium v135, and the lab already runs Playwright's Chromium 153 on Linux arm64. Graviton is therefore viable, provided Goodfetch builds its own stealth Chromium for arm64.

Fargate cannot enlarge `/dev/shm` or relax seccomp, and Lambda has no fonts and no inbound connections. Plan for 1.6–5 GB per browser, and size density from ADR 0040's measured per-page memory rather than from a fixed ratio.

### Cited Findings
- **Chrome for Testing.** Stable `155.0.8059.39` lists `linux-arm64` for chrome, chromedriver and chrome-headless-shell (HTTP 200). Beta `156.0.8078.12`, Dev `157.0.8081.0` and Canary `157.0.8099.0` also have `linux-arm64` — [Chrome for Testing availability](https://googlechromelabs.github.io/chrome-for-testing/)
- **Timeline (secondary sources only).**
  - Google had pre-announced ARM64 Linux Chrome for Q2 2026 — [VideoCardz](https://videocardz.com/newz/google-will-finally-release-chrome-for-arm64-linux-in-q2-2026)
  - One blog dates the first Chrome for Testing arm64 builds to 2026-08-11 (from 153.0.8001.0), with internal date inconsistencies; unverified — [Calibre](https://calibreapp.com/blog/chrome-for-linux-arm64)
- **Sparticuz (Lambda Chromium).**
  - "Starting at Chromium v135, arm64 binaries are available as a Lambda layer zip and a pack tar". The npm package itself is x64-only.
  - "You should allocate at least 512 MB of RAM … however, 1600 MB (or more) is recommended".
  - "The AWS Lambda runtime is not provisioned with any font faces"; Open Sans is bundled.
  - The pack unpacks to `/tmp`, and Playwright's profile directories can fill `/tmp` (`ERR_INSUFFICIENT_RESOURCES`).
  - For Dockerfile deployments, it suggests installing the distribution's Chromium.
  - Source: [Sparticuz/chromium](https://github.com/Sparticuz/chromium)
- **Puppeteer on the sandbox.** "The recommended way to run Chrome is using sandboxes". `--no-sandbox` is "strongly discouraged" unless you "absolutely trust the content". AppArmor can block the user namespaces the sandbox needs. Use `--init` or dumb-init to reap zombie processes — [Puppeteer troubleshooting](https://pptr.dev/troubleshooting)
- **Fargate restrictions.** No `sharedMemorySize`, `tmpfs`, `devices`, `privileged`, `ipcMode` or `dockerSecurityOptions`, and only `CAP_SYS_PTRACE` can be added — [Fargate task definition differences](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-tasks-services.html)
- **Lab.**
  - Chromium "refuses to start" as root with its sandbox in the Linux container, so the adapters launch "with `--no-sandbox` as root on Linux, as Playwright already does" — [rnd/lab/reports/linux-container-2026-10-09.md](../../../lab/reports/linux-container-2026-10-09.md)
  - Playwright 1.63's Chromium 153 passed 78/78 in a Linux arm64 container — [rnd/lab/reports/cross-engine-2026-10-09.md](../../../lab/reports/cross-engine-2026-10-09.md)
- **AWS Graviton guide.**
  - It runs Puppeteer by swapping the bundled x86 Chrome for a distribution Chromium. Its example is Chromium 116 from Fedora Koji for AL2023. It gives no performance or cost comparison — [Graviton Chrome and Puppeteer guide](https://github.com/aws/aws-graviton-getting-started/blob/main/software/ChromeAndPuppeteer.md)
  - Graviton generations: Graviton3 (C7g/M7g/R7g), Graviton4 (C8g/M8g/R8g/X8g/I8g) and Graviton5 (M9g/M9gd) — [aws-graviton-getting-started](https://github.com/aws/aws-graviton-getting-started)
- **Memory per browser.**
  - AWS's own AgentCore Browser pricing example bills a session at 2 vCPU and 4 GB, and notes 5 GB of use — [AgentCore pricing](https://aws.amazon.com/bedrock/agentcore/pricing/)
  - The lab's Chrome with `rust-cdp` peaked at 2,860 MiB RSS, driver plus browser — [rnd/lab/reports/light-engine-paired-2026-10-09.md](../../../lab/reports/light-engine-paired-2026-10-09.md)
- **Lambda CPU.** "At 1,769 MB, a function has the equivalent of one vCPU" — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- **Lambda MicroVMs.** Sizes run from 0.25 vCPU / 0.5 GB to at least 16 vCPU / 32 GB, with endpoint bandwidth of 32 Mbps at 2 GB / 1 vCPU — [MicroVMs networking](https://docs.aws.amazon.com/lambda/latest/dg/microvms-networking.html), [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
- **ADR 0040.** Memory decides how many pages may be open; browsers are recycled after 100 pages or past twice their warm size; and "The Goodfetch-built Chromium of ADR 0035, whose fingerprint seed is a launch flag" gets its own process — [ADR 0040](../../../../docs/adr/0040-one-resource-governor-per-engine-sized-from-measured-load.md)

### Inferences

**`/dev/shm` and the sandbox.**
- On Fargate, `/dev/shm` cannot be enlarged. Chromium should run with `--disable-dev-shm-usage`, putting shared memory in `/tmp` on the 20–200 GiB ephemeral disk.
- Seccomp and AppArmor cannot be tuned on Fargate, so Chromium's namespace sandbox is likely to fail there. It would then need `--no-sandbox`, leaving the Fargate task VM as the only boundary against a renderer exploit from a hostile page.
- A self-run microVM host can keep Chromium's own sandbox inside the guest (a non-root user with namespaces enabled), which is safer.

**Fonts.** Fonts are part of the fingerprint. The image (or snapshot) should carry a persona-consistent font set rather than whatever the distribution ships. Lambda ships none.

**Instance families.** Browsers are memory-bound while idle and CPU-bursty during page loads.
- m-family (4 GiB per vCPU) suits mixed load.
- r-family (8 GiB per vCPU) suits many idle keep-alive Sessions.
- c-family (2 GiB per vCPU) suits CPU-heavy batch rendering.
- Graviton (c8g/m8g/r8g, now m9g) gives the best price per vCPU wherever an arm64 Goodfetch Chromium exists.
- Intel c8i/m8i are needed only for nested-virtualization microVMs, or if a stealth persona must be x86 Linux.

**An arm64 browser may expose its architecture to sites.** `navigator.platform`, UA-CH architecture, and the WebGL/SwiftShader renderer can reveal arm64, and Linux on ARM is a rare desktop profile. ADR 0035's stealth catalog needs to decide whether arm64 hosts may serve x86 personas. This is unverified and worth a lab check.

**Sessions per vCPU.** With roughly 2–5 GB per browser and bursty CPU, about 2 Sessions per vCPU on m-family is a plausible start. The value should come from ADR 0025's search on the reference host, not from guesswork.

### Gaps
- Playwright's docs page gave no arm64 statements, so official arm64 support was not re-verified; the lab evidence above stands in.
- The default `/dev/shm` size on Fargate was not found.
- Whether Chromium's sandbox starts on Fargate or Lambda without `--no-sandbox` was not confirmed.
- No independent benchmark of Chromium on c7g/c8g against c7i/c7a was found.
- No authoritative sessions-per-vCPU figure for AWS instance types was found.

## Q4. How can sessions start fast and capacity scale elastically (warm pools, pre-started browsers, Firecracker snapshots, SnapStart, scaling signals, quotas, Spot risk, checkpoint and migrate)?

### Takeaway
Sub-second Session start on AWS comes only from capacity that is already running:
- a pre-started pool of containers or microVMs on running hosts (the lab saved 180 ms at 39 MiB per idle container);
- snapshot restore (Firecracker, Lambda MicroVMs' "sub-second").

New capacity takes 30 s or more on any service, so elasticity means keeping pool headroom ahead of demand and raising quotas in advance.

Spot works for checkpointable runs: there is a 2-minute notice, and the rebalance signal is best effort. Interactive browser Sessions, whose page state cannot migrate, belong on On-Demand capacity.

### Cited Findings
- **EC2 Auto Scaling warm pools.**
  - States are `Stopped`, `Running` or `Hibernated`. With `Stopped` "you pay only for the volumes that you use and the Elastic IP addresses"; `Running` is "highly discouraged".
  - "Warm pools don't support Spot Instances within mixed instance groups".
  - With ECS, instances "might register with the cluster before they finish initializing" unless the user data sets a special agent variable. EKS managed node groups have a similar problem.
  - Source: [EC2 Auto Scaling warm pools](https://docs.aws.amazon.com/autoscaling/ec2/userguide/ec2-auto-scaling-warm-pools.html)
- **Lab.** A warm pool of containers discarded after use takes ≈0 ms to acquire, against 180 ms for a fresh container, at ≈39 MiB idle per container — [rnd/lab/reports/sinks-topology-2026-10-09.md](../../../lab/reports/sinks-topology-2026-10-09.md)
- **Firecracker snapshots.**
  - Restore uses a MAP_PRIVATE mapping with "runtime on-demand loading of memory pages", or a UFFD backend.
  - "Snapshots must be resumed on an software and hardware configuration which is identical to what they were generated on".
  - "High snapshot restoration latency when cgroups V1 are in use".
  - Source: [Firecracker snapshot support](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md)
- **E2B.** "Creating a sandbox means restoring one, not booting a kernel" — [e2b-dev/infra README](https://github.com/e2b-dev/infra)
- **Lambda MicroVMs.**
  - Launch is "sub-second" from a snapshot. Auto-resume holds the first request, and the added latency depends on "the size of the suspended state being restored and the duration of your `/resume` hook".
  - A `/validate` hook lets Lambda "prefetch those portions of the image to reduce latency at run time".
  - Source: [Running MicroVMs](https://docs.aws.amazon.com/lambda/latest/dg/microvms-launching.html), [Claude Managed Agents integration](https://docs.aws.amazon.com/lambda/latest/dg/microvms-integrations-claude-managed-agents.html)
- **SnapStart.** It gives "as low as sub-second startup" for supported runtimes, but not for OS-only runtimes or with ephemeral storage over 512 MB. Provisioned concurrency responds in "double-digit milliseconds" — [SnapStart docs](https://docs.aws.amazon.com/lambda/latest/dg/snapstart.html)
- **Fargate start.** SOCI cuts image-pull time to "a few seconds" for large images — [Fargate task definition differences](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-tasks-services.html). In 2022, Fargate scaling began at ~30 s, and Karpenter on EC2 at ~2.5 min — [Vlad Ionescu 2022](https://www.vladionescu.me/posts/scaling-containers-on-aws-in-2022/)
- **Quotas that cap scale-out.**
  - Lambda: 1,000 environments per 10 s per function, and 1,000 concurrency by default — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
  - Fargate: 20 tasks/s sustained, 100 burst, 500 per service per minute, and 6 vCPUs initially — [ECS and Fargate quotas](https://docs.aws.amazon.com/general/latest/gr/ecs-service.html)
  - Lambda MicroVMs: `RunMicrovm` at 5 TPS and a 400 GB memory quota — [Lambda quotas](https://docs.aws.amazon.com/lambda/latest/dg/gettingstarted-limits.html)
  - AgentCore Browser: up to 500 sessions per browser tool, per a search summary — [AgentCore Browser session management](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-session-characteristics.html)
- **Lambda Managed Instances scaling.** It scales on CPU only and handles "traffic doubling within five minutes" — [Lambda Managed Instances docs](https://docs.aws.amazon.com/lambda/latest/dg/lambda-managed-instances.html)
- **Fargate Spot.**
  - "a two-minute warning is sent … as a task state change event to Amazon EventBridge and as a SIGTERM signal". `stopTimeout` can be up to 120 s (default 30 s).
  - "During periods of extremely high demand, Fargate Spot capacity might be unavailable … Fargate doesn't replace Spot capacity with on-demand capacity".
  - Source: [Fargate capacity providers](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-capacity-providers.html)
- **EC2 rebalance recommendations.** The signal "can arrive sooner than the two-minute Spot Instance interruption notice", but "It is not always possible … the rebalance recommendation signal can arrive along with the two-minute interruption notice". Events are "best effort", and AWS recommends polling the metadata every 5 seconds — [EC2 rebalance recommendations](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/rebalance-recommendations.html)
- **Karpenter and EKS Auto Mode.** Karpenter acts on the 2-minute notice via SQS but not on rebalance recommendations. Disruption budgets default to `nodes: 10%`, and "do not prevent Karpenter from terminating expired nodes" — [Karpenter disruption](https://karpenter.sh/docs/concepts/disruption/). Auto Mode handles interruption notices and enforces the 21-day node lifetime — [EKS Auto Mode](https://docs.aws.amazon.com/eks/latest/userguide/automode.html)
- **ECS Managed Instances Spot.** It carries the same two-minute interruption notice, per a third-party summary of the docs — [What's New](https://aws.amazon.com/about-aws/whats-new/2025/12/amazon-ecs-managed-instances-ec2-spot-instances)
- **ADR 0002.** "Checkpoints are taken on the `checkpoint` and `migrate` signals, on a clean stop, and periodically at an interval the caller sets". After losing a machine, "the run redoes the work since its last checkpoint" — [ADR 0002](../../../../docs/adr/0002-portable-run-state-resume-from-checkpoints.md)
- **ADR 0022.** Cookies and storage held by the hosted browser are copied back into run state, so a checkpoint covers them — [ADR 0022](../../../../docs/adr/0022-remote-browsers-driven-through-a-browser-host.md)

### Inferences

**Pool design.** Keep a pool of pre-started Browser hosts, or pre-restored microVMs, per Region and instance class.
- Each is discarded after one Session (ADR 0022) and replaced asynchronously.
- The scaling signal is pool headroom, meaning idle ready Sessions against the arrival rate, not CPU.
- Host capacity (Karpenter NodePools or ASGs) scales on that headroom with enough lead time to absorb new instances' ~30 s to minutes of start.

Pooling has one condition: the per-Session fingerprint seed and other unique state must be injected after the pool member is created (see Q2).

**HTTP and agent runs.** These have no sub-second requirement. Scale on queue backlog per run slot; the common KEDA or ECS target-tracking patterns qualify, but are unverified here.

**Spot handling.** The cloud should send `migrate` on the first of the rebalance recommendation, the 2-minute notice or Fargate's SIGTERM. The engine must then finish the checkpoint and upload within ~90 s (Fargate's `stopTimeout` maximum is 120 s). Two practices keep that window achievable:
- a periodic checkpoint interval in the cloud, so the final delta stays small;
- `karpenter.sh/do-not-disrupt` on live runs, so consolidation never evicts them. Forceful disruption still can.

**Interactive browser Sessions.** On interruption these lose their DOM and JavaScript state; only cookies and storage migrate. So they belong on On-Demand capacity, covered by Savings Plans, with Spot only for batch browser crawls.

**Quotas.** New-account defaults, such as 6 Fargate vCPUs, 5 `RunMicrovm` TPS and 1,000 Lambda concurrency, need raising before launch. Lambda MicroVMs' 5 TPS default means at most 300 new Sessions per minute until it is raised.

### Gaps
- KEDA's SQS scaler docs and ECS custom-metric scaling were not retrieved (search budget).
- Spot interruption frequencies for c7g, m7g, r7g, c8g, m8g, c7i, c7a and `.metal` were not extracted. The [Spot Advisor dataset](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json) is 1.19M characters and its us-east-1 block could not be reached through the fetch tool. Its legend: r=0 means <5%, 1 means 5–10%, 2 means 10–15%, 3 means 15–20%, 4 means >20%.
- Whether Karpenter supports warm pools was not checked.
- No measured Firecracker restore latency for a Chromium-sized guest (2–4 GB) on Graviton was found.

## Q5. How should WebSocket and JSON-RPC Sessions be routed to the right host (ALB, NLB, Global Accelerator, a custom gateway), and where should Sessions be placed?

### Takeaway
For hours-long WebSocket Sessions pinned to one host, AWS's managed L7 options either cap the connection or cannot route by Session ID:
- **API Gateway WebSocket:** 2-hour connection limit, 10-minute idle timeout, 128 KB messages.
- **ALB and NLB:** no routing by Session ID; the NLB's TLS listener idle timeout is fixed at 350 s.

Three patterns work:
1. A Session-aware gateway that maps Session ID to host, as E2B's edge proxy does, behind NLB TCP listeners (idle timeout up to 6,000 s) or an ALB (up to 4,000 s).
2. Direct-to-host URLs, optionally through Global Accelerator custom routing.
3. Lambda MicroVMs' per-VM HTTPS endpoint with JWE tokens.

Every action costs one round trip (ADR 0022), so Session hosts should sit near clients.

### Cited Findings
- **API Gateway WebSocket.**
  - Connection duration is 2 hours and idle timeout is 10 minutes; neither can be raised.
  - Frames are capped at 32 KB, messages at 128 KB and integration timeouts at 29 s.
  - New connections are limited to 500 per second.
  - Source: [API Gateway WebSocket quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/apigateway-execution-service-websocket-limits-table.html)
- **ALB.** The connection idle timeout defaults to 60 s, "The valid range is 1 through 4000 seconds", and "Application Load Balancers do not support HTTP/2 PING frames". The HTTP client keepalive defaults to 3,600 s and goes up to 604,800 s — [ALB attributes](https://docs.aws.amazon.com/elasticloadbalancing/latest/application/edit-load-balancer-attributes.html). ALB WebSocket support is from prior knowledge; it was not re-verified in this pass.
- **NLB.**
  - TCP idle timeout defaults to 350 s and is configurable from 60 to 6,000 s.
  - "The connection idle timeout for TLS listeners is 350 seconds and can't be modified".
  - Targets' ENI connection tracking (`TcpEstablishedTimeout`, 350 s on Nitro v6+) must be at least the NLB value.
  - Source: [NLB idle timeout](https://docs.aws.amazon.com/elasticloadbalancing/latest/network/update-idle-timeout.html)
- **Global Accelerator custom routing.** It lets "custom application logic … direct one or more users to a specific destination among many destinations". It "routes traffic only to ports on Amazon EC2 instances that are running in virtual private cloud (VPC) subnets", does not route by "geoproximity or health", and supports IPv4 only — [Global Accelerator custom routing](https://docs.aws.amazon.com/global-accelerator/latest/dg/about-custom-routing-accelerators.html)
- **Lambda MicroVMs endpoints.**
  - "Each MicroVM has its own dedicated endpoint. There is no load-balancing across MicroVMs from a single endpoint".
  - WebSocket clients pass the token and port as subprotocols (`lambda-microvms.authentication.<token>`, `lambda-microvms.port.N`).
  - PrivateLink endpoints exist for both management and data traffic.
  - Source: [Running MicroVMs](https://docs.aws.amazon.com/lambda/latest/dg/microvms-launching.html), [MicroVMs networking](https://docs.aws.amazon.com/lambda/latest/dg/microvms-networking.html)
- **AgentCore Runtime.** It "uses the session header to route requests to the same microVM instance" ("MicroVM stickiness") — [AgentCore Runtime sessions](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/runtime-sessions.html)
- **E2B.** "Sandbox traffic goes straight from the edge to the node and never passes through the API" (a client-proxy edge router) — [e2b-dev/infra README](https://github.com/e2b-dev/infra)
- **ADR 0022.** Raw CDP was rejected because "A click takes about five CDP commands, each a network round trip". Browserbase measured 42 ms per round trip on one route. The Browser host gives one round trip per action, and as a remote endpoint it "requires TLS and a token" — [ADR 0022](../../../../docs/adr/0022-remote-browsers-driven-through-a-browser-host.md)
- **Public IPv4.** Each address costs $0.005 per hour, "in-use" included — [VPC pricing](https://aws.amazon.com/vpc/pricing/)

### Inferences

**Recommended routing.**
1. Issue each Session a URL containing the host (or cell) and the Session ID, plus a short-lived token, as Lambda MicroVMs and AgentCore do.
2. Terminate TLS in a stateless Goodfetch gateway fleet: Envoy or a Rust proxy, behind an NLB **TCP** listener, avoiding the fixed 350 s TLS idle timeout.
3. The gateway looks up Session ID → host:port in a registry and proxies the WebSocket. Pings every <300 s keep idle Sessions alive under every timeout.

**Global Accelerator custom routing.** It is the AWS-native way to skip the extra proxy hop: it maps a listener port to an EC2 IP and port over the AWS backbone, with anycast entry. It suits EC2 hosts with many Sessions each, not Fargate tasks, and it is IPv4-only.

**Avoid public IPv4 addresses per Session.** At $0.005/h, that is 13% of a 1 vCPU / 2 GB Fargate ARM task and 51% of the smallest one (computed). Sessions should therefore enter through shared gateways rather than one public IP per Fargate task.

**Placement.**
- Hosted browsers belong in a few Regions near clients, for example us-east-1, eu-west-1 or eu-central-1, and ap-southeast-1, because every action pays a client↔host round trip.
- HTTP crawls and agent runs can sit in the cheapest Region with capacity, since target-site latency is masked by concurrency and the exit location is set by proxies.
- Run multiple AZs per Region, with Sessions pinned to one AZ.

### Gaps
- I found no source on how Browserbase, Steel or Hyperbrowser route Sessions.
- Global Accelerator custom routing limits (ports, destinations) were not retrieved.
- CloudFront WebSocket limits were not retrieved.
- No measured user-to-Region or Region-to-target latencies were found.

## Q6. What do public case studies say (AWS blogs and talks on browser fleets, scraping at scale, Firecracker sandboxes on AWS, Lambda Chromium at scale)?

### Takeaway
The public record converges on one pattern: one Firecracker microVM per session, started from a snapshot, with suspend or discard at the end.
- **AWS's own products:** Lambda MicroVMs, which AWS documents as the sandbox for Claude Managed Agents, Cursor and OpenAI agents; and AgentCore Runtime and Browser, which use one microVM per session with an 8-hour cap.
- **E2B:** open-source, Firecracker with userfaultfd templates, GCP-first and AWS in beta.
- **Vercel Sandbox:** Firecracker.
- **The self-managed path:** shown by AWS's Kata-on-EKS guide, which itself discourages that path for most users.

I could not retrieve re:Invent talks or AWS blog posts specifically about headless-browser fleets or Lambda Chromium at scale.

### Cited Findings
- **AgentCore Browser.** Each session runs in its own microVM with separate CPU, memory and filesystem, which is "fully terminated when the session ends" with memory sanitized. Sessions default to a 15-minute timeout, configurable "up to 8 hours". Up to 500 sessions run per browser tool, a live view allows takeover and hand-back, and recordings go to S3. Per a search summary — [AgentCore Browser session management](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-session-characteristics.html), [AgentCore Browser tool](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/browser-tool.html)
- **AgentCore pricing.**
  - Browser, and Runtime v1: $0.0895 per vCPU-hour and $0.00945 per GB-hour. "I/O wait and idle time is free, if no other background process is running"; memory is billed at "peak memory consumed up to that second".
  - Runtime v2 consumption: $0.1276 per vCPU-hour and $0.0169 per GB-hour. Committed baseline: $0.0997 per vCPU-hour and $0.0132 per GB-hour, listed as "launching by October 2026".
  - In v2 "Idle memory is reclaimed … after 120 seconds".
  - Instances mode costs EC2 plus a fee (12% in the example).
  - The worked browser example runs 2 vCPU for 120 s of CPU and 4 GB for 600 s, for $0.012267 per session.
  - Source: [AgentCore pricing](https://aws.amazon.com/bedrock/agentcore/pricing/)
- **Lambda MicroVMs for Claude Managed Agents (AWS reference design).**
  - "A control plane launches one MicroVM per Claude session". A webhook reaches a launcher Lambda function, which calls `RunMicroVM`; the MicroVM claims the session and runs the tool calls.
  - "Your Anthropic organization API key never reaches AWS compute".
  - The idle policy for per-session MicroVMs is `suspendedDurationSeconds: 0` and `autoResumeEnabled: false`.
  - A sample repository is [aws-samples/sample-lambda-microvm-claude-managed-agents](https://github.com/aws-samples/sample-lambda-microvm-claude-managed-agents).
  - Source: [MicroVMs Claude integration](https://docs.aws.amazon.com/lambda/latest/dg/microvms-integrations-claude-managed-agents.html)
  - The guide's table of contents also lists "Cursor Cloud Agents" and "OpenAI Agents API" integrations — [Lambda guide table of contents](https://docs.aws.amazon.com/lambda/latest/dg/toc-contents.json)
- **E2B's architecture.** One Firecracker microVM per sandbox, a per-sandbox network namespace and nftables egress firewall, pre-booted templates restored lazily via userfaultfd, and pause stored as a diff in object storage. Fork can start "up to a hundred per request" — [e2b-dev/infra README](https://github.com/e2b-dev/infra)
- **Vercel Sandbox.** Firecracker microVMs on Amazon Linux 2023, in `iad1`. A related InfoQ write-up of Vercel's build system ("Hive") describes "boxes" (bare metal) and "cells" (VMs) — [Vercel Sandbox docs](https://vercel.com/docs/sandbox), [InfoQ on Vercel Hive](https://www.infoq.com/news/2025/01/vercel-hive) (both from search summaries)
- **Kata on EKS.** The AWS Containers blog walkthrough (May 2024) used `i3.metal` with Firecracker and Cloud Hypervisor runtime classes and gave no performance numbers — [AWS Containers blog](https://aws.amazon.com/blogs/containers/enhancing-kubernetes-workload-isolation-and-security-using-kata-containers/)
- **Firecracker in production.** It is deployed in Lambda and Fargate, "where it supports millions of production workloads, and trillions of requests per month" — [NSDI'20](https://www.usenix.org/conference/nsdi20/presentation/agache)
- **Container scaling benchmark.** It compares ECS and EKS on Fargate and EC2 (Karpenter), and Lambda, at up to 3,500–10,000 containers — [Vlad Ionescu 2022](https://www.vladionescu.me/posts/scaling-containers-on-aws-in-2022/)
- **Chromium on Lambda.** The community standard is Sparticuz/chromium: 1,600 MB recommended, arm64 since v135, no system fonts — [Sparticuz/chromium](https://github.com/Sparticuz/chromium)

### Inferences
- **AWS's direction.** AWS's newest isolation products (MicroVMs, AgentCore) are microVMs with snapshots and suspend, which supports a microVM-per-run design for Goodfetch Cloud (ADR 0001).
- **AgentCore and Lambda MicroVMs may share infrastructure.** AgentCore Runtime v2's committed-baseline rates ($0.0997 per vCPU-hour, $0.0132 per GB-hour) equal Lambda MicroVMs' per-second rates converted to hours (computed). This is a coincidence of prices, not evidence; speculative.
- **What managed products cannot do.** A managed AWS browser (AgentCore Browser) settles the hosting question but not stealth. It runs AWS's Chrome, so ADR 0035's Goodfetch-built Chromium requires running the browser yourself, on Lambda MicroVMs, Fargate or EC2. This rests on the unverified assumption that AgentCore Browser rejects custom browser builds.

### Gaps
- Not retrieved because the search budget ran out:
  - re:Invent 2024 and 2025 sessions;
  - AWS Compute and Containers blog posts on headless Chrome fleets, Lambda Chromium at scale, and Apify's AWS architecture;
  - Browserbase, Browserless and Steel infrastructure posts;
  - E2B and Fly.io engineering blogs;
  - independent Lambda cold-start studies.
- No public cost-per-browser-hour data from these operators was collected here.

## Q7. Bottom line: which design gives the lowest cost per browser-hour and per 1,000 HTTP pages while scaling with demand, and where are the break-even points?

### Takeaway
The lowest-cost design is a Graviton EC2 fleet with Karpenter, mostly on Spot:
- one isolated engine per run, densely packed, for HTTP and agent runs;
- one Firecracker microVM (or Kata pod) per Session, from a pre-started pool, for hosted browsers and untrusted Recipes, on On-Demand capacity for interactive Sessions.

A browser-hour costs about $0.036 at 1 vCPU / 2 GB on On-Demand, falling to ~$0.014–0.019 on Spot (assumed discount). Compute for 1,000 HTTP pages costs fractions of a cent on every platform, so bandwidth and proxies, covered by the other researcher, dominate.

Break-even points:
- Lambda costs ~2.1–2.3× Fargate or EC2 On-Demand per busy hour, so it wins below ~44–48% utilization (~17% against Spot).
- Lambda MicroVMs must be suspended ≥69% of the time to match Fargate ARM.
- AgentCore v1 beats Fargate ARM below 23% average CPU.

### Cited Findings
- **Unit prices, us-east-1.**
  - Lambda: $0.0000133334 per GB-second (Arm) and $0.0000166667 (x86); requests $0.20 per million — [Lambda pricing](https://aws.amazon.com/lambda/pricing/)
  - Lambda MicroVMs: $0.0000276944 per vCPU-second and $0.0000036667 per GB-second; snapshots $0.0038 per GB written and $0.00155 per GB read — [Lambda pricing](https://aws.amazon.com/lambda/pricing/)
  - Fargate ARM: $0.0000089944 per vCPU-second and $0.0000009889 per GB-second; x86: $0.000011244 and $0.000001235; 1-minute minimum; Spot "up to a 70% discount" — [Fargate pricing](https://aws.amazon.com/fargate/pricing/)
  - c7g.2xlarge (8 vCPU, 16 GiB) On-Demand: $0.289 per hour, quoted in AWS's own AgentCore example — [AgentCore pricing](https://aws.amazon.com/bedrock/agentcore/pricing/)
  - EKS Auto Mode fee: ≈12% of the EC2 price, derived from the example — [EKS pricing](https://aws.amazon.com/eks/pricing/)
  - Lambda Managed Instances fee: 15% of the EC2 price — [Lambda Managed Instances](https://docs.aws.amazon.com/lambda/latest/dg/lambda-managed-instances.html)
  - Public IPv4: $0.005 per hour. NAT Gateway: $0.045 per hour plus $0.045 per GB processed, on the page's US East (Ohio) example — [VPC pricing](https://aws.amazon.com/vpc/pricing/)
- **Lab cost per task.** HTTP: ~1.5 ms CPU per 8-page task, <11 MiB. Chrome: ~1.2 s CPU per task, 2.8 GiB peak — [rnd/lab/reports/http-transport-2026-10-09.md](../../../lab/reports/http-transport-2026-10-09.md), [rnd/lab/reports/light-engine-paired-2026-10-09.md](../../../lab/reports/light-engine-paired-2026-10-09.md)

### Inferences

**Browser-hour at 1 vCPU / 2 GB (computed from the prices above):**

| Platform | Cost per browser-hour | Caveats |
|---|---|---|
| EC2 c7g.2xlarge slot, On-Demand | $0.036 | At full packing; $0.041 with Auto Mode's fee |
| EC2 c7g.2xlarge slot, Spot | ~$0.014 | Assumes a 60% Spot discount (not retrieved); ~$0.019 if Auto Mode's 12% fee is charged on the On-Demand price (unverified) |
| Fargate ARM, On-Demand | $0.040 | $0.045 with a public IPv4 address |
| Fargate ARM, Spot | ~$0.012 | At the maximum 70% discount; no warm start; capacity "might be unavailable" |
| Fargate x86, On-Demand | $0.049 | |
| AgentCore Browser (v1 rates) | $0.028 at 10% CPU, $0.037 at 20%, $0.108 at 100% | AWS's own Chrome |
| Lambda MicroVMs, running | $0.126 | $0 compute while suspended. A suspend-resume cycle of a 2 GB VM costs $0.0107 in snapshot fees, about 5 minutes of running, so suspending pays only for idle gaps over ~5 minutes |
| Lambda (default), 2 GB Arm | $0.096 | Unusable for multi-hour inbound Sessions |

Memory-bound overcommit on m- or r-family Graviton cuts the EC2 figure further: idle keep-alive Sessions need RAM, not a full vCPU.

**Per 1,000 HTTP pages (computed).** CPU alone is 1.5 vCPU-seconds, about $0.000015 on EC2 and $0.000013 on Fargate. Cost is set by residency, that is how long the run stays up per 1,000 pages.

For a polite crawl at 1 page/s (1,000 s of residency):

| Platform | Cost per 1,000 pages | Caveats |
|---|---|---|
| Lambda, 256 MB / 512 MB | $0.0033 / $0.0067 | |
| Fargate, smallest ARM task | $0.0027 | $0.0041 with public IPv4; ~$0.0022 on Spot with IPv4 |
| Lambda MicroVMs, smallest | $0.0088 | |
| Dense EC2 c7g.2xlarge, On-Demand | $0.00027 at ~300 runs per node, $0.0008 at ~100 microVMs per node | Assumed densities: ~50 MiB per containerized run, ~160 MiB per microVM |

At 50 pages/s (20 s of residency per 1,000 pages), every option costs ≤ $0.0002. All of these sit well below proxy and bandwidth cost.

**Break-even points (computed).**
- **Lambda (default) against provisioned capacity.** At ~1 vCPU (1,769 MB), Lambda Arm costs $0.0829/h, against $0.0395 for Fargate ARM 1 vCPU / 2 GB (2.1×) and $0.0361 for an EC2 c7g On-Demand slot (2.3×). Lambda is cheaper only if the alternative's average utilization would fall below ~48% (Fargate), ~44% (EC2 On-Demand) or ~17% (EC2 Spot at an assumed −60%). It is also cheaper for jobs shorter than Fargate's ~30 s start plus 1-minute minimum.
- **Fargate ARM against EC2 On-Demand.** At full packing they are within ~10%. EC2 pulls ahead through three things:
  - overcommit, which Fargate lacks;
  - density, because Fargate's 0.25 vCPU / 0.5 GB floor is far above the engine's ~11 MiB;
  - no per-task ENI or public IPv4.
- **Lambda MicroVMs against Fargate ARM.** MicroVMs cost 3.2× per running hour, so they break even only if suspended ≥69% of wall time. A Session counts as idle only when no traffic crosses its endpoint, so Sessions holding an open WebSocket rarely qualify.
- **AgentCore v1 against Fargate ARM On-Demand.** AgentCore is cheaper below 23% average CPU, which typical agent Sessions waiting on model calls fall under. At v2 consumption rates the threshold drops to 4.5%.

**Recommended architecture.** The main path:
- **Platform.** One EKS cluster (or ECS) per Region with Karpenter. Spread Graviton NodePools across ≥15 instance types, for Spot-to-Spot consolidation:
  - c8g, m8g and r8g Spot for HTTP, agent and batch-browser runs;
  - m8g and r8g On-Demand, with Savings Plans, for interactive Sessions;
  - Graviton `.metal` for Kata, or a Firecracker pool, for untrusted Recipe runs and per-Session microVMs.
- **Ingress.** A Session gateway behind an NLB TCP listener.
- **Scheduling.** EventBridge Scheduler → queue → Goodfetch dispatcher.
- **Spot.** Checkpoint and migrate on Spot signals.

Fallbacks:
- **Managed escape hatches for bursts and early stages:**
  - Fargate (ARM Spot) for runs that tolerate ~30 s starts;
  - Lambda MicroVMs for untrusted, sub-second sandboxes up to 8 hours without running a fleet;
  - Lambda for short scrape-API calls.
- **Nested virtualization (C8i/M8i/R8i).** A fallback for finer-grained microVM hosts on x86, at a performance cost AWS warns about.

### Gaps
- Spot prices and interruption rates per instance type were not retrieved.
- m7g, m8g, r8g and m9g On-Demand prices were not fetched (the other researcher's price sheet).
- Retail browser-hour prices of Browserbase and others were not collected.
- Real per-page CPU on live TLS pages with extraction is unknown: the lab's 1.5 ms is per 8 local pages.
- Achievable packing efficiency and microVM guest overhead for Goodfetch's images are unknown.
- I did not confirm whether EKS Auto Mode's fee applies at the On-Demand rate on Spot nodes.
- All of these should be measured before the cost model is fixed.
