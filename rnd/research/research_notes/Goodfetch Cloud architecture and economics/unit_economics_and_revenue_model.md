# Goodfetch Cloud on AWS: unit economics, gross margins and revenue scenarios (model of 2026-10-12)

How to read these notes:
- **Inputs.** The model is [unit_economics_model.py](unit_economics_model.py), in this folder. Its 128 inputs sit in one assumptions block. Each input names its source file and section. The 55 inputs that no note gives are marked **MODEL ASSUMPTION**.
- **Running it.** `python3 unit_economics_model.py` prints every table below; `python3 unit_economics_model.py units margins` prints only the sections named. It uses the standard library only, and two runs give byte-identical output.
- **Prices.** USD list prices, dated as in the notes: AWS price-list files published 2026-09-11 to 2026-10-10, and vendor pages read 2026-10-12. A month is 730 hours.
- **Facts versus outputs.** Cited Findings hold facts from the notes, with their sources. Every figure under an Inferences heading is a **model output** of the script, not a fact. The revenue scenarios are scenarios, not forecasts.
- **Deployment.** "Recommended AWS deployment" means:
  - EKS with Karpenter on Graviton;
  - 60% of interactive browser capacity on a 1-year Compute Savings Plan, the rest On-Demand;
  - batch crawl capacity on 80% Spot;
  - Recipe runs in Firecracker microVMs on Graviton `.metal` (80% Spot);
  - fleet utilization 65%.
  This follows [aws_runtime_compute_isolation.md](aws_runtime_compute_isolation.md) Q7 and [aws_platform_networking_billing_policy.md](aws_platform_networking_billing_policy.md) §8–§9.

## 1. What does one unit cost to produce on AWS (us-east-1, with eu-west-1), under each deployment option?

### Takeaway
On the recommended AWS deployment, a billed browser-hour costs about $0.035 (lean headless) to $0.14 (rich headful with live view). 1,000 pages cost about $0.003 (static HTML), $0.04 (JS with resources blocked) and $1.59 (protected). A Compute Unit costs about $0.01. A 10-step agent task costs $0.03 to $1.10 by model class, and compiled replay cuts mid and frontier tasks 2.6–4.2x. eu-west-1 is 3–8% dearer for compute and up to 10% dearer for Claude via Bedrock's regional endpoint. Cost is set by proxy bytes, model tokens and fleet utilization, not by CPU: compute is 9% of the static class's tiny cost, 3.5% of the JS class and 0.2% of the protected class.

### Cited Findings

