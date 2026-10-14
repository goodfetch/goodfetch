#!/usr/bin/env python3
"""Goodfetch Cloud on AWS: unit costs, gross margins and 36-month revenue scenarios.

A reproducible model built only from the research notes in this folder (all dated
2026-10-12), the Goodfetch lab reports and the ADRs. Nothing here is a forecast.

Run (standard library only, Python 3.9+):
    python3 unit_economics_model.py                 # every section
    python3 unit_economics_model.py units margins   # chosen sections
Sections: inputs, units, market, margins, blended, fleet, scenarios, sensitivity,
          disagreements

Conventions
- Every input lives in the ASSUMPTIONS block below, with its source file and section.
  "MODEL ASSUMPTION" marks a value no note gives: it is stated, varied in the
  sensitivity section, and never presented as a fact.
- Every printed number is a MODEL OUTPUT derived from those inputs, not a fact.
- USD list prices, dated as in the notes (AWS price list files of 2026-09-11 to
  2026-10-10; vendor pages read 2026-10-12). A month is 730 hours. GB and GiB are
  treated as equal when packing browsers or Compute Units onto nodes.
"""

import argparse
import copy
import math
import statistics

# =====================================================================================
# ASSUMPTIONS BLOCK. Each input: name, value, unit, source (file and section), note.
# =====================================================================================

REG = {}


def inp(name, value, unit, src, note=""):
    if name in REG:
        raise ValueError(f"duplicate input {name}")
    REG[name] = {"value": value, "unit": unit, "src": src, "note": note}


PS = "aws_price_sheet.md"
RT = "aws_runtime_compute_isolation.md"
PN = "aws_platform_networking_billing_policy.md"
PA = "provider_alternatives_vs_aws.md"
WR = "workload_resource_profiles.md"
HB = "pricing_hosted_browsers_and_agents.md"
SA = "pricing_scraping_apis_and_actor_platforms.md"
IC = "input_costs_proxies_captchas_llm.md"
BM = "business_models_and_financials.md"
CC = "competitor_cloud_architectures.md"
OS = "open_source_cloud_layers.md"
SIG = "rnd/research/market/pricing-signals.md"
LAB = "rnd/lab/reports/"
MA = "MODEL ASSUMPTION"

REGIONS = ("us-east-1", "eu-west-1")

# ---- General --------------------------------------------------------------------------
inp("hours_month", 730.0, "h", f"{PS} §1 (tables: '$/mo assumes 730 hours')")
inp("eur_usd", 1.1206, "USD per EUR", f"{PA} conventions (ECB reference rate, 9 Oct 2026)")

# ---- EC2 instances (Linux, shared tenancy) -------------------------------------------
# tuple: (vCPU, GiB, on-demand $/h, Spot snapshot $/h, Spot Advisor 30-day savings,
#         Advisor interruption band, Compute SP 1y, Compute SP 3y, EC2 Instance SP 1y,
#         EC2 Instance SP 3y)  -- SP values are discounts vs on-demand, No Upfront.
inp("ec2", {
    "us-east-1": {
        "c7g.2xlarge":    (8, 16, 0.2900, 0.1427, 0.58, "5-10%", 0.283, 0.516, 0.341, 0.561),
        "c7g.4xlarge":    (16, 32, 0.5800, 0.2143, 0.54, "<5%", 0.283, 0.516, 0.341, 0.561),
        "c7g.8xlarge":    (32, 64, 1.1600, 0.4047, 0.65, "<5%", 0.283, 0.516, 0.341, 0.561),
        "c7g.metal":      (64, 128, 2.3200, 0.8281, 0.62, "5-10%", 0.283, 0.516, 0.341, 0.561),
        "c8g.4xlarge":    (16, 32, 0.6381, 0.2503, 0.57, ">20%", 0.283, 0.516, 0.341, 0.561),
        "c8g.metal-24xl": (96, 192, 3.8285, 0.4156, 0.68, "15-20%", 0.283, 0.516, 0.341, 0.561),
        "m7g.4xlarge":    (16, 64, 0.6528, 0.3695, 0.59, "15-20%", 0.265, 0.493, 0.339, 0.546),
        "m7g.metal":      (64, 256, 2.6112, 0.5272, 0.74, "5-10%", 0.265, 0.493, 0.339, 0.546),
        "m8g.4xlarge":    (16, 64, 0.7181, 0.2641, 0.56, "10-15%", 0.265, 0.493, 0.338, 0.546),
        "m8g.metal-24xl": (96, 384, 4.3085, 1.1193, 0.74, "10-15%", 0.265, 0.493, 0.338, 0.546),
        "r8g.4xlarge":    (16, 128, 0.9426, 0.2494, 0.63, "5-10%", 0.276, 0.478, 0.339, 0.546),
        "c8i.4xlarge":    (16, 32, 0.7497, 0.3049, 0.43, "5-10%", 0.277, 0.517, 0.339, 0.560),
        "c8i.8xlarge":    (32, 64, 1.4994, 0.6270, 0.61, ">20%", 0.277, 0.517, 0.338, 0.560),
        "m8i.4xlarge":    (16, 64, 0.8467, 0.2909, 0.61, ">20%", 0.265, 0.493, 0.338, 0.546),
        "m8i.8xlarge":    (32, 128, 1.6934, 0.6394, 0.63, ">20%", 0.265, 0.493, 0.339, 0.546),
    },
    "eu-west-1": {
        "c7g.2xlarge":    (8, 16, 0.3101, 0.1528, 0.51, "<5%", 0.236, 0.478, 0.338, 0.544),
        "c7g.4xlarge":    (16, 32, 0.6202, 0.2310, 0.61, "<5%", 0.236, 0.478, 0.338, 0.544),
        "c7g.8xlarge":    (32, 64, 1.2403, 0.5565, 0.55, "<5%", 0.236, 0.478, 0.338, 0.544),
        "c7g.metal":      (64, 128, 2.4806, 0.9584, 0.64, "<5%", 0.236, 0.478, 0.338, 0.544),
        "c8g.4xlarge":    (16, 32, 0.6822, 0.3213, 0.59, "<5%", 0.236, 0.478, 0.338, 0.544),
        "c8g.metal-24xl": (96, 192, 4.0934, 1.3096, 0.74, "<5%", 0.236, 0.478, 0.338, 0.544),
        "m7g.4xlarge":    (16, 64, 0.7276, 0.3710, 0.50, "<5%", 0.241, 0.475, 0.339, 0.546),
        "m7g.metal":      (64, 256, 2.9104, 0.8040, 0.75, "<5%", 0.241, 0.475, 0.339, 0.546),
        "m8g.4xlarge":    (16, 64, 0.8003, 0.4087, 0.53, "5-10%", 0.241, 0.475, 0.339, 0.546),
        "m8g.metal-24xl": (96, 384, 4.8019, 1.4287, 0.68, "5-10%", 0.241, 0.475, 0.339, 0.546),
        "r8g.4xlarge":    (16, 128, 1.0547, 0.5138, 0.45, "5-10%", 0.276, 0.478, 0.338, 0.546),
        "c8i.4xlarge":    (16, 32, 0.8044, 0.4264, None, "n/a", 0.236, 0.477, 0.338, 0.544),
        "c8i.8xlarge":    (32, 64, 1.6088, 0.8087, None, "n/a", 0.236, 0.477, 0.338, 0.544),
        "m8i.4xlarge":    (16, 64, 0.9438, 0.4802, 0.51, "15-20%", 0.241, 0.475, 0.338, 0.546),
        "m8i.8xlarge":    (32, 128, 1.8875, 0.9536, 0.49, ">20%", 0.241, 0.475, 0.339, 0.546),
    },
}, "see tuple layout", f"{PS} §1 tables 'specifications, on-demand and Spot (Linux)' and "
   "'Savings Plans effective hourly rates, No Upfront' (us-east-1 and eu-west-1)",
   "Spot snapshot fetched 2026-10-12T01:37Z; Advisor savings are 30-day averages. "
   "eu-west-1 c8i has no Advisor entry, so its snapshot discount is used.")

inp("browser_families", ["c7g.4xlarge", "c7g.8xlarge", "c7g.metal", "c8g.4xlarge",
                         "m7g.4xlarge", "m8g.4xlarge", "r8g.4xlarge"],
    "candidate node types", f"{RT} Q7 'Recommended architecture' (c8g/m8g/r8g Graviton NodePools)",
    "The model picks the cheapest slot per profile and purchase option.")
inp("metal_families", ["c7g.metal", "m7g.metal", "c8g.metal-24xl", "m8g.metal-24xl"],
    "candidate .metal hosts", f"{RT} Q2 table (Firecracker or Kata on Graviton .metal); {PA} §1")
inp("nested_families", ["c8i.4xlarge", "c8i.8xlarge", "m8i.4xlarge", "m8i.8xlarge"],
    "candidate nested-virtualization hosts", f"{RT} Q1 'Nested virtualization ... Intel 7i/8i'; {PA} §1")
inp("crawl_families", ["c7g.4xlarge", "c7g.8xlarge", "c8g.4xlarge", "m8g.4xlarge"],
    "candidate crawl-fleet nodes", f"{RT} Q7 (c8g, m8g Spot for HTTP, agent and batch-browser runs)")

# ---- Purchase options -------------------------------------------------------------------
inp("spot_basis", "advisor", "advisor | snapshot | flat discount",
    f"{PS} §1 Inferences ('the Advisor's 30-day savings ... is the safer planning figure')",
    "Notes disagree: snapshot 43-90% below on-demand ({PS} §1), 60% assumed ({RT} Q7), "
    "58% on c7g.4xlarge via Vantage ({PA} §1). Varied in 'disagreements'.")
inp("spot_overhead", 0.05, "fraction added to Spot cost", f"{MA}; mechanism from {RT} Q4 "
    "(2-minute notice, checkpoint/migrate) and ADR 0002 (redo work since last checkpoint)",
    "Covers redone work, migration time and diversification over >=15 instance types.")
inp("spot_share_batch", 0.80, "share of batch capacity on Spot",
    f"{MA}; policy from {RT} Q4/Q7 (Spot for checkpointable runs, On-Demand for interactive Sessions)")
inp("sp_share_interactive", 0.60, "share of interactive browser capacity under Savings Plans",
    f"{MA}; sized like the 60% base of {PA} §6 'Hybrid illustration'")
inp("interactive_sp", "csp1", "Savings Plan used for interactive capacity",
    f"{PS} §1 (Compute SP 1 yr keeps family and Region flexibility)")

# ---- Packing and utilization -----------------------------------------------------------
inp("packing", 0.85, "usable share of node vCPU and RAM", f"{PA} §6 Assumptions ('Packing: 85%')")
inp("util_elastic", 0.65, "billed / provisioned slot-hours, elastic EC2 browser fleet",
    f"{MA}; no note publishes fleet utilization ({WR} §7 Gaps; {RT} Q7 Gaps)",
    "Covers scale-up lag, Session fragmentation (Sessions cannot be moved) and node minimums.")
inp("util_batch", 0.75, "busy / provisioned vCPU-hours, queue-driven crawl fleet", MA)
inp("util_cu", 0.70, "packed / provisioned GB-hours, Recipe microVM fleet", MA)
inp("pool_frac", 0.10, "pre-booted sandboxes per active Session", MA,
    "Chromium starts after claim to keep the per-Session fingerprint seed unique "
    f"({RT} Q2 Inferences), so the pool holds sandboxes, not browsers.")
inp("pool_mem_container_gb", 0.039, "GB per pooled container", f"{WR} §1 (lab: ~39 MiB idle per pooled container); "
    f"{LAB}sinks-topology-2026-10-09.md")
inp("pool_mem_microvm_gb", 0.16, "GB per pooled microVM", f"{RT} Q7 (assumed ~160 MiB per microVM)")
inp("fargate_pool_frac", 0.20, "idle pre-started Fargate tasks per active Session",
    f"{MA}; Fargate starts take ~30 s+ and have no warm pool ({RT} Q1 Inferences (b))")

# ---- Browser profiles (lean and rich, headless and headful) ----------------------------
inp("browser_profiles", {
    "lean headless": {"vcpu": 0.5, "gb": 1.0, "headful": False},
    "lean headful":  {"vcpu": 0.75, "gb": 1.5, "headful": True},
    "rich headless": {"vcpu": 1.0, "gb": 2.0, "headful": False},
    "rich headful":  {"vcpu": 2.0, "gb": 4.0, "headful": True},
}, "vCPU and GB reserved per Session",
    f"lean/rich = 'L' 0.5 vCPU + 1 GB and 'R' 1 vCPU + 2 GB ({PA} §6 Assumptions); "
    f"evidence {WR} §1 (vendor sizing 0.16-0.4 vCPU, 0.3-0.8 GB; full Chrome 1.1-1.4 GB in lab) and §2; "
    f"rich headful = AgentCore's 2 vCPU / 4 GB example ({RT} Q3, 'plan for 1.6-5 GB'); "
    f"lean headful +50% is a {MA}",
    "Notes disagree on browser memory: 0.3-0.8 GB (vendor sizing), 1-2 GB (provider note), "
    "1.6-5 GB (runtime note), 8 GB headful (Kernel default, HB §1). Varied in 'disagreements'.")

# ---- MicroVM and isolation overheads -----------------------------------------------------
inp("fc_vm_overhead_gb", 0.045, "GB per Firecracker microVM (VMM + guest kernel/init)",
    f"{RT} Q1/Q2 (VMM <= 5 MiB); guest kernel and init ~40 MiB is a {MA}")
inp("kata_overhead", {"gb": 0.130, "vcpu": 0.25}, "per pod", f"{RT} Q1 (kata-fc RuntimeClass: 130Mi and 250m CPU per pod)")
inp("nested_cpu_perf", 0.90, "useful CPU share under nested virtualization",
    f"{PA} §1 (GCP: 'a 10% or greater decrease in performance'); {RT} Q1 (AWS recommends bare metal "
    "for performance-sensitive work)")

# ---- Fargate, Lambda, Lambda MicroVMs, AgentCore, EKS ----------------------------------
inp("fargate", {"arm_vcpu_h": 0.03238, "arm_gb_h": 0.00356, "x86_vcpu_h": 0.04048, "x86_gb_h": 0.004445},
    "$/h", f"{PS} §2 (identical in us-east-1 and eu-west-1)")
inp("fargate_spot_discount", 0.70, "fraction", f"{PS} §2 / {RT} Q1 ('up to a 70% discount'; no published rate)",
    "The advertised maximum; varied in sensitivity.")
inp("public_ipv4_h", 0.005, "$/h per address", f"{PS} §6")
inp("lambda", {"arm_gb_s": 0.0000133334, "x86_gb_s": 0.0000166667, "req": 0.20e-6}, "$",
    f"{PS} §3 (first-tier rates, all three regions)")
