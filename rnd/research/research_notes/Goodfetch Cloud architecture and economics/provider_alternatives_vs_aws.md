# Cloud provider alternatives vs AWS for Goodfetch Cloud (as of 2026-10-12)

Conventions: prices exclude VAT. EUR is converted at the ECB reference rate of **1.1206 USD per EUR (9 October 2026)** ([ECB](https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/eurofxref-graph-usd.en.html)). A month is 730 hours. "My calculation" means arithmetic done in Python during this session from the cited unit prices. AWS figures are us-east-1 reference prices from a third-party mirror of AWS pricing (instances.vantage.sh, region not labelled on the page). Another researcher compiles the authoritative AWS price sheet. Prices for OVHcloud, Vultr and Oracle were read from their public pricing APIs on 2026-10-12. The web-search budget shared by all researchers ran out early in this session, so the notes rely on directly fetched official pages and APIs. Items that could not be retrieved are listed under Gaps.

## 1. How do the providers compare on compute price, bare metal/KVM, ARM, egress, IPv4, elasticity, regions, managed platforms, reliability and support?

### Takeaway
AWS on-demand is among the most expensive ways to buy browser-shaped compute: about **$26.5 per month for one "1 vCPU + 2 GB" unit** on Graviton3. GCP costs the same and Azure is 14% cheaper. AWS spot and 3-year commitments cut the price by 56–58%. EU bare metal is 63–78% below AWS on-demand: Scaleway Elastic Metal, OVHcloud Scale and Hetzner AX. Hetzner's shared Arm cloud is 89% below, and Oracle's flex shapes are 48–64% below with hyperscaler-style elasticity. Networking widens the gap. AWS and Azure charge about $0.05–0.09/GB to the internet, and AWS's NAT gateway also charges $0.045/GB in **both** directions. OVHcloud, Scaleway, Hetzner, DigitalOcean, Vultr and Latitude.sh include most or all traffic.

### Cited Findings

#### Compute unit prices, normalised to the browser shape
The unit is one vCPU plus 2 GB RAM, which matches the 2 GB-per-vCPU browser assumption in section 6. Unit price = node price ÷ min(vCPU, RAM/2), with no packing loss (my calculation from the cited prices). On bare metal, every SMT hardware thread is counted as one vCPU. Section 6 tests that assumption.