**AWS compute (Linux, shared tenancy)**
- us-east-1 on-demand c7g.4xlarge (16 vCPU, 32 GiB) is $0.58/h, $0.03625 per vCPU-hour, and the per-vCPU rate is flat from 2xlarge through `.metal` within a family. m7g.metal (64 vCPU, 256 GiB) is $2.6112/h. c8i.4xlarge is $0.7497/h and m8i.4xlarge $0.8467/h. In eu-west-1: c7g.4xlarge $0.6202/h, m7g.metal $2.9104/h, c8i.4xlarge $0.8044/h — [aws_price_sheet.md §1](aws_price_sheet.md), from the [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json) and [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json).
- No Upfront Savings Plans in us-east-1 take 26.5–28.3% off (Compute SP 1 yr), 47.8–58.2% (Compute SP 3 yr), 33.8–37.4% (EC2 Instance SP 1 yr) and 54.6–63.7% (EC2 Instance SP 3 yr). In eu-west-1 the Compute SP 1 yr discount is weaker: 17.9–27.6% (c7g 23.6%) — [aws_price_sheet.md §1](aws_price_sheet.md), from the [SP offer us-east-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/us-east-1/index.json).
- Spot sits 43–90% below on-demand in the 2026-10-12 snapshot. The Spot Advisor's 30-day savings are 43–80% in us-east-1 (c7g.4xlarge 54% at <5% interruption; m7g.metal 74%; m8i.4xlarge 61% at >20%). The note calls the Advisor figure "the safer planning figure" because 16 snapshot values equal exactly 10% of on-demand — [aws_price_sheet.md §1](aws_price_sheet.md), from the [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json) and [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).
- The notes disagree on the Spot discount. The runtime note assumed 60% ([aws_runtime_compute_isolation.md Q7](aws_runtime_compute_isolation.md)). The provider note used $0.242/h for c7g.4xlarge Spot from a third-party mirror, 58.3% off ([provider_alternatives_vs_aws.md §1](provider_alternatives_vs_aws.md), via [Vantage](https://instances.vantage.sh/aws/ec2/c7g.4xlarge)).
- Fargate ARM costs $0.03238 per vCPU-hour and $0.00356 per GB-hour, identical in us-east-1 and eu-west-1. Fargate Spot is advertised "at up to a 70% discount" and has no published rate. Billing is per second with a 1-minute minimum — [aws_price_sheet.md §2](aws_price_sheet.md), from [Fargate pricing](https://aws.amazon.com/fargate/pricing/).
- Lambda Arm costs $0.0000133334 per GB-second. Lambda MicroVMs (Arm, us-east-1) cost $0.0000276944 per vCPU-second and $0.0000036667 per GB-second, at a 2:1 GB:vCPU ratio; snapshot reads cost $0.0015467699/GB and writes $0.0037977138/GB. eu-west-1 is about 5% dearer — [aws_price_sheet.md §3](aws_price_sheet.md), from [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
- AgentCore Browser and Runtime v1 cost $0.0895 per vCPU-hour of active CPU and $0.00945 per GB-hour of peak memory, with "I/O wait and idle time" free; v2 consumption is $0.1276/$0.0169 — [aws_runtime_compute_isolation.md Q6](aws_runtime_compute_isolation.md), from [AgentCore pricing](https://aws.amazon.com/bedrock/agentcore/pricing/).
- EKS costs $0.10 per cluster-hour; Auto Mode adds 12% of the instance price; ECS Managed Instances adds 12%; Lambda Managed Instances adds 15% — [aws_price_sheet.md §1, §3](aws_price_sheet.md), from [EKS pricing](https://aws.amazon.com/eks/pricing/).
- Nested virtualization (since 2026-02-16) covers only Intel 7i/8i families (C8i, M8i, R8i and others), not Graviton or AMD. AWS recommends bare metal for performance-sensitive work — [aws_runtime_compute_isolation.md Q1](aws_runtime_compute_isolation.md), from the [EC2 nested virtualization docs](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-nested-virtualization.html). GCP reports nested CPU-bound VMs "might experience a 10% or greater decrease in performance" — [provider_alternatives_vs_aws.md §1](provider_alternatives_vs_aws.md), from the [GCP docs](https://docs.cloud.google.com/compute/docs/instances/nested-virtualization/overview).
- Isolation overheads:
  - Firecracker: "<= 5 MiB" of VMM memory and "<= 125 ms" to guest init — [aws_runtime_compute_isolation.md Q1](aws_runtime_compute_isolation.md), from [Firecracker SPECIFICATION.md](https://github.com/firecracker-microvm/firecracker/blob/main/SPECIFICATION.md).
  - Kata's `kata-fc` RuntimeClass: a fixed 130Mi of memory and 250m of CPU per pod — [aws_runtime_compute_isolation.md Q1](aws_runtime_compute_isolation.md), from the [AWS Containers blog](https://aws.amazon.com/blogs/containers/enhancing-kubernetes-workload-isolation-and-security-using-kata-containers/).
  - Firecracker is tested only on `.metal` instances; no nested-virtualization benchmark was found — [open_source_cloud_layers.md §3](open_source_cloud_layers.md), from the [Firecracker repository](https://github.com/firecracker-microvm/firecracker).

**AWS network, storage and observability**
- Internet egress costs $0.09/GB after 100 GB free, then $0.085, $0.07 and $0.05 at 10, 50 and 150 TB, identically in all three Regions. Ingress is free; cross-AZ is $0.01/GB; NAT Gateway is $0.045/h plus $0.045/GB in us-east-1 ($0.048 and $0.048 in eu-west-1); public IPv4 is $0.005/h — [aws_price_sheet.md §6](aws_price_sheet.md), from the [DataTransfer offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/us-east-1/index.json) and [VPC pricing](https://aws.amazon.com/vpc/pricing/). The provider note quoted the same tiers as unverified ([provider_alternatives_vs_aws.md §1](provider_alternatives_vs_aws.md)).
- NLB and ALB cost $0.0225/h each in us-east-1 ($0.0252 in eu-west-1), plus $0.006 per NLCU-hour or $0.008 per LCU-hour. The LCU and NLCU dimensions themselves are in no note — [aws_price_sheet.md §6](aws_price_sheet.md); [aws_platform_networking_billing_policy.md §2 Gaps](aws_platform_networking_billing_policy.md).
- The recommended session front door is a Regional NLB on a TCP listener in front of a Rust session gateway, which meters bytes per tenant. Crawler and browser hosts sit in public subnets with no NAT on the bulk path, and customer downloads go through CloudFront — [aws_platform_networking_billing_policy.md §2, §3, §9](aws_platform_networking_billing_policy.md).
- The CloudFront flat-rate Business plan costs $200/month for 125M requests and 50 TB with no overage. WebSocket use under these plans is unverified — [aws_platform_networking_billing_policy.md §2](aws_platform_networking_billing_policy.md), from [CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/).
- S3 Standard costs $0.023/GB-month, $0.005 per 1,000 PUTs and $0.0004 per 1,000 GETs — [aws_price_sheet.md §4](aws_price_sheet.md).
- Log and metric prices:
  - CloudWatch logs: $0.50/GB Standard, $0.25/GB Infrequent Access ([aws_price_sheet.md §7](aws_price_sheet.md)).
  - Better Stack: $0.15/GB to ingest in the US ($0.10 EU), plus $0.08 ($0.05) per GB-month of retention.
  - Grafana Cloud Pro: $19/month plus $6.50 per 1,000 series.
  - Sentry Team: $26/month.
  - These three are in [aws_platform_networking_billing_policy.md §7](aws_platform_networking_billing_policy.md), from [Better Stack](https://betterstack.com/pricing), [Grafana](https://grafana.com/pricing/) and [Sentry](https://sentry.io/pricing/).

**Workload resource profiles**
- Vendor sizing works out to 0.16–0.4 vCPU and 0.32–0.8 GB per concurrent Session. In lab runs, full Chrome held 1.1–1.4 GB, while chrome-headless-shell idles at about 47 MiB plus about 16 MiB per tab. Kernel defaults to 1 GB headless and 8 GB headful — [workload_resource_profiles.md §1–§2](workload_resource_profiles.md), from [Browserless best practices](https://docs.browserless.io/enterprise/docker/best-practices); [pricing_hosted_browsers_and_agents.md §1](pricing_hosted_browsers_and_agents.md), from [Kernel limits](https://kernel.sh/docs/browsers/concurrency-and-limits.md).
- The notes disagree on browser memory. The provider note defines a lean browser "L" as 0.5 vCPU + 1 GB and a rich "R" as 1 vCPU + 2 GB ([provider_alternatives_vs_aws.md §6](provider_alternatives_vs_aws.md)). The runtime note says "plan for 1.6–5 GB per browser", and AWS's AgentCore example bills 2 vCPU and 4 GB ([aws_runtime_compute_isolation.md Q3](aws_runtime_compute_isolation.md)).
- Steel ran one Session per machine at "about 2 GB and 2 vCPU each" (2024). Browser Use places "each browser on both hyperthreads of one core" — [competitor_cloud_architectures.md §2](competitor_cloud_architectures.md), from the [Steel launch on HN](https://news.ycombinator.com/item?id=42245573) and [Browser Use's Firecracker post](https://browser-use.com/posts/firecracker-browser-infra).
- CPU per page:
  - Chrome on light JS pages: about 116–127 ms ([workload_resource_profiles.md §1](workload_resource_profiles.md), from [lightpanda-io/demo BENCHMARKS.md](https://github.com/lightpanda-io/demo/blob/main/BENCHMARKS.md)).
  - Lightpanda: 6–11 ms ([workload_resource_profiles.md §1](workload_resource_profiles.md), from [lightpanda-io/demo BENCHMARKS.md](https://github.com/lightpanda-io/demo/blob/main/BENCHMARKS.md)).
  - Rust HTTP fetch: about 0.19–0.31 ms ([workload_resource_profiles.md §3](workload_resource_profiles.md)).
  - rs-trafilatura main-content extraction: 14.46 ms at p50 ([workload_resource_profiles.md §2](workload_resource_profiles.md)).
- Bytes per 1,000 pages: about 0.014–0.033 GB for HTML only, 2.2–2.9 GB for a full cold load, and 0.73–0.80 GB with images, fonts and media blocked — [workload_resource_profiles.md §4](workload_resource_profiles.md), from [Web Almanac 2025](https://almanac.httparchive.org/en/2025/page-weight). ZenRows prices residential bandwidth as "roughly 1,000 hard-protected pages" per GB — [input_costs_proxies_captchas_llm.md §2](input_costs_proxies_captchas_llm.md), from [ZenRows pricing](https://www.zenrows.com/pricing).
- Rendered pages carry only 16–18% more words than raw HTML at the median. No primary source measures the share of pages that need JavaScript, nor a captcha encounter rate — [workload_resource_profiles.md §5](workload_resource_profiles.md), from [Web Almanac 2025 SEO](https://almanac.httparchive.org/en/2025/seo).
- Learned HTTP or API routes ran a page at 0.4–0.8% of the browser's time in the lab. Lightpanda used 8–20x less CPU and 15–65x less memory on compatible pages, but fails shadow DOM — [workload_resource_profiles.md §2](workload_resource_profiles.md); [rnd/lab/reports/light-engine-paired-2026-10-09.md](../../../lab/reports/light-engine-paired-2026-10-09.md).
- A warm pool of discarded containers cut acquisition from 180 ms to about 0 ms, at about 39 MiB idle per container. Reusing a container without reset leaked a tenant's cookie 30 times out of 30 — [rnd/lab/reports/sinks-topology-2026-10-09.md](../../../lab/reports/sinks-topology-2026-10-09.md).
- Steel claims "35M+ sessions served, 1.5M+ browser hours" (about 2.6 minutes per Session, by division) — [competitor_cloud_architectures.md §5](competitor_cloud_architectures.md), from [steel.dev](https://steel.dev).
- Browser Use says moving from Unikraft to Firecracker on regular EC2 cut its cost from $0.06 to $0.02 per browser-hour (a vendor claim with no stated method). The note concludes: "Idle capacity is the dominant cost driver in browser fleets" — [competitor_cloud_architectures.md §5](competitor_cloud_architectures.md), from [Browser Use's Firecracker post](https://browser-use.com/posts/firecracker-browser-infra).
- The Goodfetch-built Chromium's fingerprint seed is a launch flag. A pool of snapshotted, already-launched browsers would therefore clone the seed, so Chromium must start after restore — [aws_runtime_compute_isolation.md Q2](aws_runtime_compute_isolation.md); [ADR 0040](../../../../docs/adr/0040-one-resource-governor-per-engine-sized-from-measured-load.md).

**Agent tokens and inputs**
- A 10-step task with 1 browser-minute and 10 MB of residential traffic uses about 100k input and 10k output tokens on the light profile, and three times that on the heavy one — [pricing_hosted_browsers_and_agents.md §3, Table N5](pricing_hosted_browsers_and_agents.md), from [Notte's worked example](https://docs.notte.cc/intro/pricing).
- In the lab, compiled replay used 3 to 6 times fewer tokens with zero false successes — [workload_resource_profiles.md §6](workload_resource_profiles.md); [rnd/lab/reports/ai-real-models-2026-10-09.md](../../../lab/reports/ai-real-models-2026-10-09.md).
- List prices per million input/output tokens — [input_costs_proxies_captchas_llm.md §5](input_costs_proxies_captchas_llm.md), from [Anthropic](https://platform.claude.com/docs/en/about-claude/pricing), [OpenAI](https://developers.openai.com/api/docs/pricing) and [Gemini](https://ai.google.dev/gemini-api/docs/pricing) pricing:
  - Claude Haiku 5.5 $0.10/$0.50 (cache read $0.01); Sonnet 5.5 $2/$10 (cache $0.10); Opus 5.5 $4/$20 (cache $0.20); Fable 5.1 $10/$50.
  - GPT-6 Luna $0.10/$0.50; GPT-6.1 Sol $2/$10; GPT-6 Astra $10/$50.
  - Gemini 3.1 Flash-Lite $0.25/$1.50; Gemini 3.8 Flash $0.75/$3.75, doubling on 2027-01-01.
- Bedrock: Claude's global endpoint costs the same as Anthropic's list, and regional endpoints cost 10% more. The Claude 5.5 models have no Bedrock batch rows, and Bedrock's Claude endpoint lacks Message Batches and the 2026-08 browser toolset — [aws_price_sheet.md §8](aws_price_sheet.md); [input_costs_proxies_captchas_llm.md §7](input_costs_proxies_captchas_llm.md), from [Claude in Amazon Bedrock](https://platform.claude.com/docs/en/build-with-claude/claude-in-amazon-bedrock).
- Residential proxies cost $0.49–2.60/GB on self-serve plans at about 1 TB a month. The defensible COGS is "$2.00–2.50/GB for premium pools ... $0.70–1.00/GB for budget pools", and enterprise floors are $0.35–0.85 (SOAX). Against protected targets the median success rate is 74.43% — [input_costs_proxies_captchas_llm.md §1](input_costs_proxies_captchas_llm.md), from [Proxyway 2026](https://proxyway.com/research/proxy-market-research-2026).
- Token captcha solving costs about $0.60–1.30 per 1,000 for reCAPTCHA v2/v3 and Turnstile, and $1.20–1.30 for Cloudflare challenge pages. Third-party tests found 86–100% success with 6–93 s average latency — [input_costs_proxies_captchas_llm.md §4](input_costs_proxies_captchas_llm.md), from [HasData](https://hasdata.com/blog/captcha-solving).
- SERP APIs cost $0.30–1.00 per 1,000 queries at Serper — [input_costs_proxies_captchas_llm.md §8](input_costs_proxies_captchas_llm.md), from [Serper](https://serper.dev/).

### Inferences
Model outputs of `unit_economics_model.py` (sections `units` and `disagreements`).

**Assumptions I had to choose.** Each is varied in question 6.
- **Utilization:** 65% of provisioned browser slots billed; no note publishes fleet utilization.
- **Packing:** 85% of node vCPU and RAM usable, as in the provider note.
- **Warm pool:** 10% pre-booted sandboxes, sized at 39 MiB per container or 160 MiB per microVM, with Chromium started after claim.
- **Session length:** 3 minutes on average.
- **Traffic per browser-hour:** live-view egress 0.30 GB/h for headful Sessions, 0.05 GB/h otherwise (the provider note's two cases).
- **Logs:** 10 MB per browser-hour at Better Stack prices.
- **Profiles:** lean headless 0.5 vCPU / 1 GB; rich headless 1 vCPU / 2 GB; rich headful 2 vCPU / 4 GB (AgentCore's example); lean headful 0.75 vCPU / 1.5 GB (+50%, my assumption).
- **Where the notes disagree:**
  - **Spot:** I use the Advisor 30-day savings, as the price sheet advises. The snapshot, the runtime note's 60% and the provider note's 58% are shown in question 6. They move rich-headless Spot cost by -0.3% to +15%.
  - **Browser memory:** I use the provider note's L/R sizes, which sit inside the evidence range. Doubling memory (the runtime note's 1.6–5 GB) raises rich headless by 16%; Kernel's 8 GB headful raises it by 61%.
  - **Egress:** per-unit tables use the first tier, $0.09/GB. The fleet section uses the tiered rate, which falls to about $0.052/GB at 2,190 TB a month.
  - **Fargate Spot:** I use the advertised maximum of 70%.
  - **NLCU:** I take AWS's published dimension of 1 GB/h per NLCU. It is in no note, and the cost is under $0.002/h.

**(a) Hosted browser-hour, $ per billed hour, us-east-1** (eu-west-1 on the last line)

| deployment | lean headless | lean headful | rich headless | rich headful |
|---|---|---|---|---|
| EKS+Karpenter Graviton, On-Demand | 0.0404 | 0.0812 | 0.0735 | 0.1638 |
| same, 1-yr Compute SP | 0.0310 | 0.0671 | 0.0547 | 0.1264 |
| same, 3-yr EC2 Instance SP | 0.0218 | 0.0533 | 0.0363 | 0.0896 |
| same, Spot (batch Sessions only) | 0.0194 | 0.0498 | 0.0316 | 0.0820 |
| **interactive mix: 60% 1-yr SP + 40% On-Demand (recommended)** | **0.0348** | **0.0727** | **0.0622** | **0.1413** |
| Firecracker per Session, Graviton `.metal` On-Demand | 0.0420 | 0.0829 | 0.0750 | 0.1692 |
| Firecracker per Session, Graviton `.metal` Spot | 0.0175 | 0.0469 | 0.0277 | 0.0722 |
| Firecracker per Session, Intel nested virtualization On-Demand / Spot | 0.0558 / 0.0272 | 0.1041 / 0.0612 | 0.1039 / 0.0468 | 0.2243 / 0.1104 |
| Kata, Intel nested virtualization On-Demand | 0.0779 | 0.1244 | 0.1293 | 0.2418 |
| Fargate ARM On-Demand / Spot | 0.0370 / 0.0204 | 0.0731 / 0.0482 | 0.0607 / 0.0275 | 0.1323 / 0.0660 |
| Lambda MicroVMs (Arm) | 0.1009 | 0.1704 | 0.1949 | 0.4054 |
| AgentCore Browser v1 (AWS's Chrome; reference only) | 0.0253 | 0.0570 | 0.0437 | 0.1030 |
| eu-west-1, recommended interactive mix | 0.0369 | 0.0763 | 0.0672 | 0.1522 |

**Rich headful, recommended mix, us-east-1, by component ($ per billed hour):**
- compute 0.1097;
- live-view egress 0.0270 (0.30 GB at $0.09);
- gateway and NLB 0.0021 (most of it the NLCU bytes dimension);
- warm pool 0.0001;
- logs and recordings 0.0024;
- control-plane allocation 0.0016 (one us-east-1 control plane plus staging, $3,537/month, a third of it charged to browsers over 1,000 concurrent Sessions).
- Fully loaded total: $0.1430.

**Utilization sweep** (recommended mix, us-east-1):

| utilization | lean headless | rich headless | rich headful |
|---|---|---|---|
| 30% | $0.0668 | $0.1262 | $0.2694 |
| 45% | $0.0470 | $0.0866 | $0.1901 |
| 65% | $0.0348 | $0.0622 | $0.1413 |
| 80% | $0.0296 | $0.0519 | $0.1208 |
| 90% | $0.0272 | $0.0470 | $0.1109 |

Readings:
- **Purchase option.** Commitments and Spot cut the compute line roughly in half: rich headless costs $0.0735 on-demand, $0.0363 on a 3-yr Instance SP and $0.0277 on `.metal` Spot. Interactive Sessions cannot migrate, so only batch Sessions should use Spot ([aws_runtime_compute_isolation.md Q4](aws_runtime_compute_isolation.md)).
- **Isolation option.** Firecracker on Graviton `.metal` costs the same as plain EC2 per slot; the VMM overhead is negligible. Firecracker on Intel nested virtualization costs 26–69% more than on Graviton `.metal`, from Intel's higher per-vCPU price and the assumed 10% CPU loss. Kata's 250m CPU per pod makes it the dearest self-run option.
- **Fargate ARM.** On-Demand rich headless costs $0.0607. It bills per task-second, so it beats the EKS interactive mix below about 67% fleet utilization and EKS On-Demand below about 81%.
- **Lambda MicroVMs.** Per running hour they cost 2.3x the EKS interactive mix and up to 5x Spot, plus a snapshot read at every Session start: $0.0031 per 2 GB start, or about $0.06/h at 3-minute Sessions. They fit spiky or zero-fleet use only.
- **AgentCore.** It is the cheapest managed option for idle-heavy agent Sessions (20% CPU busy assumed). It runs AWS's Chrome, so it cannot carry the Goodfetch stealth Chromium (unverified, per [aws_runtime_compute_isolation.md Q6](aws_runtime_compute_isolation.md)).

**(b) Crawl and extraction API, $ per 1,000 pages**

| class | recommended (EKS Graviton, 80% Spot), us-east-1 | eu-west-1 | Fargate ARM Spot | Lambda Arm | largest component |
|---|---|---|---|---|---|
| static HTML over HTTP | 0.0033 | 0.0031 | 0.0031 | 0.0098 | result egress 0.0013 (40%) |
| JS-rendered, resources blocked | 0.0398 | 0.0398 | 0.0391 | 0.0533 | datacenter proxy bytes 0.0362 (91%) |
| protected (residential + browser) | 1.59 | 1.59 | 1.59 | 1.66 | residential bytes 1.50 (94%); captcha 0.078; compute 0.0035 |

Inputs behind the table:
- **Static.** 19.8 ms of CPU per page (fetch, a 5 ms TLS/decompression allowance, extraction); 20% of HTML bytes through datacenter proxies at $0.15/GB; results 15 KB compressed per page, kept 1 month in S3 and sent once to the customer.
- **JS route mix.** 25% learned HTTP or API route, 15% Lightpanda, 60% Chrome at 250 ms of CPU per page; half of the bytes through datacenter proxies.
- **Protected.** 40% HTTP with the Session's browser cookies (one browser clearance per 20 pages), 60% full browser at 1 MB per load; 75% success with failed attempts at half the bytes; 10% captcha rate per browser load at $1.20 per 1,000 solves and 95% solve success; residential at $2.00/GB.

Protected-class sensitivity, $ per 1,000 pages:

| HTTP-route share \ residential $/GB | 0.70 | 1.00 | 2.00 | 2.50 |
|---|---|---|---|---|
| 0% (browser on every page) | 0.96 | 1.31 | 2.48 | 3.06 |
| 40% (base) | 0.62 | 0.84 | 1.59 | 1.97 |
| 70% | 0.36 | 0.49 | 0.93 | 1.15 |
| 90% | 0.19 | 0.26 | 0.49 | 0.60 |

- **Captcha rate.** At 2%, 10%, 25% and 50% per browser load the class costs $1.53, $1.59, $1.72 and $1.94. Solving plus browser waiting is $0.02 to $0.43 of that.
- **Bytes per browser load.** At 0.5, 1.0, 2.0 and 2.9 GB per 1,000 loads the class costs $0.87, $1.59, $3.04 and $4.34.

**(c) Compute Unit (1 GB-hour, 0.25 vCPU) in an isolated sandbox, $/CU, us-east-1 / eu-west-1**
- **Firecracker on Graviton `.metal` (m7g.metal), recommended:**
  - On-Demand: $0.0198 / $0.0214
  - 3-yr Instance SP: $0.0102 / $0.0107
  - Spot: $0.0071 / $0.0070
  - 80% Spot mix (**recommended**): **$0.0096 / $0.0099**
- **Firecracker on Intel nested virtualization (m8i):** On-Demand $0.0270 / $0.0294; Spot $0.0119 / $0.0160.
- **Kata on Intel nested virtualization:** On-Demand $0.0381 / $0.0418; Spot $0.0162 / $0.0224.
- **Fargate ARM (0.25 vCPU, 1 GB task):** On-Demand $0.0177 / $0.0173; Spot $0.0087 / $0.0083. Both include a public IPv4 address shared over a 2 GB run.
- **Managed runtimes:**
  - Lambda MicroVMs: $0.0715 / $0.0748, because the 2:1 ratio forces 0.5 vCPU per GB.
  - Lambda Arm (runs under 15 minutes): $0.0503 / $0.0499.
  - AgentCore Runtime v1 at 50% CPU busy: $0.0229 / $0.0225.

Each figure carries 70% host utilization, a 2 GB average run with 45 MB of microVM overhead, and small result-egress, storage and log costs.

**(d) AI agent task, about 10 steps, light profile, $ per task (us-east-1)**

| model (class) | agent on every run | compiled replay | Bedrock regional (+10%) | eu-west-1 agent / replay | heavy profile | model's share of agent cost |
|---|---|---|---|---|---|---|
| Claude Haiku 5.5 (cheap) | 0.0335 | 0.0237 | 0.0346 | 0.0347 / 0.0240 | 0.0550 | 32% |
| GPT-6 Luna (cheap) | 0.0335 | 0.0237 | not on Bedrock | 0.0337 / 0.0238 | 0.0550 | 32% |
| Gemini 3.1 Flash-Lite (cheap) | 0.0522 | 0.0279 | not on Bedrock | 0.0523 / 0.0279 | 0.1109 | 56% |
| gpt-oss-120b on Bedrock (cheap) | 0.0442 | 0.0261 | 0.0442 | 0.0482 / 0.0270 | 0.0869 | 48% |
| Claude Sonnet 5.5 (mid) | 0.2328 | 0.0680 | 0.2538 | 0.2539 / 0.0727 | 0.6528 | 90% |
| GPT-6.1 Sol (mid) | 0.2328 | 0.0680 | not on Bedrock | 0.2329 / 0.0680 | 0.6528 | 90% |
| Gemini 3.8 Flash, 2026 / 2027 price (mid) | 0.1034 / 0.1840 | 0.0393 / 0.0572 | not on Bedrock | same | 0.2647 / 0.5065 | 78% / 88% |
| Claude Opus 5.5 (frontier) | 0.4428 | 0.1147 | 0.4848 | 0.4849 / 0.1240 | 1.28 | 95% |
| GPT-6 Astra (frontier) | 1.10 | 0.2602 | 1.21 | 1.10 / 0.2603 | 3.25 | 98% |
| Claude Fable 5.1 (frontier) | 1.06 | 0.2519 | 1.16 | 1.16 / 0.2750 | 3.14 | 98% |

How the task cost is built:
- **Tokens.** 50% of input tokens are cache reads, plus a 10% cache-write premium.
- **Replay.** It divides model tokens by 4.5 (the lab's 3–6x) and cuts browser time from 3 minutes to 0.5.
- **Other inputs.** 10 MB of residential traffic costs $0.020, which is 60% of a cheap-model task. One SERP query costs $0.001.
- **Endpoints.** The eu-west-1 column routes Claude through Bedrock's regional endpoint for EU data routing (+10%). Bedrock's lack of batch for the 5.5 models and of the browser toolset favours the direct API, or Claude Platform on AWS, for agents.

**(e) Resale units, COGS**
- **Residential GB:** $2.00 in both Regions.
- **1,000 captcha solves:** $1.26 (at 95% solve success).
- **1M tokens at a 90/10 input/output split, no cache:**
  - Claude Haiku 5.5: $0.14 (eu-west-1, Bedrock regional: $0.154)
  - Claude Sonnet 5.5: $2.80 ($3.08)
  - Claude Opus 5.5: $5.60 ($6.16)

### Gaps
- No Linux PSS or cgroup measurement exists yet of the Goodfetch stealth Chromium's memory and CPU per Session, headless or headful. The profiles are therefore anchored on vendor sizing and macOS lab footprints ([workload_resource_profiles.md §1–§2 Gaps](workload_resource_profiles.md)).
- No source publishes fleet utilization, session-duration distributions, the share of pages needing JavaScript, captcha encounter rates or per-session log volume. All of these are my assumptions.
- The NLB/ALB capacity-unit dimensions, GuardDuty/Security Hub/Config/WAF prices and the Aurora minimum ACU are not in the notes. The NLCU dimension and a $600/month security lump sum are my assumptions.
- No benchmark exists of nested-virtualization overhead on Chromium or of Firecracker restore time for a 2–4 GB guest ([open_source_cloud_layers.md §3 Gaps](open_source_cloud_layers.md)).
- Whether AgentCore Browser accepts a custom Chromium build, and the launch status of Lambda MicroVMs, are unverified ([aws_runtime_compute_isolation.md Q2, Q6 Gaps](aws_runtime_compute_isolation.md)).

## 2. What does the market pay per unit, and what gross margin does Goodfetch Cloud earn at the median, 20–30% below it, and at a premium?

### Takeaway
At volume (largest self-serve) prices on the recommended AWS deployment, the units fall into three groups:
- **Large margins:** static and JS pages (93–99%), the Compute Unit (93%), replayed agent tasks (79–85%), captcha resale (84%) and residential-proxy resale (72%).
- **Thin or negative:** the bare browser-hour (20% for the profile mix: 56% lean headless, -77% rich headful with live view), protected pages (5%), and resold LLM tokens (2.4% at the median markup, before payment fees).
- **Effect of a 25% price cut:** browser-hours and protected pages go negative, and agent-every-run tasks on mid and frontier models fall to 21–23%. The other lines stay at 61% or more, except tokens, which are passed through at cost.

Goodfetch's structural advantages each raise one of these lines:
- **HTTP-first routing:** protected pages reach 44% gross margin at a 70% HTTP share.
- **Lightpanda:** cheaper JS pages.
- **Compiled replay:** mid and frontier agent tasks rise from 41–43% to 83–85%.
- **Rust density:** large Compute Unit margins.

### Cited Findings
- **Hosted browser-hour.** At the margin, entry plans charge $0.00–0.456 per hour. Largest self-serve plans: Browserbase $0.10, Steel $0.08, Browser Use $0.02, Kernel headless $0.06, Browserless $0.18, Cloudflare $0.09, Hyperbrowser $0.10, ZenRows Scale $0.020, Notte $0.00. Kernel bills headful at $0.48/h. Browser Use and Anchor bill direct or own-proxy traffic at $0.20/GB — [pricing_hosted_browsers_and_agents.md §1, §3 Table N1/N2](pricing_hosted_browsers_and_agents.md), from [Browserbase](https://docs.browserbase.com/account/billing/plans.md), [Steel](https://docs.steel.dev/overview/pricinglimits), [Kernel](https://www.kernel.sh/docs/info/pricing) and [Browser Use](https://browser-use.com/pricing). The note concludes "Browser time is in a price war" and that "Goodfetch Cloud is unlikely to win on the browser-second price alone" ([pricing_hosted_browsers_and_agents.md §4](pricing_hosted_browsers_and_agents.md)).
- **Crawl pages, at the largest self-serve plan** — [pricing_scraping_apis_and_actor_platforms.md §3](pricing_scraping_apis_and_actor_platforms.md), from [Firecrawl](https://www.firecrawl.dev/pricing.md), [ScrapingBee](https://www.scrapingbee.com/pricing/), [Bright Data](https://brightdata.com/pricing/web-unlocker) and [Zyte](https://www.zyte.com/pricing/):
  - Static: $0.05–0.10 per 1,000 with option-credit vendors, $0.21–1.30 with flat-rate vendors.
  - JS: $0.26–0.96 and $0.23–1.30.
  - Anti-bot: $1.20–2.73, against $0.40 (Olostep) and $0.60–0.75 (Firecrawl) at vendors that bundle escalation, and up to $3.84–7.68 at Zyte's hardest tiers.
  - Entry plans cost 1.15–6.3x the largest plan per unit.
- **Compute Unit.** Apify charges $0.20 (Free/Starter), $0.16 (Scale) and $0.13 (Business) — [pricing_scraping_apis_and_actor_platforms.md §1](pricing_scraping_apis_and_actor_platforms.md), from [apify.com/pricing](https://apify.com/pricing). Sandbox vendors price compute far lower: E2B at $0.000014 per vCPU-second and $0.0000045 per GiB-second, Daytona at $0.0504 per vCPU-hour and $0.0162 per GiB-hour — [pricing_hosted_browsers_and_agents.md §1](pricing_hosted_browsers_and_agents.md), from [E2B](https://e2b.dev/pricing) and [Daytona](https://www.daytona.io/pricing).
- **Agent task (10 steps, light profile)** — [pricing_hosted_browsers_and_agents.md §3 Table N5](pricing_hosted_browsers_and_agents.md):
  - Token pass-through offers: $0.023–0.137 with Haiku 5.5; $0.31–0.42 with Sonnet 5; $0.72–0.77 with Opus 5.5.
  - Flat per-step offers: TinyFish $0.16, HyperAgent $0.30, Skyvern about $0.24, Anchor Standard $0.59.
  - Corpus anchor: a replayed JSON plan cuts a task "$0.50–3.00 → $0.01–0.05" (vendor claim) — [rnd/research/market/pricing-signals.md §1](../../market/pricing-signals.md).
- **Residential proxy resale.** Apify $8 → $7/GB, Browserbase $12 → $10, Steel $10 → $6, Browser Use $5, Bright Data Browser API $8 → $7.03, Anchor $8, Notte $10. ZenRows' credits work out to $10.56 → $1.67/GB and Scrapeless charges $1.80. That is "about 3–6x a premium wholesale-scale rate" — [input_costs_proxies_captchas_llm.md §2](input_costs_proxies_captchas_llm.md), from [Apify](https://apify.com/pricing) and [Browserbase](https://www.browserbase.com/pricing).
- **Captcha resale.** Steel charges $3 → $1 per 1,000 solves. Browserless works out to $20 → $15 at overage and $12.50 → $7 at its included-unit rate. Bright Data, Kernel, Notte, Scrapeless and Browserbase's paid plans bundle solving — [input_costs_proxies_captchas_llm.md §4](input_costs_proxies_captchas_llm.md); [pricing_hosted_browsers_and_agents.md §1](pricing_hosted_browsers_and_agents.md).
- **LLM token resale.** Markups are 0% at Notte, Browserbase's Model Gateway and Hugging Face; about 5% at Hyperbrowser; a 5.5% credit fee at OpenRouter; 20% at Browser Use — [pricing_hosted_browsers_and_agents.md §2](pricing_hosted_browsers_and_agents.md); [business_models_and_financials.md §3](business_models_and_financials.md); [input_costs_proxies_captchas_llm.md §6](input_costs_proxies_captchas_llm.md), from [OpenRouter's FAQ](https://openrouter.ai/docs/faq).
- **What buyers reward and punish.** They punish credit multipliers, billing for failures, expiring credits and missing caps. They accept success-only billing, pay-per-event (97.1% of Apify Store Actors) and per-result pricing. "No vendor was found that bills per semantically verified result" — [business_models_and_financials.md §6](business_models_and_financials.md); [rnd/research/market/pricing-signals.md §2](../../market/pricing-signals.md).

### Inferences
Model outputs (sections `market` and `margins`).

**Market prices used by the model** (medians across the vendors above):

| unit | entry-plan median | largest self-serve median | range |
|---|---|---|---|
| browser-hour | $0.10 | $0.08 | $0–0.456 |
| 1,000 static pages | $0.49 | $0.22 | $0.05–3.80 |
| 1,000 JS pages | $1.20 | $0.60 | $0.23–4.90 |
| 1,000 protected pages | $3.80 | $1.67 | $0.40–23.75 |
| Compute Unit | $0.20 | $0.13 | $0.13–0.20 |
| agent task: cheap / mid / frontier | $0.116 / $0.405 / $0.745 | same | $0.023–0.77 |
| residential GB | $9.00 | $7.02 | $1.67–12 |
| 1,000 captcha solves | $11.50 | $8.00 | $1–20 |
| LLM markup over list | 2.5% | — | 0–20% |

The captcha market is thin: only two vendors price solves separately.

**Unit gross margin, us-east-1, recommended deployment.** Price positions apply to the largest-plan median; payment fees, payouts and fixed costs are excluded here and deducted in question 3.

| unit | COGS | GM at entry median | GM at median | GM at -25% | GM at +25% |
|---|---|---|---|---|---|
| browser-hour, profile mix (50% lean headless, 30% rich headless, 20% rich headful) | $0.0643 | 35.7% | 19.6% | -7.2% | 35.7% |
| browser-hour lean headless | $0.0348 | 65.2% | 56.5% | 42.0% | 65.2% |
| browser-hour rich headless | $0.0622 | 37.8% | 22.2% | -3.7% | 37.8% |
| browser-hour rich headful with live view | $0.1413 | -41.3% | -76.7% | -135.6% | -41.3% |
| rich headful priced like Kernel ($0.48) | $0.1413 | 70.6% | 70.6% | 60.7% | 76.4% |
| 1,000 static pages | $0.0033 | 99.3% | 98.5% | 98.0% | 98.8% |
| 1,000 JS pages | $0.0398 | 96.7% | 93.4% | 91.1% | 94.7% |
| 1,000 protected pages | $1.59 | 58.1% | 4.6% | -27.3% | 23.6% |
| Compute Unit | $0.0096 | 95.2% | 92.6% | 90.1% | 94.1% |
| agent task cheap: agent / replay | $0.0335 / $0.0237 | 71.1% / 79.5% | 71.1% / 79.5% | 61.4% / 72.7% | 76.9% / 83.6% |
| agent task mid: agent / replay | $0.2328 / $0.0680 | 42.5% / 83.2% | same | 23.4% / 77.6% | 54.0% / 86.6% |
| agent task frontier: agent / replay | $0.4428 / $0.1147 | 40.6% / 84.6% | same | 20.8% / 79.5% | 52.5% / 87.7% |
| residential GB | $2.00 | 77.8% | 71.5% | 62.0% | 77.2% |
| 1,000 captcha solves | $1.26 | 89.0% | 84.2% | 78.9% | 87.4% |
| 1M LLM tokens resold | list price | 2.4% | 2.4% (2.5% markup) | 0.0% (pass-through) | 16.7% (20% markup) |

eu-west-1 margins differ in three places:
- browser-hours are 2.6–13.5 points lower (profile mix 13.7% at median);
- agent tasks are 0.2–5.7 points lower, mostly from Claude's +10% Bedrock regional price;
- every other line is within 0.3 points.

For LLM tokens, pricing below list would lose money on every token. The positions are therefore pass-through (0%), median (2.5%) and Browser Use's 20%.

**How each business makes money** (none is dropped):
- **Hosted browsers.** The bare hour at $0.08 earns 19.6% on AWS. The money is in what rides on the hour:
  - residential bytes (71.5%) and captcha (84%);
  - agent tasks (66–85%);
  - stealth and identity tiers, which competitors gate to enterprise plans ([pricing_hosted_browsers_and_agents.md §1 Inferences](pricing_hosted_browsers_and_agents.md));
  - metered traffic: billing non-proxied traffic at Browser Use's $0.20/GB lifts the browser line from 19.6% to 67.8%.

  Headful live-view Sessions need their own price (Kernel's $0.48/h gives 70.6%) or metered egress. At the shared median they lose 77%.
- **Crawl and extraction API.** Static and JS pages earn 93–99% even at the largest-plan price. Protected pages are the volume trap: 4.6% at the $1.67 median, or 58% at entry prices. HTTP-first escalation is the lever: at a 70% HTTP share the cost is $0.93 and the margin 44%; at 90% it is 71%. A residential contract near $1.00/GB is the other lever.
- **Actor platform and marketplace.** The Compute Unit earns 93% at Apify's Business price. Sandbox vendors at about $0.03 per CU-equivalent cap how far that price can be defended. The 20% commission is pure margin on event revenue. Recipes also pull proxies and Compute Units.
- **AI agents.** On mid and frontier models, an agent loop on every run earns about 41–43% at the market price. Compiled replay earns 83–85% at the same price, and every repair or first run is priced like the market.
- **Resale of operations:**
  - Proxies earn 72% at the median $7/GB and still 20% at $2.50/GB. Scrapeless ($1.80) and ZenRows' top tier ($1.67) price below this $2.00 COGS.
  - Captcha resale is high-margin but thin.
  - LLM tokens are a pass-through. They need a markup of at least about 4% to cover payment fees, and 10–20% to matter.

**Structural advantages and disadvantages, quantified:**
- **HTTP-first escalation.** JS pages cost $0.040 per 1,000 with 25% HTTP and 15% Lightpanda routes. A Chrome-only JS page would carry about 0.77 GB of proxied bytes and 250 ms of CPU.
- **Rust density.** Static compute is $0.0003 per 1,000 pages; the engine needs about 11 MiB, against Fargate's 0.5 GB floor ([aws_runtime_compute_isolation.md Q1](aws_runtime_compute_isolation.md)).
- **Compiled replay.** It cuts token cost 4.5x and browser time about 6x.
- **Disadvantages:**
  - AWS internet egress on live view: $0.027/h, 19% of the rich-headful cost.
  - The stealth Chromium must start after restore, adding about 0.1–0.5 s to Session start ([workload_resource_profiles.md §1](workload_resource_profiles.md)).
  - Arm hosts may not serve x86 personas ([aws_runtime_compute_isolation.md Q3](aws_runtime_compute_isolation.md)).
  - Proxies are resold, so margins follow upstream prices.

### Gaps
- Market medians weight every vendor equally. No source gives revenue-weighted prices, or the share of revenue sold at entry versus volume prices.
- Captcha resale prices exist for only two vendors. Agent-task prices come from one normalized table built on assumed token counts ([pricing_hosted_browsers_and_agents.md §3 Gaps](pricing_hosted_browsers_and_agents.md)).
- Willingness to pay a premium for verified results is unmeasured ([rnd/research/market/pricing-signals.md §4](../../market/pricing-signals.md)), so the +25% position is an assumption.

## 3. What is the blended gross margin for plausible revenue mixes after payment fees, creator payouts, free tier, AWS Support, observability, an abuse/KYC desk and fixed platform minimums, and how does it compare with public benchmarks?

### Takeaway
Before overheads, the four revenue mixes earn 45–59% gross margin at market-median prices. After payment fees, creator payouts, the free tier, AWS Support, observability, a trust and safety desk and fixed platform minimums, they earn 14–29% at $100k of monthly revenue and 31–46% at $1M. A merchant of record costs another 1.0–3.4 points. The blend sits between Twilio (49%, pass-through costs in COGS) and DigitalOcean (about 60%) before overheads, and below every public infrastructure benchmark after them.

### Cited Findings
- **Stripe, EU direct (France).** Standard EEA cards cost 1.5% + €0.25, premium EEA 2.8% + €0.25, UK 2.5% + €0.25, international 3.15% + €0.25 (plus 2% with conversion). Billing adds 0.7%, Tax 0.5% and Managed Payments (merchant of record) 3.5% on top — [business_models_and_financials.md §7](business_models_and_financials.md), from [Stripe pricing (FR)](https://stripe.com/en-fr/pricing).
- **Other merchants of record.** Paddle charges 5% + 50¢ and Polar 3.4–5% + 30–50¢. "Paddle's policy prohibits ... digital marketplaces"; Stripe Managed Payments requires selling "not through a platform or marketplace"; Paddle restricts "VPN and Proxies" — [business_models_and_financials.md §5, §7](business_models_and_financials.md), from [Paddle](https://www.paddle.com/help/start/intro-to-paddle/what-am-i-not-allowed-to-sell-on-paddle) and [Stripe eligibility](https://docs.stripe.com/payments/managed-payments/eligibility).
- **Stripe Connect.** €2 per monthly active account plus 0.25% + €0.10 per payout — [business_models_and_financials.md §5](business_models_and_financials.md), from [Stripe Connect pricing](https://stripe.com/en-fr/connect/pricing).
- **Apify's marketplace.** Developer profit = 0.8 × revenue − platform costs, and Apify covers free-plan users' platform costs. The Store paid out "$1.6M ... last month" across 4,500 community developers, against Apify's 27,000 paying customers — [business_models_and_financials.md §2, §5](business_models_and_financials.md), from [Apify docs](https://docs.apify.com/actors/publishing/monetize/pricing-and-costs.md) and [Apify partners](https://apify.com/partners/actor-developers).
- **Free tiers and abuse.** Allowances run $1–10 a month equivalent (Apify $5, Kernel $5, Exa $10). Heroku and PlanetScale removed free tiers citing abuse and profitability — [business_models_and_financials.md §4](business_models_and_financials.md), from [Heroku](https://www.heroku.com/blog/next-chapter) and [PlanetScale](https://planetscale.com/blog/planetscale-forever).
- **AWS Support.** Business Support+ costs the greater of $29 or 9% of AWS charges up to $10K, 7% to $80K, 5% to $250K and 3% above. Enterprise costs the greater of $5,000 or 10/7/5/3%. AWS Marketplace SaaS listings pay 3% — [aws_price_sheet.md §9](aws_price_sheet.md), from [AWS Support pricing](https://aws.amazon.com/premiumsupport/pricing/).
- **Abuse desk and KYC.** AWS may block resources if an abuse notice goes unanswered for 24 hours. The note recommends KYC tiers for proxy tiers and high concurrency. No note gives a staffing or cost figure — [aws_platform_networking_billing_policy.md §5](aws_platform_networking_billing_policy.md), from [AWS re:Post](https://repost.aws/knowledge-center/aws-abuse-report); [business_models_and_financials.md §7 Gaps](business_models_and_financials.md).
- **Recommended control plane.** Aurora PostgreSQL per geography; SQS fair queues; EventBridge Scheduler; Valkey Serverless; KMS keys per Region × environment × purpose; Firehose into S3 Iceberg for usage events (no Kinesis below 10–25 MB/s; Kinesis On-demand Advantage has a floor of about $3,100 a month); CloudWatch only for AWS signals — [aws_platform_networking_billing_policy.md §1, §9](aws_platform_networking_billing_policy.md).
- **Enterprise SSO.** WorkOS charges $125 per SSO connection per month for the first 15; AuthKit is free to 1M MAU — [aws_platform_networking_billing_policy.md §2](aws_platform_networking_billing_policy.md), from [WorkOS pricing](https://workos.com/pricing).
- **GAAP gross-margin benchmarks** — [business_models_and_financials.md §3](business_models_and_financials.md), from [Cloudflare's FY2025 release](https://cloudflare.net/news/news-details/2026/Cloudflare-Announces-Fourth-Quarter-and-Fiscal-Year-2025-Financial-Results/default.aspx) and [Twilio's 10-K](https://www.sec.gov/Archives/edgar/data/1447669/000144766926000021/twlo-20251231.htm):
  - Network and compute: Cloudflare 74.5%, Elastic 76%, Confluent 74.3%, MongoDB 72%, Snowflake product 72%, DigitalOcean about 60%, Fastly 57.1%.
  - Twilio: 49% (carrier pass-through in COGS).
  - GitLab: 87% (mostly seats).
  - AI products: 50–60% (a16z); Bessemer "Shooting Stars" about 60%, "Supernovas" about 25%.
  - The note's own benchmark: compute "should land" at about 60–75%; pass-through inputs "drag the blend toward 50% unless marked up".

### Inferences
Model outputs (section `blended`). Mixes are shares of gross revenue (my assumptions); marketplace revenue is Recipe event revenue before the 80% payout.

| mix | browser | crawl | CU | agent | proxy | captcha | LLM | marketplace |
|---|---|---|---|---|---|---|---|---|
| proxy-heavy | 12% | 15% | 8% | 5% | 45% | 5% | 3% | 7% |
| compute-heavy | 25% | 20% | 30% | 5% | 10% | 2% | 2% | 6% |
| LLM-heavy | 12% | 10% | 5% | 35% | 10% | 2% | 25% | 1% |
| marketplace-heavy | 6% | 8% | 25% | 3% | 15% | 2% | 1% | 40% |

**Line margins at the market median (us-east-1):**

| line | GM | | line | GM |
|---|---|---|---|---|
| browser | 19.6% | | proxy | 71.5% |
| crawl | 54.0% | | captcha | 84.2% |
| CU | 92.6% | | LLM | 2.4% |
| agent | 66.0% | | marketplace (kept share) | 20.0% |

**Waterfall at the market median, Stripe direct (EU), ARPU $100, two production Regions plus staging:**

| mix | before overheads | after payment fees and Connect | after free tier | after AWS Support | after observability | after abuse/KYC desk | after fixed platform minimums |
|---|---|---|---|---|---|---|---|
| **$100k/month** | | | | | | | |
| proxy-heavy | 59.0% | 54.8% | 50.2% | 48.8% | 48.5% | 35.7% | 29.4% |
| compute-heavy | 56.9% | 52.6% | 47.8% | 45.6% | 45.2% | 32.5% | 26.2% |
| LLM-heavy | 45.1% | 40.9% | 34.8% | 33.3% | 33.0% | 20.2% | 13.9% |
| marketplace-heavy | 51.1% | 46.8% | 41.3% | 40.1% | 39.8% | 27.0% | 20.7% |
| **$1M/month** | | | | | | | |
| proxy-heavy | 59.0% | 54.8% | 50.2% | 49.1% | 49.0% | 46.8% | 46.2% |
| compute-heavy | 56.9% | 52.6% | 47.8% | 45.7% | 45.7% | 43.5% | 42.9% |
| LLM-heavy | 45.1% | 40.9% | 34.8% | 33.6% | 33.5% | 31.4% | 30.7% |
| marketplace-heavy | 51.1% | 46.8% | 41.3% | 40.5% | 40.5% | 38.3% | 37.7% |

What each step assumes:
- **Payments.** About 3.8% of revenue: a 2.3% card mix (45% EEA standard, 10% premium, 5% UK, 40% international), the €0.25 fee on $100, Billing and Tax. Connect payouts add the rest.
- **Free tier.** Three free active users per payer, using half of a $5 allowance at the mix's cost ratio, ×1.5 for abuse.
- **Trust and safety desk.** 1.5 FTE at $8,500/month, plus 1 FTE per 4,000 payers.
- **Platform minimums.** About $2,600 (us-east-1) and $2,800 (eu-west-1) per Region per month, plus staging at 35% of a Region. The largest lines: three c7g.2xlarge control-plane nodes ($635), the security baseline ($609), Aurora at 4 ACU ($370) and CloudWatch for AWS signals ($375).

**Total fixed platform cost per month:**

| scale | control plane | staging | observability | trust and safety | AWS Support | total |
|---|---|---|---|---|---|---|
| 1,000 payers | $5,384 | $917 | $346 | $12,769 | $1,691 | $21,107 |
| 10,000 payers | $5,384 | $917 | $638 | $21,438 | $15,441 | $43,818 |

At 10,000 payers, Enterprise Support is on an assumed AWS bill of 15% of revenue.

**Merchant-of-record variants** (GM after all overheads, $1M/month):

| mix | Stripe direct | Stripe Managed Payments | Paddle |
|---|---|---|---|
| proxy-heavy | 46.2% | 43.0% | 44.7% |
| compute-heavy | 42.9% | 39.6% | 41.3% |
| LLM-heavy | 30.7% | 27.3% | 29.1% |
| marketplace-heavy | 37.7% | 35.6% | 36.7% |

A merchant of record can only cover the non-marketplace flows, and Paddle restricts proxies. Stripe direct plus Connect is therefore the only structure that serves all three businesses.

**Net-revenue view.** Counting marketplace revenue net of creator payouts, the marketplace-heavy mix earns 75.1% before overheads (51.1% on gross), and proxy-heavy 62.5%.

**Against the benchmarks:**
- **Before overheads.** The blend (45–59%) sits where Twilio and Fastly do. Pass-through inputs (proxies, tokens, solves) and the browser line pull it below the 60–75% compute benchmark.
- **After overheads.** At $1M a month the blend (31–46%) is below every public benchmark, but those companies book trust and safety and support outside gross margin.
- **On a basis closer to public filings** (unit COGS, payments, free usage and the AWS Support plan, with trust and safety and platform overhead left out), the mixes earn 49.1% (proxy-heavy), 45.7% (compute-heavy), 40.5% (marketplace-heavy, gross revenue) and 33.6% (LLM-heavy) at $1M a month.
- **What lifts the blend toward 60%+:**
  - metering live-view and direct traffic (the browser line to 68%);
  - an HTTP-first share of 70% or more on protected pages;
  - residential contracts near $1/GB (+10.8 points on proxy-heavy, +4.3–6.4 on the others);
  - a 15–20% markup on tokens (+4 points on LLM-heavy);
  - scale, which dilutes the trust-and-safety desk: 12.8 points at $100k/month, 2.2 at $1M;
  - hybrid fleets (question 4).

### Gaps
- No note gives a cost or headcount for an abuse or KYC desk, the real free-user cost, the card mix of a scraping cloud, or whether Stripe would underwrite a proxy reseller on standard terms ([business_models_and_financials.md §7 Gaps](business_models_and_financials.md)). These are assumptions, varied in question 6.
- The revenue mixes are assumptions. No competitor discloses its revenue split by product or its COGS split between proxies, tokens and compute ([business_models_and_financials.md §3 Gaps](business_models_and_financials.md)).

## 4. All-AWS versus hybrid: monthly cost and gross margin at 100, 1,000 and 10,000 concurrent browsers

### Takeaway
On the browser fleet alone, AWS wins at 100 concurrent browsers (peak): a 3-year Savings Plan base costs about $2,300 a month, against $5,300–11,700 for the hybrid options once operations overhead is counted. At 1,000 the options converge ($17–23k a month; 43–58% fleet gross margin at $0.08/h). At 10,000 the hybrid wins clearly:
- EU bare metal base plus AWS Spot burst: $99–128k a month (68–75% gross margin);
- with On-Demand burst: $141–151k;
- AWS's interactive mix: $324k (19%);
- AWS 3-year Savings Plan base plus On-Demand: $210k (48%).

The control plane stays on AWS in every layout.

### Cited Findings
- **Per-browser prices outside AWS:**
  - OVHcloud SCALE-a8 with 768 GB costs €2,783.99 a month (FR) or $3,566 (US catalogue) and holds 326 R browsers.
  - Hetzner AX162-2 costs €842.30 a month plus €419 setup and holds 81 R browsers.
  - Scaleway EM-I620E costs €679.99 a month and holds 108 R browsers.
  - OCI A1 Flex costs $0.01 per OCPU-hour plus $0.0015 per GB-hour.
  - Egress is included at OVHcloud, Hetzner and Scaleway. OCI charges $0.0085/GB after a free tier.
  - Sources: [provider_alternatives_vs_aws.md §1, §6](provider_alternatives_vs_aws.md), from the [OVH bare-metal catalogue](https://api.ovh.com/1.0/order/catalog/public/baremetalServers?ovhSubsidiary=FR), the [Hetzner price adjustment](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/), [Scaleway Elastic Metal](https://www.scaleway.com/en/pricing/elastic-metal/) and the [OCI price list API](https://apexapps.oracle.com/pls/apex/cetools/api/v1/products/?currencyCode=USD).
- **The provider note's own 24/7 results at 10,000 R browsers:**
  - AWS on-demand $350,869 a month, AWS Spot $160,879, AWS 3-yr RI $168,187.
  - OVH FR $96,712, Scaleway $71,246, Hetzner AX $117,277.
  - Hybrid illustration (OVH base plus AWS Spot burst): $71,010, compute only.
  - With hyperthreads counted at 0.65 of a Graviton core, bare metal and AWS Spot are "roughly at parity".
  - Source: [provider_alternatives_vs_aws.md §6](provider_alternatives_vs_aws.md).
- **Traffic direction.** AWS ingress is free, so fleets elsewhere can push results into an AWS control plane at no AWS cost. Data flowing out of AWS pays egress — [provider_alternatives_vs_aws.md §4](provider_alternatives_vs_aws.md).
- **Price and capacity risk.** Hetzner raised prices up to 37% on 2026-04-01 and again on 2026-06-15. Capacity for about 124 AX servers or 31 SCALE-a8 is unverified — [provider_alternatives_vs_aws.md §1, §6 Gaps](provider_alternatives_vs_aws.md), from [heise](https://heise.de/-11185981).
- **What competitors run on** — [competitor_cloud_architectures.md §1, §5 Inferences](competitor_cloud_architectures.md), from [Kernel's scale post](https://www.kernel.sh/blog/scale) and the [Zyte subprocessor list](https://www.zyte.com/terms-policies/subprocessor/):
  - Kernel runs browsers on bare metal ("meant to cut cloud spend sharply").
  - Zyte uses Hetzner and Servers.com beside AWS and GCP.
  - Lightpanda's EU cloud runs on Hetzner, Browserless on DigitalOcean, and Bright Data's super proxies on Vultr.
  - Apify runs only in AWS us-east-1.
  - Pattern: "a hyperscaler runs the API, account and data plane, and a cheaper provider for bandwidth-heavy edges".
- **Interactive Sessions and Spot.** They "belong on On-Demand capacity", since only cookies and storage migrate — [aws_runtime_compute_isolation.md Q4](aws_runtime_compute_isolation.md).

### Inferences
Model outputs (section `fleet`). Setup:
- **Demand.** An hourly curve averaging 69% of the daily peak. Peaks of 100, 1,000 and 10,000 give 49,883, 498,833 and 4,988,333 billed browser-hours a month.
- **Profile and traffic.** Rich headless profile with 30% of hours carrying headful live-view traffic.
- **Revenue.** The largest-plan median of $0.08 per browser-hour, with proxies, captcha and models excluded.
- **AWS.** Elastic layers at 65% utilization.
- **Hybrid.** A bare-metal base sized at 60% of peak running 24/7; the gateway sits on the fleet side, so base traffic leaves through included bandwidth.
- **Operations overhead.** $8,000 a month plus $50 per node for bare metal, and half the fixed part for OCI. This is my assumption: the provider note leaves staff costs unquantified.

| layout | 100 peak: $/month (fleet GM) | 1,000 peak | 10,000 peak |
|---|---|---|---|
| AWS on-demand | 4,023 (-0.8%) | 39,722 (0.5%) | 380,467 (4.7%) |
| AWS interactive mix (60% 1-yr SP + on-demand) | 3,464 (13.2%) | 34,123 (14.5%) | 324,481 (18.7%) |
| AWS 3-yr Instance SP base + on-demand burst | 2,316 (42.0%) | 22,644 (43.3%) | 209,691 (47.5%) |
| AWS Spot (batch Sessions only) | 1,938 (51.4%) | 18,868 (52.7%) | 171,923 (56.9%) |
| OVH SCALE-a8 FR + AWS Spot burst | 11,290 (-182.9%) | 18,393 (53.9%) | 111,879 (72.0%) |
| OVH SCALE-a8 US catalogue + AWS Spot burst | 11,736 (-194.1%) | 19,286 (51.7%) | 120,358 (69.8%) |
| Hetzner AX162-2 + AWS Spot burst | 9,213 (-130.9%) | 20,058 (49.7%) | 127,869 (68.0%) |
| Scaleway EM-I620E + AWS Spot burst | 8,932 (-123.8%) | 16,979 (57.5%) | 99,180 (75.1%) |
| OVH SCALE-a8 FR + AWS on-demand burst | 11,290 (-182.9%) | 21,758 (45.5%) | 150,897 (62.2%) |
| Scaleway EM-I620E + AWS on-demand burst | 8,932 (-123.8%) | 20,407 (48.9%) | 140,797 (64.7%) |
| OCI A1 (elastic) | 5,294 (-32.7%) | 17,385 (56.4%) | 138,611 (65.3%) |

Readings:
- **Break-even.** The hybrid crosses AWS's best interactive layout (3-yr SP base) between 1,000 and 10,000 peak. Below that, node granularity (one SCALE-a8 holds 326 browsers) and the fixed operations cost dominate.
- **The control plane at small scale.** One control plane ($2,620 a month) turns every layout negative at 100 peak, since revenue is only $3,991.
- **Agreement with the provider note.** The model's AWS on-demand cost at 10,000 peak ($380k) is within 8% of the note's 24/7 estimate ($351k). The hybrid costs more than the note's $71k because it adds burst, gateway, logs and operations.
- **Burst purchase.** Spot burst follows the brief and On-Demand burst follows the runtime note. The difference ($39–42k a month at 10,000 peak) is the price of not putting interactive Sessions on Spot.
- **SMT.** At 0.65 of a vCPU per hyperthread, the OVH FR hybrid at 10,000 peak rises from $111,879 to $144,223, and Hetzner from $127,869 to $167,095. Both still undercut AWS's 3-yr SP layout ($209,691).
- **Crawl fleets.** These favour the hybrid even more. Their compute is a tiny share of cost, while included bandwidth avoids AWS egress and NAT, which the provider note sizes at $18,068 a month per 1,000 browsers if fleets sat behind NAT.

### Gaps
- Bare-metal operations staff, hardware failure rates, provisioning lead times and stock at the 10,000-browser scale are unquantified ([provider_alternatives_vs_aws.md §6 Gaps](provider_alternatives_vs_aws.md)). The $8,000 a month is my assumption.
- Scaleway Elastic Metal and Hetzner dedicated egress allowances are unverified, as are OVHcloud's "unmetered" bandwidth and the 10 TB OCI free tier ([provider_alternatives_vs_aws.md §1 Gaps](provider_alternatives_vs_aws.md)).
- Whether AWS credits (up to $200k Portfolio) would cover the early fleet changes the early-stage comparison. The model ignores credits ([provider_alternatives_vs_aws.md §3](provider_alternatives_vs_aws.md)).

## 5. Revenue scenarios over 36 months (scenarios, not forecasts)

### Takeaway
These are scenarios built from funnel and ARPU benchmarks, not forecasts. At month 36 they reach:
- **Conservative:** about $2.1M ARR, priced 25% below the market median.
- **Base:** about $15M.
- **Upside:** about $39M, close to Bessemer's "Shooting Star" year-3 average of about $40M.

Gross profit, 38–45% of revenue after payments and the free tier, first covers fixed platform costs in month 20, 6 and 4 respectively. Fixed platform costs are the AWS control plane, staging, observability, the AWS Support plan and the trust and safety desk. They exclude engineering, sales and G&A.

### Cited Findings
- **Apify.** About $31M ARR at end-2025, "still maintains profitability". Active users rose from about 20,000 to 56,000 and paying customers from about 7,500 to 27,000, about half the revenue from the US. 2024 revenue was about CZK 300M (+65%) — [business_models_and_financials.md §2](business_models_and_financials.md), from [CzechCrunch 2026-02-02](https://cc.cz/pred-lety-prosel-cesky-startup-odtucnovaci-kurou-ted-ho-o-to-vic-nakopl-boom-umele-inteligence/). That implies about $96 per payer per month, a calculation in the note ([business_models_and_financials.md §4](business_models_and_financials.md)).
- **Conversion benchmarks.** ChartMogul and ProductLed (2026): median free-to-paid conversion 8% within six months; freemium "good" 3–5% and "great" 8–12%; AI-native "good" 6–8%; typical respondents at $50–249 average revenue per customer per month. The "0.5–3%" open-source heuristic is unsourced — [business_models_and_financials.md §4](business_models_and_financials.md), from [ChartMogul](https://chartmogul.com/reports/saas-conversion-report/).
- **Growth benchmarks.** Bessemer "Shooting Stars" average about $3M, $12M and $40M ARR in years 1, 2 and 3 — [business_models_and_financials.md §2](business_models_and_financials.md), from [Bessemer State of AI 2025](https://www.bvp.com/atlas/the-state-of-ai-2025).
- **Other datapoints:**
  - Firecrawl job ads claim "an eight-figure ARR in its first year" ([competitor_cloud_architectures.md §5](competitor_cloud_architectures.md), from a [Firecrawl job ad](https://haystackapp.io/jobs/6690a216-14d3-407a-ba74-4066b1b843a1)).
  - Browserbase: about $4.4M ARR in 2025 (third-party estimate, low confidence) and 10,000 customers claimed ([business_models_and_financials.md §2](business_models_and_financials.md), from [Sacra](https://sacra.com/c/browserbase/)).
  - Tavily: about $2.8M of 2025 revenue at its $275M sale ([business_models_and_financials.md §2](business_models_and_financials.md)).
- **Budgets by segment** — [rnd/research/market/pricing-signals.md §1](../../market/pricing-signals.md):
  - hobbyists about $15–190 a month (about $50 typical);
  - agent-developer stacks $30–400 a month;
  - a data team's 500-site monitoring run about $310 a month;
  - B2B sales-led plans $2,500 a month, objected to without pay-as-you-go;
  - enterprise consolidations $5.9k–10k a month (vendor claims).

### Inferences
Model outputs (section `scenarios`). Inputs, all assumptions except conversion:

| input | conservative | base | upside |
|---|---|---|---|
| month-1 sign-ups | 800 | 2,000 | 3,000 |
| monthly sign-up growth (falls linearly over 36 months) | 6% → 1% | 9% → 2% | 11% → 2% |
| sign-up to paid within the window (ChartMogul bands) | 3% | 5% | 7% |
| enterprise logos per month | 0.25 → 0.75, from month 12 | 0.5 → 2, from month 9 | 1 → 3, from month 6 |
| price position | 0.75 of median | median | median |

Shared inputs:
- **Payer mix:** new self-serve payers split 60% hobbyist ($25 ARPU, 6% monthly churn), 28% agent developer ($120, 4%, +0.5% ARPU a month) and 12% data team ($450, 2.5%, +1% a month).
- **Enterprise:** $5,000 ARPU (+1% a month), 1% churn, $125 a month of SSO each.
- **Gross margin:** each segment carries its mix's margin (hobbyist compute-heavy, agent developer LLM-heavy, data team marketplace-heavy, enterprise proxy-heavy), then payment fees and the free tier.
- **Fixed platform costs:** as in question 3, with eu-west-1 opening in month 7.

| scenario, month | active users | payers: hobbyist / agent dev / data team / enterprise | ARPU | MRR | ARR | COGS | gross profit | fixed platform | GP − fixed |
|---|---|---|---|---|---|---|---|---|---|
| conservative M12 | 1,013 | 146 / 74 / 33 / 0.2 | $92 | $23,381 | $0.28M | $15,398 | $7,984 | $20,237 | −$12,254 |
| conservative M24 | 2,449 | 337 / 182 / 88 / 4.6 | $137 | $83,874 | $1.01M | $53,239 | $30,635 | $20,824 | $9,811 |
| conservative M36 | 3,979 | 529 / 302 / 153 / 11.4 | $176 | $175,538 | $2.11M | $109,349 | $66,189 | $21,670 | $44,519 |
| base M12 | 4,865 | 702 / 352 / 160 / 2.3 | $128 | $155,248 | $1.86M | $87,496 | $67,752 | $21,221 | $46,531 |
| base M24 | 14,234 | 1,983 / 1,058 / 504 / 13.8 | $159 | $563,961 | $6.77M | $311,350 | $252,611 | $25,815 | $226,795 |
| base M36 | 27,218 | 3,692 / 2,058 / 1,023 / 31.6 | $185 | $1,261,827 | $15.14M | $689,124 | $572,703 | $34,298 | $538,405 |
| upside M12 | 11,198 | 1,616 / 809 / 366 / 8.2 | $133 | $372,399 | $4.47M | $208,366 | $164,033 | $22,925 | $141,108 |
| upside M24 | 36,629 | 5,126 / 2,715 / 1,288 / 28.2 | $153 | $1,399,108 | $16.79M | $776,504 | $622,604 | $41,183 | $581,421 |
| upside M36 | 75,189 | 10,266 / 5,677 / 2,800 / 55.0 | $173 | $3,241,863 | $38.90M | $1,789,623 | $1,452,240 | $74,498 | $1,377,742 |

Readings:
- **Break-even.** Gross profit first covers fixed platform costs in month 20 (conservative), 6 (base) and 4 (upside). The trust and safety desk (1.5 FTE minimum) is most of the early fixed cost.
- **Benchmarks.** Base month 36 has about 6,800 payers at a $185 blended ARPU. Apify reached $31M with 27,000 payers at about $96. Base ARR therefore rests more on data teams and enterprise than Apify's mix does. Upside tracks the Shooting Star curve, a little faster in year 1 ($4.5M against $3M).
- **Gross profit share.** It stays about 38–46% of revenue across scenarios, because the segment mixes carry the proxy, token and browser lines priced at market.

### Gaps
- No primary benchmark ties GitHub stars or installs to paid conversion ([business_models_and_financials.md §4 Gaps](business_models_and_financials.md)).
- Sign-up volumes, growth, churn, ARPU expansion and enterprise pace are assumptions without a sourced Goodfetch baseline. The notes recommend interviews and a price test before any forecast ([rnd/research/market/pricing-signals.md §5](../../market/pricing-signals.md)).
- The scenarios exclude AWS Activate credits, engineering, sales and G&A, and the time needed to reach V1.

## 6. Sensitivity: which assumptions move gross margin most?

### Takeaway
Ranked by swing in the four-mix average at $1M a month after all overheads (base 39.4%), the five largest drivers are:
1. price position: 25.1% at 0.7x the median, 47.2% at 1.25x;
2. free active users per payer: 42.9% at 1, 27.1% at 10;
3. residential cost: 46.1% at $1.00/GB, 36.0% at $2.50;
4. browser fleet utilization: 32.7% at 40%, 41.9% at 85%;
5. bytes per protected browser load: 42.3% at 0.5 GB per 1,000 loads, 33.5% at 2.0.

Spot share (1.9 points) and LLM markup (1.4 points on the average, 4.6 on the LLM-heavy mix) matter much less than expected.

### Cited Findings
The ranges tested come from the notes:
- **Spot:** snapshot 43–90% vs Advisor 43–80% ([aws_price_sheet.md §1](aws_price_sheet.md)).
- **Residential:** $0.70–2.50/GB ([input_costs_proxies_captchas_llm.md §1](input_costs_proxies_captchas_llm.md)).
- **Bytes per page:** 0.73–0.80 GB per 1,000 blocked loads and 2.2–2.9 GB per 1,000 full loads ([workload_resource_profiles.md §4](workload_resource_profiles.md)).
- **LLM markup:** 0–20% ([pricing_hosted_browsers_and_agents.md §2](pricing_hosted_browsers_and_agents.md)).
- **Replay:** 3–6x fewer tokens ([rnd/lab/reports/ai-real-models-2026-10-09.md](../../../lab/reports/ai-real-models-2026-10-09.md)).
- **Logs:** $0.05–0.57/GB ([aws_price_sheet.md §7](aws_price_sheet.md); [aws_platform_networking_billing_policy.md §7](aws_platform_networking_billing_policy.md)).
- **Browser memory:** 0.3 GB to 8 GB per Session ([workload_resource_profiles.md §1](workload_resource_profiles.md); [aws_runtime_compute_isolation.md Q3](aws_runtime_compute_isolation.md); [pricing_hosted_browsers_and_agents.md §1](pricing_hosted_browsers_and_agents.md)).
- **Egress:** $0.09–0.05/GB by tier ([aws_price_sheet.md §6](aws_price_sheet.md)).
- **Fargate Spot:** "up to 70%" ([aws_price_sheet.md §2](aws_price_sheet.md)).
- **SMT:** 0.65 tested ([provider_alternatives_vs_aws.md §6](provider_alternatives_vs_aws.md)).

### Inferences
Model outputs (sections `sensitivity` and `disagreements`).

| assumption (low / base / high) | blended GM | swing |
|---|---|---|
| price position, x median (0.7 / 1.0 / 1.25) | 25.1% / 39.4% / 47.2% | 22.1 pts |
| free active users per payer (1 / 3 / 10) | 42.9% / 39.4% / 27.1% | 15.9 pts |
| residential COGS, $/GB (1.00 / 2.00 / 2.50) | 46.1% / 39.4% / 36.0% | 10.1 pts |
| browser fleet utilization (40% / 65% / 85%) | 32.7% / 39.4% / 41.9% | 9.2 pts |
| GB per 1,000 protected browser loads (0.5 / 1.0 / 2.0) | 42.3% / 39.4% / 33.5% | 8.8 pts |
| protected HTTP-route share (0% / 40% / 70%) | 35.8% / 39.4% / 42.1% | 6.3 pts |
| browser memory multiplier (0.5x / 1x / 2x) | 39.4% / 39.4% / 37.1% | 2.3 pts |
| Spot share of batch capacity (0% / 80% / 95%) | 37.8% / 39.4% / 39.7% | 1.9 pts |
| captcha solves per protected browser load (2% / 10% / 50%) | 39.7% / 39.4% / 38.0% | 1.7 pts |
| trust and safety FTE cost, $/month (6,000 / 8,500 / 12,000) | 40.0% / 39.4% / 38.5% | 1.5 pts |
| LLM markup over list (0% / 2.5% / 20%) | 39.2% / 39.4% / 40.6% | 1.4 pts |
| log price, $/GB (0.05 / 0.23 / 0.50) | 39.9% / 39.4% / 38.6% | 1.3 pts |
| egress, $/GB (0.05 / 0.09) | 40.3% / 39.4% | 0.9 pts |
| datacenter proxy, $/GB (0.03 / 0.15 / 0.45) | 39.6% / 39.4% / 38.8% | 0.9 pts |
| replay token reduction (3x / 4.5x / 6x) | 39.1% / 39.4% / 39.5% | 0.5 pts |

**The five largest drivers, and the two named in the brief, by mix** (GM after all overheads, $1M a month; columns are low / base / high):

| driver | proxy-heavy | compute-heavy | LLM-heavy | marketplace-heavy |
|---|---|---|---|---|
| price position (0.7 / 1.0 / 1.25) | 30.2 / 46.2 / 54.1% | 24.8 / 42.9 / 51.6% | 15.5 / 30.7 / 41.5% | 29.7 / 37.7 / 41.5% |
| free users per payer (1 / 3 / 10) | 49.3 / 46.2 / 35.5% | 46.1 / 42.9 / 31.6% | 34.9 / 30.7 / 16.3% | 41.4 / 37.7 / 24.8% |
| residential $/GB (1.00 / 2.00 / 2.50) | 57.0 / 46.2 / 40.8% | 49.3 / 42.9 / 39.7% | 36.1 / 30.7 / 28.1% | 42.0 / 37.7 / 35.5% |
| browser utilization (40% / 65% / 85%) | 40.3 / 46.2 / 48.4% | 30.9 / 42.9 / 47.4% | 24.8 / 30.7 / 33.0% | 34.7 / 37.7 / 38.8% |
| GB per 1,000 protected loads (0.5 / 1.0 / 2.0) | 49.5 / 46.2 / 39.6% | 47.3 / 42.9 / 34.1% | 33.0 / 30.7 / 26.3% | 39.5 / 37.7 / 34.1% |
| Spot share (0% / 80% / 95%) | 45.4 / 46.2 / 46.4% | 40.1 / 42.9 / 43.4% | 30.2 / 30.7 / 30.8% | 35.3 / 37.7 / 38.1% |
| LLM markup (0% / 2.5% / 20%) | 46.1 / 46.2 / 46.7% | 42.8 / 42.9 / 43.2% | 30.1 / 30.7 / 34.7% | 37.7 / 37.7 / 37.8% |

**Where the notes disagree** (chosen value first):

*Spot basis* (rich headless Spot $/h; JS pages $/1,000; CU at 80% Spot):

| basis | rich headless Spot | JS pages | CU |
|---|---|---|---|
| Advisor 30-day (chosen) | $0.0316 | $0.0398 | $0.0096 |
| snapshot | $0.0315 | $0.0398 | $0.0088 |
| 60% flat (runtime note) | $0.0351 | $0.0399 | $0.0117 |
| 58.3% (provider note) | $0.0363 | $0.0400 | $0.0119 |

*Browser memory* (recommended mix, $/h):

| basis | rich headless | rich headful |
|---|---|---|
| vendor sizing, half memory | $0.0623 | $0.1414 |
| provider note L/R (chosen) | $0.0622 | $0.1413 |
| runtime note 1.6–5 GB (2x) | $0.0723 | $0.1723 |
| Kernel 8 GB headful (4x) | $0.1004 | $0.2332 |

*Egress tier*: the effective rate is $0.0875/GB at 3.65 TB a month, $0.0861 at 36.5 TB, $0.0604 at 365 TB and $0.0517 at 2,190 TB.

*Fargate Spot discount* (rich headless $/h; CU):

| discount | rich headless | CU |
|---|---|---|
| 50% | $0.0370 | $0.0113 |
| 60% | $0.0322 | $0.0100 |
| 70% (chosen) | $0.0275 | $0.0087 |

*SMT thread value* (10,000-peak hybrid, $/month): OVH FR $111,879 at 1.0 and $144,223 at 0.65; Hetzner $127,869 and $167,095.

Readings:
- **Price position** dominates because three lines are thin at the volume price: browser-hours, protected pages and tokens. A 25% cut turns the first two negative.
- **The free tier** is the largest controllable cost after price. It argues for currency-capped allowances that exclude proxies and solving, the abuse surface in [business_models_and_financials.md §4](business_models_and_financials.md).
- **Proxy economics** (residential $/GB, bytes per protected load, HTTP-route share) together move the blend by about 25 points. That makes the HTTP-first engine and a negotiated residential contract worth more than any AWS purchase option.
- **Spot share** is minor. Batch compute is cheap either way, and interactive Sessions stay off Spot.

### Gaps
- The sensitivity varies one input at a time. Correlated moves are not modelled: a price war that lowers prices and raises free usage together, or a residential price rise from upstream supply shocks such as the NetNut seizure ([input_costs_proxies_captchas_llm.md §3](input_costs_proxies_captchas_llm.md)).
- No probability is attached to any range. The "base" values are midpoints of the evidence or my assumptions, not measured Goodfetch figures.