inp("lmvm", {
    "us-east-1": {"vcpu_s": 0.0000276944, "gb_s": 0.0000036667, "snap_read_gb": 0.0015467699, "snap_write_gb": 0.0037977138},
    "eu-west-1": {"vcpu_s": 0.0000291572, "gb_s": 0.0000038603, "snap_read_gb": 0.0016403088, "snap_write_gb": 0.0040636422},
}, "$", f"{PS} §3 'AWS Lambda MicroVMs (new)' (Arm; 2 GB per vCPU)")
inp("agentcore", {"v1_vcpu_h": 0.0895, "v1_gb_h": 0.00945, "v2_vcpu_h": 0.1276, "v2_gb_h": 0.0169},
    "$/h", f"{RT} Q6 'AgentCore pricing' (I/O wait and idle CPU free; memory at peak)")
inp("agent_session_cpu_active", 0.20, "share of Session time the CPU is busy",
    f"{MA}; {WR} §1 Inferences (agent Sessions mostly idle) and {RT} Q7 (AgentCore wins below 23% CPU)")
inp("eks_cluster_h", 0.10, "$/cluster-hour", f"{PS} §3 'EKS'")
inp("eks_auto_mode_fee", 0.12, "fraction of on-demand", f"{PS} §1/§3 (not used in base: self-managed Karpenter)")

# ---- Network ---------------------------------------------------------------------------
inp("egress_free_gb", 100.0, "GB/month", f"{PS} §6 (aggregated globally)")
inp("egress_tiers", [(10_000, 0.09), (50_000, 0.085), (150_000, 0.07), (float("inf"), 0.05)],
    "(cumulative GB cap, $/GB)", f"{PS} §6 (identical in us-east-1, eu-west-1, eu-west-3)",
    f"{PA} §1 quoted the same tiers as unverified; the price sheet verifies them.")
inp("egress_rate_unit", 0.09, "$/GB used in per-unit costs", f"{PS} §6 first paid tier",
    "Per-unit tables use the first tier; the fleet section uses the blended tiered rate.")
inp("nat", {"us-east-1": (0.045, 0.045), "eu-west-1": (0.048, 0.048)}, "($/h, $/GB processed)", f"{PS} §6")
inp("nlb", {"us-east-1": 0.0225, "eu-west-1": 0.0252}, "$/h per load balancer", f"{PS} §6 table")
inp("nlcu_h", 0.006, "$/NLCU-hour", f"{PS} §6 table")
inp("alb_lcu_h", 0.008, "$/LCU-hour", f"{PS} §6 table")
inp("nlcu_gb_per_h", 1.0, "GB processed per NLCU-hour (TCP)",
    f"{MA}: AWS's published NLCU dimension, not in any note ({PN} §2 Gaps: 'LCU/NLCU ... charges' missing)",
    "Bytes are the binding NLCU dimension for streaming Sessions; the cost is small either way.")
inp("alb_requests_per_lcu_h", 3_600_000.0, "requests per LCU-hour with keep-alive (1,000 rule evaluations/s)",
    f"{MA}: AWS's published LCU dimension, not in any note ({PN} §2 Gaps)")
inp("cloudfront_flat_business", (200.0, 50_000.0), "($/month, GB included) per distribution",
    f"{PN} §2 / {PS} §6 (flat-rate Business plan: 125M requests, 50 TB, no overage)",
    "A lever for customer dataset downloads ({PN} §3: 'Customer downloads go through CloudFront'); "
    "not used in the base case because WebSocket and API use of the plans is unverified ({PN} §2 Gaps).")
inp("traffic_per_browser_h", {"ingress": 0.5, "egress_base": 0.05, "egress_liveview": 0.30},
    "GB per browser-hour", f"{PA} §6 Assumptions (base and 'heavy hosted-browser case')",
    "Headful profiles are billed the live-view (heavy) egress; headless the base egress.")

# ---- Session gateway, observability, storage --------------------------------------------
inp("gw_sessions_per_vcpu", {"headless": 1000.0, "headful": 128.0}, "Sessions per gateway vCPU",
    f"{MA}; design from {RT} Q5 (Rust/Envoy gateway behind an NLB TCP listener)",
    "Headful is bandwidth-bound: ~45 GB/h per vCPU at 0.35 GB/h per Session.")
inp("logs_gb", {"browser_h": 0.010, "cu_h": 0.005, "per_1000_pages": 0.002}, "GB of logs",
    f"{MA}; {PN} §7 Gaps ('no per-session log volume')")
inp("log_price", {"us-east-1": 0.23, "eu-west-1": 0.15}, "$/GB ingested + 1 month retention (Better Stack)",
    f"{PN} §7 (Better Stack: ingest $0.15/GB US, $0.10/GB EU; retention $0.08 and $0.05 per GB-month)",
    "Alternatives: CloudWatch Infrequent Access $0.25/GB, Standard $0.50/GB ({PS} §7); Grafana ~$0.55/GB all-in.")
inp("recording_gb_per_session_h", 0.005, "GB per Session-hour kept 1 month", MA)
inp("s3", {"gb_month": 0.023, "put_per_1000": 0.005, "get_per_1000": 0.0004}, "$", f"{PS} §4 (us-east-1 = eu-west-1)")
inp("avg_session_minutes", 3.0, "minutes per hosted Session",
    f"{CC} §5 (Steel claims 35M+ sessions and 1.5M+ browser-hours, about 2.6 min per session; arithmetic "
    f"by the extraction); {WR} §7 (no platform publishes a distribution)")

# ---- Crawl workload (per page) -----------------------------------------------------------
inp("cpu_ms", {"http_fetch": 0.3, "tls_decompress": 5.0, "extraction": 14.5,
               "lightpanda_page": 11.0, "chrome_js_page": 250.0, "chrome_protected_page": 500.0,
               "browser_start_per_100_pages": 1000.0},
    "CPU milliseconds", f"{WR} §2-§3 (reqwest/wreq 0.19-0.31 ms; rs-trafilatura 14.46 ms p50; Lightpanda 6-11 ms; "
    f"Chrome 116-127 ms light, 294 ms heavier, lab 0.8-3.1 s per task upper bound); "
    f"TLS/decompression 5 ms and protected 500 ms are {MA}")
inp("run_residency_gb_s_per_page", 0.05, "GB-seconds of engine memory per HTTP page",
    f"{RT} Q7 (~50 MiB per containerized run, polite crawl at ~1 page/s)")
inp("gb_per_1000_pages", {"html": 0.025, "js_blocked": 0.77, "lightpanda": 0.10,
                          "protected_browser": 1.0, "protected_http": 0.06, "result": 0.015},
    "GB per 1,000 pages", f"{WR} §4 (HTML 0.014-0.033; blocked browser 0.73-0.80); {IC} §2 (ZenRows "
    "'roughly 1,000 hard-protected pages' per GB); result size from {SA} §2 Spider case study (49.2 KB "
    f"Markdown) gzipped ~3x; Lightpanda 0.10 and protected HTTP 0.06 are {MA}")
inp("route_mix", {"js": {"http": 0.25, "lightpanda": 0.15, "chrome": 0.60},
                  "protected": {"http": 0.40, "browser": 0.60}},
    "share of pages per route", f"{MA}; evidence {WR} §5 (rendered pages add only 16-18% words; lab: raw HTML "
    f"matched rendering on 511 pages), {WR} §2 (learned routes at 0.4-0.8% of browser time), "
    f"{LAB}light-engine-paired-2026-10-09.md (Lightpanda only on compatible pages)")
inp("clearance_pages_per_browser", 20.0, "HTTP pages served per browser clearance on protected sites",
    f"{MA}; mechanism ADR 0020/0021 (browser-fetch keeps the Session's TLS fingerprint and cookies)")
inp("proxy_share", {"static": 0.20, "js": 0.50}, "share of bytes through datacenter proxies",
    f"{MA}; AWS ranges are classed as hosting ({PA} §2; {PN} §4)")
inp("dc_proxy_gb", 0.15, "$/GB datacenter proxy",
    f"{MA} within {IC} §1 datacenter range (per-GB plans $0.23-0.60; per-IP pools with 100 GB fair use "
    "$0.9-1.4/IP are far cheaper per GB)")
inp("res_proxy_gb", 2.00, "$/GB residential (COGS)",
    f"{IC} §1 Inferences ('$2.00-2.50/GB for premium pools ... $0.70-1.00/GB for budget pools' at 1-10 TB/month)")
inp("protected_success", 0.75, "share of attempts that succeed on protected targets",
    f"{IC} §1 (Proxyway: median 74.43% against protected targets)")
inp("failed_attempt_bw", 0.5, "bandwidth of a failed attempt vs a success", MA)
inp("captcha_rate", 0.10, "token solves per browser page load on protected sites",
    f"{MA}; {WR} §5 Gaps ('No reliable captcha encounter rate was found')")
inp("captcha_cost_per_1000", 1.20, "$ per 1,000 solves (COGS)",
    f"{IC} §4 Inferences ($0.60-1.30 reCAPTCHA/Turnstile; $1.20-1.30 Cloudflare challenge)")
inp("captcha_success", 0.95, "solve success", f"{IC} §4 (HasData: 86-100% on token types)")
inp("captcha_latency_s", 20.0, "seconds a browser waits per solve", f"{IC} §4 (HasData averages 6-93 s)")
inp("result_retention_months", 1.0, "months results stay in S3", f"{MA}; {HB} §1 retention 7-30 days typical")
inp("puts_per_1000_pages", 10.0, "S3 PUTs per 1,000 pages (100 pages per object)", MA)

# ---- Compute Units (Apify-style: 1 GB-hour with 0.25 vCPU) -----------------------------
inp("cu_shape", {"gb": 1.0, "vcpu": 0.25}, "per CU-hour", f"{SA} §1 (Apify: 1 CU = 1 GB for 1 hour; 1 core per 4,096 MB)")
inp("avg_run_gb", 2.0, "average Recipe run memory", f"{MA}; {SA} §1 (Apify: 4,096 MB 'a good middle ground', 1 GB minimum for browsers)")
inp("avg_run_hours", 0.25, "average Recipe run duration", f"{MA}; {WR} §7 (Apify example: 15-minute runs)")
inp("cu_cpu_active", 0.5, "CPU busy share of a Recipe run (AgentCore billing)", MA)
inp("cu_side_costs", {"egress_gb": 0.01, "storage_gb": 0.01}, "GB per CU-hour (results out, results kept 1 month)", MA)

# ---- AI agent task -----------------------------------------------------------------------
inp("agent_tokens", {"light": (100_000, 10_000), "heavy": (300_000, 30_000)},
    "(input, output) tokens per 10-step task", f"{HB} §3 Table N5 (light ~10k input tokens per step; heavy 3x)")
inp("agent_cache_share", 0.50, "share of input tokens read from prompt cache",
    f"{MA}; {HB} §3 Gaps ('input side could fall by about 90% with caching'); {IC} §5 toolset caching example")
inp("cache_write_share", 0.10, "share of input tokens written to cache at 1.25x", f"{IC} §5/§7 (5-min cache write 1.25x)")
inp("replay_factor", 4.5, "x fewer model tokens with compiled-plan replay",
    f"{WR} §6 and {LAB}ai-real-models-2026-10-09.md (3-6x fewer tokens, zero false successes)")
inp("agent_browser_minutes", {"agent": 3.0, "replay": 0.5}, "browser minutes per task",
    f"{WR} §6 Inferences (1-11 minutes per agent task); replay value is a {MA} from {WR} §2 "
    "(learned route: 10-20 ms instead of 0.8-1.3 s per run)")
inp("agent_proxy_mb", 10.0, "MB residential per task", f"{HB} §3 Table N5")
inp("agent_captcha_per_task", 0.05, "solves per task", MA)
inp("agent_search_per_task", 1.0, "SERP queries per task", MA)
inp("serp_cost_per_1000", 1.00, "$ per 1,000 queries", f"{IC} §8 (Serper $1.00 -> $0.30 by volume)")

# ---- LLM prices (per million tokens) ------------------------------------------------------
# (class, provider, input, cache read, output)
inp("llm_models", {
    "Claude Haiku 5.5":       ("cheap", "anthropic", 0.10, 0.01, 0.50),
    "GPT-6 Luna":             ("cheap", "openai", 0.10, 0.01, 0.50),
    "Gemini 3.1 Flash-Lite":  ("cheap", "google", 0.25, 0.025, 1.50),
    "gpt-oss-120b (Bedrock)": ("cheap", "bedrock-open", 0.15, 0.15, 0.60),
    "Claude Sonnet 5.5":      ("mid", "anthropic", 2.00, 0.10, 10.00),
    "GPT-6.1 Sol":            ("mid", "openai", 2.00, 0.10, 10.00),
    "Gemini 3.8 Flash (2026)": ("mid", "google", 0.75, 0.075, 3.75),
    "Gemini 3.8 Flash (2027)": ("mid", "google", 1.50, 0.15, 7.50),
    "Claude Opus 5.5":        ("frontier", "anthropic", 4.00, 0.20, 20.00),
    "GPT-6 Astra":            ("frontier", "openai", 10.00, 1.00, 50.00),
    "Claude Fable 5.1":       ("frontier", "anthropic", 10.00, 0.25, 50.00),
}, "$ per million tokens", f"{IC} §5 (Anthropic, OpenAI, Google tables) and §6; {PS} §8 (Bedrock tables)",
    "gpt-oss-120b has no Bedrock cache rate; Gemini 3.8 Flash doubles on 2027-01-01.")
inp("bedrock_regional_premium", 1.10, "x list price", f"{PS} §8 / {IC} §7 (regional endpoints +10%; global = list)",
    "Used for Claude in the eu-west-1 column (EU data routing). Bedrock lacks batch for the 5.5 models.")
inp("bedrock_open_eu_mult", {"gpt-oss-120b (Bedrock)": 1.183}, "eu-west-1 / us-east-1",
    f"{PS} §8 (gpt-oss-120b $0.18/$0.70 vs $0.15/$0.60)")