| Option (node) | Node price | $/unit-month | × AWS c7g on-demand | Billing | Source |
|---|---|---|---|---|---|
| AWS c7g.4xlarge (16 vCPU Graviton3, 32 GiB), on-demand | $0.580/h | 26.46 | 1.00 | hourly | [Vantage](https://instances.vantage.sh/aws/ec2/c7g.4xlarge) |
| AWS c7g.4xlarge spot / 1-yr RI / 3-yr RI | $0.242 / $0.382 / $0.255 per h | 11.04 / 17.43 / 11.63 | 0.42 / 0.66 / 0.44 | interruptible / commit | [Vantage](https://instances.vantage.sh/aws/ec2/c7g.4xlarge) |
| AWS c7a.4xlarge (16 vCPU AMD, 32 GiB), on-demand / spot | $0.821 / $0.332 per h | 37.46 / 15.15 | 1.42 / 0.57 | hourly | [Vantage](https://instances.vantage.sh/aws/ec2/c7a.4xlarge) |
| AWS c7g.metal (64 vCPU, 128 GiB), on-demand / spot | $2.320 / $1.056 per h | 26.46 / 12.05 | 1.00 / 0.46 | hourly, bare metal | [Vantage](https://instances.vantage.sh/aws/ec2/c7g.metal) |
| GCP c4a-highcpu-16 (16 vCPU Axion, 32 GB), us-east1 on-demand / spot | $0.6061 / $0.286 per h | 27.65 / 13.05 | 1.04 / 0.49 | hourly | [gcloud-compute.com](https://gcloud-compute.com/c4a-highcpu-16.html) (data dated 11 Oct 2026) |
| GCP c4a-highcpu-16, europe-west1 on-demand | $0.6674/h | 30.45 | 1.15 | hourly | [gcloud-compute.com](https://gcloud-compute.com/c4a-highcpu-16.html) |
| GCP c4a-highcpu-16, 3-yr CUD | $199.14/mo | 12.45 | 0.47 | commit | [gcloud-compute.com](https://gcloud-compute.com/c4a-highcpu-16.html) |
| Azure D16pls_v6 (16 vCPU Arm, 32 GiB), pay-as-you-go / spot | $0.496 / $0.197 per h | 22.63 / 8.99 | 0.86 / 0.34 | hourly | [Vantage](https://instances.vantage.sh/azure/vm/d16pls-v6) (region not labelled) |
| OCI A1 Flex (Arm): $0.01/OCPU-h + $0.0015/GB-h | flex | 9.49 | 0.36 | per second (flex) | [OCI price list API](https://apexapps.oracle.com/pls/apex/cetools/api/v1/products/?currencyCode=USD) (updated 2026-10-08) |
| OCI E5 Flex (x86): $0.03/OCPU-h (1 OCPU = 2 vCPU) + $0.002/GB-h | flex | 13.87 | 0.52 | flex | [OCI price list API](https://apexapps.oracle.com/pls/apex/cetools/api/v1/products/?currencyCode=USD) |
| OCI preemptible (A1/E5): 50% off | flex | 4.75 / 6.94 | 0.18 / 0.26 | reclaimable | [OCI docs](https://docs.oracle.com/en-us/iaas/Content/Compute/Concepts/preemptible.htm) |
| Hetzner Cloud CAX41 (16 shared Arm vCPU, 32 GB) | €40.99/mo cap, €0.0657/h | 2.87 (cap) / 3.36 (hourly) | 0.11 / 0.13 | hourly with monthly cap | [Hetzner price adjustment](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/) |
| Hetzner Cloud CCX63 (48 dedicated vCPU, 192 GB) | €853.49/mo | 19.93 | 0.75 | hourly with cap | same |
| Hetzner AX162-2 (AMD EPYC 9454P, 256 GB DDR5 ECC) | €842.30/mo + €419 setup | 9.83 | 0.37 | monthly | [price](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/), [specs](https://docs.hetzner.com/robot/dedicated-server/server-lines/ax-server/) |
| Hetzner AX102-1 (Ryzen 9 7950X3D, 128 GB DDR5) | €257.30/mo + €129 setup | 9.01 | 0.34 | monthly | same |
| OVHcloud Public Cloud c3-64 (32 vCore, 64 GB), FR | €0.7301/h | 18.66 | 0.71 | per second | [OVH cloud catalog API](https://api.ovh.com/1.0/order/catalog/public/cloud?ovhSubsidiary=FR) |
| OVHcloud SCALE-a8 (AMD EPYC 9965) + 768 GB RAM, FR | €1,199.99 + €1,584 = €2,783.99/mo | 8.12 | 0.31 | monthly | [OVH bare-metal catalog API (FR)](https://api.ovh.com/1.0/order/catalog/public/baremetalServers?ovhSubsidiary=FR) |
| OVHcloud SCALE-a8 + 768 GB, US-subsidiary catalogue | $1,530 + $2,036 = $3,566/mo | 9.29 | 0.35 | monthly | [OVHcloud US catalog API](https://api.us.ovhcloud.com/1.0/order/catalog/public/baremetalServers?ovhSubsidiary=US) |
| Scaleway POP2-HC-32C-64G (32 vCPU, 64 GB) | €0.8512/h | 21.76 | 0.82 | hourly | [Scaleway Instances](https://www.scaleway.com/en/pricing/virtual-instances/) |
| Scaleway Elastic Metal EM-I620E-NVMe (EPYC 8534P 64C/128T, 576 GB) | €679.99/mo or €1.863/h | 5.95 (monthly) / 11.91 (hourly) | 0.22 / 0.45 | monthly or hourly | [Scaleway Elastic Metal](https://www.scaleway.com/en/pricing/elastic-metal/) |
| DigitalOcean CPU-Optimized (32 dedicated vCPU, 64 GiB) | $672/mo ($1.00/h) | 22.81 | 0.86 | hourly with cap | [DO pricing](https://www.digitalocean.com/pricing/droplets) |
| Vultr voc-c-32c-64gb (32 vCPU AMD, 64 GB) | $640/mo ($0.877/h) | 20.01 | 0.76 | hourly with cap | [Vultr plans API](https://api.vultr.com/v2/plans) |
| Vultr bare metal vbm-24c-384gb-amd (EPYC 9254 24C/48T, 384 GB) | $825/mo ($1.13/h) | 17.19 | 0.65 | hourly | [Vultr metal plans API](https://api.vultr.com/v2/plans-metal) |
| Akamai (Linode) G8 Dedicated 1:2, 64x32 (32 vCPU, 64 GB) | $1.08/h | 24.64 | 0.93 | hourly | [Akamai CPU pricing](https://www.akamai.com/cloud/pricing/cpu) |
| Latitude.sh c4.metal.large (AMD 9655P, 96 cores, 384 GB) | $4.00/h, $2,920/mo | 15.21 | 0.57 | hourly or monthly | [Latitude.sh pricing](https://www.latitude.sh/pricing) |
| Fly.io performance-16x (16 vCPU, 32 GB, iad/ewr) | $528.02/mo | 33.46 | 1.26 (0.76 with the 40% reservation discount) | per second | [Fly.io pricing](https://docs.fly.io/about/pricing) |
| Cloudflare Containers | $0.000020/vCPU-s (active), $0.0000025/GiB-s | $0.072 per fully busy vCPU-h + $0.009/GiB-h | n/a | 10 ms | [Cloudflare Containers pricing](https://developers.cloudflare.com/containers/pricing/) |

#### Hetzner
- Hetzner raised prices twice in 2026. First, an increase of up to 37% took effect on 1 April 2026, announced on 19 February 2026. Hetzner attributed it to RAM/SSD costs, saying DRAM costs had risen up to 500% since September 2025 ([heise](https://heise.de/-11185981); summary via search results, not read in full). Reports disagree on whether existing servers were repriced ([findstack](https://findstack.com/resources/hetzner-price-increase-2026)).
- A second "price adjustment" took effect on 15 June 2026 at 8 AM CEST, for new orders and cloud instance rescales ([Hetzner docs](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/)). Examples in Germany/Finland: CCX13 €15.99→€42.99, CCX63 €374.49→€853.49, CPX22 €7.99→€19.49 and CAX41 €31.49→€40.99. Cloud prices in the USA: CPX51 €237.99, CCX63 €853.49. The dedicated price list shows only new prices: AX42-1 €97.30, AX102-1 €257.30, AX162-1 €612.30, AX162-2 €842.30 and AX162-3 €1,597.30 per month, plus setup fees of €49–€799. The page does not say why prices changed.
- "Limited offerings" sell old hardware at lower prices "as long as supply lasts". Examples: AX162-1-LTD at €317.30 per month (€39 setup) and AX102-1-LTD at €157.30 ([Hetzner docs](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/)).
- Hetzner also publishes USD list prices, about 5–6% above the ECB conversion. CAX41 is listed at $48.49 per month, against €40.99 ≈ $45.93 ([Hetzner docs](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/); my calculation).
- AX162 specs: all variants use an AMD EPYC 9454P. AX162-1 has 128 GB DDR5 ECC, AX162-2 256 GB and AX162-3 512 GB. AX102 uses a Ryzen 9 7950X3D with 128–192 GB. The guaranteed connection is 1 Gbit, with 10G/25G NIC add-ons (not for AX42) ([Hetzner AX docs](https://docs.hetzner.com/robot/dedicated-server/server-lines/ax-server/)).
- The 15 June price list shows dedicated servers in Falkenstein, Nuremberg and Helsinki only. Cloud is in Germany, Finland, Singapore and the USA (Ashburn, Hillsboro) ([price adjustment](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/), [Hetzner Cloud](https://www.hetzner.com/cloud/)). The US cloud price table lists only CCX and CPX servers, not the CAX Arm or CX lines ([price adjustment](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/)).
- IPv4: a cloud Primary IPv4 costs €0.50/month. A dedicated-server IPv4 costs €1.70/month, and an additional single IP adds a €4.90 setup fee ([Hetzner IPv4 pricing](https://docs.hetzner.com/general/others/ipv4-pricing/)).
- Traffic: "We only bill for outgoing traffic. Incoming and internal traffic is free". Billing FAQ example: "Your server has 20TB included traffic". Overage is billed in 100 MB blocks ([Hetzner billing FAQ](https://docs.hetzner.com/cloud/billing/faq/)).
- Nested virtualization on cloud servers: "No, this is not possible on cloud server". Each customer has a default resource limit, and an increase can be requested only after one month as a customer and payment of the first invoice. Plan availability "is checked and updated about every 15 minutes" ([Hetzner cloud FAQ](https://docs.hetzner.com/cloud/servers/faq/)).

#### OVHcloud
- Public Cloud (FR catalogue, EUR, hourly): c3-32 (16 vCore/32 GB) €0.365/h, c3-64 (32/64) €0.7301/h, b3-64 (16/64) €0.4092/h. Outbound instance bandwidth (`bandwidth_instance_out`) is priced at €0 in the main regions. Singapore and Sydney have a tier at €0.01. A floating IP costs €0.0027/h or €1.80/month ([OVH cloud catalog API](https://api.ovh.com/1.0/order/catalog/public/cloud?ovhSubsidiary=FR)).
- Bare metal (FR catalogue, EUR per month, no commitment): SCALE-a8 (AMD EPYC 9965) €1,199.99 with 128 GB, plus €648 for 384 GB or €1,584 for 768 GB. SCALE-a6 (EPYC 9655) is €979.99 and SCALE-a5 (EPYC 9555) €817.99. ADVANCE-5 (EPYC 8224P, 96 GB) is €394.99 and ADVANCE-2 (EPYC 4344P, 64 GB) €154.99. Scale servers include a 5 Gbps public port at €0, with 10–25 Gbps for €500–€1,250 more. Advance servers include 3 Gbps. Paying 12 months upfront costs about 5% less (SCALE-a8: €13,679.89 per year = €1,139.99 per month) ([OVH bare-metal catalog API](https://api.ovh.com/1.0/order/catalog/public/baremetalServers?ovhSubsidiary=FR)).
- The catalogue's "installation" line equals one month's rental on monthly billing and €0 on 12- or 24-month upfront terms. Its description is "rental (only applicable for 1 time)". That reads like a first-month charge rather than a separate setup fee (same source; my reading).
- The US-subsidiary catalogue, in USD, prices SCALE-a8 at $1,530 plus $2,036 for 768 GB. SCALE-a6 is $1,249 and ADVANCE-5 $504. Plan codes end in "-ca" ([OVHcloud US catalog API](https://api.us.ovhcloud.com/1.0/order/catalog/public/baremetalServers?ovhSubsidiary=US)).
- The OVH web price page showed RISE-L (Ryzen 9 9950X 16C/32T, 128 GB) at $177 per month with a 1 Gbps port, marked "Soon available" ([OVH bare-metal prices](https://www.ovhcloud.com/en/bare-metal/prices/)).

#### Scaleway
- The Instances page says "List prices include egress and IPv6 addresses" and that pricing is "without egress fees". Attached public IPv4 addresses are excluded. Examples: POP2-HC-32C-64G €0.8512/h, POP2-32C-128G €1.18/h, PLAY2-MICRO (4 vCPU/8 GB) €0.05508/h ([Scaleway Instances](https://www.scaleway.com/en/pricing/virtual-instances/)).
- Elastic Metal is billed hourly or monthly. EM-I620E-NVMe (EPYC 8534P 64C/128T, 576 GB) is €679.99 per month or €1.863/h. EM-I520E (48C/96T, 384 GB) is €569.99 and EM-I420E (32C/64T, 256 GB) €449.99. A flexible IPv4 costs €0.005/h and IPv6 is free. The page lists only PAR-1 and PAR-2 zones and no commitment discounts ([Scaleway Elastic Metal](https://www.scaleway.com/en/pricing/elastic-metal/)).

#### DigitalOcean, Vultr, Akamai, Fly.io, Latitude.sh, Cloudflare, Equinix Metal
- DigitalOcean: the CPU-Optimized droplet with 32 vCPU and 64 GiB costs $672 per month and includes 9,000 GiB of transfer. General Purpose 32 vCPU/128 GiB costs $1,008 ([DO pricing](https://www.digitalocean.com/pricing/droplets)). Transfer allowance is "pooled cumulatively across all Droplets at the team level". Overage is $0.01/GiB, inbound is free, and DOKS worker nodes draw on the same pool ([DO bandwidth docs](https://docs.digitalocean.com/platform/billing/bandwidth/)). DigitalOcean lists "Bare Metal GPUs" as a product, among the Hatch credit exclusions ([DO Hatch](https://www.digitalocean.com/hatch)). No CPU bare-metal offer appears on the Droplet pricing page. Whether droplets support nested virtualization was not verified.
- Vultr (live API, 2026-10-12): voc-c-32c-64gb costs $640 per month ($0.877/h) with 9,216 GB of bandwidth. The shared vhp-12c-24gb-amd costs $144 per month. Bare metal vbm-24c-384gb-amd (EPYC 9254, 24C/48T, 384 GB) costs $825 per month ($1.13/h) with 10,240 GB. The "preemptible" price fields equal the on-demand prices, and `deploy_preemptible` is false for these plans ([Vultr plans API](https://api.vultr.com/v2/plans), [metal API](https://api.vultr.com/v2/plans-metal)). Plan locations include US sites (ewr, ord, dfw, lax, sea, sjc, atl, mia) and EU sites (ams, fra, cdg, lhr, mad, waw, sto, man, mxp) (same APIs).
- Akamai (Linode): G8 Dedicated plans are hourly-only and include no bundled transfer, for example 1:2 64x32 at $1.08/h. G7 Dedicated 16 vCPU/32 GB costs $346 per month with 7 TB. Egress overage is "US$0.005 per GB" ([Akamai CPU pricing](https://www.akamai.com/cloud/pricing/cpu)). Managed Kubernetes has "no per-cluster fee", and an extra IP is billed separately ([Akamai pricing](https://www.akamai.com/cloud/pricing)).
- Fly.io: performance-1x (2 GB) costs $33.00 per month and shared-cpu-1x (256 MB) $2.19. Extra RAM is $6.00/GB/month. Reservations give "a 40% discount". Egress costs $0.02/GB in North America and Europe, inbound is free, and a dedicated IPv4 costs $2/month. Other regions carry a price multiplier, for example `fra` 1.15 ([Fly.io pricing](https://docs.fly.io/about/pricing)).
- Latitude.sh: Gen 4 bare metal includes m4.metal.small (6 cores, 48 GB) at $0.41/h and c4.metal.large (AMD 9655P, 96 cores, 384 GB) at $4/h ($2,920 per month). Every plan lists "20 TB free out / ∞ in". The Metal lineup has no ARM plans ([Latitude.sh pricing](https://www.latitude.sh/pricing)).
- Cloudflare Containers: memory and disk are billed as provisioned and CPU only on active use. NA/EU egress costs $0.025/GB after 1 TB included. Instance types run from lite (1/16 vCPU, 256 MiB) to standard-4 (4 vCPU, 12 GiB) ([pricing](https://developers.cloudflare.com/containers/pricing/)). Default account limits are 1,500 concurrent vCPU, 6 TiB memory and 30 TB disk, and can be raised on request. Custom instance types need "at least 3 GiB memory per vCPU" and allow at most 4 vCPU ([limits](https://developers.cloudflare.com/containers/platform-details/limits/)).
- Cloudflare Browser Rendering, now named "Browser Run" in the docs: $0.09 per browser-hour beyond 10 included hours on Workers Paid, plus $2.00 per concurrent browser beyond 10. Concurrency is the monthly average of daily peaks ([pricing](https://developers.cloudflare.com/browser-rendering/pricing/)).
- Equinix Metal: end-of-life was announced 7 November 2024, with end-of-life and end-of-support on 30 June 2026 ([Equinix EOL list](https://docs.equinix.com/releases/end-of-life/eol-products-services), [Equinix blog](https://deploy.equinix.com/blog/sunsetting-equinix-metal); summarised via search results). Bare-metal rivals marketed migration help ([Latitude.sh blog](https://www.latitude.sh/blog/equinix-metal-sunset-how-to-reduce-the-migration-burden)).

#### Hyperscaler network reference
- AWS NAT Gateway costs $0.045 per hour plus $0.045 per GB processed. The charge applies "regardless of the traffic's source or destination". A public IPv4 costs $0.005/h, in use or idle ([AWS VPC pricing](https://aws.amazon.com/vpc/pricing/)).
- AWS gives "100 GB of free data transfer out to the internet free each month", aggregated across services and regions, and asks customers above 500 TB/month to contact sales ([AWS EC2 pricing](https://aws.amazon.com/ec2/pricing/on-demand/)). The per-GB tiers did not render. The usual us-east-1 reference is $0.09 for the first 10 TB, $0.085 for the next 40 TB, $0.07 for the next 100 TB and $0.05 above 150 TB. I did not re-verify these tiers this session, so they should be checked against the AWS sheet.
- Azure internet egress from North America/Europe: the first 100 GB is free. After that, the Microsoft premium network costs $0.087/GB for the next 10 TB, $0.083 for the next 40 TB, $0.07 for the next 100 TB and $0.05 for the next 350 TB. Transit-ISP routing costs $0.08, $0.065, $0.06 and $0.04 ([Azure bandwidth](https://azure.microsoft.com/en-us/pricing/details/bandwidth/)).
- OCI outbound data transfer from North America, Europe and the UK: a $0 first tier, then $0.0085/GB ([OCI price list API](https://apexapps.oracle.com/pls/apex/cetools/api/v1/products/?currencyCode=USD)). The free tier is 10 TB per month by long-standing OCI policy; the API shows only a $0 tier, so the size was not re-verified.

#### Firecracker / KVM availability (needed for microVMs running untrusted Recipe code)
- AWS now supports nested virtualization on virtual (non-metal) instances: M7i, M7i-flex, M8i, M8id, M8i-flex, C7i, C7i-flex, C8i, C8id, C8i-flex, R7i, R7iz, R8i, R8id, R8i-flex, X8i, I7i and I7ie. Supported L1 hypervisors are KVM and Hyper-V, at no extra cost. For performance-sensitive work AWS recommends evaluating bare metal ([AWS docs](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-nested-virtualization.html)). Graviton families are not on the list. AWS .metal instances cost the same per vCPU as virtual ones: c7g.metal at $2.32/h for 64 vCPU equals 4 × c7g.4xlarge ([Vantage](https://instances.vantage.sh/aws/ec2/c7g.metal); my calculation).
- GCP: nested virtualization is excluded on E2, memory-optimized, Arm and AMD VMs (except N4D). The only L1 hypervisor is KVM. CPU-bound nested VMs "might experience a 10% or greater decrease in performance" ([GCP docs](https://docs.cloud.google.com/compute/docs/instances/nested-virtualization/overview)).
- Hetzner Cloud: no nested virtualization ([Hetzner cloud FAQ](https://docs.hetzner.com/cloud/servers/faq/)). Hourly bare metal is available from Scaleway Elastic Metal ([Scaleway](https://www.scaleway.com/en/pricing/elastic-metal/)), Vultr ([Vultr metal API](https://api.vultr.com/v2/plans-metal)) and Latitude.sh ([pricing](https://www.latitude.sh/pricing)). Monthly bare metal comes from Hetzner and OVHcloud (sources above). OCI bare metal shapes cannot be preemptible ([OCI docs](https://docs.oracle.com/en-us/iaas/Content/Compute/Concepts/preemptible.htm)).
- E2B's open-source Firecracker runtime "needs Linux with KVM" and ships Terraform for GCP, AWS and Azure plus a Kubernetes manifest ([e2b-dev/infra README](https://github.com/e2b-dev/infra)).

#### ARM availability
- Arm compute is offered by AWS Graviton (c7g), GCP Axion (c4a), Azure (D16pls_v6) and OCI (A1, A2, A4 price lines) (sources above). Hetzner offers it only in its EU cloud (CAX). Latitude.sh's Metal lineup lists no ARM. The Vultr plans read here are Intel/AMD, apart from a GH200 bare metal (Grace, Frankfurt only) ([Vultr metal API](https://api.vultr.com/v2/plans-metal)).

#### Elasticity
- Hourly or finer billing with spot or preemptible capacity: AWS, GCP, Azure and OCI. OCI preemptible capacity "can be reclaimed at any time", with a two-minute event ([OCI](https://docs.oracle.com/en-us/iaas/Content/Compute/Concepts/preemptible.htm)).
- Hourly bare metal: Scaleway, Vultr and Latitude.sh (sources above). Hetzner dedicated servers are monthly with setup fees. OVHcloud bare metal is monthly, or 12/24 months upfront.
- Hetzner Cloud is hourly but subject to account limits and plan availability ([Hetzner FAQ](https://docs.hetzner.com/cloud/servers/faq/)). Cloudflare Containers bill per 10 ms but cap concurrency by default ([limits](https://developers.cloudflare.com/containers/platform-details/limits/)).

#### Reliability and provider-risk events
- **AWS us-east-1, 19–20 October 2025:** "a latent race condition in the DynamoDB DNS management system that resulted in an incorrect empty DNS record". Impact ran from 11:48 PM PDT on 19 October to 2:20 PM PDT on 20 October. EC2 launches failed from 2:25 to 10:36 AM, and EKS, ECS, Fargate, Lambda and IAM sign-in were also affected ([AWS post-event summary](https://aws.amazon.com/message/101925/)).
- **Cloudflare, 18 November 2025:** a global outage from 11:20 to 17:06 UTC, caused by an oversized Bot Management feature file ([Cloudflare blog](https://blog.cloudflare.com/18-november-2025-outage/)).
- **Equinix Metal:** shut down on 30 June 2026 (above).
- **Hetzner:** the 2026 price shocks hit RAM-heavy and dedicated-vCPU products hardest (above).

### Inferences
- **Hetzner is no longer the cheap default for every shape.** After the June 2026 change, its dedicated-vCPU cloud (CCX) costs only 25% less than AWS on-demand. The cheapest Hetzner lines are the shared CX/CAX cloud servers and the AX dedicated servers.
- **The CAX41 has three limits.** At about $2.9–3.4 per browser unit it is the cheapest elastic capacity found. But it is EU-only, its vCPUs are shared, and it cannot host Firecracker.
- **EU bare metal is 2.7–4.5× cheaper than AWS on-demand per browser unit** when SMT threads are counted as vCPUs, and roughly 1.1–2.0× cheaper than AWS spot or 3-year RIs (Hetzner AX162-2 is the least cheap, Scaleway the most). This covers Scaleway EM-I620E, OVH SCALE-a8 and Hetzner AX162/AX102. Section 6 shows the advantage shrinks if a hyperthread is worth less than a Graviton core.
- **OCI is the outlier among hyperscalers.** Its x86 flex price per vCPU is about half of AWS on-demand, and its Arm A1 price is about one third, and egress is almost free. It is the only big cloud that matches EU bare-metal unit costs while keeping per-second, API-driven elasticity.
- **The "AWS tax" is mostly network for a crawler or hosted-browser product.** The NAT-gateway trap and $0.05–0.09/GB egress matter more than the compute premium, which spot or Savings Plans largely remove (worked numbers in section 6).
- **Firecracker no longer forces .metal on AWS.** Nested virtualization now covers 7th- and 8th-generation Intel families, and .metal carries no per-vCPU premium. On GCP, Firecracker means Intel (or N4D) plus nested overhead. Hetzner Cloud rules it out, but Hetzner dedicated, OVH, Scaleway Elastic Metal, Vultr bare metal and Latitude.sh allow it.

### Gaps
- GCP internet egress and IPv4 prices: the GCP network pricing page did not render. They are usually cited as $0.085–0.12/GB, which is unverified.
- Azure and GCP NAT pricing and per-IPv4 prices were not retrieved.
- Managed Kubernetes and serverless-container pricing (EKS, GKE, AKS, OKE, DOKS, Kapsule, OVH MKS, VKE) was not re-verified, apart from Akamai ("no per-cluster fee"). Whether Hetzner offers any managed Kubernetes is unverified.
- Not verified: Hetzner dedicated "unlimited traffic", OVHcloud bare-metal "unmetered" EU bandwidth (inferred from the catalogue's port-speed-only options), egress pricing for Scaleway Elastic Metal, and Vultr's overage price per GB.
- The exact datacentre locations behind OVHcloud's US-catalogue "-ca" plans (Canada, US or both) are unknown.
- Support tiers, response-time SLAs, and independent price-performance benchmarks (Cloud Mercato, VPSBenchmarks) were not retrieved; searches were exhausted.
- Unverified: the 2021 OVHcloud Strasbourg fire and any Hetzner or OVH outage history.
- AWS Paris and Frankfurt prices were not fetched. GCP's europe-west1 price is 10% above us-east1, which suggests AWS's EU premium is similar; that is an inference.

## 2. What do providers' terms say about scraping, crawling and browser automation for third parties, and how are their IP ranges rated?

### Takeaway
None of the policies retrieved bans web scraping outright. They ban harm: disrupting the systems being crawled, scanning, spoofing source IPs, and evading use limits. All of them let the provider suspend or lock accounts without notice. DigitalOcean's wording is the most hostile to a stealth-scraping platform. Hetzner's terms allow locking servers "without prior notice". Cloudflare's Browser Run is structurally unusable for stealth, because Cloudflare always flags it as a bot. IP-intelligence data classify both AWS and Hetzner as "Hosting" networks, so no provider's own addresses escape datacenter treatment. Suspension anecdotes and vendor-specific reputation scores could not be verified this session.

### Cited Findings
- **AWS AUP** (last updated 1 July 2021). The retrieved text has no crawling-specific clause. It prohibits using the services "to violate the security, integrity, or availability of any user, network, computer or communications system", and AWS "may investigate any suspected violation … and remove or disable access" ([AWS AUP](https://aws.amazon.com/aup/)).
- **DigitalOcean AUP** (last updated 20 March 2026) prohibits:
  - "Monitoring or crawling of a System that impairs or disrupts the System being monitored or crawled";
  - "Using manual or electronic means to avoid any use limitations placed on a System";
  - collecting information "by deceit or covert means".

  It also says suspension "applies even if a violation is committed unintentionally or without your authorization" and "You are responsible for violations of this AUP by anyone using your account" ([DO AUP](https://www.digitalocean.com/legal/acceptable-use-policy)).
- **Hetzner** cloud and dedicated agreements prohibit "The scanning of foreign networks or foreign IP addresses", "The use of fake source IPs" and crypto mining. They say nothing specific about scraping ([cloud agreement](https://www.hetzner.com/legal/cloud-server/), [dedicated agreement](https://www.hetzner.com/legal/dedicated-server/)). The General Terms (version 2.0.0, 27 October 2021) add:
  - §5.2: "we reserve the right - without prior notice - to lock the server and terminate the contract without notice";
  - §8.3–8.4: locking for non-compliance or illegal activity;
  - §9.2: the customer indemnifies Hetzner against third-party claims;
  - US special terms: Hetzner "may immediately terminate any account which we believe, at our sole and absolute discretion" ([Hetzner T&C](https://www.hetzner.com/legal/terms-and-conditions/)).
- **Fly.io AUP**: prohibits "Security testing (network or application scanning, phishing, or penetration testing)" and spam proxying. Fly.io "reserves the right to suspend or terminate your account, with or without notice". It has no explicit scraping clause ([Fly.io AUP](https://fly.io/legal/acceptable-use-policy/)).
- **Cloudflare Browser Run**: "Browser Run requests are always identified as bot traffic by Cloudflare". Requests carry identifying headers (`cf-biso-request-id`, `cf-biso-devtools`). Site owners need an Enterprise plan to allowlist it ([Cloudflare FAQ](https://developers.cloudflare.com/browser-rendering/faq/)).
- **IP intelligence**: IPinfo classifies AS16509 (Amazon.com, Inc., 174,498,304 IPv4 addresses) as "Hosting" ([IPinfo](https://ipinfo.io/AS16509)). It classifies AS24940 (Hetzner Online GmbH, 3,237,120 IPv4) as "Hosting" and "Hosting / Cloud", with no abuse flag displayed ([IPinfo](https://ipinfo.io/AS24940)).
- **A good-citizen pattern**: Common Crawl's CCBot "is now run on dedicated IP address ranges with reverse DNS" ([Common Crawl FAQ](https://commoncrawl.org/faq)).

### Inferences
- **Abuse complaints will land on the provider, not on Goodfetch.** The marketplace runs third-party Recipes, so whichever provider hosts the fleets receives the complaints. Goodfetch Cloud needs its own abuse desk, customer verification, and per-tenant rate limits and egress controls. These map onto the engine's existing egress policy (ADR 0017) and fair per-domain admission (ADR 0004). Providers that lock servers first and ask later (Hetzner §5.2) raise the operational risk of mixing fleets and the control plane on one account.
- **DigitalOcean is a poor fit for stealth fleets.** Its "avoid any use limitations" and "deceit or covert means" clauses could be read against stealth browsing or anti-bot evasion. That makes DigitalOcean riskier for the stealth product than for the control plane. Hetzner and OVH bans on scanning and spoofing are compatible with ordinary port-80/443 crawling.
- **Cloudflare Browser Run cannot replace the Goodfetch stealth Chromium.** It is a managed Chrome on Cloudflare's network that is always labelled a bot.
- **Provider choice matters little for anti-bot outcomes.** Both hyperscaler and budget-host ASNs are "Hosting", so serious targets will treat direct fetches from any of them as datacenter traffic. Goodfetch Cloud already sells managed proxies (ADR 0018 context), and routing target-facing traffic through residential or mobile pools moves reputation risk off the provider. Provider choice then mainly affects complaint volume and how the provider reacts.
- **Many sites treat Hetzner and OVH ranges worse than AWS.** This is widely believed (for example, sites that block whole budget-host ASNs) but was not verified this session.

### Gaps
- Not retrieved, because the pages did not render, returned 403, or searches ran out: the GCP AUP, Azure/Microsoft AUP, Oracle, OVHcloud, Scaleway, Vultr and Akamai policy texts.
- Documented cases of accounts suspended for scraping or abuse complaints (Hacker News or Reddit reports about Hetzner, OVH, DigitalOcean and others) could not be searched, so none are cited.
- Anti-bot vendor statements (Cloudflare Bot Management, DataDome, Akamai, HUMAN) on how they score specific cloud ASNs were not found.

## 3. What startup credit programs are available in 2026, for how much and under what conditions?

### Takeaway
Credits favour the hyperscalers and Cloudflare. AWS Activate goes up to $200k for pre-Series B companies, with invite-only "$200,000+" AI credits on top. Microsoft offers up to $150k and Cloudflare up to $350k. Among EU providers, OVHcloud offers up to €100k and Scaleway up to €36k. Hetzner has no program found. AWS credits can also pay for Amazon Bedrock model usage, which matters for the AI-agent workload.

### Cited Findings
- **AWS Activate**:
  - Founders tier (self-funded): starts at "$1,000 USD in Activate Credits", up to $5,000.
  - Portfolio tier: "Up to $200,000 USD", for pre-Series B companies founded in the last 10 years, on a paid tier plan, with an Activate Provider Org ID from an accelerator, angel or VC.
  - "AWS Credits for AI Startups": "$200,000+", invite-only after Portfolio.
  - Credits can be used to build "AI-native products with Amazon Bedrock".
  - The page does not state an expiry ([AWS Activate](https://aws.amazon.com/startups/credits)).
- **Microsoft for Startups**: "up to $150,000 in credits". Tiers, eligibility and duration were not on the page ([Microsoft for Startups](https://www.microsoft.com/en-us/startups)).
- **Cloudflare for Startups**: Tier 3 is $10k (bootstrapped, under $1M raised). Tier 2 is $100k (under $5M, affiliated-partner funded). Tier 1 is $350k ($5M+ raised), raised "from $250,000 to $350,000". Other terms:
  - Credits are valid for one year with no extensions.
  - Companies must be incorporated within 10 years and funded up to Series B within the last 12 months.
  - Credits cover Workers, Durable Objects, R2 (up to $10k) and Workers AI (capped at $2.5k/$10k/$50k by tier).
  - The page does not mention Containers or Browser Rendering ([Cloudflare for Startups](https://www.cloudflare.com/forstartups/)).
- **OVHcloud Startup Program**: Start gives €10,000 of credits for 12 months plus 6 hours of engineer time. Scale gives "Up to €100,000" for 12 months plus 20 hours. The page lists no eligibility criteria ([OVHcloud Startup Program](https://startup.ovhcloud.com/en/)).
- **Scaleway Startup Program**: Founders gets up to €1,000 over one year. Early Stage gets €1,500/month for 6 months (€9,000). Growth Stage gets €3,000/month for 12 months (€36,000). Applicants must be under 5 years old, have fewer than 50 employees and not already be Scaleway clients. The FAQ gives conflicting monthly figures (€500 and €1,500) ([Scaleway Startup Program](https://www.scaleway.com/en/startup-program/)).
- **DigitalOcean Hatch**: partner-awarded credits over 12 months, with a "$10,000 monthly limit". Eligibility: "$10m or less" raised and AI-native. Credits exclude GPU products ([DO Hatch](https://www.digitalocean.com/hatch)).

### Inferences
- **Credits can tilt early economics toward a hyperscaler.** If Goodfetch qualifies for AWS Portfolio ($200k) plus AI credits, the first 12–24 months of the control plane, and even of the fleets, could cost little on AWS. For example, $200k buys about 12,000 browser-months of AWS spot "rich" capacity including base egress (about $16.1 per browser-month, section 6). That is roughly 1,000 always-on browsers for a year. Spending credits on Bedrock model calls for the AI agents may be worth more than spending them on compute.
- **Credits expire, and architecture lock-in does not.** Keeping the fleet layer portable (engine binary plus an image that runs on any KVM host) lets Goodfetch use AWS credits now and move fleets to bare metal when credits run out.

### Gaps
- **Google for Startups Cloud Program**: the page did not render (cloud.google.com/startup, /benefits, /faq). Commonly reported figures of up to $200k (Scale) and $350k (AI tier) over two years are **unverified**.
- **Oracle for Startups**: the page returned 403. Amounts and conditions are unknown.
- Neither Hetzner nor Vultr has a startup credit program verified; none was found or searched.

## 4. What did comparable companies choose, which hybrid patterns exist, and what does inter-cloud traffic cost?

### Takeaway
The public evidence found points to AWS for the best-known actor platform (Apify, as of its 2020 whitepaper) and AWS-named regions for the leading hosted-browser company (Browserbase). The best-known example of exiting AWS for owned hardware is 37signals, which claims about $10M saved over five years; it is not a scraping company. The hybrid pattern makes sense because AWS ingress is free: fleets elsewhere can push results into an AWS control plane at no AWS cost. Data flowing out of AWS (checkpoints restored on other clouds, customer downloads) pays AWS egress.

### Cited Findings
- **Apify**: "The Apify platform's core services run on Amazon Web Services (AWS) … located in Northern Virginia, United States (the us-east-1 region)". It runs on EC2, S3, DynamoDB, SQS, ECS, Lambda and CloudFront, with MongoDB Atlas also in us-east-1. Proxy servers sit in other countries. The whitepaper is dated 13 January 2020 ([Apify security whitepaper](https://apify.com/security-whitepaper.pdf)). The current security page does not name a provider but states SOC 2 Type II ([Apify security](https://docs.apify.com/platform/security)).
- **Browserbase**: region identifiers `us-west-2` (default), `us-east-1`, `eu-central-1` and `ap-southeast-1`. The page says nothing about the underlying provider ([Browserbase docs](https://docs.browserbase.com/guides/multi-region)).
- **Browserless**: endpoints `production-sfo`, `production-lon` and `production-ams`; no provider stated ([Browserless docs](https://docs.browserless.io/baas/connection-url-patterns)).
- **Common Crawl**: "The crawl data is stored on Amazon's S3 service, allowing it to be bulk downloaded as well as directly accessed for Map-Reduce processing in EC2" ([Common Crawl FAQ](https://commoncrawl.org/faq)).
- **E2B**: its Firecracker sandbox runtime for AI agents "ships as Terraform for GCP, AWS, and Azure", and also as a single-host package for "one machine you own" with KVM ([e2b-dev/infra](https://github.com/e2b-dev/infra)).
- **Cloudflare Browser Run**: requests "originate from Cloudflare's global network" ([Cloudflare FAQ](https://developers.cloudflare.com/browser-rendering/faq/)).
- **37signals**: moved Basecamp, HEY and other apps off AWS onto its own hardware "without adding any new staff". "Leaving the cloud will save us ~$10 million over five years", a cut of "between half and two-thirds". A headline cites spending $600k on servers ([37signals cloud exit](https://basecamp.com/cloud-exit)).
- **Inter-cloud prices**:
  - AWS NAT processing costs $0.045/GB in either direction ([AWS VPC pricing](https://aws.amazon.com/vpc/pricing/)).
  - Hetzner bills only outgoing traffic, with incoming free ([Hetzner billing FAQ](https://docs.hetzner.com/cloud/billing/faq/)).
  - OVHcloud instance egress is €0 ([OVH catalog](https://api.ovh.com/1.0/order/catalog/public/cloud?ovhSubsidiary=FR)).
  - Fly private-network cross-region traffic costs $0.006/GB in NA/EU ([Fly.io pricing](https://docs.fly.io/about/pricing)).
  - AWS internet egress follows the AWS reference tiers in section 1.

### Inferences
- **Browserbase probably runs on AWS.** Its four region IDs are exactly AWS region names. This is an inference, not a disclosure.
- **The sfo/lon/ams endpoints match DigitalOcean's region codes** but also generic city codes. This is weak evidence only.
- **Hybrid layout:**
  - **Control plane on AWS**: gateway, Console, billing, marketplace metadata, run index and KMS. AWS supplies the managed services, compliance and credits.
  - **Fleets** of browsers, crawl workers and Firecracker hosts on bare metal: OVHcloud or Hetzner in the EU, OVHcloud US, Latitude.sh or Vultr in the US, or OCI for elastic overflow.
  - **Traffic directions:** fleet → AWS traffic (results, logs, run-state uploads) is free on the AWS side and free or included on the fleet side. AWS → fleet commands are tiny.
  - **What costs money:** checkpoints stored in S3 and restored on another cloud, and customers downloading datasets, screenshots or recordings from S3.
  - **Where heavy data should live:** in object storage next to the fleets or behind a zero-egress CDN, with only metadata on AWS. This fits ADR 0002, where checkpoints are portable files.
- **Cross-provider migration costs.** Run migration between machines (ADR 0002) across providers costs one egress per checkpoint on the source side, so placement should prefer migrating within one provider.
- **Live view is the largest egress stream.** For hosted browsers it is WebRTC or screencast capture at about 25 fps (ADR context), which is why egress-included providers matter most for the Browserbase-like product (section 6, "heavy" case).

### Gaps
- The hosting of Firecrawl, Spider, Steel, Hyperbrowser, Kernel, ScrapingBee, ZenRows, Bright Data and Zyte could not be researched, because searches were exhausted and repositories hold no deployment manifests (Firecrawl's repo has no infra files).
- Ahrefs' "saved $400M by not using the cloud" post returned 403, so it is not cited.
- No public example was found of a scraping company that runs its control plane on AWS and its fleets on Hetzner or OVH. This is a gap, not evidence of absence.

## 5. EU sovereignty and GDPR for an EU-based seller

### Takeaway
Using AWS is lawful for a French seller under the EU–US Data Privacy Framework of July 2023. But any US-parented provider stays within reach of the US CLOUD Act "regardless of whether" data sits outside the US. The AWS European Sovereign Cloud (Brandenburg, EU-resident operations) addresses operational access, but its pages do not mention the CLOUD Act. SecNumCloud-qualified EU providers (OVHcloud's qualified offers, 3DS Outscale at SecNumCloud 3.2) are the answer for French public-sector or regulated buyers. No evidence was found that US customers weigh sovereignty in this market.

### Cited Findings
- **CLOUD Act (18 U.S.C. § 2713)**: providers must preserve and disclose data "regardless of whether such communication, record, or other information is located within or outside of the United States" ([Cornell LII](https://www.law.cornell.edu/uscode/text/18/2713)).
- **EU–US Data Privacy Framework**: the European Commission adopted its adequacy decision on 10 July 2023 ([European Commission](https://commission.europa.eu/law/law-topic/data-protection/international-dimension-data-protection/eu-us-data-transfers_en)).
- **AWS European Sovereign Cloud**:
  - "The first AWS Region … is located in the State of Brandenburg, Germany"; the sign-in identifier is `eusc-de-east-1`.
  - Local Zones are planned in Belgium, the Netherlands and Portugal.
  - It is run by "a new parent company and three local subsidiaries incorporated in Germany (GmbH)", "led by EU citizens".
  - "only personnel who are EU residents, located in the EU, have control of day-to-day operations"; "zero operational access outside of EU borders".
  - The page does not mention the CLOUD Act. A linked press-release URL suggests a January 2026 launch ([aws.eu](https://aws.eu/)).
- **OVHcloud**: "In December 2020, OVHcloud obtained the SecNumCloud qualification for its VMware on OVHcloud solution". The navigation also lists Bare Metal Pod and an "SNC Cloud Platform". The page promises data will not be "accessible, transferred or processed outside of the European Union", without an explicit CLOUD Act claim ([OVHcloud SecNumCloud](https://www.ovhcloud.com/en/compliance/secnumcloud/)).
- **3DS Outscale**: "le cloud souverain qualifié SecNumCloud 3.2" ("the sovereign cloud qualified SecNumCloud 3.2"). It says data are "protégées des lois extra-européennes (Cloud Act, etc.)" ("protected from extra-European laws (Cloud Act, etc.)"). This is a vendor claim ([Outscale](https://fr.outscale.com/secnumcloud/)).
- **ANSSI catalogue**: ANSSI publishes the authoritative catalogue of qualified services as a PDF updated at least monthly ([ANSSI](https://cyber.gouv.fr/produits-services-qualifies)).
- **Hetzner** is Hetzner Online GmbH, with special terms for its US service ([Hetzner T&C](https://www.hetzner.com/legal/terms-and-conditions/)). **OVHcloud** runs a separate US subsidiary with its own catalogue, USD prices and tax rate 0 ([OVHcloud US catalog API](https://api.us.ovhcloud.com/1.0/order/catalog/public/baremetalServers?ovhSubsidiary=US)).

### Inferences
- **For general B2B SaaS sales, AWS EU regions (Paris, Frankfurt) are lawful and standard.** The seller's GDPR exposure comes more from what it scrapes (personal data in pages) than from where it hosts. A sovereign option matters for French and EU public-sector or regulated tenders. For those, hosting EU tenants on SecNumCloud-qualified or EU-parented infrastructure is a differentiator that Apify (AWS us-east-1) and Browserbase (AWS-named regions) apparently do not offer.
- **Fleets in the EU on EU-parented bare metal plus a US fleet** (OVHcloud US, or AWS/OCI US) give data residency per tenant at low cost. The control plane can stay on AWS, or move to the AWS European Sovereign Cloud for EU tenants if buyers ask.
- **US customers are unlikely to care about EU sovereignty.** They usually ask for SOC 2 (as Apify advertises) and US data residency. This is an inference without a cited survey.

### Gaps
- Not verified: Scaleway's and S3NS's (Thales–Google) SecNumCloud status, the scope of OVHcloud's newer qualified offers, and the ANSSI PDF itself.
- Not verified: the September 2025 EU General Court ruling upholding the Data Privacy Framework (Latombe) and the reported June 2025 French Senate testimony by Microsoft France on CLOUD Act exposure; neither page was retrieved.
- AWS European Sovereign Cloud pricing compared with standard regions was not on the page.

## 6. Bottom line: is AWS the cheapest or best option per workload, and by how much do alternatives undercut it at 100, 1,000 and 10,000 concurrent browsers?

### Takeaway
AWS is not the cheapest option for any of the four fleet workloads: it wins on managed services, credits and elasticity. At 10,000 always-on "rich" browsers, compute plus network costs about $351k/month on AWS on-demand and about $161k on AWS spot or $168k on 3-year RIs. EU bare metal costs $71k (Scaleway) to $117k (Hetzner AX), Hetzner's shared Arm cloud $36k, and OCI $85k–$115k. That puts AWS spot 1.4–2.3× above EU bare metal on a thread-equals-vCPU basis. Once SMT threads are discounted the two are roughly at parity (bare metal 32% cheaper to 11% dearer). When hosted-browser live-view egress is heavy, AWS spot is 2.2–3.5× above EU bare metal (2.6× above OVHcloud). The robust recommendation is a hybrid. Keep AWS for the control plane, AI-model credits and burst capacity. Put the steady browser, crawl and Firecracker fleets on EU and US bare metal or OCI. Never run crawlers behind an AWS NAT gateway.

### Cited Findings

#### Assumptions (stated by me, not sourced)
- **Lean browser "L":** 0.5 vCPU + 1 GB. **Rich browser "R":** 1 vCPU + 2 GB.
- **Packing:** 85% of each node's vCPU and RAM is usable for browsers; the rest goes to OS, engine, isolation and headroom.
- **Load:** fleets are sized for 24/7 peak.
- **Base traffic:** 0.5 GB/browser-hour of ingress (page downloads) and 0.05 GB/browser-hour of internet egress (requests, CDP and results).
- **Heavy hosted-browser case:** 0.30 GB/browser-hour egress (WebRTC/screencast live view watched part of the time).
- **Exclusions:** support plans, storage, proxies and model costs are excluded, as are Hetzner setup fees (shown separately).
- **Unit prices** are those cited in section 1. All totals are my calculation (Python).

#### Per-browser monthly compute cost (24/7) and ratio to AWS c7g on-demand

| Option | L $/browser-mo | R $/browser-mo | R × AWS on-demand |
|---|---|---|---|
| AWS c7g.4xlarge on-demand / spot / 3-yr RI | 15.68 / 6.54 / 6.89 | 32.57 / 13.59 / 14.32 | 1.00 / 0.42 / 0.44 |
| AWS c7a.4xlarge (x86) on-demand / spot | 22.20 / 8.98 | 46.10 / 18.64 | 1.42 / 0.57 |
| AWS c7g.metal on-demand (Firecracker host) | 15.68 | 31.36 | 0.96 |
| GCP c4a-highcpu-16 on-demand / spot | 16.39 / 7.73 | 34.03 / 16.06 | 1.04 / 0.49 |
| Azure D16pls_v6 pay-as-you-go / spot | 13.41 / 5.33 | 27.85 / 11.06 | 0.86 / 0.34 |
| OCI A1 on-demand / E5 on-demand / E5 preemptible | 5.58 / 8.16 / 4.08 | 11.16 / 16.32 / 8.16 | 0.34 / 0.50 / 0.25 |
| Hetzner CAX41 (shared Arm, EU) | 1.70 | 3.53 | 0.11 |
| Hetzner CCX63 (dedicated vCPU, EU) | 11.81 | 23.91 | 0.73 |
| Hetzner AX162-2 / AX102-1 (dedicated servers, EU) | 5.79 / 5.34 | 11.65 / 10.68 | 0.36 / 0.33 |
| OVHcloud SCALE-a8 768 GB FR / US catalogue | 4.78 / 5.47 | 9.57 / 10.94 | 0.29 / 0.34 |
| OVHcloud Public Cloud c3-64 | 11.06 | 22.12 | 0.68 |
| Scaleway EM-I620E monthly / POP2-HC-32C | 3.51 / 12.89 | 7.06 / 25.79 | 0.22 / 0.79 |
| DigitalOcean CPU-Optimized 32 | 12.44 | 24.89 | 0.76 |
| Vultr voc-c-32c / bare metal EPYC 9254 | 11.85 / 10.19 | 23.70 / 20.62 | 0.73 / 0.63 |
| Akamai G8 64x32 | 14.60 | 29.20 | 0.90 |
| Latitude.sh c4.metal.large | 8.96 | 17.91 | 0.55 |
| Fly.io performance-16x / reserved | 19.56 / 11.73 | 40.62 / 24.37 | 1.25 / 0.75 |
| Cloudflare Containers (standard-1 for L; custom 1 vCPU/3 GiB for R; CPU 50% active) | 40.89 | 47.46 | 1.46 |
| Cloudflare Browser Run (24/7, $0.09/h + $2/concurrent) | 67.70 | 67.70 | 2.08 |

#### Fleet compute cost per month (USD, 24/7, nodes rounded up)

| Option | 100 L | 100 R | 1,000 L | 1,000 R | 10,000 L | 10,000 R |
|---|---|---|---|---|---|---|
| AWS c7g on-demand | 1,694 | 3,387 | 16,089 | 32,602 | 157,081 | 326,018 |
| AWS c7g spot | 707 | 1,413 | 6,713 | 13,603 | 65,541 | 136,028 |
| AWS c7g 3-yr RI | 745 | 1,489 | 7,074 | 14,334 | 69,062 | 143,336 |
| AWS c7g.metal on-demand | 1,694 | 3,387 | 16,936 | 32,178 | 157,505 | 315,010 |
| GCP c4a on-demand / spot | 1,770 / 835 | 3,540 / 1,670 | 16,813 / 7,934 | 34,069 / 16,076 | 164,150 / 77,457 | 340,689 / 160,761 |
| Azure D16pls_v6 pay-as-you-go / spot | 1,448 / 575 | 2,897 / 1,150 | 13,759 / 5,465 | 27,880 / 11,073 | 134,332 / 53,354 | 278,802 / 110,734 |
| OCI A1 on-demand | 558 | 1,116 | 5,582 | 11,165 | 55,824 | 111,647 |
| OCI E5 preemptible | 408 | 816 | 4,079 | 8,159 | 40,794 | 81,588 |
| Hetzner CAX41 | 184 | 367 | 1,745 | 3,537 | 17,041 | 35,369 |
| Hetzner AX162-2 | 944 | 1,888 | 6,607 | 12,270 | 58,521 | 117,041 |
| Hetzner AX102-1 | 577 | 1,153 | 5,478 | 10,957 | 53,629 | 106,971 |
| OVHcloud SCALE-a8 768 GB FR | 3,120 | 3,120 | 6,239 | 12,479 | 49,916 | 96,712 |
| OVHcloud SCALE-a8 768 GB US catalogue | 3,566 | 3,566 | 7,132 | 14,264 | 57,056 | 110,546 |
| Scaleway EM-I620E | 762 | 762 | 3,810 | 7,620 | 35,814 | 70,866 |
| Vultr bare metal EPYC 9254 | 1,650 | 2,475 | 10,725 | 20,625 | 102,300 | 206,250 |
| Latitude.sh c4.metal.large | 2,920 | 2,920 | 11,680 | 20,440 | 90,520 | 181,040 |
| Fly.io performance-16x | 2,112 | 4,224 | 20,065 | 40,658 | 195,895 | 406,575 |
| Cloudflare Containers | 4,089 | 4,746 | 40,892 | 47,462 | 408,917 | 474,617 |

Node counts for R: 8, 77 and 770 AWS c7g.4xlarge; 2, 13 and 124 Hetzner AX162-2; 1, 4 and 31 OVHcloud SCALE-a8; 1, 10 and 93 Scaleway EM-I620E. At 100 browsers the big bare-metal boxes sit mostly idle (one SCALE-a8 holds 326 R or 652 L), so small fleets favour cloud VMs or small servers. Hetzner AX102-1 at $1,153 for 100 R is about one third of AWS on-demand ($3,387) and about 18% below AWS spot ($1,413).

#### Network add-ons (my calculation; prices from section 1)

| Item | 100 browsers | 1,000 | 10,000 |
|---|---|---|---|
| Egress, base (3.65 / 36.5 / 365 TB per month): AWS reference tiers | $320 | $3,144 | $22,041 |
| Same, Azure premium | $309 | $3,061 | $21,935 |
| Same, OCI | $0 | $225 | $3,018 |
| Same, Akamai / Fly.io / Cloudflare Containers | $18 / $73 / $66 | $182 / $730 / $888 | $1,825 / $7,300 / $9,100 |
| Same, Hetzner / OVHcloud / Scaleway / DigitalOcean / Vultr / Latitude.sh | ~$0, within included or unmetered allowances | ~$0 | ~$0 |
| Egress, heavy hosted browsers (21.9 / 219 / 2,190 TB): AWS | $1,902 | $14,741 | $113,291 |
| Same, OCI / Akamai | $101 / $110 | $1,777 / $1,095 | $18,530 / $10,950 |
| **AWS NAT gateway if fleets sit in private subnets** (ingress + egress processed at $0.045/GB) | $1,807 | $18,068 | $180,675 |
| AWS public IPv4 per node instead of NAT (R node counts) | $29 | $281 | $2,810 |

Latitude.sh includes 20 TB/server of egress, so the heavy 10,000-browser case (about 35 TB/server) would exceed it at an unknown overage price.

#### 10,000 R browsers, compute + network (no NAT)
- **Base egress:**
  - AWS on-demand $350,869; AWS spot $160,879; AWS 3-yr RI $168,187.
  - Azure spot $132,669; OCI A1 $114,665; OCI E5 preemptible $84,606.
  - Hetzner AX162-2 $117,277 (plus about $58,222 of one-time setup fees); Hetzner CAX41 $35,800.
  - OVHcloud SCALE-a8 FR $96,712; OVHcloud US catalogue $110,546; Scaleway EM-I620E $71,246.
  - Latitude.sh $181,040; Vultr bare metal $206,250.
  - Against AWS spot: OVH FR −40%, Scaleway −56%, Hetzner AX −27%, OCI A1 −29%. Against AWS on-demand: −72%, −80%, −67%, −67%.
- **Heavy egress:**
  - AWS spot $252,129 and AWS on-demand $442,119.
  - OVHcloud FR −62% against AWS spot, Scaleway −72%, Hetzner AX −53%, OCI A1 −48%.

#### Elastic per-browser-hour prices (R, 85% packing)
- Hetzner CAX41 $0.0057; OCI E5 preemptible $0.0112; Azure spot $0.0152; OCI A1 $0.0153; AWS c7g spot $0.0186.
- Scaleway EM-I620E hourly $0.0193; GCP spot $0.0220; OCI E5 $0.0224; Latitude.sh $0.0245; Vultr bare metal $0.0282.
- OVHcloud c3-64 $0.0303; Akamai G8 $0.0400; AWS c7g.metal on-demand $0.0430; AWS c7g on-demand $0.0446; GCP on-demand $0.0466; Fly.io $0.0564; AWS c7a on-demand $0.0632.

#### Break-even average utilisation
This is the utilisation above which monthly bare metal beats elastic cloud that is paid only for hours used (R; dedicated per-browser cost ÷ cloud per-browser full-month cost).

| Dedicated option | vs AWS on-demand | vs AWS spot | vs AWS 3-yr RI | vs Azure spot |
|---|---|---|---|---|
| OVHcloud SCALE-a8 FR | 29% | 70% | 67% | 87% |
| Hetzner AX162-2 | 36% | 86% | 81% | 105% |
| Scaleway EM-I620E monthly | 22% | 52% | 49% | 64% |

#### SMT sensitivity (my assumption)
Assume one x86 hardware thread equals 0.65 of a Graviton or c7a core for CPU-bound browsers. The R cost per browser then becomes:

| Option | R $/browser-mo, SMT-adjusted | Browsers per node |
|---|---|---|
| OVHcloud SCALE-a8 FR | 14.72 | 212 |
| Hetzner AX162-2 | 17.81 | 53 |
| Scaleway EM-I620E | 10.89 | 70 |
| Hetzner AX102-1 | 16.94 | 17 |
| Latitude.sh c4.metal.large | 27.55 | 106 |
| Vultr bare metal EPYC 9254 | 31.73 | 26 |
| OCI E5 | 23.25 | n/a (flex) |

The AWS comparison points are $32.57 on-demand and $13.59 on Graviton spot, which need no adjustment.

#### Hybrid illustration
Peak is 10,000 R browsers. A base of 60% of peak runs 24/7 on bare metal, and the remaining 40% bursts on AWS spot for 25% of hours.

| Layout | Monthly cost |
|---|---|
| OVHcloud SCALE-a8 base ($57,420) + AWS spot burst ($13,590) | $71,010 |
| Hetzner AX162-2 base + AWS spot burst | $83,490 |
| All-AWS spot, fully elastic | $95,130 |
| All-AWS on-demand, fully elastic | $227,990 |

Network costs are excluded here; they favour the hybrid further.

### Inferences
- **Hosted browsers (Browserbase-like) and actor runs (Apify-like):** AWS is not the cheapest. At 10,000 browsers with base traffic, the steady fleet costs about 3–5× more on AWS on-demand than on EU bare metal and 1.4–2.4× more on AWS spot or 3-year RIs. Spot also adds interruption handling; ADR 0002 checkpoints make that feasible, but live hosted sessions cannot be checkpointed transparently. Heavy live-view egress adds 36% (on-demand) to 85% (spot) to the AWS compute bill. The best options by region:
  - **EU:** Scaleway Elastic Metal or OVHcloud Scale for Firecracker hosts. Hetzner CAX for cheap container-isolated or per-VM jobs; one small cloud VM per run is Hetzner's own KVM boundary.
  - **US:** OVHcloud US, OCI, or AWS spot.
- **High-volume crawling (Firecrawl- or Spider-like):** compute is light. Bandwidth, IPs and abuse handling dominate. The NAT-gateway charge alone ($18/browser-month equivalent) can exceed the compute cost on AWS. Unmetered or included-traffic providers (OVHcloud, Hetzner, Scaleway, OCI with 10 TB free plus $0.0085/GB) undercut AWS by a wide margin. If crawlers stay on AWS, they need public IPv4 ($3.65/node-month) or IPv6, never NAT.
- **MicroVMs for untrusted Recipe code (Firecracker):**
  - On AWS this is now practical with nested virtualization on C8i/M8i (Intel) or .metal at no per-vCPU premium. It is therefore viable on AWS, but at AWS prices.
  - On GCP it needs Intel or N4D with at least 10% overhead. Hetzner Cloud cannot do it.
  - Hourly bare metal (Scaleway, Vultr, Latitude.sh) gives the elastic version, and monthly bare metal (OVHcloud, Hetzner) the cheap steady version.
  - Fly.io Machines and Cloudflare Containers provide microVM or container isolation but cost 1.25–1.5× AWS on-demand per browser unit. Cloudflare's 1,500-vCPU default limit and its minimum of 3 GiB per vCPU make it a poor fit for 1–2 GB browsers at scale.
- **AI agents:** the compute shape matches the browsers. The deciding factor is model-call spend, and hyperscaler credits (AWS Activate covering Bedrock; Microsoft; Google, unverified) offset that spend directly. This favours keeping agent orchestration and model gateways on AWS while the browsers they drive run on cheaper fleets.
- **Confirming or adjusting AWS.** Confirm AWS for the control plane, marketplace, billing, KMS and early-stage everything (credits, speed of building). Adjust the plan so that the browser, crawl and microVM fleets are provider-portable from day one. The engine is a static binary on any KVM host, so this is cheap. Move steady load to EU and US bare metal or OCI once utilisation is predictable, and keep AWS spot or on-demand as the burst tier.
- **Arm fleets carry a stealth question.** Arm is the cheapest everywhere (Graviton spot, OCI A1, Hetzner CAX), but a Linux-aarch64 browser may be rarer and easier to fingerprint than x86 desktop personas. Goodfetch's stealth catalogue (ADR 0035) should be checked before committing fleets to Arm. This is an inference with no source.

### Gaps
- Unmeasured: real per-browser CPU and RAM for the Goodfetch stealth Chromium, real traffic per browser-hour, and SMT thread performance (0.65 is an assumption). These three inputs move the bare-metal vs spot conclusion more than any price does.
- Not modelled: AWS Savings Plans other than the RIs shown, EU-region AWS prices (Paris/Frankfurt), AWS spot interruption rates for c7g, and spot price volatility.
- Capacity at 10,000 browsers is unverified: Hetzner stock and account limits for about 124 AX servers or 770 CAX41, OVHcloud delivery times for 31 SCALE-a8, Latitude.sh and Scaleway stock levels.
- Staff and operations costs of running bare metal (hardware failures, provisioning automation, no managed Kubernetes) are not quantified. 37signals' "no new staff" claim is one data point, not a general result.