# ---- Market prices (what customers pay) ----------------------------------------------------
inp("mkt_browser_hour", {
    "Browserbase": (0.12, 0.10), "Browser Use": (0.02, 0.02), "Steel": (0.10, 0.08),
    "Hyperbrowser": (0.10, 0.10), "Kernel headless": (0.06, 0.06), "Anchor": (0.06, 0.06),
    "Browserless": (0.24, 0.18), "Cloudflare Browser Run": (0.09, 0.09), "Notte": (0.00, 0.00),
    "Lightpanda Cloud": (0.08, 0.08), "Scrapeless": (0.09, 0.09), "ZenRows": (0.127, 0.020),
    "TinyFish": (0.12, 0.12), "GoLogin": (0.09, 0.07), "BrowserCat": (0.21, 0.18),
    "Rebrowser (low end)": (0.20, 0.17), "Firecrawl interact": (0.456, 0.072),
}, "(entry plan, largest self-serve) marginal $/browser-hour", f"{HB} §3 Table N1/N2")
inp("mkt_browser_hour_headful", {"Kernel headful": (0.48, 0.48)}, "$/browser-hour", f"{HB} §1 Kernel / §3 Table N1")
inp("mkt_crawl", {
    # vendor: {class: (entry, largest)} in $ per 1,000 pages
    "Firecrawl": {"static": (3.80, 0.749), "js": (3.80, 0.749), "protected": (3.80, 0.749)},
    "ScrapingBee": {"static": (0.196, 0.052), "js": (0.980, 0.261), "protected": (4.90, 1.30)},
    "ScraperAPI": {"static": (0.490, 0.092), "js": (4.90, 0.919), "protected": (12.25, 2.30)},
    "ZenRows": {"static": (0.422, 0.067), "js": (2.11, 0.333), "protected": (10.56, 1.67)},
    "Scrapfly": {"static": (0.150, 0.091), "js": (0.900, 0.545), "protected": (4.50, 2.73)},
    "Scrape.do": {"static": (0.116, 0.070), "js": (0.580, 0.350), "protected": (2.90, 1.75)},
    "ScrapingAnt": {"static": (0.190, 0.075), "js": (1.90, 0.749), "protected": (23.75, 9.36)},
    "Crawlbase": {"static": (0.495, 0.299), "js": (0.990, 0.599), "protected": (1.98, 1.20)},
    "ScrapeGraphAI": {"static": (2.00, 0.667), "js": (2.00, 0.667), "protected": (12.00, 4.00)},
    "Diffbot": {"static": (1.20, 0.899), "js": (1.20, 0.899), "protected": (2.39, 1.80)},
    "Olostep": {"static": (1.80, 0.399), "js": (1.80, 0.399), "protected": (1.80, 0.399)},
    "Crawl4AI Cloud": {"static": (0.500, 0.400), "js": (1.00, 0.800), "protected": (2.00, 1.60)},
    "Bright Data Unlocker": {"static": (1.50, 1.30), "js": (1.50, 1.30), "protected": (1.50, 1.30)},
    "Oxylabs": {"static": (0.50, 0.35), "js": (1.50, 1.30), "protected": (1.50, 1.30)},
    "Zyte API (T1 / T4-T5 low)": {"static": (0.13, 0.06), "js": (1.01, 0.48), "protected": (8.04, 3.84)},
    "Jina Reader": {"static": (0.250, 0.227), "js": (0.250, 0.227)},
    "Apify WCC (rescaled)": {"static": (0.16, 0.104), "js": (0.40, 0.26)},
    "Decodo": {"static": (0.50, 0.207)},
}, "$ per 1,000 pages", f"{SA} §3 tables A, B and C (entry -> largest self-serve, monthly billing)",
    "Browse AI and Import.io (no-code) are excluded as outliers; Apify browser uses the low end of $0.40-4.00.")
inp("mkt_cu", {"Apify Free/Starter": 0.20, "Apify Scale": 0.16, "Apify Business": 0.13}, "$/CU",
    f"{SA} §1 'Plans and unit prices'", "Entry (Starter) $0.20; largest self-serve (Business) $0.13.")
inp("mkt_cu_sandbox_anchor", {"E2B / Daytona": 0.0288, "Hyperbrowser sandbox": 0.02625},
    "$ per 0.25 vCPU + 1 GB hour", f"{HB} §1 'Sandboxes sold for browser or computer use' (computed from per-vCPU and per-GiB rates)")
inp("mkt_agent_task", {
    # class: list of light-profile task prices across offers
    "cheap": [0.068, 0.137, 0.117, 0.023, 0.115, 0.117],
    "mid": [0.41, 0.42, 0.40, 0.31, 0.40, 0.42],
    "frontier": [0.77, 0.72],
    "flat_per_step_any_model": [0.30, 0.16, 0.24, 0.59],
}, "$ per 10-step task (light profile)", f"{HB} §3 Table N5 (Sonnet 5 stands for 'mid'; Haiku 5.5 'cheap'; Opus 5.5 'frontier')")
inp("mkt_proxy_gb", {
    "Apify": (8.0, 7.0), "Browserbase": (12.0, 10.0), "Steel": (10.0, 6.0), "Browser Use": (5.0, 5.0),
    "Anchor": (8.0, 8.0), "Notte": (10.0, 10.0), "Hyperbrowser": (10.0, 10.0),
    "Bright Data Browser API": (8.0, 7.03), "Oxylabs Agent Browser": (8.0, 5.50), "Browserless": (12.0, 9.0),
    "ZenRows (credits)": (10.56, 1.67), "Scrapeless": (1.80, 1.80),
}, "$/GB residential (entry, largest self-serve)", f"{IC} §2 table; {HB} §1 and §3 Inferences")
inp("mkt_captcha_1000", {"Steel": (3.0, 1.0), "Browserless (overage)": (20.0, 15.0)},
    "$ per 1,000 solves (entry, largest self-serve; marginal)", f"{IC} §4 / {HB} §1 Browserless 'Computed'",
    "Thin market: only two vendors price solves; Bright Data, Kernel, Notte, Scrapeless and Browserbase "
    "bundle them. Browserless at its included-unit rate is $12.50 -> $7.")
inp("mkt_llm_markup", {"Notte": 0.0, "Browserbase Model Gateway": 0.0, "Hugging Face": 0.0,
                       "Hyperbrowser": 0.05, "OpenRouter (credit fee)": 0.055, "Browser Use": 0.20},
    "markup over provider list", f"{HB} §2; {BM} §3; {IC} §6 (OpenRouter 5.5% fee)")
inp("direct_traffic_price_gb", 0.20, "$/GB charged for non-proxied browser traffic (variant only)",
    f"{HB} §1 (Browser Use and Anchor: own proxy or no proxy $0.20/GB)", "Not used in the base case: most vendors do not charge it.")
inp("cp_alloc_ref", {"concurrency": 1000.0, "browser_share": 1 / 3.0},
    "average concurrent Sessions; share of the control plane carried by hosted browsers", MA)
inp("price_positions", {"median": 1.0, "below (-25%)": 0.75, "premium (+25%)": 1.25},
    "x market median", "Task brief: median, 20-30% below (midpoint 25%), premium",
    "The premium is a MODEL ASSUMPTION (+25%) justified only by verified-result billing and self-serve stealth.")
inp("llm_markup_positions", {"median": None, "below (-25%)": 0.0, "premium (+25%)": 0.20},
    "markup over list for resold tokens at each position (None = market median)",
    f"{HB} §2 / {BM} §3 (0% Notte, Browserbase, Hugging Face; 5% Hyperbrowser; 20% Browser Use)",
    "Tokens priced below list would lose money on every token, so the 'below' position is pass-through at cost.")

# ---- Payments, marketplace, free tier, support ---------------------------------------------
inp("stripe", {"eea": (0.015, 0.25), "eea_premium": (0.028, 0.25), "uk": (0.025, 0.25),
               "intl": (0.0315, 0.25), "fx": 0.02, "billing": 0.007, "tax": 0.005,
               "managed_payments": 0.035, "sepa_dd_eur": 0.35, "invoicing": 0.004},
    "(rate, EUR fixed) per transaction", f"{BM} §7 'Payments and merchant of record' (Stripe France pricing)")
inp("card_mix", {"eea": 0.45, "eea_premium": 0.10, "uk": 0.05, "intl": 0.40}, "share of card revenue",
    f"{MA}; {BM} §2 (about half of Apify's revenue comes from the US)")
inp("fx_applies", False, "whether a 2% conversion fee applies to non-EUR cards",
    f"{MA} (USD settlement assumed); fee from {BM} §7")
inp("mor", {"stripe_mp_extra": 0.035, "paddle": (0.05, 0.50), "polar_scale": (0.034, 0.30)},
    "rate, $ fixed", f"{BM} §7 (Managed Payments +3.5%; Paddle 5% + 50c; Polar 3.4% + 30c)",
    "MoRs exclude marketplaces (Paddle policy; Stripe Managed Payments eligibility), so marketplace "
    "flows stay on Stripe Connect in every variant.")
inp("connect", {"active_account_eur": 2.0, "payout_rate": 0.0025, "payout_eur": 0.10}, "EUR",
    f"{BM} §5 'Payment rails for a marketplace' (Stripe Connect)")
inp("creator_share", 0.80, "share of marketplace event revenue paid to creators",
    f"{SA} §1 / {BM} §5 (Apify: profit = 0.8 x revenue - platform costs)")
inp("creators_per_paying_customer", 1 / 6.0, "active paid-out creators per paying customer",
    f"{BM} §5 and §2 (Apify: 4,500 community developers vs 27,000 paying customers)")
inp("free_tier", {"allowance_usd": 5.0, "use_of_allowance": 0.5, "abuse_mult": 1.5, "free_per_paying": 3.0},
    "monthly", f"allowance from {BM} §4 (Apify $5, Kernel $5); use, abuse and ratio are {MA} "
    "(Apify had ~1.07 free active users per payer at end-2025, BM §4)")
inp("support_tiers", {"business_plus": [(10_000, 0.09), (80_000, 0.07), (250_000, 0.05), (float("inf"), 0.03)],
                      "business_plus_min": 29.0,
                      "enterprise": [(150_000, 0.10), (500_000, 0.07), (1_000_000, 0.05), (float("inf"), 0.03)],
                      "enterprise_min": 5000.0},
    "(cumulative AWS spend cap, rate)", f"{PS} §9 'Current Support plans'")
inp("support_enterprise_above", 60_000.0, "AWS monthly bill above which Enterprise Support is bought", MA)

# ---- Fixed platform minimums (per production region unless noted) --------------------------
inp("control_plane", {
    "eks_clusters": 2,                 # control-plane services + data-plane fleet
    "cp_nodes": ("c7g.2xlarge", 3),    # API, dispatcher, billing, Console, gateway minimum (3 AZ)
    "aurora_acu": 4.0,                 # writer + reader at 2 ACU each, running 24/7
    "aurora_acu_h": {"us-east-1": 0.12, "eu-west-1": 0.14},
    "aurora_storage_gb": 200.0, "aurora_storage_gb_month": {"us-east-1": 0.10, "eu-west-1": 0.11},
    "valkey_gb": 2.0, "valkey_gb_h": {"us-east-1": 0.084, "eu-west-1": 0.094},
    "nat_gateways": 3, "nat_gb": 500.0,
    "load_balancers": 2,               # ALB for the API, NLB for Sessions
    "lcu_avg": 5.0,
    "interface_endpoints": 6, "azs": 3,
    "endpoint_az_h": {"us-east-1": 0.01, "eu-west-1": 0.011},
    "usage_events_gb": 50.0,           # batched into >=5 KB records (Firehose rounds each record to 5 KB)
    "firehose_gb": {"us-east-1": 0.029, "eu-west-1": 0.031},
    "kms_keys": 9,                     # per Region: 3 environments x 3 purposes
    "cloudwatch_logs_gb": 150.0, "custom_metrics": 1000,
    "security_baseline": 600.0,        # GuardDuty, Security Hub, Config, CloudTrail, KMS, WAF
    "s3_gb": 2000.0,
    "staging_share": 0.35,             # staging environment as a share of one production region
}, "counts and $", f"prices from {PS} §3-§7 and {PN} §9 (KMS $1/key-month); topology from {PN} §9 "
    f"'Cross-cutting: recommended control plane and data plane' (Firehose into S3 Iceberg, no Kinesis "
    f"below 10-25 MB/s per {PN} §1) and {RT} Q5/Q7; counts and sizes are {MA}",
    "GuardDuty, Security Hub, Config, CloudTrail and WAF have no price in any note ({PN} §9 Gaps): "
    "the $600 lump sum is a MODEL ASSUMPTION.")
inp("observability_saas", {"sentry_team": 26.0, "grafana_pro": 19.0, "per_1000_series": 6.50,
                           "base_series": 20_000, "series_per_payer": 5, "cp_logs_gb": 300.0},
    "$/month and counts", f"prices from {PN} §7 (Sentry Team $26; Grafana Pro $19 + $6.50 per 1,000 series); "
    f"series counts and control-plane log volume are {MA}")
inp("enterprise_sso_usd", 125.0, "$ per enterprise SSO connection per month",
    f"{PN} §2 (WorkOS SSO $125/connection for the first 15; AuthKit free to 1M MAU)")
inp("trust_safety", {"fte_cost_month": 8500.0, "base_fte": 1.5, "payers_per_fte": 4000.0,
                     "kyc_check_usd": 1.5, "kyc_share_of_new_payers": 0.25},
    "$ and counts", f"{MA}; need from {PN} §5 (AWS may block resources if an abuse notice is unanswered "
    f"for 24 hours; KYC tiers for proxy tiers and high concurrency), {IC} §3 (KYC as customer of record), "
    f"{BM} §7; no note gives a staffing or cost figure ({PN} §5, {BM} §7 Gaps)")

# ---- Alternative providers for the hybrid layout ---------------------------------------------
inp("alt_nodes", {
    "OVH SCALE-a8 768GB (FR)": {"threads": 384, "gb": 768, "month": 2783.99 * 1.1206},
    "OVH SCALE-a8 768GB (US)": {"threads": 384, "gb": 768, "month": 3566.0},
    "Hetzner AX162-2": {"threads": 96, "gb": 256, "month": 842.30 * 1.1206, "setup": 419 * 1.1206},
    "Scaleway EM-I620E": {"threads": 128, "gb": 576, "month": 679.99 * 1.1206},
}, "per node", f"{PA} §1 table and §6 node counts (SCALE-a8 holds 326 R; AX162-2 81; EM-I620E 108)",
    "SMT threads counted as vCPU, as in PA §6; see smt_factor.")
inp("oci_flex", {"A1": (0.01, 0.0015, 1.0), "E5_preemptible": (0.015, 0.001, 2.0)},
    "($/OCPU-h, $/GB-h, vCPU per OCPU)", f"{PA} §1 (A1 $0.01/OCPU-h + $0.0015/GB-h; E5 $0.03 + $0.002, 50% off preemptible)")
inp("smt_factor", 1.0, "useful share of an x86 hardware thread vs a Graviton vCPU",
    f"{PA} §6 'SMT sensitivity' (0.65 tested there); 1.0 is the PA §6 base")
inp("baremetal_ops", {"fixed_month": 8000.0, "per_node_month": 50.0}, "$", f"{MA}; {PA} §6 Gaps "
    "(staff and operations costs of bare metal are not quantified; 37signals claims 'no new staff')")
inp("demand_curve", [0.45, 0.42, 0.40, 0.40, 0.42, 0.48, 0.55, 0.65, 0.75, 0.85, 0.92, 0.97,
                     1.00, 0.98, 0.95, 0.92, 0.88, 0.82, 0.75, 0.68, 0.62, 0.56, 0.51, 0.47],
    "hourly demand as a share of daily peak", f"{MA}; replaces the '60% base, 40% burst for 25% of hours' "
    f"illustration of {PA} §6 (average 0.70); this curve averages ~0.69")
inp("hybrid_base_share", 0.60, "bare-metal base as a share of peak", f"{PA} §6 'Hybrid illustration'")
inp("oci_egress", (10_000.0, 0.0085), "(free GB per month, $/GB after)", f"{PA} §1 'Hyperscaler network reference' "
    "($0 first tier then $0.0085/GB; 10 TB free tier not re-verified there)")
inp("oci_ops_share", 0.5, "share of the bare-metal fixed operations cost a second elastic cloud needs", MA)

# ---- Revenue mixes, segments and scenarios ---------------------------------------------------
inp("line_profile_mix", {"lean headless": 0.50, "rich headless": 0.30, "rich headful": 0.20},
    "share of billed browser-hours", MA)
inp("crawl_class_mix", {"static": 0.55, "js": 0.30, "protected": 0.15}, "share of pages", MA)
inp("agent_model_mix", {"cheap": 0.60, "mid": 0.30, "frontier": 0.10}, "share of agent tasks", MA)
inp("agent_replay_share", 0.50, "share of agent tasks served by compiled replay", MA)
inp("revenue_mixes", {
    "proxy-heavy":       {"browser": 0.12, "crawl": 0.15, "cu": 0.08, "agent": 0.05, "proxy": 0.45, "captcha": 0.05, "llm": 0.03, "marketplace": 0.07},
    "compute-heavy":     {"browser": 0.25, "crawl": 0.20, "cu": 0.30, "agent": 0.05, "proxy": 0.10, "captcha": 0.02, "llm": 0.02, "marketplace": 0.06},
    "LLM-heavy":         {"browser": 0.12, "crawl": 0.10, "cu": 0.05, "agent": 0.35, "proxy": 0.10, "captcha": 0.02, "llm": 0.25, "marketplace": 0.01},
    "marketplace-heavy": {"browser": 0.06, "crawl": 0.08, "cu": 0.25, "agent": 0.03, "proxy": 0.15, "captcha": 0.02, "llm": 0.01, "marketplace": 0.40},
}, "share of gross revenue (marketplace = Recipe event revenue before creator payouts)", MA)
inp("segments", {
    # ARPU $/month at start, monthly ARPU growth, monthly logo churn, revenue mix used
    "hobbyist":   {"arpu": 25.0, "arpu_growth": 0.000, "churn": 0.06, "mix": "compute-heavy"},
    "agent dev":  {"arpu": 120.0, "arpu_growth": 0.005, "churn": 0.04, "mix": "LLM-heavy"},
    "data team":  {"arpu": 450.0, "arpu_growth": 0.010, "churn": 0.025, "mix": "marketplace-heavy"},
    "enterprise": {"arpu": 5000.0, "arpu_growth": 0.010, "churn": 0.01, "mix": "proxy-heavy"},
}, "per segment", f"ARPU anchors: {SIG} §1 (hobbyists ~$15-190/month; agent stacks $30-400; data team ~$310; "
    f"sales-led $2,500/month; enterprise $5.9k-10k/month vendor claims); {BM} §4 (Apify ~$96/month per payer; "
    f"ChartMogul $50-249 band); churn and growth are {MA}")
inp("self_serve_mix", {"hobbyist": 0.60, "agent dev": 0.28, "data team": 0.12}, "share of new self-serve payers", MA)
inp("scenarios", {
    "conservative": {"signups_m1": 800, "g_start": 0.06, "g_end": 0.01, "conv": 0.03, "lag": 2,
                     "ent_start": 12, "ent_per_month": (0.25, 0.75), "price_pos": 0.75},
    "base":         {"signups_m1": 2000, "g_start": 0.09, "g_end": 0.02, "conv": 0.05, "lag": 2,
                     "ent_start": 9, "ent_per_month": (0.5, 2.0), "price_pos": 1.0},
    "upside":       {"signups_m1": 3000, "g_start": 0.11, "g_end": 0.02, "conv": 0.07, "lag": 2,
                     "ent_start": 6, "ent_per_month": (1.0, 3.0), "price_pos": 1.0},
}, "funnel inputs", f"conversion: {BM} §4 (ChartMogul freemium 'good' 3-5%, 'great' 8-12%; AI-native 6-8%); "
    f"sign-up volumes, growth and enterprise pace are {MA}, checked against {BM} §2 (Apify $31M ARR, "
    "27,000 payers; Bessemer 'Shooting Star' ~$3M/$12M/$40M ARR in years 1-3)")
inp("second_region_month", 7, "month eu-west-1 production opens", MA)
inp("aws_share_by_line", {"browser": 0.95, "crawl": 0.10, "cu": 0.95, "agent": 0.10, "proxy": 0.0,
                          "captcha": 0.0, "llm": 0.0, "marketplace": 0.0},
    "share of each line's COGS billed by AWS (drives the AWS Support fee)",
    f"{MA}; models bought direct (Claude Platform on AWS bills via Marketplace, {IC} §7); proxies and solvers are third parties")


# =====================================================================================
# MODEL. Everything below is arithmetic on the inputs above.
# =====================================================================================

def params():
    return {k: copy.deepcopy(v["value"]) for k, v in REG.items()}


def node(p, region, itype):
    keys = ("vcpu", "gib", "od", "spot_snap", "adv", "band", "csp1", "csp3", "isp1", "isp3")
    return dict(zip(keys, p["ec2"][region][itype]))


def spot_discount(p, n):
    basis = p["spot_basis"]
    if basis == "advisor":
        return n["adv"] if n["adv"] is not None else 1 - n["spot_snap"] / n["od"]
    if basis == "snapshot":
        return 1 - n["spot_snap"] / n["od"]
    return float(basis)


def node_hour(p, region, itype, purchase):
    n = node(p, region, itype)
    if purchase == "od":
        return n["od"]
    if purchase == "spot":
        return n["od"] * (1 - spot_discount(p, n)) * (1 + p["spot_overhead"])
    return n["od"] * (1 - n[purchase])


def slots(n, vcpu, gb, packing, ovh_gb=0.0, ovh_vcpu=0.0, cpu_perf=1.0):
    need_cpu = vcpu / cpu_perf + ovh_vcpu
    need_gb = gb + ovh_gb
    return math.floor(min(n["vcpu"] * packing / need_cpu, n["gib"] * packing / need_gb))


def best_slot(p, region, families, purchase, vcpu, gb, ovh_gb=0.0, ovh_vcpu=0.0, cpu_perf=1.0):
    best = None
    for it in families:
        n = node(p, region, it)
        s = slots(n, vcpu, gb, p["packing"], ovh_gb, ovh_vcpu, cpu_perf)
        if s <= 0:
            continue
        c = node_hour(p, region, it, purchase) / s
        if best is None or c < best[0]:
            best = (c, it, s)
    return best


def egress_cost(p, monthly_gb):
    gb = max(0.0, monthly_gb - p["egress_free_gb"])
    cost, prev = 0.0, 0.0
    for cap, rate in p["egress_tiers"]:
        take = min(gb, cap - prev)
        if take <= 0:
            break
        cost += take * rate
        gb -= take
        prev = cap
    return cost


def mix_purchase_cost(p, region, families, kind, vcpu, gb, **kw):
    """Slot cost for a capacity mix: interactive = SP share + on-demand; batch = Spot share + on-demand."""
    od = best_slot(p, region, families, "od", vcpu, gb, **kw)
    if kind == "interactive":
        sp = best_slot(p, region, families, p["interactive_sp"], vcpu, gb, **kw)
        s = p["sp_share_interactive"]
        return s * sp[0] + (1 - s) * od[0], od[1]
    spot = best_slot(p, region, families, "spot", vcpu, gb, **kw)
    s = p["spot_share_batch"]
    return s * spot[0] + (1 - s) * od[0], spot[1]


# ---------------------------------------------------------------- (a) hosted browser-hour
BROWSER_DEPLOYS = [
    ("EKS+Karpenter Graviton, On-Demand", "ec2", "od"),
    ("EKS+Karpenter Graviton, 1-yr Compute SP", "ec2", "csp1"),
    ("EKS+Karpenter Graviton, 3-yr EC2 Instance SP", "ec2", "isp3"),
    ("EKS+Karpenter Graviton, Spot (batch Sessions)", "ec2", "spot"),
    ("EKS+Karpenter, interactive mix (60% SP + OD)", "ec2", "mix-interactive"),
    ("Firecracker per Session, Graviton .metal On-Demand", "fc_metal", "od"),
    ("Firecracker per Session, Graviton .metal Spot", "fc_metal", "spot"),
    ("Firecracker per Session, Intel nested virtualization On-Demand", "fc_nested", "od"),
    ("Firecracker per Session, Intel nested virtualization Spot", "fc_nested", "spot"),
    ("Kata, Intel nested virtualization On-Demand", "nested", "od"),
    ("Fargate ARM On-Demand", "fargate", "od"),
    ("Fargate ARM Spot", "fargate", "spot"),
    ("Lambda MicroVMs (Arm)", "lmvm", None),
    ("AgentCore Browser v1 (AWS Chrome; reference)", "agentcore", None),
]


def gateway_cost_per_h(p, region, headful, concurrency=1000.0):
    traffic = p["traffic_per_browser_h"]
    gb_h = traffic["egress_liveview"] if headful else traffic["egress_base"]
    nlb_fixed = p["nlb"][region] / concurrency           # one NLB shared by the Region's Sessions
    nlcu = gb_h / p["nlcu_gb_per_h"] * p["nlcu_h"]       # processed bytes dimension
    per_vcpu = node(p, region, "c7g.2xlarge")["od"] / 8.0
    gw = per_vcpu / p["gw_sessions_per_vcpu"]["headful" if headful else "headless"]
    return nlb_fixed + nlcu + gw


def browser_hour(p, region, profile, deploy, purchase, util=None, concurrency=1000.0):
    prof = p["browser_profiles"][profile]
    vcpu, gb, headful = prof["vcpu"], prof["gb"], prof["headful"]
    U = p["util_elastic"] if util is None else util
    c = {}
    pool_cost = 0.0
    host = ""
    if deploy in ("ec2", "fc_metal", "nested", "fc_nested"):
        fam = {"ec2": p["browser_families"], "fc_metal": p["metal_families"], "nested": p["nested_families"],
               "fc_nested": p["nested_families"]}[deploy]
        kw = {}
        pool_gb = p["pool_mem_container_gb"]
        if deploy == "fc_metal":
            kw = {"ovh_gb": p["fc_vm_overhead_gb"]}
            pool_gb = p["pool_mem_microvm_gb"]
        if deploy == "fc_nested":
            kw = {"ovh_gb": p["fc_vm_overhead_gb"], "cpu_perf": p["nested_cpu_perf"]}
            pool_gb = p["pool_mem_microvm_gb"]
        if deploy == "nested":
            kw = {"ovh_gb": p["kata_overhead"]["gb"], "ovh_vcpu": p["kata_overhead"]["vcpu"],
                  "cpu_perf": p["nested_cpu_perf"]}
            pool_gb = p["pool_mem_microvm_gb"]
        if purchase == "mix-interactive":
            slot, host = mix_purchase_cost(p, region, fam, "interactive", vcpu, gb, **kw)
        else:
            slot, host, _ = best_slot(p, region, fam, purchase, vcpu, gb, **kw)
        c["compute"] = slot / U
        pool_cost = p["pool_frac"] * (pool_gb / gb) * slot
    elif deploy == "fargate":
        f = p["fargate"]
        task = vcpu * f["arm_vcpu_h"] + gb * f["arm_gb_h"]
        if purchase == "spot":
            task *= (1 - p["fargate_spot_discount"])
        c["compute"] = task * (1 + p["fargate_pool_frac"])
        c["public IPv4"] = p["public_ipv4_h"] * (1 + p["fargate_pool_frac"])
        host = "Fargate task"
    elif deploy == "lmvm":
        r = p["lmvm"][region]
        v = max(vcpu, gb / 2.0)
        c["compute"] = (v * r["vcpu_s"] + gb * r["gb_s"]) * 3600
        starts_per_h = 60.0 / p["avg_session_minutes"]
        c["snapshot fees"] = starts_per_h * gb * r["snap_read_gb"]
        host = "Lambda MicroVM"
    elif deploy == "agentcore":
        a = p["agentcore"]
        c["compute"] = vcpu * p["agent_session_cpu_active"] * a["v1_vcpu_h"] + gb * a["v1_gb_h"]
        host = "AgentCore"
    c["warm pool"] = pool_cost
    traffic = p["traffic_per_browser_h"]
    eg = traffic["egress_liveview"] if headful else traffic["egress_base"]
    c["egress"] = eg * p["egress_rate_unit"]
    c["gateway+NLB"] = 0.0 if deploy in ("lmvm", "agentcore") else gateway_cost_per_h(p, region, headful, concurrency)
    c["observability"] = p["logs_gb"]["browser_h"] * p["log_price"][region]
    c["recordings"] = p["recording_gb_per_session_h"] * p["s3"]["gb_month"]
    c["total"] = sum(c.values())
    c["host"] = host
    return c


# ---------------------------------------------------------------- (b) crawl API, per 1,000 pages
def vcpu_hour_price(p, region, kind="batch"):
    """$ per busy vCPU-hour on the crawl fleet (Spot mix, packing, utilization)."""
    best = None
    for it in p["crawl_families"]:
        n = node(p, region, it)
        od = n["od"] / (n["vcpu"] * p["packing"])
        sp = node_hour(p, region, it, "spot") / (n["vcpu"] * p["packing"])
        s = p["spot_share_batch"] if kind == "batch" else 0.0
        v = s * sp + (1 - s) * od
        if best is None or v < best[0]:
            best = (v, it)
    return best[0] / p["util_batch"], best[1]


def gb_hour_price(p, region):
    n = node(p, region, "m8g.4xlarge")
    s = p["spot_share_batch"]
    v = s * node_hour(p, region, "m8g.4xlarge", "spot") + (1 - s) * n["od"]
    return v / (n["gib"] * p["packing"]) / p["util_batch"]


def rich_slot_per_s(p, region):
    return browser_hour(p, region, "rich headless", "ec2", "spot", util=p["util_batch"])["compute"] / 3600.0


def crawl_1000(p, region, cls):
    cpu, g = p["cpu_ms"], p["gb_per_1000_pages"]
    vh, host = vcpu_hour_price(p, region)
    c = {}
    http_ms = cpu["http_fetch"] + cpu["tls_decompress"] + cpu["extraction"]
    if cls == "static":
        cpu_s = 1000 * http_ms / 1000.0
        c["compute"] = cpu_s / 3600 * vh + 1000 * p["run_residency_gb_s_per_page"] / 3600 * gb_hour_price(p, region)
        c["proxy"] = p["proxy_share"]["static"] * g["html"] * p["dc_proxy_gb"]
    elif cls == "js":
        m = p["route_mix"]["js"]
        per_page_ms = (m["http"] * http_ms
                       + m["lightpanda"] * (cpu["lightpanda_page"] + cpu["extraction"])
                       + m["chrome"] * (cpu["chrome_js_page"] + cpu["extraction"] + cpu["browser_start_per_100_pages"] / 100))
        c["compute"] = 1000 * per_page_ms / 1000.0 / 3600 * vh
        gb = m["http"] * g["html"] + m["lightpanda"] * g["lightpanda"] + m["chrome"] * g["js_blocked"]
        c["proxy"] = p["proxy_share"]["js"] * gb * p["dc_proxy_gb"]
    elif cls == "protected":
        m = p["route_mix"]["protected"]
        attempts = 1.0 / p["protected_success"]
        bw_factor = 1 + (attempts - 1) * p["failed_attempt_bw"]
        browser_loads = m["browser"] + m["http"] / p["clearance_pages_per_browser"]  # per page
        per_page_ms = (m["http"] * http_ms
                       + browser_loads * (cpu["chrome_protected_page"] + cpu["browser_start_per_100_pages"] / 100)
                       + m["browser"] * cpu["extraction"]) * attempts
        c["compute"] = 1000 * per_page_ms / 1000.0 / 3600 * vh
        gb = (m["http"] * g["protected_http"] + browser_loads * g["protected_browser"]) * bw_factor
        c["residential proxy"] = gb * p["res_proxy_gb"]
        solves = 1000 * browser_loads * p["captcha_rate"] / p["captcha_success"]
        c["captcha"] = solves / 1000 * p["captcha_cost_per_1000"]
        c["captcha wait"] = 1000 * browser_loads * p["captcha_rate"] * p["captcha_latency_s"] * rich_slot_per_s(p, region)
    c["result egress"] = g["result"] * p["egress_rate_unit"]
    c["storage"] = g["result"] * p["s3"]["gb_month"] * p["result_retention_months"] + \
        p["puts_per_1000_pages"] * p["s3"]["put_per_1000"] / 1000
    c["front door"] = 1000 / p["alb_requests_per_lcu_h"] * p["alb_lcu_h"]
    c["observability"] = p["logs_gb"]["per_1000_pages"] * p["log_price"][region]
    c["total"] = sum(v for k, v in c.items())
    c["host"] = host
    return c


def crawl_1000_alt(p, region, cls, deploy):
    """Compute-only alternatives for the same page mix (Fargate ARM Spot, Lambda Arm)."""
    base = crawl_1000(p, region, cls)
    cpu = p["cpu_ms"]
    if deploy == "fargate-spot":
        f = p["fargate"]
        per_vcpu = f["arm_vcpu_h"] * (1 - p["fargate_spot_discount"]) + 2 * f["arm_gb_h"] * (1 - p["fargate_spot_discount"])
        ratio = (per_vcpu / 0.9) / vcpu_hour_price(p, region)[0]   # Fargate task ~90% busy vs EC2 busy vCPU-hour
        return base["total"] - base["compute"] + base["compute"] * ratio
    if deploy == "lambda":
        lam = p["lambda"]
        # 1,769 MB = 1 vCPU; static pages use 0.5 GB for 1 s; browser pages 2 GB for wall time ~3x CPU
        if cls == "static":
            gbs = 1000 * 0.5 * 1.0
        elif cls == "js":
            m = p["route_mix"]["js"]
            gbs = 1000 * (m["http"] * 0.5 * 1.0 + m["lightpanda"] * 0.5 * 1.0 + m["chrome"] * 2.0 * 3 * cpu["chrome_js_page"] / 1000)
        else:
            m = p["route_mix"]["protected"]
            gbs = 1000 * (m["http"] * 0.5 * 1.0 + m["browser"] * 2.0 * 6 * cpu["chrome_protected_page"] / 1000) / p["protected_success"]
        lam_cost = gbs * lam["arm_gb_s"] + 1000 * lam["req"]
        return base["total"] - base["compute"] + lam_cost
    raise ValueError(deploy)


# ---------------------------------------------------------------- (c) Compute Unit
CU_DEPLOYS = [
    ("Firecracker on Graviton .metal, On-Demand", "fc_metal", "od"),
    ("Firecracker on Graviton .metal, 3-yr EC2 Instance SP", "fc_metal", "isp3"),
    ("Firecracker on Graviton .metal, Spot", "fc_metal", "spot"),
    ("Firecracker on Graviton .metal, batch mix (80% Spot)", "fc_metal", "mix-batch"),
    ("Firecracker on Intel nested virtualization, On-Demand", "fc_nested", "od"),
    ("Firecracker on Intel nested virtualization, Spot", "fc_nested", "spot"),
    ("Kata on Intel nested virtualization, On-Demand", "nested", "od"),
    ("Kata on Intel nested virtualization, Spot", "nested", "spot"),
    ("Fargate ARM On-Demand (0.25 vCPU, 1 GB task)", "fargate", "od"),
    ("Fargate ARM Spot", "fargate", "spot"),
    ("Lambda MicroVMs (Arm, 0.5 vCPU per GB)", "lmvm", None),
    ("Lambda Arm (runs under 15 min)", "lambda", None),
    ("AgentCore Runtime v1 (50% CPU busy)", "agentcore", None),
]


def cu_cost(p, region, deploy, purchase):
    sh = p["cu_shape"]
    run_gb = p["avg_run_gb"]
    c = {}
    if deploy in ("fc_metal", "nested", "fc_nested"):
        fam = p["metal_families"] if deploy == "fc_metal" else p["nested_families"]
        best = None
        for it in fam:
            n = node(p, region, it)
            if deploy == "fc_metal":
                ovh_gb, ovh_vcpu, perf = p["fc_vm_overhead_gb"], 0.0, 1.0
            elif deploy == "fc_nested":
                ovh_gb, ovh_vcpu, perf = p["fc_vm_overhead_gb"], 0.0, p["nested_cpu_perf"]
            else:
                ovh_gb, ovh_vcpu, perf = p["kata_overhead"]["gb"], p["kata_overhead"]["vcpu"], p["nested_cpu_perf"]
            runs_by_mem = n["gib"] * p["packing"] / (run_gb + ovh_gb)
            runs_by_cpu = n["vcpu"] * p["packing"] / (run_gb * sh["vcpu"] / perf + ovh_vcpu)
            cu_capacity = min(runs_by_mem, runs_by_cpu) * run_gb
            if purchase == "mix-batch":
                s = p["spot_share_batch"]
                price = s * node_hour(p, region, it, "spot") + (1 - s) * n["od"]
            else:
                price = node_hour(p, region, it, purchase)
            v = price / cu_capacity
            if best is None or v < best[0]:
                best = (v, it)
        c["compute"] = best[0] / p["util_cu"]
        host = best[1]
    elif deploy == "fargate":
        f = p["fargate"]
        task = sh["vcpu"] * f["arm_vcpu_h"] + sh["gb"] * f["arm_gb_h"]
        if purchase == "spot":
            task *= (1 - p["fargate_spot_discount"])
        c["compute"] = task / 0.9                                  # 1-minute minimum and ~30 s start
        c["public IPv4"] = p["public_ipv4_h"] / run_gb
        host = "Fargate task"
    elif deploy == "lmvm":
        r = p["lmvm"][region]
        c["compute"] = (sh["gb"] / 2.0 * r["vcpu_s"] + sh["gb"] * r["gb_s"]) * 3600
        c["snapshot fees"] = r["snap_read_gb"] / p["avg_run_hours"]
        host = "Lambda MicroVM"
    elif deploy == "lambda":
        c["compute"] = sh["gb"] * 3600 * p["lambda"]["arm_gb_s"]
        host = "Lambda"
    elif deploy == "agentcore":
        a = p["agentcore"]
        c["compute"] = sh["vcpu"] * p["cu_cpu_active"] * a["v1_vcpu_h"] + sh["gb"] * a["v1_gb_h"]
        host = "AgentCore"
    sc = p["cu_side_costs"]
    c["egress"] = sc["egress_gb"] * p["egress_rate_unit"]
    c["storage"] = sc["storage_gb"] * p["s3"]["gb_month"]
    c["observability"] = p["logs_gb"]["cu_h"] * p["log_price"][region]
    c["total"] = sum(c.values())
    c["host"] = host
    return c


# ---------------------------------------------------------------- (d) AI agent task
def model_cost(p, region, model, tin, tout):
    cls, prov, pin, pcache, pout = p["llm_models"][model]
    cs = p["agent_cache_share"]
    cost_in = tin * ((1 - cs) * pin + cs * pcache) + tin * p["cache_write_share"] * 0.25 * pin
    cost = (cost_in + tout * pout) / 1e6
    if region == "eu-west-1":
        if prov == "anthropic":
            cost *= p["bedrock_regional_premium"]
        cost *= p["bedrock_open_eu_mult"].get(model, 1.0)
    return cost


def agent_task(p, region, model, profile="light", replay=False):
    tin, tout = p["agent_tokens"][profile]
    c = {}
    m = model_cost(p, region, model, tin, tout)
    c["model tokens"] = m / p["replay_factor"] if replay else m
    minutes = p["agent_browser_minutes"]["replay" if replay else "agent"]
    bh = browser_hour(p, region, "lean headless", "ec2", "mix-interactive")["total"]
    c["browser time"] = minutes / 60.0 * bh
    c["residential proxy"] = p["agent_proxy_mb"] / 1000.0 * p["res_proxy_gb"]
    c["captcha"] = p["agent_captcha_per_task"] * p["captcha_cost_per_1000"] / 1000
    c["search"] = p["agent_search_per_task"] * p["serp_cost_per_1000"] / 1000
    c["total"] = sum(c.values())
    return c


# ---------------------------------------------------------------- (e) resale units
def resale(p, region):
    out = {}
    out["residential GB"] = {"cogs": p["res_proxy_gb"]}
    out["1,000 captcha solves"] = {"cogs": p["captcha_cost_per_1000"] / p["captcha_success"]}
    # 1M tokens at a 90/10 input/output split, cheap and mid model, no caching
    for model in ("Claude Haiku 5.5", "Claude Sonnet 5.5", "Claude Opus 5.5"):
        _, prov, pin, _, pout = p["llm_models"][model]
        cost = 0.9 * pin + 0.1 * pout
        if region == "eu-west-1" and prov == "anthropic":
            cost *= p["bedrock_regional_premium"]
        out[f"1M tokens {model} (90/10)"] = {"cogs": cost}
    return out


# ---------------------------------------------------------------- market prices
def med(xs):
    return statistics.median(xs) if xs else float("nan")


def market(p):
    m = {}
    bh = p["mkt_browser_hour"]
    m["browser-hour"] = {"entry": [v[0] for v in bh.values()], "largest": [v[1] for v in bh.values()]}
    for cls in ("static", "js", "protected"):
        rows = [v[cls] for v in p["mkt_crawl"].values() if cls in v]
        m[f"1,000 pages {cls}"] = {"entry": [r[0] for r in rows], "largest": [r[1] for r in rows]}
    cu = p["mkt_cu"]
    m["Compute Unit"] = {"entry": [cu["Apify Free/Starter"]], "largest": [cu["Apify Business"]],
                         "all": list(cu.values())}
    for k in ("cheap", "mid", "frontier"):
        xs = p["mkt_agent_task"][k]
        m[f"agent task ({k} model)"] = {"entry": xs, "largest": xs}
    pg = p["mkt_proxy_gb"]
    m["residential GB"] = {"entry": [v[0] for v in pg.values()], "largest": [v[1] for v in pg.values()]}
    cp = p["mkt_captcha_1000"]
    m["1,000 captcha solves"] = {"entry": [v[0] for v in cp.values()], "largest": [v[1] for v in cp.values()]}
    out = {}
    for k, v in m.items():
        allv = v.get("all", v["entry"] + v["largest"])
        out[k] = {"entry_median": med(v["entry"]), "largest_median": med(v["largest"]),
                  "median_all": med(allv), "min": min(allv), "max": max(allv)}
    return out


# ---------------------------------------------------------------- unit COGS used for margins
def unit_cogs(p, region):
    """Recommended AWS deployment per unit (the base case of the margin tables)."""
    u = {}
    mixb = p["line_profile_mix"]
    bh = sum(w * browser_hour(p, region, prof, "ec2", "mix-interactive")["total"] for prof, w in mixb.items())
    u["browser-hour"] = bh
    u["browser-hour lean headless"] = browser_hour(p, region, "lean headless", "ec2", "mix-interactive")["total"]
    u["browser-hour rich headless"] = browser_hour(p, region, "rich headless", "ec2", "mix-interactive")["total"]
    u["browser-hour rich headful"] = browser_hour(p, region, "rich headful", "ec2", "mix-interactive")["total"]
    for cls in ("static", "js", "protected"):
        u[f"1,000 pages {cls}"] = crawl_1000(p, region, cls)["total"]
    u["Compute Unit"] = cu_cost(p, region, "fc_metal", "mix-batch")["total"]
    rep = p["agent_replay_share"]
    for k, model in (("cheap", "Claude Haiku 5.5"), ("mid", "Claude Sonnet 5.5"), ("frontier", "Claude Opus 5.5")):
        a = agent_task(p, region, model)["total"]
        r = agent_task(p, region, model, replay=True)["total"]
        u[f"agent task ({k} model)"] = a
        u[f"agent task ({k} model) replay"] = r
        u[f"agent task ({k} model) blended"] = (1 - rep) * a + rep * r
    rs = resale(p, region)
    u["residential GB"] = rs["residential GB"]["cogs"]
    u["1,000 captcha solves"] = rs["1,000 captcha solves"]["cogs"]
    return u


def gm(price, cogs):
    return (price - cogs) / price if price > 0 else float("nan")


def llm_markup_at(p, pos):
    mark = med(list(p["mkt_llm_markup"].values()))
    if pos < 1.0:
        return p["llm_markup_positions"]["below (-25%)"]
    if pos > 1.0:
        return p["llm_markup_positions"]["premium (+25%)"]
    return mark


def line_economics(p, region, pos=1.0, basis="largest_median", direct_traffic=False):
    """Revenue-line gross margin (before payment fees, payouts and fixed costs) at a price position."""
    mk = market(p)
    u = unit_cogs(p, region)
    lines = {}
    # browser-hours: price at the market median of the matching tier
    price_bh = mk["browser-hour"][basis] * pos
    rev = price_bh
    if direct_traffic:
        t = p["traffic_per_browser_h"]
        hf = sum(w for k, w in p["line_profile_mix"].items() if p["browser_profiles"][k]["headful"])
        gb = t["ingress"] + hf * t["egress_liveview"] + (1 - hf) * t["egress_base"]
        rev += gb * p["direct_traffic_price_gb"] * pos
    lines["browser"] = gm(rev, u["browser-hour"])
    cm = p["crawl_class_mix"]
    rev = sum(w * mk[f"1,000 pages {c}"][basis] * pos for c, w in cm.items())
    cost = sum(w * u[f"1,000 pages {c}"] for c, w in cm.items())
    lines["crawl"] = gm(rev, cost)
    lines["cu"] = gm(mk["Compute Unit"][basis] * pos, u["Compute Unit"])
    am = p["agent_model_mix"]
    rev = sum(w * mk[f"agent task ({k} model)"]["largest_median"] * pos for k, w in am.items())
    cost = sum(w * u[f"agent task ({k} model) blended"] for k, w in am.items())
    lines["agent"] = gm(rev, cost)
    lines["proxy"] = gm(mk["residential GB"][basis] * pos, u["residential GB"])
    lines["captcha"] = gm(mk["1,000 captcha solves"][basis] * pos, u["1,000 captcha solves"])
    mark = llm_markup_at(p, pos)
    lines["llm"] = mark / (1 + mark)
    # Marketplace: Recipe event revenue; Goodfetch keeps (1 - creator share); platform usage is
    # billed separately under 'cu' and the other lines, so the event revenue itself has no COGS.
    lines["marketplace"] = 1 - p["creator_share"]
    return lines


# ---------------------------------------------------------------- payments and overheads
def payment_rate(p, arpu, variant="stripe", marketplace_share=0.0):
    """Effective payment cost as a share of revenue, for one charge of `arpu` per customer-month."""
    s, mix, eur = p["stripe"], p["card_mix"], p["eur_usd"]
    pct = sum(mix[k] * s[k][0] for k in mix)
    if p["fx_applies"]:
        pct += mix["intl"] * s["fx"]
    fixed = s["eea"][1] * eur / arpu
    base = pct + fixed + s["billing"] + s["tax"]
    if variant == "stripe":
        return base
    non_mkt = 1 - marketplace_share
    if variant == "stripe_managed_payments":
        return base + non_mkt * p["mor"]["stripe_mp_extra"]
    if variant == "paddle":
        r, f = p["mor"]["paddle"]
        return non_mkt * (r + f / arpu) + marketplace_share * base
    raise ValueError(variant)


def support_fee(p, aws_bill):
    if aws_bill >= p["support_enterprise_above"]:
        tiers, minimum = p["support_tiers"]["enterprise"], p["support_tiers"]["enterprise_min"]
    else:
        tiers, minimum = p["support_tiers"]["business_plus"], p["support_tiers"]["business_plus_min"]
    fee, prev = 0.0, 0.0
    for cap, rate in tiers:
        take = min(aws_bill, cap) - prev
        if take <= 0:
            break
        fee += take * rate
        prev = cap
    return max(minimum, fee)


def control_plane_month(p, region):
    cp = p["control_plane"]
    h = p["hours_month"]
    itype, count = cp["cp_nodes"]
    cost = {}
    cost["EKS clusters"] = cp["eks_clusters"] * p["eks_cluster_h"] * h
    cost["control-plane nodes"] = count * node(p, region, itype)["od"] * h
    cost["Aurora Serverless v2"] = cp["aurora_acu"] * cp["aurora_acu_h"][region] * h + \
        cp["aurora_storage_gb"] * cp["aurora_storage_gb_month"][region]
    cost["Valkey Serverless"] = cp["valkey_gb"] * cp["valkey_gb_h"][region] * h
    nat_h, nat_gb = p["nat"][region]
    cost["NAT gateways"] = cp["nat_gateways"] * nat_h * h + cp["nat_gb"] * nat_gb
    cost["load balancers"] = cp["load_balancers"] * p["nlb"][region] * h + cp["lcu_avg"] * p["alb_lcu_h"] * h
    cost["interface endpoints"] = cp["interface_endpoints"] * cp["azs"] * cp["endpoint_az_h"][region] * h
    cost["usage-event stream (Firehose)"] = cp["usage_events_gb"] * cp["firehose_gb"][region]
    cost["CloudWatch (AWS signals only)"] = cp["cloudwatch_logs_gb"] * 0.50 + cp["custom_metrics"] * 0.30
    cost["security baseline"] = cp["security_baseline"] + cp["kms_keys"] * 1.0
    cost["S3"] = cp["s3_gb"] * p["s3"]["gb_month"]
    return cost


def fixed_platform_month(p, regions=("us-east-1",), payers=0, new_payers=0, aws_variable=0.0):
    """Fixed platform minimums: control plane per Region, staging, observability SaaS, support plan,
    trust & safety (abuse and KYC desk). Returns a dict of monthly costs."""
    cp_total = 0.0
    for r in regions:
        cp_total += sum(control_plane_month(p, r).values())
    staging = p["control_plane"]["staging_share"] * sum(control_plane_month(p, "us-east-1").values())
    ts = p["trust_safety"]
    fte = max(ts["base_fte"], payers / ts["payers_per_fte"])
    o = p["observability_saas"]
    series = o["base_series"] + o["series_per_payer"] * payers
    obs = o["sentry_team"] + o["grafana_pro"] + series / 1000 * o["per_1000_series"] + \
        o["cp_logs_gb"] * len(regions) * p["log_price"]["us-east-1"]
    out = {
        "control plane (AWS)": cp_total,
        "staging (AWS)": staging,
        "observability SaaS": obs,
        "trust & safety desk": fte * ts["fte_cost_month"] + new_payers * ts["kyc_share_of_new_payers"] * ts["kyc_check_usd"],
    }
    out["AWS Support plan"] = support_fee(p, cp_total + staging + aws_variable)
    return out


def blended(p, mix_name, region="us-east-1", pos=1.0, revenue_month=250_000.0, payment="stripe",
            arpu=100.0, regions=("us-east-1", "eu-west-1"), payers=None):
    """Blended gross margin waterfall for one revenue mix at a monthly revenue scale."""
    lines = line_economics(p, region, pos)
    mix = p["revenue_mixes"][mix_name]
    rev = revenue_month
    w = {}
    # unit-level gross profit (marketplace kept share already net of creators)
    gp_units = sum(mix[k] * rev * lines[k] for k in mix)
    cogs_units = rev - gp_units
    w["revenue"] = rev
    w["GM before overheads"] = gp_units / rev
    pay = payment_rate(p, arpu, payment, marketplace_share=mix["marketplace"])
    connect = mix["marketplace"] * rev * p["creator_share"] * p["connect"]["payout_rate"]
    payers = payers if payers is not None else rev / arpu
    creators = payers * p["creators_per_paying_customer"]
    connect += creators * (p["connect"]["active_account_eur"] + p["connect"]["payout_eur"]) * p["eur_usd"]
    gp = gp_units - pay * rev - connect
    w["after payment fees and Connect payouts"] = gp / rev
    ft = p["free_tier"]
    # free users consume list-price allowance at this mix's cost ratio
    cost_ratio = cogs_units / rev
    free_cost = payers * ft["free_per_paying"] * ft["allowance_usd"] * ft["use_of_allowance"] * cost_ratio * ft["abuse_mult"]
    gp -= free_cost
    w["after free tier"] = gp / rev
    aws_var = sum(mix[k] * rev * (1 - lines[k]) * p["aws_share_by_line"][k] for k in mix)
    fixed = fixed_platform_month(p, regions, payers=payers, new_payers=payers * 0.05, aws_variable=aws_var)
    gp -= fixed["AWS Support plan"]
    w["after AWS Support"] = gp / rev
    gp -= fixed["observability SaaS"]
    w["after observability"] = gp / rev
    gp -= fixed["trust & safety desk"]
    w["after abuse/KYC desk"] = gp / rev
    gp -= fixed["control plane (AWS)"] + fixed["staging (AWS)"]
    w["after fixed platform minimums"] = gp / rev
    return w


# ---------------------------------------------------------------- fleet: all-AWS vs hybrid
def fleet_month(p, peak, layout, profile="rich headless", region="us-east-1", headful_share=0.3):
    """Monthly cost for a browser fleet whose daily peak is `peak` concurrent Sessions."""
    curve = p["demand_curve"]
    avg = sum(curve) / len(curve)
    billed_h = peak * avg * p["hours_month"]
    prof = p["browser_profiles"][profile]
    U = p["util_elastic"]
    traffic = p["traffic_per_browser_h"]
    eg_gb_h = headful_share * traffic["egress_liveview"] + (1 - headful_share) * traffic["egress_base"]
    cost = {}
    if layout.startswith("AWS"):
        purchase = {"AWS on-demand": "od", "AWS interactive mix (60% SP)": "mix-interactive",
                    "AWS 3-yr Instance SP base + on-demand": None, "AWS Spot (batch only)": "spot"}[layout]
        if purchase is None:
            base = p["hybrid_base_share"]
            sp, it, _ = best_slot(p, region, p["browser_families"], "isp3", prof["vcpu"], prof["gb"])
            od, _, _ = best_slot(p, region, p["browser_families"], "od", prof["vcpu"], prof["gb"])
            base_h = sum(min(d, base) for d in curve) / len(curve) * peak * p["hours_month"]
            burst_h = billed_h - base_h
            cost["compute"] = base * peak * p["hours_month"] * sp / 0.9 + burst_h * od / U
        elif purchase == "mix-interactive":
            slot, _ = mix_purchase_cost(p, region, p["browser_families"], "interactive", prof["vcpu"], prof["gb"])
            cost["compute"] = billed_h * slot / U
        else:
            slot, _, _ = best_slot(p, region, p["browser_families"], purchase, prof["vcpu"], prof["gb"])
            cost["compute"] = billed_h * slot / U
        cost["egress"] = egress_cost(p, billed_h * eg_gb_h)
        cost["gateway+NLB"] = billed_h * gateway_cost_per_h(p, region, False, concurrency=max(peak * avg, 1)) + \
            headful_share * billed_h * (gateway_cost_per_h(p, region, True, max(peak * avg, 1)) - gateway_cost_per_h(p, region, False, max(peak * avg, 1)))
        cost["observability"] = billed_h * p["logs_gb"]["browser_h"] * p["log_price"][region]
    else:
        base = p["hybrid_base_share"]
        nodes_spec = p["alt_nodes"].get(layout.split(" + ")[0])
        burst_purchase = "spot" if "Spot burst" in layout else "od"
        if layout.startswith("OCI"):
            ocpu_h, gb_h, vcpu_per_ocpu = p["oci_flex"]["A1"]
            per_slot = prof["vcpu"] / vcpu_per_ocpu * ocpu_h + prof["gb"] * gb_h
            cost["compute (OCI A1, elastic)"] = billed_h * per_slot / p["packing"] / U
            gb_out = billed_h * eg_gb_h
            free_gb, rate = p["oci_egress"]
            cost["egress (OCI)"] = max(0.0, gb_out - free_gb) * rate
            cost["second-provider operations"] = p["baremetal_ops"]["fixed_month"] * p["oci_ops_share"]
        else:
            cap = math.floor(min(nodes_spec["threads"] * p["smt_factor"] * p["packing"] / prof["vcpu"],
                                 nodes_spec["gb"] * p["packing"] / prof["gb"]))
            base_sessions = base * peak
            nodes = max(1, math.ceil(base_sessions / cap))
            cost["base compute (bare metal)"] = nodes * nodes_spec["month"]
            base_h = sum(min(d * peak, nodes * cap) for d in curve) / len(curve) * p["hours_month"]
            burst_h = max(0.0, billed_h - base_h)
            bslot, _, _ = best_slot(p, region, p["browser_families"], burst_purchase, prof["vcpu"], prof["gb"])
            burst_cost = burst_h * bslot / U
            cost[f"burst compute (AWS {burst_purchase})"] = burst_cost
            # burst traffic leaves AWS
            cost["egress (AWS burst share)"] = egress_cost(p, burst_h * eg_gb_h)
            cost["bare-metal operations"] = p["baremetal_ops"]["fixed_month"] + nodes * p["baremetal_ops"]["per_node_month"]
        per_vcpu = node(p, region, "c7g.2xlarge")["od"] / 8.0
        gw = p["gw_sessions_per_vcpu"]
        cost["gateway (fleet side, no NLB)"] = billed_h * per_vcpu * (headful_share / gw["headful"] + (1 - headful_share) / gw["headless"])
        cost["observability"] = billed_h * p["logs_gb"]["browser_h"] * p["log_price"][region]
    total = sum(cost.values())
    return billed_h, cost, total


# ---------------------------------------------------------------- 36-month scenarios
def scenario(p, name, months=36):
    sc = p["scenarios"][name]
    seg = p["segments"]
    ssm = p["self_serve_mix"]
    payers = {k: 0.0 for k in seg}
    arpu = {k: seg[k]["arpu"] for k in seg}
    signups_hist = []
    rows = []
    # unit GM per mix at the scenario price position (us-east-1, recommended deployment)
    gm_by_mix, aws_by_mix = {}, {}
    lines = line_economics(p, "us-east-1", sc["price_pos"])
    for mix_name, mix in p["revenue_mixes"].items():
        gm_by_mix[mix_name] = sum(mix[k] * lines[k] for k in mix)
        aws_by_mix[mix_name] = sum(mix[k] * (1 - lines[k]) * p["aws_share_by_line"][k] for k in mix)
    for t in range(1, months + 1):
        g = sc["g_start"] + (sc["g_end"] - sc["g_start"]) * (t - 1) / (months - 1)
        s = sc["signups_m1"] if t == 1 else signups_hist[-1] * (1 + g)
        signups_hist.append(s)
        conv_source = signups_hist[t - 1 - sc["lag"]] if t - 1 - sc["lag"] >= 0 else 0.0
        new_ss = conv_source * sc["conv"]
        new = {k: new_ss * ssm[k] for k in ssm}
        if t >= sc["ent_start"]:
            lo, hi = sc["ent_per_month"]
            frac = (t - sc["ent_start"]) / max(1, months - sc["ent_start"])
            new["enterprise"] = lo + (hi - lo) * frac
        else:
            new["enterprise"] = 0.0
        mrr = 0.0
        gp = 0.0
        aws_var = 0.0
        for k in seg:
            payers[k] = payers[k] * (1 - seg[k]["churn"]) + new.get(k, 0.0)
            if t > 1:
                arpu[k] *= (1 + seg[k]["arpu_growth"])
            r = payers[k] * arpu[k] * sc["price_pos"]
            mrr += r
            gp += r * gm_by_mix[seg[k]["mix"]]
            aws_var += r * aws_by_mix[seg[k]["mix"]]
        gp -= payers["enterprise"] * p["enterprise_sso_usd"]
        total_payers = sum(payers.values())
        avg_arpu = mrr / total_payers if total_payers else 0.0
        pay = payment_rate(p, max(avg_arpu, 20.0), "stripe", marketplace_share=0.1)
        gp -= pay * mrr
        ft = p["free_tier"]
        free_users = total_payers * ft["free_per_paying"]
        gp -= free_users * ft["allowance_usd"] * ft["use_of_allowance"] * 0.35 * ft["abuse_mult"]
        regions = ("us-east-1", "eu-west-1") if t >= p["second_region_month"] else ("us-east-1",)
        cogs = mrr - gp
        fixed = fixed_platform_month(p, regions, payers=total_payers,
                                     new_payers=sum(new.values()), aws_variable=aws_var)
        fixed_total = sum(fixed.values())
        rows.append({"month": t, "signups": s, "active users": total_payers + free_users,
                     "payers": dict(payers), "total payers": total_payers, "MRR": mrr, "ARR": mrr * 12,
                     "ARPU": avg_arpu, "COGS": cogs, "gross profit": gp, "fixed platform": fixed_total,
                     "GP - fixed": gp - fixed_total})
    return rows


# =====================================================================================
# PRINTING
# =====================================================================================

def fmt(x, nd=4):
    if isinstance(x, str):
        return x
    if x != x:  # NaN
        return "n/a"
    if abs(x) >= 1000:
        return f"{x:,.0f}"
    if abs(x) >= 100:
        return f"{x:,.1f}"
    if abs(x) >= 1:
        return f"{x:,.2f}"
    return f"{x:.{nd}f}"


def pct(x):
    return "n/a" if x != x else f"{100 * x:.1f}%"


def table(headers, rows):
    print("| " + " | ".join(headers) + " |")
    print("|" + "|".join("---" for _ in headers) + "|")
    for r in rows:
        print("| " + " | ".join(str(c) for c in r) + " |")
    print()


def banner(title):
    print(f"\n## {title}\n")
    print("_All figures below are MODEL OUTPUTS computed from the assumptions block, not facts._\n")


def section_inputs(p):
    banner("Inputs (assumptions block)")
    rows = []
    for k, v in REG.items():
        val = v["value"]
        sval = val if isinstance(val, (int, float, str, bool)) else repr(val)
        if len(str(sval)) > 90:
            sval = str(sval)[:87] + "..."
        rows.append((k, str(sval).replace("|", "/"), v["unit"], v["src"]))
    table(["input", "value (truncated; see the code)", "unit", "source"], rows)


def section_units(p):
    banner("(a) Hosted browser-hour cost by deployment option")
    ref = p["cp_alloc_ref"]
    cp_month = sum(control_plane_month(p, "us-east-1").values()) * (1 + p["control_plane"]["staging_share"])
    alloc = cp_month * ref["browser_share"] / (ref["concurrency"] * p["hours_month"])
    print(f"Utilization of elastic EC2 fleets {pct(p['util_elastic'])}; warm pool {pct(p['pool_frac'])} "
          f"pre-booted sandboxes; egress at ${p['egress_rate_unit']}/GB; gateway and NLB at 1,000 concurrent Sessions; "
          f"control-plane allocation = one us-east-1 control plane plus staging (${fmt(cp_month)}/month) x "
          f"{fmt(ref['browser_share'], 2)} / ({fmt(ref['concurrency'])} Sessions x 730 h).\n")
    for profile in p["browser_profiles"]:
        rows = []
        for label, dep, pur in BROWSER_DEPLOYS:
            cells = [label]
            for r in REGIONS:
                c = browser_hour(p, r, profile, dep, pur)
                cells.append(f"{fmt(c['total'])} ({c['host']})")
            c = browser_hour(p, "us-east-1", profile, dep, pur)
            cells += [fmt(c["compute"]), fmt(c["egress"]), fmt(c["gateway+NLB"]),
                      fmt(c["warm pool"] + c.get("snapshot fees", 0) + c.get("public IPv4", 0)),
                      fmt(c["observability"] + c["recordings"]), fmt(alloc), fmt(c["total"] + alloc)]
            rows.append(cells)
        print(f"**{profile}** ($ per billed browser-hour)\n")
        table(["deployment", "us-east-1 total (host)", "eu-west-1 total (host)", "compute", "egress",
               "gateway+NLB", "pool/IPv4/snapshots", "obs+recordings", "control-plane alloc.", "us fully loaded"], rows)
    print("Utilization sweep, rich headless, interactive mix (us-east-1):\n")
    rows = []
    for U in (0.30, 0.45, 0.65, 0.80, 0.90):
        rows.append((pct(U), fmt(browser_hour(p, "us-east-1", "rich headless", "ec2", "mix-interactive", util=U)["total"]),
                     fmt(browser_hour(p, "us-east-1", "rich headful", "ec2", "mix-interactive", util=U)["total"]),
                     fmt(browser_hour(p, "us-east-1", "lean headless", "ec2", "mix-interactive", util=U)["total"])))
    table(["utilization", "rich headless $/h", "rich headful $/h", "lean headless $/h"], rows)

    banner("(b) Crawl and extraction API, $ per 1,000 pages")
    rows = []
    for cls in ("static", "js", "protected"):
        cu = crawl_1000(p, "us-east-1", cls)
        ce = crawl_1000(p, "eu-west-1", cls)
        comps = "; ".join(f"{k} {fmt(v)}" for k, v in cu.items() if k not in ("total", "host") and v > 0)
        rows.append((cls, fmt(cu["total"]), fmt(ce["total"]),
                     fmt(crawl_1000_alt(p, "us-east-1", cls, "fargate-spot")),
                     fmt(crawl_1000_alt(p, "us-east-1", cls, "lambda")), comps))
    table(["class", "EKS Graviton batch mix us-east-1", "eu-west-1", "Fargate ARM Spot (us)", "Lambda Arm (us)",
           "components, us-east-1"], rows)
    print(f"Protected class assumes captcha rate {pct(p['captcha_rate'])} per browser load, success "
          f"{pct(p['protected_success'])}, residential ${p['res_proxy_gb']}/GB, route mix {p['route_mix']['protected']}.\n")
    print("**Protected-class variants**, $ per 1,000 pages (us-east-1): HTTP-first share x residential $/GB\n")
    rows = []
    for share in (0.0, 0.4, 0.7, 0.9):
        cells = [pct(share)]
        for res in (0.70, 1.00, 2.00, 2.50):
            q = copy.deepcopy(p)
            q["route_mix"]["protected"] = {"http": share, "browser": 1 - share}
            q["res_proxy_gb"] = res
            cells.append(fmt(crawl_1000(q, "us-east-1", "protected")["total"]))
        rows.append(cells)
    table(["HTTP-route share", "res $0.70/GB", "res $1.00/GB", "res $2.00/GB", "res $2.50/GB"], rows)
    rows = []
    for rate in (0.02, 0.10, 0.25, 0.50):
        q = copy.deepcopy(p)
        q["captcha_rate"] = rate
        rows.append((pct(rate), fmt(crawl_1000(q, "us-east-1", "protected")["total"]),
                     fmt(crawl_1000(q, "us-east-1", "protected")["captcha"] + crawl_1000(q, "us-east-1", "protected")["captcha wait"])))
    table(["captcha solves per browser load", "protected $/1,000", "of which solving + waiting"], rows)
    rows = []
    for mb in (0.5, 1.0, 2.0, 2.9):
        q = copy.deepcopy(p)
        q["gb_per_1000_pages"]["protected_browser"] = mb
        rows.append((fmt(mb), fmt(crawl_1000(q, "us-east-1", "protected")["total"])))
    table(["GB per 1,000 protected browser loads", "protected $/1,000"], rows)

    banner("(c) Apify-style Compute Unit (1 GB-hour, 0.25 vCPU) inside an isolated sandbox")
    rows = []
    for label, dep, pur in CU_DEPLOYS:
        cu = cu_cost(p, "us-east-1", dep, pur)
        ce = cu_cost(p, "eu-west-1", dep, pur)
        rows.append((label, f"{fmt(cu['total'])} ({cu['host']})", f"{fmt(ce['total'])} ({ce['host']})"))
    table(["deployment", "us-east-1 $/CU", "eu-west-1 $/CU"], rows)

    banner("(d) AI agent task (~10 steps), $ per task")
    rows = []
    for model in p["llm_models"]:
        prov = p["llm_models"][model][1]
        cells = [model, p["llm_models"][model][0]]
        a = agent_task(p, "us-east-1", model)
        r = agent_task(p, "us-east-1", model, replay=True)
        cells += [fmt(a["total"]), fmt(r["total"])]
        if prov == "anthropic":
            bed = a["total"] + a["model tokens"] * (p["bedrock_regional_premium"] - 1)
            cells.append(fmt(bed))
        elif prov == "bedrock-open":
            cells.append(fmt(a["total"]))
        else:
            cells.append("not on Bedrock" if model != "GPT-6 Astra" else fmt(a["total"] + a["model tokens"] * 0.1))
        e = agent_task(p, "eu-west-1", model)
        er = agent_task(p, "eu-west-1", model, replay=True)
        cells += [fmt(e["total"]), fmt(er["total"])]
        h = agent_task(p, "us-east-1", model, "heavy")
        cells += [fmt(h["total"]), pct(a["model tokens"] / a["total"])]
        rows.append(cells)
    table(["model", "class", "us direct/Bedrock global, agent", "us replay", "us Bedrock regional (+10%), agent",
           "eu agent", "eu replay", "us heavy agent", "model share (us agent)"], rows)
    print("Direct API and Bedrock global endpoints cost the same for Claude; the eu-west-1 column uses Bedrock "
          "regional endpoints (+10%) for EU data routing. Browser time uses the lean headless interactive-mix cost.\n")

    banner("(e) Resale units, COGS")
    rows = []
    for k in resale(p, "us-east-1"):
        rows.append((k, fmt(resale(p, "us-east-1")[k]["cogs"]), fmt(resale(p, "eu-west-1")[k]["cogs"])))
    table(["unit", "us-east-1 $", "eu-west-1 $"], rows)


def section_market(p):
    banner("Market prices per unit (from the pricing notes)")
    rows = []
    for k, v in market(p).items():
        rows.append((k, fmt(v["entry_median"]), fmt(v["largest_median"]), fmt(v["median_all"]),
                     f"{fmt(v['min'])} - {fmt(v['max'])}"))
    table(["unit", "entry-plan median", "largest self-serve median", "median (all plans)", "range"], rows)
    print(f"LLM token markup median: {pct(med(list(p['mkt_llm_markup'].values())))} "
          f"(range 0% - 20%). Headful browser-hour reference: Kernel $0.48.\n")


def margin_rows(p, region):
    mk = market(p)
    u = unit_cogs(p, region)
    rows = []
    mapping = [
        ("browser-hour, profile mix (50% lean headless, 30% rich headless, 20% rich headful)", "browser-hour", u["browser-hour"]),
        ("browser-hour lean headless", "browser-hour", u["browser-hour lean headless"]),
        ("browser-hour rich headless", "browser-hour", u["browser-hour rich headless"]),
        ("browser-hour rich headful (live view)", "browser-hour", u["browser-hour rich headful"]),
        ("browser-hour rich headful at Kernel's headful $0.48", None, u["browser-hour rich headful"]),
        ("1,000 pages static", "1,000 pages static", u["1,000 pages static"]),
        ("1,000 pages js", "1,000 pages js", u["1,000 pages js"]),
        ("1,000 pages protected", "1,000 pages protected", u["1,000 pages protected"]),
        ("Compute Unit", "Compute Unit", u["Compute Unit"]),
        ("agent task cheap (agent every run)", "agent task (cheap model)", u["agent task (cheap model)"]),
        ("agent task cheap (replay)", "agent task (cheap model)", u["agent task (cheap model) replay"]),
        ("agent task mid (agent every run)", "agent task (mid model)", u["agent task (mid model)"]),
        ("agent task mid (replay)", "agent task (mid model)", u["agent task (mid model) replay"]),
        ("agent task frontier (agent every run)", "agent task (frontier model)", u["agent task (frontier model)"]),
        ("agent task frontier (replay)", "agent task (frontier model)", u["agent task (frontier model) replay"]),
        ("residential GB", "residential GB", u["residential GB"]),
        ("1,000 captcha solves", "1,000 captcha solves", u["1,000 captcha solves"]),
    ]
    for label, key, cogs in mapping:
        if key is None:
            price = med([v[1] for v in p["mkt_browser_hour_headful"].values()])
            entry = price
        else:
            price = mk[key]["largest_median"]
            entry = mk[key]["entry_median"]
        cells = [label, fmt(cogs), fmt(entry), pct(gm(entry, cogs)), fmt(price)]
        for name, f in p["price_positions"].items():
            cells.append(pct(gm(price * f, cogs)))
        rows.append(cells)
    mark = med(list(p["mkt_llm_markup"].values()))
    cells = ["1M LLM tokens resold (any model; markup over list)", "list", "list x (1 + markup)",
             pct(mark / (1 + mark)), f"markup {pct(mark)}"]
    for name, fpos in p["price_positions"].items():
        m = llm_markup_at(p, fpos)
        cells.append(f"{pct(m / (1 + m))} (markup {pct(m)})")
    rows.append(cells)
    return rows


def section_margins(p):
    for region in REGIONS:
        banner(f"Unit gross margin by price position ({region}, recommended deployment)")
        print("Recommended deployment: EKS+Karpenter on Graviton with 60% of interactive capacity on a 1-yr Compute "
              "Savings Plan; crawl on an 80% Spot batch fleet; Recipes in Firecracker microVMs on Graviton .metal "
              "(80% Spot). Margins exclude payment fees, payouts and fixed costs (see 'blended').\n")
        heads = ["unit", "COGS $", "entry median $", "GM at entry median", "largest-plan median $"] + \
                [f"GM {k}" for k in p["price_positions"]]
        table(heads, margin_rows(p, region))
    print("LLM tokens: the 'below' position is pass-through at cost (0%), the premium is Browser Use's 20%. "
          "Payment fees (~3.8%) are not yet deducted here, so 0-2.5% markups lose money after fees.\n")
    print("**Browser-hour variants** (us-east-1, profile mix, market median $/h)\n")
    rows = []
    for label, kw in (("browser-hour only (base)", {}), ("plus non-proxied traffic billed at $0.20/GB", {"direct_traffic": True})):
        rows.append((label, pct(line_economics(p, "us-east-1", 1.0, **kw)["browser"]),
                     pct(line_economics(p, "us-east-1", 0.75, **kw)["browser"]),
                     pct(line_economics(p, "us-east-1", 1.25, **kw)["browser"])))
    table(["pricing", "GM median", "GM -25%", "GM +25%"], rows)


def section_blended(p):
    banner("Blended gross margin waterfall by revenue mix (us-east-1 unit costs, two production Regions)")
    for scale in (100_000.0, 1_000_000.0):
        print(f"**Monthly revenue ${fmt(scale)}**, ARPU $100, price at market median, Stripe direct (EU)\n")
        rows = []
        keys = None
        for mix_name in p["revenue_mixes"]:
            w = blended(p, mix_name, revenue_month=scale)
            keys = [k for k in w if k != "revenue"]
            rows.append([mix_name] + [pct(w[k]) for k in keys])
        table(["mix"] + keys, rows)
    print("**Payment variants** at $1M/month, base mixes (GM after fixed platform minimums)\n")
    rows = []
    for mix_name in p["revenue_mixes"]:
        cells = [mix_name]
        for variant in ("stripe", "stripe_managed_payments", "paddle"):
            cells.append(pct(blended(p, mix_name, revenue_month=1e6, payment=variant)["after fixed platform minimums"]))
        rows.append(cells)
    table(["mix", "Stripe direct (EU)", "Stripe Managed Payments (MoR)", "Paddle (MoR)"], rows)
    print("MoR variants apply only to non-marketplace revenue: Paddle and Stripe Managed Payments exclude marketplaces.\n")
    print("**Net-revenue view of the marketplace** (event revenue counted net of the 80% creator share):\n")
    rows = []
    for mix_name in p["revenue_mixes"]:
        mix = p["revenue_mixes"][mix_name]
        lines = line_economics(p, "us-east-1")
        gross_gp = sum(mix[k] * lines[k] for k in mix)
        net_rev = 1 - mix["marketplace"] * p["creator_share"]
        rows.append((mix_name, pct(gross_gp), pct(gross_gp / net_rev)))
    table(["mix", "GM on gross revenue", "GM on net revenue (after creator payouts)"], rows)
    print("Public benchmarks (business_models_and_financials.md §3): Cloudflare 74.5%, Elastic 76%, Confluent 74.3%, "
          "MongoDB 72%, Snowflake product 72%, DigitalOcean ~60%, Fastly 57.1%, Twilio 49% (carrier pass-through), "
          "GitLab 87%; AI companies 50-60% (a16z), Bessemer Shooting Stars ~60%, Supernovas ~25%.\n")
    print("**Line gross margins (us-east-1) by price position**\n")
    rows = []
    l1, l0, l2 = (line_economics(p, "us-east-1", f) for f in (1.0, 0.75, 1.25))
    for k in l1:
        rows.append((k, pct(l1[k]), pct(l0[k]), pct(l2[k])))
    table(["revenue line", "GM median", "GM -25%", "GM +25%"], rows)
    print("**Fixed platform minimums per month** (two production Regions + staging)\n")
    rows = []
    for payers in (1000, 10000):
        f = fixed_platform_month(p, ("us-east-1", "eu-west-1"), payers=payers, new_payers=payers * 0.05,
                                 aws_variable=payers * 100 * 0.15)
        rows.append([f"{payers:,} payers"] + [fmt(v) for v in f.values()] + [fmt(sum(f.values()))])
    table(["scale"] + list(f.keys()) + ["total"], rows)
    print("(AWS Support is computed on an assumed AWS bill of 15% of revenue, with ARPU $100.)\n")
    print("**Control plane per Region, by line (us-east-1 / eu-west-1)**\n")
    cu, ce = control_plane_month(p, "us-east-1"), control_plane_month(p, "eu-west-1")
    table(["item", "us-east-1 $/month", "eu-west-1 $/month"],
          [(k, fmt(cu[k]), fmt(ce[k])) for k in cu] + [("total", fmt(sum(cu.values())), fmt(sum(ce.values())))])


def section_fleet(p):
    banner("All-AWS versus hybrid browser fleet (rich headless profile, 30% headful live-view traffic)")
    print("Daily peak concurrency P; demand follows the hourly curve (average ~69% of peak). Revenue at the "
          "largest-plan median browser-hour price; proxies, captcha and models excluded. Hybrid = bare-metal base "
          "sized at 60% of peak, running 24/7, burst on AWS; the session gateway sits on the fleet side, so base "
          "traffic leaves through the provider's included bandwidth. 'Spot burst' follows the brief; 'on-demand "
          "burst' follows the runtime note (interactive Sessions cannot migrate).\n")
    layouts = ["AWS on-demand", "AWS interactive mix (60% SP)", "AWS 3-yr Instance SP base + on-demand",
               "AWS Spot (batch only)",
               "OVH SCALE-a8 768GB (FR) + AWS Spot burst", "OVH SCALE-a8 768GB (US) + AWS Spot burst",
               "Hetzner AX162-2 + AWS Spot burst", "Scaleway EM-I620E + AWS Spot burst",
               "OVH SCALE-a8 768GB (FR) + AWS on-demand burst", "Scaleway EM-I620E + AWS on-demand burst",
               "OCI A1 (elastic)"]
    price = market(p)["browser-hour"]["largest_median"]
    for peak in (100, 1000, 10000):
        rows = []
        for lay in layouts:
            billed_h, cost, total = fleet_month(p, peak, lay)
            rev = billed_h * price
            cp = sum(control_plane_month(p, "us-east-1").values())
            rows.append((lay, fmt(total), fmt(total / billed_h), pct(gm(rev, total)), pct(gm(rev, total + cp))))
        print(f"**Peak {peak:,} concurrent browsers** ({fmt(peak * sum(p['demand_curve']) / 24 * p['hours_month'])} "
              f"billed browser-hours/month; revenue ${fmt(peak * sum(p['demand_curve']) / 24 * p['hours_month'] * price)})\n")
        table(["layout", "fleet cost $/month", "$ per billed hour", "GM (fleet only)", "GM incl. one control plane"], rows)


def section_scenarios(p):
    banner("Revenue scenarios over 36 months (scenarios, not forecasts)")
    for name in p["scenarios"]:
        rows = scenario(p, name)
        sel = [r for r in rows if r["month"] in (3, 6, 9, 12, 18, 24, 30, 36)]
        out = []
        for r in sel:
            pay = r["payers"]
            out.append((r["month"], fmt(r["active users"]), fmt(pay["hobbyist"]), fmt(pay["agent dev"]),
                        fmt(pay["data team"]), fmt(pay["enterprise"], 1), fmt(r["ARPU"]), fmt(r["MRR"]),
                        fmt(r["ARR"]), fmt(r["COGS"]), fmt(r["gross profit"]), fmt(r["fixed platform"]),
                        fmt(r["GP - fixed"])))
        be = next((r["month"] for r in rows if r["GP - fixed"] >= 0), None)
        print(f"**{name}** (price position x{p['scenarios'][name]['price_pos']}); gross profit first covers fixed "
              f"platform costs in month: {be if be else 'not within 36 months'}\n")
        table(["month", "active users", "hobbyist payers", "agent-dev payers", "data-team payers", "enterprise",
               "ARPU $", "MRR $", "ARR $", "COGS $", "gross profit $", "fixed platform $", "GP - fixed $"], out)


SENS = [
    ("util_elastic", "browser fleet utilization", [0.40, 0.65, 0.85]),
    ("spot_share_batch", "Spot share of batch capacity", [0.0, 0.8, 0.95]),
    ("res_proxy_gb", "residential COGS $/GB", [1.00, 2.00, 2.50]),
    ("gb_per_1000_pages", "protected browser GB per 1,000 pages", [0.5, 1.0, 2.0]),
    ("route_mix", "protected HTTP-route share", [0.0, 0.40, 0.70]),
    ("mkt_llm_markup", "LLM markup over list (replaces the market median)", [0.0, 0.025, 0.20]),
    ("price_pos", "price position x median", [0.70, 1.0, 1.25]),
    ("replay_factor", "replay token reduction", [3.0, 4.5, 6.0]),
    ("captcha_rate", "captcha solves per protected browser load", [0.02, 0.10, 0.50]),
    ("dc_proxy_gb", "datacenter proxy $/GB", [0.03, 0.15, 0.45]),
    ("egress_rate_unit", "egress $/GB (tier position)", [0.05, 0.09, 0.09]),
    ("log_price", "log $/GB (Better Stack -> CloudWatch Standard)", [{"us-east-1": 0.05, "eu-west-1": 0.05},
                                                                     {"us-east-1": 0.23, "eu-west-1": 0.15},
                                                                     {"us-east-1": 0.50, "eu-west-1": 0.57}]),
    ("free_tier", "free active users per payer", [1.0, 3.0, 10.0]),
    ("trust_safety", "trust & safety FTE cost $/month", [6000.0, 8500.0, 12000.0]),
    ("browser_mem", "browser memory multiplier", [0.5, 1.0, 2.0]),
]


def apply(p, key, val):
    q = copy.deepcopy(p)
    if key == "gb_per_1000_pages":
        q[key]["protected_browser"] = val
    elif key == "route_mix":
        q[key]["protected"] = {"http": val, "browser": 1 - val}
    elif key == "mkt_llm_markup":
        q[key] = {"all": val}
    elif key == "browser_mem":
        for prof in q["browser_profiles"].values():
            prof["gb"] *= val
    elif key == "price_pos":
        q["price_positions"] = {"median": val}
    elif key == "free_tier":
        q[key]["free_per_paying"] = val
    elif key == "trust_safety":
        q[key]["fte_cost_month"] = val
    else:
        q[key] = val
    return q


def blended_at(p, pos=1.0):
    vals = [blended(p, m, revenue_month=1e6, pos=pos)["after fixed platform minimums"] for m in p["revenue_mixes"]]
    return sum(vals) / len(vals)


def section_sensitivity(p):
    banner("Sensitivity: blended GM (average of the four mixes, $1M/month, after all overheads)")
    base = blended_at(p)
    print(f"Base: {pct(base)}\n")
    swings = []
    for key, label, vals in SENS:
        res = []
        for v in vals:
            q = apply(p, key, v)
            pos = v if key == "price_pos" else 1.0
            res.append(blended_at(q, pos))
        swings.append((abs(max(res) - min(res)), key, label, vals, res))
    swings.sort(reverse=True)
    def show(v):
        return str(v["us-east-1"]) if isinstance(v, dict) else str(v)
    rows = [(label, " / ".join(show(v) for v in vals), " / ".join(pct(r) for r in res), f"{100 * s:.1f} pts")
            for s, key, label, vals, res in swings]
    table(["assumption", "low / base / high", "blended GM", "swing"], rows)
    print("Top five, plus the drivers named in the brief, by mix (GM after all overheads, $1M/month):\n")
    named = {"util_elastic", "spot_share_batch", "gb_per_1000_pages", "mkt_llm_markup", "price_pos"}
    chosen = swings[:5] + [x for x in swings[5:] if x[1] in named]
    for s, key, label, vals, res in chosen:
        rows = []
        for mix_name in p["revenue_mixes"]:
            cells = [mix_name]
            for v in vals:
                q = apply(p, key, v)
                pos = v if key == "price_pos" else 1.0
                cells.append(pct(blended(q, mix_name, revenue_month=1e6, pos=pos)["after fixed platform minimums"]))
            rows.append(cells)
        print(f"**{label}** (swing on the average: {100 * s:.1f} pts)\n")
        table(["mix"] + [show(v) for v in vals], rows)


def section_disagreements(p):
    banner("Where the notes disagree: chosen value and the effect of the alternatives")
    rows = []
    for basis, label in (("advisor", "Advisor 30-day savings (chosen)"), ("snapshot", "Spot feed snapshot"),
                         (0.60, "60% flat (runtime note)"), (0.583, "58.3% (Vantage, provider note)")):
        q = copy.deepcopy(p)
        q["spot_basis"] = basis
        rows.append((label, fmt(browser_hour(q, "us-east-1", "rich headless", "ec2", "spot")["total"]),
                     fmt(crawl_1000(q, "us-east-1", "js")["total"]),
                     fmt(cu_cost(q, "us-east-1", "fc_metal", "mix-batch")["total"])))
    print("**Spot discount basis**\n")
    table(["basis", "rich headless Spot $/h", "JS pages $/1,000", "CU $ (80% Spot)"], rows)
    rows = []
    for mem, label in ((0.5, "vendor sizing (0.3-0.8 GB)"), (1.0, "provider note L/R 1-2 GB (chosen)"),
                       (2.0, "runtime note 1.6-5 GB (x2)"), (4.0, "Kernel 8 GB headful (x4)")):
        q = apply(p, "browser_mem", mem)
        rows.append((label, fmt(browser_hour(q, "us-east-1", "rich headless", "ec2", "mix-interactive")["total"]),
                     fmt(browser_hour(q, "us-east-1", "rich headful", "ec2", "mix-interactive")["total"])))
    print("**Browser memory per Session** (CPU unchanged)\n")
    table(["memory basis", "rich headless $/h", "rich headful $/h"], rows)
    rows = []
    for gb in (3.65e3, 36.5e3, 365e3, 2190e3):
        rows.append((fmt(gb / 1000), fmt(egress_cost(p, gb) / gb)))
    print("**Egress tier position** (same tiers in both notes; the effective rate depends on volume)\n")
    table(["TB per month", "effective $/GB"], rows)
    rows = []
    for d in (0.50, 0.60, 0.70):
        q = copy.deepcopy(p)
        q["fargate_spot_discount"] = d
        rows.append((pct(d), fmt(browser_hour(q, "us-east-1", "rich headless", "fargate", "spot")["total"]),
                     fmt(cu_cost(q, "us-east-1", "fargate", "spot")["total"])))
    print("**Fargate Spot discount** (only 'up to 70%' is published)\n")
    table(["discount", "rich headless $/h", "CU $"], rows)
    rows = []
    for smt in (1.0, 0.65):
        q = copy.deepcopy(p)
        q["smt_factor"] = smt
        _, _, t = fleet_month(q, 10000, "OVH SCALE-a8 768GB (FR) + AWS Spot burst")
        _, _, t2 = fleet_month(q, 10000, "Hetzner AX162-2 + AWS Spot burst")
        rows.append((smt, fmt(t), fmt(t2)))
    print("**SMT thread value on bare metal** (provider note tests 0.65), 10,000 peak browsers\n")
    table(["thread = x vCPU", "OVH FR hybrid $/month", "Hetzner hybrid $/month"], rows)


SECTIONS = {
    "inputs": section_inputs, "units": section_units, "market": section_market, "margins": section_margins,
    "blended": section_blended, "fleet": section_fleet, "scenarios": section_scenarios,
    "sensitivity": section_sensitivity, "disagreements": section_disagreements,
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sections", nargs="*", help="sections to print (default: all): " + ", ".join(SECTIONS))
    args = ap.parse_args()
    unknown = [x for x in args.sections if x not in SECTIONS]
    if unknown:
        ap.error(f"unknown section(s): {', '.join(unknown)}; choose from {', '.join(SECTIONS)}")
    p = params()
    chosen = args.sections or list(SECTIONS)
    print("# Goodfetch Cloud unit economics model (MODEL OUTPUT; inputs dated 2026-10-12)")
    for s in chosen:
        SECTIONS[s](p)


if __name__ == "__main__":
    main()
