# AWS price sheet for Goodfetch Cloud: list prices in us-east-1, eu-west-1 and eu-west-3 (checked 2026-10-12)

All figures are USD public list prices for Linux and shared tenancy, taken on 2026-10-12 from the AWS Price List bulk offer files on pricing.us-east-1.amazonaws.com (each link below names the exact offer version; publication dates run from 2026-09-11 to 2026-10-10), the Spot price feed behind the EC2 Spot pricing page, the Spot Instance Advisor data file, and official AWS and Anthropic pricing pages. Every derived value ($ per vCPU-hour, $ per GiB-hour, monthly cost at 730 hours, discount percentages) was computed with Python from those files, not by hand. Spot prices are a dated snapshot (feed fetched 2026-10-12T01:37:10Z). eu-west-3 (Paris) is shown wherever it differs from the other two regions, which is the case for almost every compute and AI line. The shared web-search budget for this session ran out partway through, so a few context facts (launch dates, product definitions) could only be checked against pages fetched directly; those limits are listed under each Gaps heading. No architecture recommendations are made.

## 1. EC2 instance prices (Linux): on-demand, Spot, Savings Plans, specifications and Spot Advisor bands

### Takeaway
In us-east-1, on-demand Linux costs $0.03625 per vCPU-hour on c7g, $0.03988 on c8g, $0.04347 on the newly listed Graviton5 c9g, $0.04462 on c7i and $0.04686 on c8i, and the per-vCPU rate is flat from 2xlarge through bare metal within each family. No Upfront Savings Plans take 26.5-28.3% off (Compute SP, 1 yr), 47.8-58.2% (Compute SP, 3 yr), 33.8-37.4% (EC2 Instance SP, 1 yr) and 54.6-63.7% (EC2 Instance SP, 3 yr). The Spot snapshot sits 43-90% below on-demand. Ireland is 6.9-12.9% and Paris 11.9-18.8% dearer than us-east-1 for the same type, and Paris has no AMD (c7a, m7a, c8a, m8a, r8a) or Graviton5 types.

### Cited Findings
- **Sources and filters.** On-demand prices and specifications come from EC2 offer version 20261009203134 (published 2026-10-09T20:31:34Z; every row used carries term effective date 2026-10-01), filtered to operatingSystem=Linux, tenancy=Shared, preInstalledSw=NA, licenseModel=No License required, capacitystatus=Used, operation=RunInstances: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json), [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json), [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json).
- **Savings Plans.** Rates are the per-SKU `discountedRate` for operation RunInstances under No Upfront plans, from Compute Savings Plans offer version 20261010020024 (published 2026-10-10T02:00:24Z). That offer file carries both Compute Savings Plans and EC2 Instance Savings Plans: [SP offer us-east-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/us-east-1/index.json), [SP offer eu-west-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-1/index.json), [SP offer eu-west-3](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-3/index.json).
- **Spot.** One Linux price per type per region from the feed behind the EC2 Spot pricing page, fetched 2026-10-12T01:37:10Z. The feed does not say which Availability Zone or time each value refers to: [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json).
- **Spot Advisor.** The Advisor defines "frequency of interruption" as "the rate at which Spot has reclaimed capacity during the trailing month," in bands of <5%, 5-10%, 10-15%, 15-20% and >20%. "Savings compared to On-Demand are calculated over the last 30 days," with "price history data … averaged across Availability Zones": [Spot Instance Advisor](https://aws.amazon.com/ec2/spot/instance-advisor/). Values come from its data file, read on 2026-10-12: [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).
- **New families.** The us-east-1 price list now includes Graviton5 c9g, m9g and r9g, plus c9gd/m9gd/r9gd. It also has AMD c8a/m8a/r8a, Intel c8i/m8i/r8i (with -flex variants), c8gn, and t8i, x8i and m8azn: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json). In the three target regions, c9g and r9g are listed only in us-east-1, m9g in us-east-1 and eu-west-1, and c8gn metal sizes only in us-east-1. The AMD families are absent from eu-west-3: [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json), [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json).
- **Generation steps (us-east-1 on-demand, same size, computed).** Each step costs more per vCPU: c8g is +10.0% over c7g, and c9g/m9g/r9g are +9.0% over c8g/m8g/r8g. c8i/m8i and c8a are +5.0% over c7i/m7i and c7a: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json).
- **Regional premium over us-east-1 (on-demand, mean across the tabulated sizes, computed).** eu-west-1: c7g/c8g +6.9%; c7i, c8i, c7a, c8a +7.3%; m-families +11.4-11.5%; r-families +11.9%; t4g +9.5%; c8gn +12.9%. eu-west-3: c7g/c8g +18.4%; c8gn +18.5%; c7i/c8i +18.8%; m-families +16.7%; r-families +17.4-17.5%; t4g +11.9%. Sources: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json), [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json), [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json).
- **Savings Plans discount ranges (No Upfront vs on-demand, across the tabulated types, computed).**
  - us-east-1: Compute SP 1 yr 26.5-28.3%, 3 yr 47.8-58.2%; EC2 Instance SP 1 yr 33.8-37.4%, 3 yr 54.6-63.7%. The c8gn family has the deepest 3 yr discounts.
  - eu-west-1: Compute SP 1 yr 17.9-27.6% (t4g lowest), 3 yr 42.1-52.3%; EC2 Instance SP 1 yr 33.8-37.0%, 3 yr 54.4-58.1%.
  - eu-west-3: Compute SP 1 yr 18.6-27.6%, 3 yr 41.4-51.9%; EC2 Instance SP 1 yr 33.7-37.0%, 3 yr 54.4-58.2%.
  - Sources: [SP offer us-east-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/us-east-1/index.json), [SP offer eu-west-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-1/index.json), [SP offer eu-west-3](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-3/index.json).
- **Spot snapshot discount vs on-demand (computed).** us-east-1 runs from 43.4% (m7g.4xlarge) to 90.0% (c8gn.metal-48xl), eu-west-1 from 35.7% (m7i.2xlarge) to 90.0%, and eu-west-3 from 53.7% to 90.0%. The Advisor's 30-day savings figures run 43-80% in us-east-1, 37-80% in eu-west-1 and 47-86% in eu-west-3: [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json), [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).
- **Suspect Spot values.** 16 feed values equal exactly 10% of on-demand, all on large metal or 48xl/96xl sizes. Examples: us-east-1 c8g.metal-48xl $0.7657 vs $7.65696 on-demand, us-east-1 c8gn.metal-48xl $1.1376 vs $11.376, and eu-west-1 c8g.metal-48xl $0.8187 vs $8.18688: [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json). For the same us-east-1 types the Advisor gives 73% (c8g.metal-48xl) and 70% (c8gn.metal-48xl) average savings: [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).
- **Missing Advisor data.** The Advisor has no entry for c9g, m9g or r9g in any of the three regions, nor for c8g/c8gn in eu-west-3: [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).
- **Managed-compute fees (per instance-hour, billed on top of the EC2 price, computed against on-demand).**
  - ECS Managed Instances is 12.0% of on-demand on every tabulated type that has a fee row. One outlier: us-east-1 m8a.metal-24xl at $1.40221/h, which is 24.0%. Missing fee rows: c9g/m9g/r9g in us-east-1; c8gn, m9g and the 8th-generation x86 families (c8i, m8i, r8i, c8a, m8a, r8a) in eu-west-1; c8g, c8gn, r8g 2xlarge-8xlarge and c8i/m8i/r8i in eu-west-3: [ECS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/us-east-1/index.json), [ECS offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/eu-west-1/index.json), [ECS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/eu-west-3/index.json).
  - EKS Auto Mode is 12.0% on all 241 tabulated type-region rows: [EKS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/us-east-1/index.json).
  - Lambda Managed Instances is 15.0% on all 96 rows that exist. There are no rows for t4g, metal sizes, r9g, or c8i/m8i/r8i/c8a/m8a/r8a: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json).
  - us-east-1 examples: c7g.2xlarge $0.0348/h (ECS MI, EKS Auto) and $0.0435/h (Lambda MI); m7g.2xlarge $0.039168 and $0.04896; c8g.2xlarge $0.03828 and $0.04785; c7i.2xlarge $0.04284 (ECS MI): [ECS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/us-east-1/index.json), [EKS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/us-east-1/index.json), [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json).

The tables below were generated from the files above. "OD" means on-demand. "$/mo" assumes 730 hours. "Network" is AWS's `networkPerformance` label. "Spot below OD" compares the feed snapshot with on-demand. "Advisor" columns are the 30-day figures from the Spot Advisor data file. The Savings Plans cells show the effective hourly rate with its discount in brackets.

#### us-east-1: specifications, on-demand and Spot (Linux)

Sources: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json), [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json) (fetched 2026-10-12T01:37:10Z), [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).

| Type | vCPU | GiB | Network | OD $/h | OD $/vCPU-h | OD $/GiB-h | OD $/mo (730 h) | Spot $/h (snapshot) | Spot below OD | Advisor savings | Advisor interruption |
|---|---|---|---|---|---|---|---|---|---|---|---|
| t4g.medium | 2 | 4 | Up to 5 Gigabit | 0.0336 | 0.01680 | 0.00840 | 24.53 | 0.0131 | 61.0% | 49% | 15-20% |
| t4g.large | 2 | 8 | Up to 5 Gigabit | 0.0672 | 0.03360 | 0.00840 | 49.06 | 0.0239 | 64.4% | 58% | 15-20% |
| t4g.xlarge | 4 | 16 | Up to 5 Gigabit | 0.1344 | 0.03360 | 0.00840 | 98.11 | 0.0548 | 59.2% | 53% | 10-15% |
| t4g.2xlarge | 8 | 32 | Up to 5 Gigabit | 0.2688 | 0.03360 | 0.00840 | 196.22 | 0.0741 | 72.4% | 60% | >20% |
| c7g.2xlarge | 8 | 16 | Up to 15 Gigabit | 0.2900 | 0.03625 | 0.01812 | 211.70 | 0.1427 | 50.8% | 58% | 5-10% |
| c7g.4xlarge | 16 | 32 | Up to 15 Gigabit | 0.5800 | 0.03625 | 0.01812 | 423.40 | 0.2143 | 63.1% | 54% | <5% |
| c7g.8xlarge | 32 | 64 | 15 Gigabit | 1.1600 | 0.03625 | 0.01812 | 846.80 | 0.4047 | 65.1% | 65% | <5% |
| c7g.metal | 64 | 128 | 30 Gigabit | 2.3200 | 0.03625 | 0.01812 | 1693.60 | 0.8281 | 64.3% | 62% | 5-10% |
| c8g.2xlarge | 8 | 16 | Up to 15 Gigabit | 0.3190 | 0.03988 | 0.01994 | 232.90 | 0.1456 | 54.4% | 56% | 10-15% |
| c8g.4xlarge | 16 | 32 | Up to 15 Gigabit | 0.6381 | 0.03988 | 0.01994 | 465.80 | 0.2503 | 60.8% | 57% | >20% |
| c8g.8xlarge | 32 | 64 | 15 Gigabit | 1.2762 | 0.03988 | 0.01994 | 931.60 | 0.4513 | 64.6% | 64% | 5-10% |
| c8g.metal-24xl | 96 | 192 | 40 Gigabit | 3.8285 | 0.03988 | 0.01994 | 2794.79 | 0.4156 | 89.1% | 68% | 15-20% |
| c8g.metal-48xl | 192 | 384 | 50 Gigabit | 7.6570 | 0.03988 | 0.01994 | 5589.58 | 0.7657 | 90.0% | 73% | <5% |
| c8gn.2xlarge | 8 | 16 | Up to 50 Gigabit | 0.4740 | 0.05925 | 0.02962 | 346.02 | 0.1165 | 75.4% | 64% | <5% |
| c8gn.4xlarge | 16 | 32 | 50 Gigabit | 0.9480 | 0.05925 | 0.02962 | 692.04 | 0.3872 | 59.2% | 67% | <5% |
| c8gn.8xlarge | 32 | 64 | 100 Gigabit | 1.8960 | 0.05925 | 0.02962 | 1384.08 | 0.4460 | 76.5% | 67% | <5% |
| c8gn.metal-24xl | 96 | 192 | 300 Gigabit | 5.6880 | 0.05925 | 0.02962 | 4152.24 | 0.9949 | 82.5% | 76% | <5% |
| c8gn.metal-48xl | 192 | 384 | 600 Gigabit | 11.3760 | 0.05925 | 0.02962 | 8304.48 | 1.1376 | 90.0% | 70% | <5% |
| c9g.2xlarge | 8 | 16 | Up to 17 Gigabit | 0.3478 | 0.04347 | 0.02174 | 253.86 | 0.1510 | 56.6% | n/a | n/a |
| c9g.4xlarge | 16 | 32 | Up to 17 Gigabit | 0.6955 | 0.04347 | 0.02174 | 507.73 | 0.2669 | 61.6% | n/a | n/a |
| c9g.8xlarge | 32 | 64 | 17 Gigabit | 1.3910 | 0.04347 | 0.02174 | 1015.46 | 0.4987 | 64.1% | n/a | n/a |
| c9g.metal-48xl | 192 | 384 | 100 Gigabit | 8.3462 | 0.04347 | 0.02174 | 6092.76 | 2.0210 | 75.8% | n/a | n/a |
| m7g.2xlarge | 8 | 32 | Up to 15 Gigabit | 0.3264 | 0.04080 | 0.01020 | 238.27 | 0.1667 | 48.9% | 53% | >20% |
| m7g.4xlarge | 16 | 64 | Up to 15 Gigabit | 0.6528 | 0.04080 | 0.01020 | 476.54 | 0.3695 | 43.4% | 59% | 15-20% |
| m7g.8xlarge | 32 | 128 | 15 Gigabit | 1.3056 | 0.04080 | 0.01020 | 953.09 | 0.5450 | 58.3% | 60% | 5-10% |
| m7g.metal | 64 | 256 | 30 Gigabit | 2.6112 | 0.04080 | 0.01020 | 1906.18 | 0.5272 | 79.8% | 74% | 5-10% |
| m8g.2xlarge | 8 | 32 | Up to 15 Gigabit | 0.3590 | 0.04488 | 0.01122 | 262.10 | 0.1095 | 69.5% | 62% | 15-20% |
| m8g.4xlarge | 16 | 64 | Up to 15 Gigabit | 0.7181 | 0.04488 | 0.01122 | 524.20 | 0.2641 | 63.2% | 56% | 10-15% |
| m8g.8xlarge | 32 | 128 | 15 Gigabit | 1.4362 | 0.04488 | 0.01122 | 1048.40 | 0.5297 | 63.1% | 65% | 5-10% |
| m8g.metal-24xl | 96 | 384 | 40 Gigabit | 4.3085 | 0.04488 | 0.01122 | 3145.19 | 1.1193 | 74.0% | 74% | 10-15% |
| m8g.metal-48xl | 192 | 768 | 50 Gigabit | 8.6170 | 0.04488 | 0.01122 | 6290.38 | 2.3033 | 73.3% | 73% | 5-10% |
| m9g.2xlarge | 8 | 32 | Up to 17000 Megabit | 0.3914 | 0.04892 | 0.01223 | 285.69 | 0.1737 | 55.6% | n/a | n/a |
| m9g.4xlarge | 16 | 64 | Up to 17000 Megabit | 0.7827 | 0.04892 | 0.01223 | 571.39 | 0.3123 | 60.1% | n/a | n/a |
| m9g.8xlarge | 32 | 128 | 17000 Megabit | 1.5654 | 0.04892 | 0.01223 | 1142.77 | 0.5252 | 66.5% | n/a | n/a |
| m9g.metal-48xl | 192 | 768 | 100000 Megabit | 9.3926 | 0.04892 | 0.01223 | 6856.63 | 1.7809 | 81.0% | n/a | n/a |
| r7g.2xlarge | 8 | 64 | Up to 15 Gigabit | 0.4284 | 0.05355 | 0.00669 | 312.73 | 0.1415 | 67.0% | 58% | >20% |
| r7g.4xlarge | 16 | 128 | Up to 15 Gigabit | 0.8568 | 0.05355 | 0.00669 | 625.46 | 0.2918 | 65.9% | 61% | >20% |
| r7g.8xlarge | 32 | 256 | 15 Gigabit | 1.7136 | 0.05355 | 0.00669 | 1250.93 | 0.4127 | 75.9% | 70% | >20% |
| r7g.metal | 64 | 512 | 30 Gigabit | 3.4272 | 0.05355 | 0.00669 | 2501.86 | 0.7794 | 77.3% | 73% | >20% |
| r8g.2xlarge | 8 | 64 | Up to 15 Gigabit | 0.4713 | 0.05891 | 0.00736 | 344.03 | 0.1599 | 66.1% | 64% | 15-20% |
| r8g.4xlarge | 16 | 128 | Up to 15 Gigabit | 0.9426 | 0.05891 | 0.00736 | 688.07 | 0.2494 | 73.5% | 63% | 5-10% |
| r8g.8xlarge | 32 | 256 | 15 Gigabit | 1.8851 | 0.05891 | 0.00736 | 1376.14 | 0.4682 | 75.2% | 74% | 5-10% |
| r8g.metal-24xl | 96 | 768 | 40 Gigabit | 5.6554 | 0.05891 | 0.00736 | 4128.41 | 1.3903 | 75.4% | 72% | >20% |
| r8g.metal-48xl | 192 | 1536 | 50 Gigabit | 11.3107 | 0.05891 | 0.00736 | 8256.83 | 1.7036 | 84.9% | 75% | 5-10% |
| r9g.2xlarge | 8 | 64 | Up to 17 Gigabit | 0.5137 | 0.06421 | 0.00803 | 374.99 | 0.2236 | 56.5% | n/a | n/a |
| r9g.4xlarge | 16 | 128 | Up to 17 Gigabit | 1.0274 | 0.06421 | 0.00803 | 749.97 | 0.4462 | 56.6% | n/a | n/a |
| r9g.8xlarge | 32 | 256 | 17 Gigabit | 2.0547 | 0.06421 | 0.00803 | 1499.95 | 0.7354 | 64.2% | n/a | n/a |
| r9g.metal-48xl | 192 | 1536 | 100 Gigabit | 12.3283 | 0.06421 | 0.00803 | 8999.67 | 2.5515 | 79.3% | n/a | n/a |
| c7i.2xlarge | 8 | 16 | Up to 12500 Megabit | 0.3570 | 0.04462 | 0.02231 | 260.61 | 0.1825 | 48.9% | 56% | >20% |
| c7i.4xlarge | 16 | 32 | Up to 12500 Megabit | 0.7140 | 0.04462 | 0.02231 | 521.22 | 0.2586 | 63.8% | 57% | >20% |
| c7i.8xlarge | 32 | 64 | 12500 Megabit | 1.4280 | 0.04462 | 0.02231 | 1042.44 | 0.6710 | 53.0% | 66% | >20% |
| c7i.metal-24xl | 96 | 192 | 37500 Megabit | 4.2840 | 0.04462 | 0.02231 | 3127.32 | 1.0639 | 75.2% | 72% | 15-20% |
| c7i.metal-48xl | 192 | 384 | 50000 Megabit | 8.5680 | 0.04462 | 0.02231 | 6254.64 | 2.2568 | 73.7% | 75% | 10-15% |
| c8i.2xlarge | 8 | 16 | Up to 15000 Megabit | 0.3748 | 0.04686 | 0.02343 | 273.63 | 0.1609 | 57.1% | 58% | 15-20% |
| c8i.4xlarge | 16 | 32 | Up to 15000 Megabit | 0.7497 | 0.04686 | 0.02343 | 547.27 | 0.3049 | 59.3% | 43% | 5-10% |
| c8i.8xlarge | 32 | 64 | 15000 Megabit | 1.4994 | 0.04686 | 0.02343 | 1094.53 | 0.6270 | 58.2% | 61% | >20% |
| c8i.metal-48xl | 192 | 384 | 75000 Megabit | 8.9962 | 0.04686 | 0.02343 | 6567.20 | 1.9831 | 78.0% | 66% | <5% |
| c8i.metal-96xl | 384 | 768 | 100000 Megabit | 17.9923 | 0.04686 | 0.02343 | 13134.39 | 2.7527 | 84.7% | 72% | <5% |
| c7a.2xlarge | 8 | 16 | Up to 12500 Megabit | 0.4106 | 0.05132 | 0.02566 | 299.71 | 0.1882 | 54.2% | 56% | 5-10% |
| c7a.4xlarge | 16 | 32 | Up to 12500 Megabit | 0.8211 | 0.05132 | 0.02566 | 599.42 | 0.3132 | 61.9% | 55% | 5-10% |
| c7a.8xlarge | 32 | 64 | 12500 Megabit | 1.6422 | 0.05132 | 0.02566 | 1198.84 | 0.5134 | 68.7% | 56% | 15-20% |
| c7a.metal-48xl | 192 | 384 | 50000 Megabit | 9.8534 | 0.05132 | 0.02566 | 7193.01 | 2.0881 | 78.8% | 72% | <5% |
| c8a.2xlarge | 8 | 16 | Up to 15000 Megabit | 0.4311 | 0.05389 | 0.02694 | 314.69 | 0.1783 | 58.6% | 56% | 15-20% |
| c8a.4xlarge | 16 | 32 | Up to 15000 Megabit | 0.8622 | 0.05389 | 0.02694 | 629.38 | 0.2673 | 69.0% | 60% | 10-15% |
| c8a.8xlarge | 32 | 64 | 15000 Megabit | 1.7243 | 0.05389 | 0.02694 | 1258.75 | 0.5820 | 66.2% | 57% | 15-20% |
| c8a.metal-24xl | 96 | 192 | 40000 Megabit | 5.1730 | 0.05388 | 0.02694 | 3776.26 | 1.4839 | 71.3% | 71% | 5-10% |
| c8a.metal-48xl | 192 | 384 | 75000 Megabit | 10.3459 | 0.05388 | 0.02694 | 7552.52 | 2.5570 | 75.3% | 75% | 5-10% |
| m7i.2xlarge | 8 | 32 | Up to 12500 Megabit | 0.4032 | 0.05040 | 0.01260 | 294.34 | 0.1641 | 59.3% | 59% | >20% |
| m7i.4xlarge | 16 | 64 | Up to 12500 Megabit | 0.8064 | 0.05040 | 0.01260 | 588.67 | 0.3144 | 61.0% | 55% | >20% |
| m7i.8xlarge | 32 | 128 | 12500 Megabit | 1.6128 | 0.05040 | 0.01260 | 1177.34 | 0.5504 | 65.9% | 50% | 5-10% |
| m7i.metal-24xl | 96 | 384 | 37500 Megabit | 4.8384 | 0.05040 | 0.01260 | 3532.03 | 1.0814 | 77.6% | 71% | >20% |
| m7i.metal-48xl | 192 | 768 | 50000 Megabit | 9.6768 | 0.05040 | 0.01260 | 7064.06 | 1.4634 | 84.9% | 75% | 10-15% |
| m8i.2xlarge | 8 | 32 | Up to 15000 Megabit | 0.4234 | 0.05292 | 0.01323 | 309.05 | 0.1776 | 58.0% | 59% | >20% |
| m8i.4xlarge | 16 | 64 | Up to 15000 Megabit | 0.8467 | 0.05292 | 0.01323 | 618.11 | 0.2909 | 65.6% | 61% | >20% |
| m8i.8xlarge | 32 | 128 | 15000 Megabit | 1.6934 | 0.05292 | 0.01323 | 1236.21 | 0.6394 | 62.2% | 63% | >20% |
| m8i.metal-48xl | 192 | 768 | 75000 Megabit | 10.1606 | 0.05292 | 0.01323 | 7417.27 | 2.6948 | 73.5% | 65% | >20% |
| m8i.metal-96xl | 384 | 1536 | 100000 Megabit | 20.3213 | 0.05292 | 0.01323 | 14834.53 | 4.4706 | 78.0% | 80% | >20% |
| m7a.2xlarge | 8 | 32 | Up to 12500 Megabit | 0.4637 | 0.05796 | 0.01449 | 338.49 | 0.2039 | 56.0% | 57% | >20% |
| m7a.4xlarge | 16 | 64 | Up to 12500 Megabit | 0.9274 | 0.05796 | 0.01449 | 676.97 | 0.3591 | 61.3% | 55% | >20% |
| m7a.8xlarge | 32 | 128 | 12500 Megabit | 1.8547 | 0.05796 | 0.01449 | 1353.95 | 0.6016 | 67.6% | 60% | 10-15% |
| m7a.metal-48xl | 192 | 768 | 50000 Megabit | 11.1283 | 0.05796 | 0.01449 | 8123.67 | 2.5443 | 77.1% | 79% | 10-15% |
| m8a.2xlarge | 8 | 32 | Up to 15000 Megabit | 0.4869 | 0.06086 | 0.01521 | 355.42 | 0.1657 | 66.0% | 62% | >20% |
| m8a.4xlarge | 16 | 64 | Up to 15000 Megabit | 0.9738 | 0.06086 | 0.01521 | 710.84 | 0.3337 | 65.7% | 60% | >20% |
| m8a.8xlarge | 32 | 128 | 15000 Megabit | 1.9475 | 0.06086 | 0.01521 | 1421.69 | 0.5875 | 69.8% | 65% | >20% |
| m8a.metal-24xl | 96 | 384 | 40000 Megabit | 5.8426 | 0.06086 | 0.01521 | 4265.07 | 1.5236 | 73.9% | 75% | 5-10% |
| m8a.metal-48xl | 192 | 768 | 75000 Megabit | 11.6851 | 0.06086 | 0.01521 | 8530.14 | 1.4105 | 87.9% | 77% | >20% |
| r8i.2xlarge | 8 | 64 | Up to 15000 Megabit | 0.5557 | 0.06946 | 0.00868 | 405.65 | 0.1443 | 74.0% | 58% | >20% |
| r8i.4xlarge | 16 | 128 | Up to 15000 Megabit | 1.1114 | 0.06946 | 0.00868 | 811.29 | 0.3042 | 72.6% | 56% | >20% |
| r8i.8xlarge | 32 | 256 | 15000 Megabit | 2.2227 | 0.06946 | 0.00868 | 1622.59 | 0.6389 | 71.3% | 56% | >20% |
| r8i.metal-48xl | 192 | 1536 | 75000 Megabit | 13.3363 | 0.06946 | 0.00868 | 9735.51 | 3.2106 | 75.9% | 60% | 5-10% |
| r8i.metal-96xl | 384 | 3072 | 100000 Megabit | 26.6726 | 0.06946 | 0.00868 | 19471.03 | 4.6301 | 82.6% | 72% | >20% |
| r8a.2xlarge | 8 | 64 | Up to 15000 Megabit | 0.6390 | 0.07988 | 0.00999 | 466.50 | 0.2618 | 59.0% | 50% | 10-15% |
| r8a.4xlarge | 16 | 128 | Up to 15000 Megabit | 1.2781 | 0.07988 | 0.00999 | 933.00 | 0.4457 | 65.1% | 49% | 5-10% |
| r8a.8xlarge | 32 | 256 | 15000 Megabit | 2.5562 | 0.07988 | 0.00999 | 1866.00 | 0.7864 | 69.2% | 56% | 5-10% |
| r8a.metal-24xl | 96 | 768 | 40000 Megabit | 7.6685 | 0.07988 | 0.00998 | 5597.99 | 1.1050 | 85.6% | 70% | <5% |
| r8a.metal-48xl | 192 | 1536 | 75000 Megabit | 15.3370 | 0.07988 | 0.00998 | 11195.98 | 2.3671 | 84.6% | 72% | <5% |

#### us-east-1: Savings Plans effective hourly rates, No Upfront (discount vs on-demand)

Sources: [SP offer us-east-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/us-east-1/index.json), [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json).

| Type | OD $/h | Compute SP 1 yr | Compute SP 3 yr | EC2 Instance SP 1 yr | EC2 Instance SP 3 yr | CSP 3 yr $/vCPU-h | CSP 3 yr $/mo (730 h) |
|---|---|---|---|---|---|---|---|
| t4g.medium | 0.0336 | 0.0242 (−28.0%) | 0.0167 (−50.3%) | 0.0211 (−37.2%) | 0.0145 (−56.8%) | 0.00835 | 12.19 |
| t4g.large | 0.0672 | 0.0485 (−27.8%) | 0.0334 (−50.3%) | 0.0421 (−37.4%) | 0.0290 (−56.8%) | 0.01670 | 24.38 |
| t4g.xlarge | 0.1344 | 0.0969 (−27.9%) | 0.0668 (−50.3%) | 0.0843 (−37.3%) | 0.0580 (−56.8%) | 0.01670 | 48.76 |
| t4g.2xlarge | 0.2688 | 0.1938 (−27.9%) | 0.1337 (−50.3%) | 0.1686 (−37.3%) | 0.1161 (−56.8%) | 0.01671 | 97.60 |
| c7g.2xlarge | 0.2900 | 0.2079 (−28.3%) | 0.1404 (−51.6%) | 0.1911 (−34.1%) | 0.1274 (−56.1%) | 0.01755 | 102.49 |
| c7g.4xlarge | 0.5800 | 0.4158 (−28.3%) | 0.2808 (−51.6%) | 0.3822 (−34.1%) | 0.2549 (−56.1%) | 0.01755 | 204.98 |
| c7g.8xlarge | 1.1600 | 0.8316 (−28.3%) | 0.5616 (−51.6%) | 0.7644 (−34.1%) | 0.5098 (−56.1%) | 0.01755 | 409.97 |
| c7g.metal | 2.3200 | 1.6632 (−28.3%) | 1.1232 (−51.6%) | 1.5288 (−34.1%) | 1.0195 (−56.1%) | 0.01755 | 819.94 |
| c8g.2xlarge | 0.3190 | 0.2287 (−28.3%) | 0.1545 (−51.6%) | 0.2102 (−34.1%) | 0.1402 (−56.1%) | 0.01931 | 112.76 |
| c8g.4xlarge | 0.6381 | 0.4574 (−28.3%) | 0.3089 (−51.6%) | 0.4205 (−34.1%) | 0.2804 (−56.1%) | 0.01931 | 225.51 |
| c8g.8xlarge | 1.2762 | 0.9149 (−28.3%) | 0.6178 (−51.6%) | 0.8409 (−34.1%) | 0.5608 (−56.1%) | 0.01931 | 451.02 |
| c8g.metal-24xl | 3.8285 | 2.7446 (−28.3%) | 1.8535 (−51.6%) | 2.5228 (−34.1%) | 1.6824 (−56.1%) | 0.01931 | 1353.06 |
| c8g.metal-48xl | 7.6570 | 5.4893 (−28.3%) | 3.7070 (−51.6%) | 5.0457 (−34.1%) | 3.3648 (−56.1%) | 0.01931 | 2706.13 |
| c8gn.2xlarge | 0.4740 | 0.3434 (−27.6%) | 0.1982 (−58.2%) | 0.2986 (−37.0%) | 0.1721 (−63.7%) | 0.02477 | 144.69 |
| c8gn.4xlarge | 0.9480 | 0.6868 (−27.6%) | 0.3964 (−58.2%) | 0.5972 (−37.0%) | 0.3442 (−63.7%) | 0.02477 | 289.37 |
| c8gn.8xlarge | 1.8960 | 1.3737 (−27.5%) | 0.7929 (−58.2%) | 1.1945 (−37.0%) | 0.6884 (−63.7%) | 0.02478 | 578.82 |
| c8gn.metal-24xl | 5.6880 | 4.1210 (−27.5%) | 2.3786 (−58.2%) | 3.5834 (−37.0%) | 2.0653 (−63.7%) | 0.02478 | 1736.38 |
| c8gn.metal-48xl | 11.3760 | 8.2419 (−27.6%) | 4.7572 (−58.2%) | 7.1669 (−37.0%) | 4.1306 (−63.7%) | 0.02478 | 3472.76 |
| c9g.2xlarge | 0.3478 | 0.2493 (−28.3%) | 0.1684 (−51.6%) | 0.2292 (−34.1%) | 0.1528 (−56.1%) | 0.02105 | 122.90 |
| c9g.4xlarge | 0.6955 | 0.4986 (−28.3%) | 0.3367 (−51.6%) | 0.4583 (−34.1%) | 0.3056 (−56.1%) | 0.02105 | 245.81 |
| c9g.8xlarge | 1.3910 | 0.9972 (−28.3%) | 0.6735 (−51.6%) | 0.9166 (−34.1%) | 0.6113 (−56.1%) | 0.02105 | 491.63 |
| c9g.metal-48xl | 8.3462 | 5.9834 (−28.3%) | 4.0407 (−51.6%) | 5.4999 (−34.1%) | 3.6677 (−56.1%) | 0.02105 | 2949.73 |
| m7g.2xlarge | 0.3264 | 0.2398 (−26.5%) | 0.1655 (−49.3%) | 0.2159 (−33.9%) | 0.1481 (−54.6%) | 0.02069 | 120.82 |
| m7g.4xlarge | 0.6528 | 0.4796 (−26.5%) | 0.3311 (−49.3%) | 0.4318 (−33.9%) | 0.2961 (−54.6%) | 0.02069 | 241.70 |
| m7g.8xlarge | 1.3056 | 0.9592 (−26.5%) | 0.6621 (−49.3%) | 0.8637 (−33.8%) | 0.5922 (−54.6%) | 0.02069 | 483.33 |
| m7g.metal | 2.6112 | 1.9184 (−26.5%) | 1.3243 (−49.3%) | 1.7273 (−33.9%) | 1.1844 (−54.6%) | 0.02069 | 966.74 |
| m8g.2xlarge | 0.3590 | 0.2638 (−26.5%) | 0.1821 (−49.3%) | 0.2375 (−33.9%) | 0.1629 (−54.6%) | 0.02276 | 132.93 |
| m8g.4xlarge | 0.7181 | 0.5275 (−26.5%) | 0.3642 (−49.3%) | 0.4750 (−33.8%) | 0.3257 (−54.6%) | 0.02276 | 265.84 |
| m8g.8xlarge | 1.4362 | 1.0551 (−26.5%) | 0.7284 (−49.3%) | 0.9500 (−33.8%) | 0.6514 (−54.6%) | 0.02276 | 531.70 |
| m8g.metal-24xl | 4.3085 | 3.1653 (−26.5%) | 2.1850 (−49.3%) | 2.8501 (−33.8%) | 1.9543 (−54.6%) | 0.02276 | 1595.09 |
| m8g.metal-48xl | 8.6170 | 6.3306 (−26.5%) | 4.3701 (−49.3%) | 5.7001 (−33.8%) | 3.9087 (−54.6%) | 0.02276 | 3190.17 |
| m9g.2xlarge | 0.3914 | 0.2875 (−26.5%) | 0.1985 (−49.3%) | 0.2589 (−33.9%) | 0.1775 (−54.6%) | 0.02481 | 144.89 |
| m9g.4xlarge | 0.7827 | 0.5750 (−26.5%) | 0.3970 (−49.3%) | 0.5178 (−33.8%) | 0.3550 (−54.6%) | 0.02481 | 289.78 |
| m9g.8xlarge | 1.5654 | 1.1501 (−26.5%) | 0.7939 (−49.3%) | 1.0355 (−33.8%) | 0.7101 (−54.6%) | 0.02481 | 579.55 |
| m9g.metal-48xl | 9.3926 | 6.9005 (−26.5%) | 4.7635 (−49.3%) | 6.2132 (−33.9%) | 4.2605 (−54.6%) | 0.02481 | 3477.34 |
| r7g.2xlarge | 0.4284 | 0.3104 (−27.5%) | 0.2235 (−47.8%) | 0.2834 (−33.8%) | 0.1943 (−54.6%) | 0.02794 | 163.16 |
| r7g.4xlarge | 0.8568 | 0.6208 (−27.5%) | 0.4470 (−47.8%) | 0.5668 (−33.8%) | 0.3886 (−54.6%) | 0.02794 | 326.31 |
| r7g.8xlarge | 1.7136 | 1.2415 (−27.6%) | 0.8939 (−47.8%) | 1.1335 (−33.9%) | 0.7773 (−54.6%) | 0.02793 | 652.55 |
| r7g.metal | 3.4272 | 2.4830 (−27.6%) | 1.7878 (−47.8%) | 2.2671 (−33.8%) | 1.5546 (−54.6%) | 0.02793 | 1305.09 |
| r8g.2xlarge | 0.4713 | 0.3414 (−27.6%) | 0.2459 (−47.8%) | 0.3118 (−33.9%) | 0.2138 (−54.6%) | 0.03073 | 179.47 |
| r8g.4xlarge | 0.9426 | 0.6829 (−27.6%) | 0.4917 (−47.8%) | 0.6235 (−33.9%) | 0.4275 (−54.6%) | 0.03073 | 358.93 |
| r8g.8xlarge | 1.8851 | 1.3658 (−27.5%) | 0.9834 (−47.8%) | 1.2470 (−33.8%) | 0.8551 (−54.6%) | 0.03073 | 717.87 |
| r8g.metal-24xl | 5.6554 | 4.0973 (−27.5%) | 2.9502 (−47.8%) | 3.7410 (−33.9%) | 2.5653 (−54.6%) | 0.03073 | 2153.62 |
| r8g.metal-48xl | 11.3107 | 8.1946 (−27.5%) | 5.9003 (−47.8%) | 7.4820 (−33.9%) | 5.1305 (−54.6%) | 0.03073 | 4307.23 |
| r9g.2xlarge | 0.5137 | 0.3722 (−27.6%) | 0.2680 (−47.8%) | 0.3398 (−33.8%) | 0.2330 (−54.6%) | 0.03349 | 195.61 |
| r9g.4xlarge | 1.0274 | 0.7443 (−27.6%) | 0.5359 (−47.8%) | 0.6796 (−33.8%) | 0.4660 (−54.6%) | 0.03350 | 391.23 |
| r9g.8xlarge | 2.0547 | 1.4886 (−27.6%) | 1.0719 (−47.8%) | 1.3592 (−33.8%) | 0.9320 (−54.6%) | 0.03350 | 782.46 |
| r9g.metal-48xl | 12.3283 | 8.9319 (−27.5%) | 6.4312 (−47.8%) | 8.1552 (−33.9%) | 5.5921 (−54.6%) | 0.03350 | 4694.75 |
| c7i.2xlarge | 0.3570 | 0.2580 (−27.7%) | 0.1724 (−51.7%) | 0.2362 (−33.8%) | 0.1572 (−56.0%) | 0.02155 | 125.83 |
| c7i.4xlarge | 0.7140 | 0.5160 (−27.7%) | 0.3447 (−51.7%) | 0.4723 (−33.9%) | 0.3144 (−56.0%) | 0.02155 | 251.66 |
| c7i.8xlarge | 1.4280 | 1.0319 (−27.7%) | 0.6895 (−51.7%) | 0.9446 (−33.9%) | 0.6287 (−56.0%) | 0.02155 | 503.31 |
| c7i.metal-24xl | 4.2840 | 3.0958 (−27.7%) | 2.0684 (−51.7%) | 2.8339 (−33.8%) | 1.8861 (−56.0%) | 0.02155 | 1509.95 |
| c7i.metal-48xl | 8.5680 | 6.1917 (−27.7%) | 4.1368 (−51.7%) | 5.6677 (−33.9%) | 3.7722 (−56.0%) | 0.02155 | 3019.89 |
| c8i.2xlarge | 0.3748 | 0.2709 (−27.7%) | 0.1810 (−51.7%) | 0.2480 (−33.8%) | 0.1650 (−56.0%) | 0.02262 | 132.12 |
| c8i.4xlarge | 0.7497 | 0.5418 (−27.7%) | 0.3620 (−51.7%) | 0.4959 (−33.9%) | 0.3301 (−56.0%) | 0.02262 | 264.23 |
| c8i.8xlarge | 1.4994 | 1.0835 (−27.7%) | 0.7239 (−51.7%) | 0.9918 (−33.8%) | 0.6601 (−56.0%) | 0.02262 | 528.47 |
| c8i.metal-48xl | 8.9962 | 6.5011 (−27.7%) | 4.3436 (−51.7%) | 5.9510 (−33.8%) | 3.9607 (−56.0%) | 0.02262 | 3170.81 |
| c8i.metal-96xl | 17.9923 | 13.0022 (−27.7%) | 8.6871 (−51.7%) | 11.9019 (−33.8%) | 7.9213 (−56.0%) | 0.02262 | 6341.61 |
| c7a.2xlarge | 0.4106 | 0.2967 (−27.7%) | 0.1982 (−51.7%) | 0.2716 (−33.8%) | 0.1807 (−56.0%) | 0.02478 | 144.71 |
| c7a.4xlarge | 0.8211 | 0.5934 (−27.7%) | 0.3965 (−51.7%) | 0.5432 (−33.9%) | 0.3615 (−56.0%) | 0.02478 | 289.42 |
| c7a.8xlarge | 1.6422 | 1.1868 (−27.7%) | 0.7929 (−51.7%) | 1.0863 (−33.9%) | 0.7230 (−56.0%) | 0.02478 | 578.82 |
| c7a.metal-48xl | 9.8534 | 7.1206 (−27.7%) | 4.7575 (−51.7%) | 6.5180 (−33.9%) | 4.3381 (−56.0%) | 0.02478 | 3472.97 |
| c8a.2xlarge | 0.4311 | 0.3115 (−27.7%) | 0.2081 (−51.7%) | 0.2852 (−33.8%) | 0.1898 (−56.0%) | 0.02602 | 151.94 |
| c8a.4xlarge | 0.8622 | 0.6230 (−27.7%) | 0.4163 (−51.7%) | 0.5703 (−33.8%) | 0.3796 (−56.0%) | 0.02602 | 303.88 |
| c8a.8xlarge | 1.7243 | 1.2461 (−27.7%) | 0.8325 (−51.7%) | 1.1406 (−33.8%) | 0.7591 (−56.0%) | 0.02602 | 607.75 |
| c8a.metal-24xl | 5.1730 | 3.7382 (−27.7%) | 2.4976 (−51.7%) | 3.4219 (−33.9%) | 2.2775 (−56.0%) | 0.02602 | 1823.27 |
| c8a.metal-48xl | 10.3459 | 7.4765 (−27.7%) | 4.9953 (−51.7%) | 6.8438 (−33.8%) | 4.5549 (−56.0%) | 0.02602 | 3646.55 |
| m7i.2xlarge | 0.4032 | 0.2962 (−26.5%) | 0.2045 (−49.3%) | 0.2667 (−33.8%) | 0.1829 (−54.6%) | 0.02556 | 149.27 |
| m7i.4xlarge | 0.8064 | 0.5925 (−26.5%) | 0.4090 (−49.3%) | 0.5334 (−33.9%) | 0.3658 (−54.6%) | 0.02556 | 298.54 |
| m7i.8xlarge | 1.6128 | 1.1849 (−26.5%) | 0.8179 (−49.3%) | 1.0669 (−33.8%) | 0.7316 (−54.6%) | 0.02556 | 597.09 |
| m7i.metal-24xl | 4.8384 | 3.5547 (−26.5%) | 2.4538 (−49.3%) | 3.2006 (−33.9%) | 2.1947 (−54.6%) | 0.02556 | 1791.26 |
| m7i.metal-48xl | 9.6768 | 7.1094 (−26.5%) | 4.9075 (−49.3%) | 6.4012 (−33.9%) | 4.3894 (−54.6%) | 0.02556 | 3582.51 |
| m8i.2xlarge | 0.4234 | 0.3110 (−26.5%) | 0.2147 (−49.3%) | 0.2801 (−33.9%) | 0.1920 (−54.6%) | 0.02684 | 156.74 |
| m8i.4xlarge | 0.8467 | 0.6221 (−26.5%) | 0.4294 (−49.3%) | 0.5601 (−33.8%) | 0.3841 (−54.6%) | 0.02684 | 313.47 |
| m8i.8xlarge | 1.6934 | 1.2441 (−26.5%) | 0.8588 (−49.3%) | 1.1202 (−33.9%) | 0.7681 (−54.6%) | 0.02684 | 626.94 |
| m8i.metal-48xl | 10.1606 | 7.4649 (−26.5%) | 5.1529 (−49.3%) | 6.7213 (−33.9%) | 4.6089 (−54.6%) | 0.02684 | 3761.64 |
| m8i.metal-96xl | 20.3213 | 14.9297 (−26.5%) | 10.3059 (−49.3%) | 13.4425 (−33.8%) | 9.2177 (−54.6%) | 0.02684 | 7523.28 |
| m7a.2xlarge | 0.4637 | 0.3407 (−26.5%) | 0.2351 (−49.3%) | 0.3067 (−33.9%) | 0.2103 (−54.6%) | 0.02939 | 171.66 |
| m7a.4xlarge | 0.9274 | 0.6813 (−26.5%) | 0.4703 (−49.3%) | 0.6135 (−33.8%) | 0.4207 (−54.6%) | 0.02939 | 343.33 |
| m7a.8xlarge | 1.8547 | 1.3626 (−26.5%) | 0.9406 (−49.3%) | 1.2269 (−33.8%) | 0.8413 (−54.6%) | 0.02939 | 686.65 |
| m7a.metal-48xl | 11.1283 | 8.1758 (−26.5%) | 5.6437 (−49.3%) | 7.3614 (−33.9%) | 5.0478 (−54.6%) | 0.02939 | 4119.89 |
| m8a.2xlarge | 0.4869 | 0.3577 (−26.5%) | 0.2469 (−49.3%) | 0.3221 (−33.9%) | 0.2208 (−54.6%) | 0.03087 | 180.25 |
| m8a.4xlarge | 0.9738 | 0.7154 (−26.5%) | 0.4938 (−49.3%) | 0.6441 (−33.9%) | 0.4417 (−54.6%) | 0.03087 | 360.50 |
| m8a.8xlarge | 1.9475 | 1.4308 (−26.5%) | 0.9877 (−49.3%) | 1.2883 (−33.9%) | 0.8834 (−54.6%) | 0.03087 | 721.01 |
| m8a.metal-24xl | 5.8426 | 4.2924 (−26.5%) | 2.9630 (−49.3%) | 3.8649 (−33.9%) | 2.6502 (−54.6%) | 0.03086 | 2163.01 |
| m8a.metal-48xl | 11.6851 | 8.5849 (−26.5%) | 5.9261 (−49.3%) | 7.7297 (−33.8%) | 5.3004 (−54.6%) | 0.03086 | 4326.03 |
| r8i.2xlarge | 0.5557 | 0.4026 (−27.6%) | 0.2899 (−47.8%) | 0.3676 (−33.9%) | 0.2521 (−54.6%) | 0.03623 | 211.60 |
| r8i.4xlarge | 1.1114 | 0.8052 (−27.6%) | 0.5797 (−47.8%) | 0.7352 (−33.9%) | 0.5041 (−54.6%) | 0.03623 | 423.20 |
| r8i.8xlarge | 2.2227 | 1.6104 (−27.6%) | 1.1595 (−47.8%) | 1.4703 (−33.8%) | 1.0082 (−54.6%) | 0.03623 | 846.41 |
| r8i.metal-48xl | 13.3363 | 9.6622 (−27.6%) | 6.9568 (−47.8%) | 8.8220 (−33.8%) | 6.0493 (−54.6%) | 0.03623 | 5078.43 |
| r8i.metal-96xl | 26.6726 | 19.3243 (−27.5%) | 13.9135 (−47.8%) | 17.6440 (−33.9%) | 12.0987 (−54.6%) | 0.03623 | 10156.87 |
| r8a.2xlarge | 0.6390 | 0.4630 (−27.6%) | 0.3333 (−47.8%) | 0.4227 (−33.9%) | 0.2899 (−54.6%) | 0.04167 | 243.35 |
| r8a.4xlarge | 1.2781 | 0.9260 (−27.5%) | 0.6667 (−47.8%) | 0.8455 (−33.8%) | 0.5797 (−54.6%) | 0.04167 | 486.69 |
| r8a.8xlarge | 2.5562 | 1.8519 (−27.5%) | 1.3334 (−47.8%) | 1.6909 (−33.8%) | 1.1595 (−54.6%) | 0.04167 | 973.38 |
| r8a.metal-24xl | 7.6685 | 5.5558 (−27.6%) | 4.0002 (−47.8%) | 5.0727 (−33.8%) | 3.4784 (−54.6%) | 0.04167 | 2920.14 |
| r8a.metal-48xl | 15.3370 | 11.1116 (−27.5%) | 8.0004 (−47.8%) | 10.1454 (−33.8%) | 6.9569 (−54.6%) | 0.04167 | 5840.27 |

#### eu-west-1: specifications, on-demand and Spot (Linux)

Sources: [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json), [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json) (fetched 2026-10-12T01:37:10Z), [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).

| Type | vCPU | GiB | Network | OD $/h | OD $/vCPU-h | OD $/GiB-h | OD $/mo (730 h) | Spot $/h (snapshot) | Spot below OD | Advisor savings | Advisor interruption |
|---|---|---|---|---|---|---|---|---|---|---|---|
| t4g.medium | 2 | 4 | Up to 5 Gigabit | 0.0368 | 0.01840 | 0.00920 | 26.86 | 0.0167 | 54.6% | 55% | >20% |
| t4g.large | 2 | 8 | Up to 5 Gigabit | 0.0736 | 0.03680 | 0.00920 | 53.73 | 0.0373 | 49.3% | 54% | >20% |
| t4g.xlarge | 4 | 16 | Up to 5 Gigabit | 0.1472 | 0.03680 | 0.00920 | 107.46 | 0.0761 | 48.3% | 51% | 15-20% |
| t4g.2xlarge | 8 | 32 | Up to 5 Gigabit | 0.2944 | 0.03680 | 0.00920 | 214.91 | 0.1482 | 49.7% | 49% | 5-10% |
| c7g.2xlarge | 8 | 16 | Up to 15 Gigabit | 0.3101 | 0.03876 | 0.01938 | 226.37 | 0.1528 | 50.7% | 51% | <5% |
| c7g.4xlarge | 16 | 32 | Up to 15 Gigabit | 0.6202 | 0.03876 | 0.01938 | 452.75 | 0.2310 | 62.8% | 61% | <5% |
| c7g.8xlarge | 32 | 64 | 15 Gigabit | 1.2403 | 0.03876 | 0.01938 | 905.42 | 0.5565 | 55.1% | 55% | <5% |
| c7g.metal | 64 | 128 | 30 Gigabit | 2.4806 | 0.03876 | 0.01938 | 1810.84 | 0.9584 | 61.4% | 64% | <5% |
| c8g.2xlarge | 8 | 16 | Up to 15 Gigabit | 0.3411 | 0.04264 | 0.02132 | 249.02 | 0.1887 | 44.7% | 49% | 5-10% |
| c8g.4xlarge | 16 | 32 | Up to 15 Gigabit | 0.6822 | 0.04264 | 0.02132 | 498.04 | 0.3213 | 52.9% | 59% | <5% |
| c8g.8xlarge | 32 | 64 | 15 Gigabit | 1.3645 | 0.04264 | 0.02132 | 996.07 | 0.7086 | 48.1% | 51% | 5-10% |
| c8g.metal-24xl | 96 | 192 | 40 Gigabit | 4.0934 | 0.04264 | 0.02132 | 2988.21 | 1.3096 | 68.0% | 74% | <5% |
| c8g.metal-48xl | 192 | 384 | 50 Gigabit | 8.1869 | 0.04264 | 0.02132 | 5976.42 | 0.8187 | 90.0% | 76% | 5-10% |
| c8gn.2xlarge | 8 | 16 | Up to 50 Gigabit | 0.5354 | 0.06692 | 0.03346 | 390.84 | 0.2267 | 57.7% | 62% | <5% |
| c8gn.4xlarge | 16 | 32 | 50 Gigabit | 1.0709 | 0.06693 | 0.03347 | 781.76 | 0.4258 | 60.2% | 75% | <5% |
| c8gn.8xlarge | 32 | 64 | 100 Gigabit | 2.1418 | 0.06693 | 0.03347 | 1563.51 | 0.6224 | 70.9% | 65% | <5% |
| m7g.2xlarge | 8 | 32 | Up to 15 Gigabit | 0.3638 | 0.04548 | 0.01137 | 265.57 | 0.1882 | 48.3% | 53% | <5% |
| m7g.4xlarge | 16 | 64 | Up to 15 Gigabit | 0.7276 | 0.04548 | 0.01137 | 531.15 | 0.3710 | 49.0% | 50% | <5% |
| m7g.8xlarge | 32 | 128 | 15 Gigabit | 1.4552 | 0.04548 | 0.01137 | 1062.30 | 0.7146 | 50.9% | 56% | <5% |
| m7g.metal | 64 | 256 | 30 Gigabit | 2.9104 | 0.04548 | 0.01137 | 2124.59 | 0.8040 | 72.4% | 75% | <5% |
| m8g.2xlarge | 8 | 32 | Up to 15 Gigabit | 0.4002 | 0.05002 | 0.01251 | 292.12 | 0.2083 | 47.9% | 54% | <5% |
| m8g.4xlarge | 16 | 64 | Up to 15 Gigabit | 0.8003 | 0.05002 | 0.01251 | 584.23 | 0.4087 | 48.9% | 53% | 5-10% |
| m8g.8xlarge | 32 | 128 | 15 Gigabit | 1.6006 | 0.05002 | 0.01251 | 1168.47 | 0.8328 | 48.0% | 51% | 5-10% |
| m8g.metal-24xl | 96 | 384 | 40 Gigabit | 4.8019 | 0.05002 | 0.01251 | 3505.40 | 1.4287 | 70.2% | 68% | 5-10% |
| m8g.metal-48xl | 192 | 768 | 50 Gigabit | 9.6038 | 0.05002 | 0.01251 | 7010.80 | 2.2624 | 76.4% | 74% | <5% |
| m9g.2xlarge | 8 | 32 | Up to 17000 Megabit | 0.4362 | 0.05452 | 0.01363 | 318.40 | 0.2101 | 51.8% | n/a | n/a |
| m9g.4xlarge | 16 | 64 | Up to 17000 Megabit | 0.8723 | 0.05452 | 0.01363 | 636.79 | 0.3786 | 56.6% | n/a | n/a |
| m9g.8xlarge | 32 | 128 | 17000 Megabit | 1.7446 | 0.05452 | 0.01363 | 1273.59 | 0.8464 | 51.5% | n/a | n/a |
| m9g.metal-48xl | 192 | 768 | 100000 Megabit | 10.4678 | 0.05452 | 0.01363 | 7641.52 | 3.0467 | 70.9% | n/a | n/a |
| r7g.2xlarge | 8 | 64 | Up to 15 Gigabit | 0.4794 | 0.05992 | 0.00749 | 349.96 | 0.1995 | 58.4% | 57% | <5% |
| r7g.4xlarge | 16 | 128 | Up to 15 Gigabit | 0.9588 | 0.05992 | 0.00749 | 699.92 | 0.3806 | 60.3% | 56% | <5% |
| r7g.8xlarge | 32 | 256 | 15 Gigabit | 1.9176 | 0.05992 | 0.00749 | 1399.85 | 0.7674 | 60.0% | 58% | <5% |
| r7g.metal | 64 | 512 | 30 Gigabit | 3.8352 | 0.05992 | 0.00749 | 2799.70 | 1.3074 | 65.9% | 76% | <5% |
| r8g.2xlarge | 8 | 64 | Up to 15 Gigabit | 0.5274 | 0.06592 | 0.00824 | 384.97 | 0.2477 | 53.0% | 48% | 5-10% |
| r8g.4xlarge | 16 | 128 | Up to 15 Gigabit | 1.0547 | 0.06592 | 0.00824 | 769.95 | 0.5138 | 51.3% | 45% | 5-10% |
| r8g.8xlarge | 32 | 256 | 15 Gigabit | 2.1094 | 0.06592 | 0.00824 | 1539.89 | 0.9521 | 54.9% | 47% | 5-10% |
| r8g.metal-24xl | 96 | 768 | 40 Gigabit | 6.3283 | 0.06592 | 0.00824 | 4619.67 | 1.5132 | 76.1% | 80% | <5% |
| r8g.metal-48xl | 192 | 1536 | 50 Gigabit | 12.6566 | 0.06592 | 0.00824 | 9239.35 | 2.9788 | 76.5% | 80% | <5% |
| c7i.2xlarge | 8 | 16 | Up to 12500 Megabit | 0.3830 | 0.04788 | 0.02394 | 279.62 | 0.2240 | 41.5% | 44% | >20% |
| c7i.4xlarge | 16 | 32 | Up to 12500 Megabit | 0.7661 | 0.04788 | 0.02394 | 559.24 | 0.3964 | 48.3% | 49% | 10-15% |
| c7i.8xlarge | 32 | 64 | 12500 Megabit | 1.5322 | 0.04788 | 0.02394 | 1118.48 | 0.7725 | 49.6% | 50% | 5-10% |
| c7i.metal-24xl | 96 | 192 | 37500 Megabit | 4.5965 | 0.04788 | 0.02394 | 3355.43 | 1.2217 | 73.4% | 71% | <5% |
| c7i.metal-48xl | 192 | 384 | 50000 Megabit | 9.1930 | 0.04788 | 0.02394 | 6710.86 | 2.5250 | 72.5% | 77% | <5% |
| c8i.2xlarge | 8 | 16 | Up to 15000 Megabit | 0.4022 | 0.05028 | 0.02514 | 293.61 | 0.2065 | 48.7% | n/a | n/a |
| c8i.4xlarge | 16 | 32 | Up to 15000 Megabit | 0.8044 | 0.05028 | 0.02514 | 587.21 | 0.4264 | 47.0% | n/a | n/a |
| c8i.8xlarge | 32 | 64 | 15000 Megabit | 1.6088 | 0.05028 | 0.02514 | 1174.42 | 0.8087 | 49.7% | n/a | n/a |
| c8i.metal-48xl | 192 | 384 | 75000 Megabit | 9.6528 | 0.05027 | 0.02514 | 7046.54 | 2.6844 | 72.2% | n/a | n/a |
| c8i.metal-96xl | 384 | 768 | 100000 Megabit | 19.3056 | 0.05027 | 0.02514 | 14093.09 | 3.4340 | 82.2% | n/a | n/a |
| c7a.2xlarge | 8 | 16 | Up to 12500 Megabit | 0.4405 | 0.05506 | 0.02753 | 321.55 | 0.2631 | 40.3% | 46% | 15-20% |
| c7a.4xlarge | 16 | 32 | Up to 12500 Megabit | 0.8810 | 0.05506 | 0.02753 | 643.10 | 0.4331 | 50.8% | 41% | >20% |
| c7a.8xlarge | 32 | 64 | 12500 Megabit | 1.7619 | 0.05506 | 0.02753 | 1286.20 | 0.7576 | 57.0% | 50% | 15-20% |
| c7a.metal-48xl | 192 | 384 | 50000 Megabit | 10.5715 | 0.05506 | 0.02753 | 7717.21 | 2.6710 | 74.7% | 71% | >20% |
| c8a.2xlarge | 8 | 16 | Up to 15000 Megabit | 0.4625 | 0.05781 | 0.02891 | 337.64 | 0.1985 | 57.1% | 45% | <5% |
| c8a.4xlarge | 16 | 32 | Up to 15000 Megabit | 0.9250 | 0.05781 | 0.02891 | 675.28 | 0.4294 | 53.6% | 41% | <5% |
| c8a.8xlarge | 32 | 64 | 15000 Megabit | 1.8501 | 0.05781 | 0.02891 | 1350.56 | 0.8188 | 55.7% | 42% | <5% |
| c8a.metal-24xl | 96 | 192 | 40000 Megabit | 5.5502 | 0.05781 | 0.02891 | 4051.68 | 1.4145 | 74.5% | 67% | <5% |
| c8a.metal-48xl | 192 | 384 | 75000 Megabit | 11.1005 | 0.05781 | 0.02891 | 8103.35 | 2.6609 | 76.0% | 67% | 5-10% |
| m7i.2xlarge | 8 | 32 | Up to 12500 Megabit | 0.4494 | 0.05618 | 0.01404 | 328.06 | 0.2891 | 35.7% | 43% | 15-20% |
| m7i.4xlarge | 16 | 64 | Up to 12500 Megabit | 0.8988 | 0.05618 | 0.01404 | 656.12 | 0.4817 | 46.4% | 37% | 5-10% |
| m7i.8xlarge | 32 | 128 | 12500 Megabit | 1.7976 | 0.05618 | 0.01404 | 1312.25 | 0.8667 | 51.8% | 53% | 5-10% |
| m7i.metal-24xl | 96 | 384 | 37500 Megabit | 5.3928 | 0.05618 | 0.01404 | 3936.74 | 1.3563 | 74.8% | 75% | <5% |
| m7i.metal-48xl | 192 | 768 | 50000 Megabit | 10.7856 | 0.05618 | 0.01404 | 7873.49 | 2.6006 | 75.9% | 79% | <5% |
| m8i.2xlarge | 8 | 32 | Up to 15000 Megabit | 0.4719 | 0.05899 | 0.01475 | 344.47 | 0.2496 | 47.1% | 51% | 15-20% |
| m8i.4xlarge | 16 | 64 | Up to 15000 Megabit | 0.9438 | 0.05899 | 0.01475 | 688.94 | 0.4802 | 49.1% | 51% | 15-20% |
| m8i.8xlarge | 32 | 128 | 15000 Megabit | 1.8875 | 0.05899 | 0.01475 | 1377.89 | 0.9536 | 49.5% | 49% | >20% |
| m8i.metal-48xl | 192 | 768 | 75000 Megabit | 11.3251 | 0.05899 | 0.01475 | 8267.34 | 2.8364 | 75.0% | 53% | <5% |
| m8i.metal-96xl | 384 | 1536 | 100000 Megabit | 22.6502 | 0.05899 | 0.01475 | 16534.68 | 5.7982 | 74.4% | 58% | >20% |
| m7a.2xlarge | 8 | 32 | Up to 12500 Megabit | 0.5168 | 0.06460 | 0.01615 | 377.26 | 0.3009 | 41.8% | 49% | 15-20% |
| m7a.4xlarge | 16 | 64 | Up to 12500 Megabit | 1.0336 | 0.06460 | 0.01615 | 754.53 | 0.4835 | 53.2% | 49% | 10-15% |
| m7a.8xlarge | 32 | 128 | 12500 Megabit | 2.0672 | 0.06460 | 0.01615 | 1509.06 | 0.9085 | 56.1% | 56% | 5-10% |
| m7a.metal-48xl | 192 | 768 | 50000 Megabit | 12.4032 | 0.06460 | 0.01615 | 9054.34 | 2.9185 | 76.5% | 76% | >20% |
| m8a.2xlarge | 8 | 32 | Up to 15000 Megabit | 0.5426 | 0.06783 | 0.01696 | 396.13 | 0.2642 | 51.3% | n/a | n/a |
| m8a.4xlarge | 16 | 64 | Up to 15000 Megabit | 1.0853 | 0.06783 | 0.01696 | 792.25 | 0.5146 | 52.6% | n/a | n/a |
| m8a.8xlarge | 32 | 128 | 15000 Megabit | 2.1706 | 0.06783 | 0.01696 | 1584.51 | 0.9879 | 54.5% | n/a | n/a |
| m8a.metal-24xl | 96 | 384 | 40000 Megabit | 6.5117 | 0.06783 | 0.01696 | 4753.53 | 2.0077 | 69.2% | n/a | n/a |
| m8a.metal-48xl | 192 | 768 | 75000 Megabit | 13.0234 | 0.06783 | 0.01696 | 9507.05 | 3.8450 | 70.5% | n/a | n/a |
| r8i.2xlarge | 8 | 64 | Up to 15000 Megabit | 0.6218 | 0.07773 | 0.00972 | 453.91 | 0.3010 | 51.6% | 38% | >20% |
| r8i.4xlarge | 16 | 128 | Up to 15000 Megabit | 1.2436 | 0.07773 | 0.00972 | 907.83 | 0.5648 | 54.6% | 48% | 10-15% |
| r8i.8xlarge | 32 | 256 | 15000 Megabit | 2.4872 | 0.07773 | 0.00972 | 1815.66 | 1.2196 | 51.0% | 49% | >20% |
| r8i.metal-48xl | 192 | 1536 | 75000 Megabit | 14.9232 | 0.07773 | 0.00972 | 10893.94 | 2.8530 | 80.9% | 45% | <5% |
| r8i.metal-96xl | 384 | 3072 | 100000 Megabit | 29.8464 | 0.07773 | 0.00972 | 21787.87 | 5.8147 | 80.5% | 63% | 15-20% |
| r8a.2xlarge | 8 | 64 | Up to 15000 Megabit | 0.7151 | 0.08939 | 0.01117 | 522.01 | 0.3028 | 57.7% | 38% | 5-10% |
| r8a.4xlarge | 16 | 128 | Up to 15000 Megabit | 1.4302 | 0.08939 | 0.01117 | 1044.02 | 0.6842 | 52.2% | 38% | <5% |
| r8a.8xlarge | 32 | 256 | 15000 Megabit | 2.8603 | 0.08939 | 0.01117 | 2088.03 | 1.1636 | 59.3% | 48% | 5-10% |
| r8a.metal-24xl | 96 | 768 | 40000 Megabit | 8.5810 | 0.08938 | 0.01117 | 6264.10 | 2.6028 | 69.7% | 67% | <5% |
| r8a.metal-48xl | 192 | 1536 | 75000 Megabit | 17.1619 | 0.08938 | 0.01117 | 12528.20 | 4.1886 | 75.6% | 66% | <5% |

#### eu-west-1: Savings Plans effective hourly rates, No Upfront (discount vs on-demand)

Sources: [SP offer eu-west-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-1/index.json), [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json).

| Type | OD $/h | Compute SP 1 yr | Compute SP 3 yr | EC2 Instance SP 1 yr | EC2 Instance SP 3 yr | CSP 3 yr $/vCPU-h | CSP 3 yr $/mo (730 h) |
|---|---|---|---|---|---|---|---|
| t4g.medium | 0.0368 | 0.0302 (−17.9%) | 0.0213 (−42.1%) | 0.0232 (−37.0%) | 0.0159 (−56.8%) | 0.01065 | 15.55 |
| t4g.large | 0.0736 | 0.0603 (−18.1%) | 0.0426 (−42.1%) | 0.0464 (−37.0%) | 0.0318 (−56.8%) | 0.02130 | 31.10 |
| t4g.xlarge | 0.1472 | 0.1207 (−18.0%) | 0.0851 (−42.2%) | 0.0929 (−36.9%) | 0.0636 (−56.8%) | 0.02127 | 62.12 |
| t4g.2xlarge | 0.2944 | 0.2414 (−18.0%) | 0.1702 (−42.2%) | 0.1857 (−36.9%) | 0.1271 (−56.8%) | 0.02127 | 124.25 |
| c7g.2xlarge | 0.3101 | 0.2369 (−23.6%) | 0.1620 (−47.8%) | 0.2053 (−33.8%) | 0.1414 (−54.4%) | 0.02025 | 118.26 |
| c7g.4xlarge | 0.6202 | 0.4739 (−23.6%) | 0.3240 (−47.8%) | 0.4105 (−33.8%) | 0.2827 (−54.4%) | 0.02025 | 236.52 |
| c7g.8xlarge | 1.2403 | 0.9478 (−23.6%) | 0.6480 (−47.8%) | 0.8211 (−33.8%) | 0.5655 (−54.4%) | 0.02025 | 473.04 |
| c7g.metal | 2.4806 | 1.8955 (−23.6%) | 1.2961 (−47.8%) | 1.6422 (−33.8%) | 1.1310 (−54.4%) | 0.02025 | 946.15 |
| c8g.2xlarge | 0.3411 | 0.2607 (−23.6%) | 0.1782 (−47.8%) | 0.2258 (−33.8%) | 0.1555 (−54.4%) | 0.02228 | 130.11 |
| c8g.4xlarge | 0.6822 | 0.5213 (−23.6%) | 0.3564 (−47.8%) | 0.4516 (−33.8%) | 0.3110 (−54.4%) | 0.02228 | 260.21 |
| c8g.8xlarge | 1.3645 | 1.0426 (−23.6%) | 0.7129 (−47.8%) | 0.9033 (−33.8%) | 0.6221 (−54.4%) | 0.02228 | 520.42 |
| c8g.metal-24xl | 4.0934 | 3.1279 (−23.6%) | 2.1387 (−47.8%) | 2.7099 (−33.8%) | 1.8663 (−54.4%) | 0.02228 | 1561.26 |
| c8g.metal-48xl | 8.1869 | 6.2558 (−23.6%) | 4.2774 (−47.8%) | 5.4198 (−33.8%) | 3.7326 (−54.4%) | 0.02228 | 3122.52 |
| c8gn.2xlarge | 0.5354 | 0.4242 (−20.8%) | 0.2554 (−52.3%) | 0.3373 (−37.0%) | 0.2244 (−58.1%) | 0.03193 | 186.44 |
| c8gn.4xlarge | 1.0709 | 0.8484 (−20.8%) | 0.5108 (−52.3%) | 0.6747 (−37.0%) | 0.4487 (−58.1%) | 0.03193 | 372.88 |
| c8gn.8xlarge | 2.1418 | 1.6968 (−20.8%) | 1.0217 (−52.3%) | 1.3493 (−37.0%) | 0.8975 (−58.1%) | 0.03193 | 745.84 |
| m7g.2xlarge | 0.3638 | 0.2761 (−24.1%) | 0.1911 (−47.5%) | 0.2407 (−33.8%) | 0.1650 (−54.6%) | 0.02389 | 139.50 |
| m7g.4xlarge | 0.7276 | 0.5523 (−24.1%) | 0.3823 (−47.5%) | 0.4813 (−33.9%) | 0.3300 (−54.6%) | 0.02389 | 279.08 |
| m7g.8xlarge | 1.4552 | 1.1046 (−24.1%) | 0.7645 (−47.5%) | 0.9626 (−33.9%) | 0.6601 (−54.6%) | 0.02389 | 558.08 |
| m7g.metal | 2.9104 | 2.2091 (−24.1%) | 1.5290 (−47.5%) | 1.9252 (−33.9%) | 1.3202 (−54.6%) | 0.02389 | 1116.17 |
| m8g.2xlarge | 0.4002 | 0.3037 (−24.1%) | 0.2102 (−47.5%) | 0.2647 (−33.8%) | 0.1815 (−54.6%) | 0.02628 | 153.47 |
| m8g.4xlarge | 0.8003 | 0.6075 (−24.1%) | 0.4205 (−47.5%) | 0.5294 (−33.9%) | 0.3630 (−54.6%) | 0.02628 | 306.94 |
| m8g.8xlarge | 1.6006 | 1.2149 (−24.1%) | 0.8409 (−47.5%) | 1.0588 (−33.9%) | 0.7260 (−54.6%) | 0.02628 | 613.87 |
| m8g.metal-24xl | 4.8019 | 3.6449 (−24.1%) | 2.5228 (−47.5%) | 3.1765 (−33.9%) | 2.1782 (−54.6%) | 0.02628 | 1841.61 |
| m8g.metal-48xl | 9.6038 | 7.2897 (−24.1%) | 5.0455 (−47.5%) | 6.3529 (−33.9%) | 4.3563 (−54.6%) | 0.02628 | 3683.23 |
| m9g.2xlarge | 0.4362 | 0.3311 (−24.1%) | 0.2291 (−47.5%) | 0.2885 (−33.8%) | 0.1978 (−54.6%) | 0.02864 | 167.27 |
| m9g.4xlarge | 0.8723 | 0.6621 (−24.1%) | 0.4583 (−47.5%) | 0.5770 (−33.8%) | 0.3957 (−54.6%) | 0.02864 | 334.55 |
| m9g.8xlarge | 1.7446 | 1.3243 (−24.1%) | 0.9166 (−47.5%) | 1.1541 (−33.8%) | 0.7914 (−54.6%) | 0.02864 | 669.10 |
| m9g.metal-48xl | 10.4678 | 7.9455 (−24.1%) | 5.4994 (−47.5%) | 6.9245 (−33.8%) | 4.7482 (−54.6%) | 0.02864 | 4014.58 |
| r7g.2xlarge | 0.4794 | 0.3473 (−27.6%) | 0.2501 (−47.8%) | 0.3171 (−33.9%) | 0.2175 (−54.6%) | 0.03126 | 182.57 |
| r7g.4xlarge | 0.9588 | 0.6947 (−27.5%) | 0.5001 (−47.8%) | 0.6342 (−33.9%) | 0.4349 (−54.6%) | 0.03126 | 365.07 |
| r7g.8xlarge | 1.9176 | 1.3893 (−27.6%) | 1.0003 (−47.8%) | 1.2685 (−33.8%) | 0.8698 (−54.6%) | 0.03126 | 730.22 |
| r7g.metal | 3.8352 | 2.7786 (−27.6%) | 2.0005 (−47.8%) | 2.5370 (−33.8%) | 1.7396 (−54.6%) | 0.03126 | 1460.37 |
| r8g.2xlarge | 0.5274 | 0.3821 (−27.6%) | 0.2751 (−47.8%) | 0.3488 (−33.8%) | 0.2392 (−54.6%) | 0.03438 | 200.81 |
| r8g.4xlarge | 1.0547 | 0.7641 (−27.6%) | 0.5502 (−47.8%) | 0.6977 (−33.8%) | 0.4784 (−54.6%) | 0.03438 | 401.62 |
| r8g.8xlarge | 2.1094 | 1.5283 (−27.5%) | 1.1003 (−47.8%) | 1.3954 (−33.9%) | 0.9568 (−54.6%) | 0.03438 | 803.23 |
| r8g.metal-24xl | 6.3283 | 4.5849 (−27.5%) | 3.3010 (−47.8%) | 4.1862 (−33.9%) | 2.8705 (−54.6%) | 0.03438 | 2409.70 |
| r8g.metal-48xl | 12.6566 | 9.1697 (−27.5%) | 6.6019 (−47.8%) | 8.3724 (−33.8%) | 5.7411 (−54.6%) | 0.03438 | 4819.40 |
| c7i.2xlarge | 0.3830 | 0.2927 (−23.6%) | 0.2001 (−47.7%) | 0.2536 (−33.8%) | 0.1746 (−54.4%) | 0.02502 | 146.10 |
| c7i.4xlarge | 0.7661 | 0.5854 (−23.6%) | 0.4003 (−47.7%) | 0.5071 (−33.8%) | 0.3493 (−54.4%) | 0.02502 | 292.21 |
| c7i.8xlarge | 1.5322 | 1.1708 (−23.6%) | 0.8006 (−47.7%) | 1.0143 (−33.8%) | 0.6986 (−54.4%) | 0.02502 | 584.42 |
| c7i.metal-24xl | 4.5965 | 3.5123 (−23.6%) | 2.4017 (−47.7%) | 3.0429 (−33.8%) | 2.0956 (−54.4%) | 0.02502 | 1753.26 |
| c7i.metal-48xl | 9.1930 | 7.0247 (−23.6%) | 4.8035 (−47.7%) | 6.0858 (−33.8%) | 4.1913 (−54.4%) | 0.02502 | 3506.53 |
| c8i.2xlarge | 0.4022 | 0.3073 (−23.6%) | 0.2102 (−47.7%) | 0.2663 (−33.8%) | 0.1834 (−54.4%) | 0.02627 | 153.42 |
| c8i.4xlarge | 0.8044 | 0.6147 (−23.6%) | 0.4203 (−47.7%) | 0.5325 (−33.8%) | 0.3667 (−54.4%) | 0.02627 | 306.83 |
| c8i.8xlarge | 1.6088 | 1.2293 (−23.6%) | 0.8406 (−47.7%) | 1.0650 (−33.8%) | 0.7335 (−54.4%) | 0.02627 | 613.65 |
| c8i.metal-48xl | 9.6528 | 7.3761 (−23.6%) | 5.0437 (−47.7%) | 6.3902 (−33.8%) | 4.4009 (−54.4%) | 0.02627 | 3681.92 |
| c8i.metal-96xl | 19.3056 | 14.7521 (−23.6%) | 10.0875 (−47.7%) | 12.7804 (−33.8%) | 8.8018 (−54.4%) | 0.02627 | 7363.85 |
| c7a.2xlarge | 0.4405 | 0.3366 (−23.6%) | 0.2302 (−47.7%) | 0.2916 (−33.8%) | 0.2008 (−54.4%) | 0.02877 | 168.02 |
| c7a.4xlarge | 0.8810 | 0.6732 (−23.6%) | 0.4603 (−47.7%) | 0.5832 (−33.8%) | 0.4017 (−54.4%) | 0.02877 | 336.03 |
| c7a.8xlarge | 1.7619 | 1.3463 (−23.6%) | 0.9206 (−47.7%) | 1.1664 (−33.8%) | 0.8033 (−54.4%) | 0.02877 | 672.06 |
| c7a.metal-48xl | 10.5715 | 8.0781 (−23.6%) | 5.5238 (−47.7%) | 6.9984 (−33.8%) | 4.8198 (−54.4%) | 0.02877 | 4032.36 |
| c8a.2xlarge | 0.4625 | 0.3534 (−23.6%) | 0.2417 (−47.7%) | 0.3062 (−33.8%) | 0.2109 (−54.4%) | 0.03021 | 176.42 |
| c8a.4xlarge | 0.9250 | 0.7069 (−23.6%) | 0.4834 (−47.7%) | 0.6124 (−33.8%) | 0.4218 (−54.4%) | 0.03021 | 352.85 |
| c8a.8xlarge | 1.8501 | 1.4137 (−23.6%) | 0.9667 (−47.7%) | 1.2248 (−33.8%) | 0.8435 (−54.4%) | 0.03021 | 705.68 |
| c8a.metal-24xl | 5.5502 | 4.2412 (−23.6%) | 2.9001 (−47.7%) | 3.6743 (−33.8%) | 2.5305 (−54.4%) | 0.03021 | 2117.06 |
| c8a.metal-48xl | 11.1005 | 8.4823 (−23.6%) | 5.8002 (−47.7%) | 7.3486 (−33.8%) | 5.0610 (−54.4%) | 0.03021 | 4234.12 |
| m7i.2xlarge | 0.4494 | 0.3411 (−24.1%) | 0.2361 (−47.5%) | 0.2973 (−33.8%) | 0.2039 (−54.6%) | 0.02951 | 172.36 |
| m7i.4xlarge | 0.8988 | 0.6823 (−24.1%) | 0.4722 (−47.5%) | 0.5946 (−33.8%) | 0.4077 (−54.6%) | 0.02951 | 344.73 |
| m7i.8xlarge | 1.7976 | 1.3645 (−24.1%) | 0.9445 (−47.5%) | 1.1891 (−33.9%) | 0.8154 (−54.6%) | 0.02951 | 689.46 |
| m7i.metal-24xl | 5.3928 | 4.0935 (−24.1%) | 2.8334 (−47.5%) | 3.5673 (−33.8%) | 2.4462 (−54.6%) | 0.02951 | 2068.36 |
| m7i.metal-48xl | 10.7856 | 8.1870 (−24.1%) | 5.6667 (−47.5%) | 7.1347 (−33.9%) | 4.8924 (−54.6%) | 0.02951 | 4136.72 |
| m8i.2xlarge | 0.4719 | 0.3582 (−24.1%) | 0.2479 (−47.5%) | 0.3121 (−33.8%) | 0.2140 (−54.6%) | 0.03099 | 180.98 |
| m8i.4xlarge | 0.9438 | 0.7164 (−24.1%) | 0.4959 (−47.5%) | 0.6243 (−33.8%) | 0.4281 (−54.6%) | 0.03099 | 361.97 |
| m8i.8xlarge | 1.8875 | 1.4327 (−24.1%) | 0.9917 (−47.5%) | 1.2486 (−33.9%) | 0.8562 (−54.6%) | 0.03099 | 723.94 |
| m8i.metal-48xl | 11.3251 | 8.5965 (−24.1%) | 5.9502 (−47.5%) | 7.4916 (−33.8%) | 5.1371 (−54.6%) | 0.03099 | 4343.65 |
| m8i.metal-96xl | 22.6502 | 17.1930 (−24.1%) | 11.9004 (−47.5%) | 14.9831 (−33.9%) | 10.2742 (−54.6%) | 0.03099 | 8687.29 |
| m7a.2xlarge | 0.5168 | 0.3923 (−24.1%) | 0.2715 (−47.5%) | 0.3419 (−33.9%) | 0.2344 (−54.6%) | 0.03394 | 198.22 |
| m7a.4xlarge | 1.0336 | 0.7846 (−24.1%) | 0.5431 (−47.5%) | 0.6837 (−33.8%) | 0.4688 (−54.6%) | 0.03394 | 396.43 |
| m7a.8xlarge | 2.0672 | 1.5691 (−24.1%) | 1.0861 (−47.5%) | 1.3675 (−33.9%) | 0.9377 (−54.6%) | 0.03394 | 792.85 |
| m7a.metal-48xl | 12.4032 | 9.4148 (−24.1%) | 6.5166 (−47.5%) | 8.2047 (−33.8%) | 5.6261 (−54.6%) | 0.03394 | 4757.13 |
| m8a.2xlarge | 0.5426 | 0.4119 (−24.1%) | 0.2851 (−47.5%) | 0.3590 (−33.8%) | 0.2461 (−54.6%) | 0.03564 | 208.12 |
| m8a.4xlarge | 1.0853 | 0.8238 (−24.1%) | 0.5702 (−47.5%) | 0.7179 (−33.9%) | 0.4923 (−54.6%) | 0.03564 | 416.25 |
| m8a.8xlarge | 2.1706 | 1.6476 (−24.1%) | 1.1404 (−47.5%) | 1.4358 (−33.8%) | 0.9846 (−54.6%) | 0.03564 | 832.50 |
| m8a.metal-24xl | 6.5117 | 4.9428 (−24.1%) | 3.4212 (−47.5%) | 4.3075 (−33.8%) | 2.9537 (−54.6%) | 0.03564 | 2497.50 |
| m8a.metal-48xl | 13.0234 | 9.8856 (−24.1%) | 6.8425 (−47.5%) | 8.6150 (−33.9%) | 5.9074 (−54.6%) | 0.03564 | 4994.99 |
| r8i.2xlarge | 0.6218 | 0.4505 (−27.6%) | 0.3244 (−47.8%) | 0.4113 (−33.9%) | 0.2821 (−54.6%) | 0.04054 | 236.78 |
| r8i.4xlarge | 1.2436 | 0.9010 (−27.5%) | 0.6487 (−47.8%) | 0.8226 (−33.9%) | 0.5641 (−54.6%) | 0.04054 | 473.56 |
| r8i.8xlarge | 2.4872 | 1.8020 (−27.5%) | 1.2974 (−47.8%) | 1.6453 (−33.9%) | 1.1282 (−54.6%) | 0.04054 | 947.12 |
| r8i.metal-48xl | 14.9232 | 10.8119 (−27.5%) | 7.7845 (−47.8%) | 9.8717 (−33.8%) | 6.7692 (−54.6%) | 0.04054 | 5682.71 |
| r8i.metal-96xl | 29.8464 | 21.6237 (−27.5%) | 15.5691 (−47.8%) | 19.7434 (−33.9%) | 13.5383 (−54.6%) | 0.04054 | 11365.43 |
| r8a.2xlarge | 0.7151 | 0.5181 (−27.5%) | 0.3730 (−47.8%) | 0.4730 (−33.8%) | 0.3244 (−54.6%) | 0.04663 | 272.30 |
| r8a.4xlarge | 1.4302 | 1.0361 (−27.6%) | 0.7460 (−47.8%) | 0.9460 (−33.9%) | 0.6487 (−54.6%) | 0.04663 | 544.60 |
| r8a.8xlarge | 2.8603 | 2.0723 (−27.6%) | 1.4921 (−47.8%) | 1.8921 (−33.9%) | 1.2974 (−54.6%) | 0.04663 | 1089.20 |
| r8a.metal-24xl | 8.5810 | 6.2169 (−27.5%) | 4.4762 (−47.8%) | 5.6763 (−33.8%) | 3.8923 (−54.6%) | 0.04663 | 3267.60 |
| r8a.metal-48xl | 17.1619 | 12.4338 (−27.6%) | 8.9523 (−47.8%) | 11.3526 (−33.9%) | 7.7847 (−54.6%) | 0.04663 | 6535.21 |

#### eu-west-3: specifications, on-demand and Spot (Linux)

Sources: [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json), [Spot price feed](https://website.spot.ec2.aws.a2z.com/spot.json) (fetched 2026-10-12T01:37:10Z), [spot-advisor-data.json](https://spot-bid-advisor.s3.amazonaws.com/spot-advisor-data.json).

| Type | vCPU | GiB | Network | OD $/h | OD $/vCPU-h | OD $/GiB-h | OD $/mo (730 h) | Spot $/h (snapshot) | Spot below OD | Advisor savings | Advisor interruption |
|---|---|---|---|---|---|---|---|---|---|---|---|
| t4g.medium | 2 | 4 | Up to 5 Gigabit | 0.0376 | 0.01880 | 0.00940 | 27.45 | 0.0155 | 58.8% | 66% | 15-20% |
| t4g.large | 2 | 8 | Up to 5 Gigabit | 0.0752 | 0.03760 | 0.00940 | 54.90 | 0.0285 | 62.1% | 64% | >20% |
| t4g.xlarge | 4 | 16 | Up to 5 Gigabit | 0.1504 | 0.03760 | 0.00940 | 109.79 | 0.0637 | 57.6% | 63% | 15-20% |
| t4g.2xlarge | 8 | 32 | Up to 5 Gigabit | 0.3008 | 0.03760 | 0.00940 | 219.58 | 0.1221 | 59.4% | 67% | 10-15% |
| c7g.2xlarge | 8 | 16 | Up to 15 Gigabit | 0.3434 | 0.04292 | 0.02146 | 250.68 | 0.1416 | 58.8% | 66% | >20% |
| c7g.4xlarge | 16 | 32 | Up to 15 Gigabit | 0.6869 | 0.04293 | 0.02147 | 501.44 | 0.2561 | 62.7% | 76% | >20% |
| c7g.8xlarge | 32 | 64 | 15 Gigabit | 1.3738 | 0.04293 | 0.02147 | 1002.87 | 0.4515 | 67.1% | 59% | >20% |
| c7g.metal | 64 | 128 | 30 Gigabit | 2.7475 | 0.04293 | 0.02146 | 2005.67 | 0.4932 | 82.0% | 49% | <5% |
| c8g.2xlarge | 8 | 16 | Up to 15 Gigabit | 0.3778 | 0.04722 | 0.02361 | 275.76 | 0.1474 | 61.0% | n/a | n/a |
| c8g.4xlarge | 16 | 32 | Up to 15 Gigabit | 0.7555 | 0.04722 | 0.02361 | 551.53 | 0.2969 | 60.7% | n/a | n/a |
| c8g.8xlarge | 32 | 64 | 15 Gigabit | 1.5110 | 0.04722 | 0.02361 | 1103.06 | 0.4719 | 68.8% | n/a | n/a |
| c8g.metal-24xl | 96 | 192 | 40 Gigabit | 4.5331 | 0.04722 | 0.02361 | 3309.18 | 0.5879 | 87.0% | n/a | n/a |
| c8g.metal-48xl | 192 | 384 | 50 Gigabit | 9.0662 | 0.04722 | 0.02361 | 6618.36 | 0.9066 | 90.0% | n/a | n/a |
| c8gn.2xlarge | 8 | 16 | Up to 50 Gigabit | 0.5618 | 0.07022 | 0.03511 | 410.11 | 0.1745 | 68.9% | n/a | n/a |
| c8gn.4xlarge | 16 | 32 | 50 Gigabit | 1.1235 | 0.07022 | 0.03511 | 820.15 | 0.3304 | 70.6% | n/a | n/a |
| c8gn.8xlarge | 32 | 64 | 100 Gigabit | 2.2470 | 0.07022 | 0.03511 | 1640.31 | 0.5916 | 73.7% | n/a | n/a |
| m7g.2xlarge | 8 | 32 | Up to 15 Gigabit | 0.3808 | 0.04760 | 0.01190 | 277.98 | 0.1613 | 57.6% | 53% | 15-20% |
| m7g.4xlarge | 16 | 64 | Up to 15 Gigabit | 0.7616 | 0.04760 | 0.01190 | 555.97 | 0.2877 | 62.2% | 50% | 10-15% |
| m7g.8xlarge | 32 | 128 | 15 Gigabit | 1.5232 | 0.04760 | 0.01190 | 1111.94 | 0.7047 | 53.7% | 47% | 5-10% |
| m7g.metal | 64 | 256 | 30 Gigabit | 3.0464 | 0.04760 | 0.01190 | 2223.87 | 0.4143 | 86.4% | 79% | 5-10% |
| m8g.2xlarge | 8 | 32 | Up to 15 Gigabit | 0.4189 | 0.05236 | 0.01309 | 305.78 | 0.1665 | 60.3% | 53% | >20% |
| m8g.4xlarge | 16 | 64 | Up to 15 Gigabit | 0.8378 | 0.05236 | 0.01309 | 611.56 | 0.3248 | 61.2% | 51% | 15-20% |
| m8g.8xlarge | 32 | 128 | 15 Gigabit | 1.6755 | 0.05236 | 0.01309 | 1223.13 | 0.7592 | 54.7% | 48% | 15-20% |
| m8g.metal-24xl | 96 | 384 | 40 Gigabit | 5.0266 | 0.05236 | 0.01309 | 3669.39 | 0.7245 | 85.6% | 84% | 5-10% |
| m8g.metal-48xl | 192 | 768 | 50 Gigabit | 10.0531 | 0.05236 | 0.01309 | 7338.78 | 1.0053 | 90.0% | 86% | <5% |
| r7g.2xlarge | 8 | 64 | Up to 15 Gigabit | 0.5032 | 0.06290 | 0.00786 | 367.34 | 0.1845 | 63.3% | 69% | >20% |
| r7g.4xlarge | 16 | 128 | Up to 15 Gigabit | 1.0064 | 0.06290 | 0.00786 | 734.67 | 0.3451 | 65.7% | 65% | >20% |
| r7g.8xlarge | 32 | 256 | 15 Gigabit | 2.0128 | 0.06290 | 0.00786 | 1469.34 | 0.5751 | 71.4% | 59% | >20% |
| r7g.metal | 64 | 512 | 30 Gigabit | 4.0256 | 0.06290 | 0.00786 | 2938.69 | 0.4026 | 90.0% | 86% | 10-15% |
| r8g.2xlarge | 8 | 64 | Up to 15 Gigabit | 0.5535 | 0.06919 | 0.00865 | 404.07 | 0.1383 | 75.0% | 78% | >20% |
| r8g.4xlarge | 16 | 128 | Up to 15 Gigabit | 1.1070 | 0.06919 | 0.00865 | 808.14 | 0.2332 | 78.9% | 75% | >20% |
| r8g.8xlarge | 32 | 256 | 15 Gigabit | 2.2141 | 0.06919 | 0.00865 | 1616.28 | 0.8535 | 61.5% | 62% | >20% |
| r8g.metal-24xl | 96 | 768 | 40 Gigabit | 6.6422 | 0.06919 | 0.00865 | 4848.84 | 0.6642 | 90.0% | 60% | <5% |
| r8g.metal-48xl | 192 | 1536 | 50 Gigabit | 13.2845 | 0.06919 | 0.00865 | 9697.67 | 1.3284 | 90.0% | 71% | <5% |
| c7i.2xlarge | 8 | 16 | Up to 12500 Megabit | 0.4242 | 0.05303 | 0.02651 | 309.67 | 0.1503 | 64.6% | 61% | >20% |
| c7i.4xlarge | 16 | 32 | Up to 12500 Megabit | 0.8484 | 0.05303 | 0.02651 | 619.33 | 0.3081 | 63.7% | 70% | >20% |
| c7i.8xlarge | 32 | 64 | 12500 Megabit | 1.6968 | 0.05303 | 0.02651 | 1238.66 | 0.4165 | 75.5% | 67% | >20% |
| c7i.metal-24xl | 96 | 192 | 37500 Megabit | 5.0904 | 0.05302 | 0.02651 | 3715.99 | 1.4534 | 71.4% | 77% | <5% |
| c7i.metal-48xl | 192 | 384 | 50000 Megabit | 10.1808 | 0.05302 | 0.02651 | 7431.98 | 2.2525 | 77.9% | 83% | <5% |
| c8i.2xlarge | 8 | 16 | Up to 15000 Megabit | 0.4454 | 0.05568 | 0.02784 | 325.14 | 0.1403 | 68.5% | 64% | <5% |
| c8i.4xlarge | 16 | 32 | Up to 15000 Megabit | 0.8908 | 0.05568 | 0.02784 | 650.28 | 0.3472 | 61.0% | 65% | 5-10% |
| c8i.8xlarge | 32 | 64 | 15000 Megabit | 1.7816 | 0.05568 | 0.02784 | 1300.57 | 0.5020 | 71.8% | 57% | 5-10% |
| c8i.metal-48xl | 192 | 384 | 75000 Megabit | 10.6896 | 0.05568 | 0.02784 | 7803.41 | 1.0690 | 90.0% | 78% | <5% |
| c8i.metal-96xl | 384 | 768 | 100000 Megabit | 21.3792 | 0.05568 | 0.02784 | 15606.82 | 2.1379 | 90.0% | 77% | <5% |
| m7i.2xlarge | 8 | 32 | Up to 12500 Megabit | 0.4704 | 0.05880 | 0.01470 | 343.39 | 0.1902 | 59.6% | 59% | 15-20% |
| m7i.4xlarge | 16 | 64 | Up to 12500 Megabit | 0.9408 | 0.05880 | 0.01470 | 686.78 | 0.3583 | 61.9% | 60% | 5-10% |
| m7i.8xlarge | 32 | 128 | 12500 Megabit | 1.8816 | 0.05880 | 0.01470 | 1373.57 | 0.6485 | 65.5% | 57% | <5% |
| m7i.metal-24xl | 96 | 384 | 37500 Megabit | 5.6448 | 0.05880 | 0.01470 | 4120.70 | 1.7476 | 69.0% | 59% | <5% |
| m7i.metal-48xl | 192 | 768 | 50000 Megabit | 11.2896 | 0.05880 | 0.01470 | 8241.41 | 1.9485 | 82.7% | 75% | <5% |
| m8i.2xlarge | 8 | 32 | Up to 15000 Megabit | 0.4939 | 0.06174 | 0.01544 | 360.56 | 0.1920 | 61.1% | 62% | <5% |
| m8i.4xlarge | 16 | 64 | Up to 15000 Megabit | 0.9878 | 0.06174 | 0.01544 | 721.12 | 0.3050 | 69.1% | 61% | <5% |
| m8i.8xlarge | 32 | 128 | 15000 Megabit | 1.9757 | 0.06174 | 0.01544 | 1442.25 | 0.6230 | 68.5% | 58% | <5% |
| m8i.metal-48xl | 192 | 768 | 75000 Megabit | 11.8541 | 0.06174 | 0.01543 | 8653.48 | 2.3094 | 80.5% | 73% | <5% |
| m8i.metal-96xl | 384 | 1536 | 100000 Megabit | 23.7082 | 0.06174 | 0.01543 | 17306.96 | 2.3708 | 90.0% | 72% | <5% |
| r8i.2xlarge | 8 | 64 | Up to 15000 Megabit | 0.6527 | 0.08159 | 0.01020 | 476.46 | 0.2194 | 66.4% | 63% | <5% |
| r8i.4xlarge | 16 | 128 | Up to 15000 Megabit | 1.3054 | 0.08159 | 0.01020 | 952.91 | 0.4027 | 69.2% | 65% | <5% |
| r8i.8xlarge | 32 | 256 | 15000 Megabit | 2.6107 | 0.08159 | 0.01020 | 1905.83 | 0.7755 | 70.3% | 57% | <5% |
| r8i.metal-48xl | 192 | 1536 | 75000 Megabit | 15.6643 | 0.08159 | 0.01020 | 11434.95 | 5.4241 | 65.4% | 81% | <5% |
| r8i.metal-96xl | 384 | 3072 | 100000 Megabit | 31.3286 | 0.08159 | 0.01020 | 22869.91 | 3.1329 | 90.0% | 80% | <5% |

#### eu-west-3: Savings Plans effective hourly rates, No Upfront (discount vs on-demand)

Sources: [SP offer eu-west-3](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-3/index.json), [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json).

| Type | OD $/h | Compute SP 1 yr | Compute SP 3 yr | EC2 Instance SP 1 yr | EC2 Instance SP 3 yr | CSP 3 yr $/vCPU-h | CSP 3 yr $/mo (730 h) |
|---|---|---|---|---|---|---|---|
| t4g.medium | 0.0376 | 0.0306 (−18.6%) | 0.0220 (−41.5%) | 0.0237 (−37.0%) | 0.0162 (−56.9%) | 0.01100 | 16.06 |
| t4g.large | 0.0752 | 0.0612 (−18.6%) | 0.0440 (−41.5%) | 0.0474 (−37.0%) | 0.0324 (−56.9%) | 0.02200 | 32.12 |
| t4g.xlarge | 0.1504 | 0.1223 (−18.7%) | 0.0881 (−41.4%) | 0.0949 (−36.9%) | 0.0648 (−56.9%) | 0.02202 | 64.31 |
| t4g.2xlarge | 0.3008 | 0.2447 (−18.7%) | 0.1761 (−41.5%) | 0.1898 (−36.9%) | 0.1297 (−56.9%) | 0.02201 | 128.55 |
| c7g.2xlarge | 0.3434 | 0.2716 (−20.9%) | 0.1923 (−44.0%) | 0.2267 (−34.0%) | 0.1561 (−54.5%) | 0.02404 | 140.38 |
| c7g.4xlarge | 0.6869 | 0.5431 (−20.9%) | 0.3847 (−44.0%) | 0.4534 (−34.0%) | 0.3121 (−54.6%) | 0.02404 | 280.83 |
| c7g.8xlarge | 1.3738 | 1.0863 (−20.9%) | 0.7693 (−44.0%) | 0.9068 (−34.0%) | 0.6242 (−54.6%) | 0.02404 | 561.59 |
| c7g.metal | 2.7475 | 2.1726 (−20.9%) | 1.5386 (−44.0%) | 1.8136 (−34.0%) | 1.2485 (−54.6%) | 0.02404 | 1123.18 |
| c8g.2xlarge | 0.3778 | 0.2987 (−20.9%) | 0.2115 (−44.0%) | 0.2493 (−34.0%) | 0.1716 (−54.6%) | 0.02644 | 154.43 |
| c8g.4xlarge | 0.7555 | 0.5974 (−20.9%) | 0.4231 (−44.0%) | 0.4987 (−34.0%) | 0.3433 (−54.6%) | 0.02644 | 308.86 |
| c8g.8xlarge | 1.5110 | 1.1948 (−20.9%) | 0.8462 (−44.0%) | 0.9974 (−34.0%) | 0.6866 (−54.6%) | 0.02644 | 617.71 |
| c8g.metal-24xl | 4.5331 | 3.5845 (−20.9%) | 2.5385 (−44.0%) | 2.9922 (−34.0%) | 2.0598 (−54.6%) | 0.02644 | 1853.14 |
| c8g.metal-48xl | 9.0662 | 7.1691 (−20.9%) | 5.0771 (−44.0%) | 5.9844 (−34.0%) | 4.1197 (−54.6%) | 0.02644 | 3706.29 |
| c8gn.2xlarge | 0.5618 | 0.4428 (−21.2%) | 0.2702 (−51.9%) | 0.3539 (−37.0%) | 0.2350 (−58.2%) | 0.03377 | 197.25 |
| c8gn.4xlarge | 1.1235 | 0.8856 (−21.2%) | 0.5404 (−51.9%) | 0.7078 (−37.0%) | 0.4701 (−58.2%) | 0.03377 | 394.49 |
| c8gn.8xlarge | 2.2470 | 1.7712 (−21.2%) | 1.0809 (−51.9%) | 1.4156 (−37.0%) | 0.9401 (−58.2%) | 0.03378 | 789.06 |
| m7g.2xlarge | 0.3808 | 0.2888 (−24.2%) | 0.2001 (−47.5%) | 0.2512 (−34.0%) | 0.1735 (−54.4%) | 0.02501 | 146.07 |
| m7g.4xlarge | 0.7616 | 0.5776 (−24.2%) | 0.4002 (−47.5%) | 0.5023 (−34.0%) | 0.3470 (−54.4%) | 0.02501 | 292.15 |
| m7g.8xlarge | 1.5232 | 1.1552 (−24.2%) | 0.8004 (−47.5%) | 1.0046 (−34.0%) | 0.6940 (−54.4%) | 0.02501 | 584.29 |
| m7g.metal | 3.0464 | 2.3105 (−24.2%) | 1.6007 (−47.5%) | 2.0092 (−34.0%) | 1.3880 (−54.4%) | 0.02501 | 1168.51 |
| m8g.2xlarge | 0.4189 | 0.3177 (−24.2%) | 0.2201 (−47.5%) | 0.2763 (−34.0%) | 0.1908 (−54.4%) | 0.02751 | 160.67 |
| m8g.4xlarge | 0.8378 | 0.6354 (−24.2%) | 0.4402 (−47.5%) | 0.5525 (−34.0%) | 0.3817 (−54.4%) | 0.02751 | 321.34 |
| m8g.8xlarge | 1.6755 | 1.2708 (−24.2%) | 0.8804 (−47.5%) | 1.1051 (−34.0%) | 0.7634 (−54.4%) | 0.02751 | 642.68 |
| m8g.metal-24xl | 5.0266 | 3.8123 (−24.2%) | 2.6412 (−47.5%) | 3.3152 (−34.0%) | 2.2902 (−54.4%) | 0.02751 | 1928.05 |
| m8g.metal-48xl | 10.0531 | 7.6246 (−24.2%) | 5.2823 (−47.5%) | 6.6304 (−34.0%) | 4.5805 (−54.4%) | 0.02751 | 3856.10 |
| r7g.2xlarge | 0.5032 | 0.3641 (−27.6%) | 0.2625 (−47.8%) | 0.3336 (−33.7%) | 0.2275 (−54.8%) | 0.03281 | 191.62 |
| r7g.4xlarge | 1.0064 | 0.7283 (−27.6%) | 0.5251 (−47.8%) | 0.6672 (−33.7%) | 0.4550 (−54.8%) | 0.03282 | 383.32 |
| r7g.8xlarge | 2.0128 | 1.4566 (−27.6%) | 1.0502 (−47.8%) | 1.3345 (−33.7%) | 0.9099 (−54.8%) | 0.03282 | 766.65 |
| r7g.metal | 4.0256 | 2.9131 (−27.6%) | 2.1004 (−47.8%) | 2.6689 (−33.7%) | 1.8198 (−54.8%) | 0.03282 | 1533.29 |
| r8g.2xlarge | 0.5535 | 0.4006 (−27.6%) | 0.2888 (−47.8%) | 0.3670 (−33.7%) | 0.2502 (−54.8%) | 0.03610 | 210.82 |
| r8g.4xlarge | 1.1070 | 0.8011 (−27.6%) | 0.5776 (−47.8%) | 0.7340 (−33.7%) | 0.5005 (−54.8%) | 0.03610 | 421.66 |
| r8g.8xlarge | 2.2141 | 1.6022 (−27.6%) | 1.1552 (−47.8%) | 1.4679 (−33.7%) | 1.0009 (−54.8%) | 0.03610 | 843.30 |
| r8g.metal-24xl | 6.6422 | 4.8067 (−27.6%) | 3.4656 (−47.8%) | 4.4037 (−33.7%) | 3.0027 (−54.8%) | 0.03610 | 2529.91 |
| r8g.metal-48xl | 13.2845 | 9.6133 (−27.6%) | 6.9313 (−47.8%) | 8.8075 (−33.7%) | 6.0055 (−54.8%) | 0.03610 | 5059.83 |
| c7i.2xlarge | 0.4242 | 0.3232 (−23.8%) | 0.2214 (−47.8%) | 0.2800 (−34.0%) | 0.1928 (−54.6%) | 0.02767 | 161.59 |
| c7i.4xlarge | 0.8484 | 0.6465 (−23.8%) | 0.4427 (−47.8%) | 0.5601 (−34.0%) | 0.3856 (−54.6%) | 0.02767 | 323.19 |
| c7i.8xlarge | 1.6968 | 1.2930 (−23.8%) | 0.8854 (−47.8%) | 1.1201 (−34.0%) | 0.7711 (−54.6%) | 0.02767 | 646.36 |
| c7i.metal-24xl | 5.0904 | 3.8790 (−23.8%) | 2.6563 (−47.8%) | 3.3604 (−34.0%) | 2.3133 (−54.6%) | 0.02767 | 1939.09 |
| c7i.metal-48xl | 10.1808 | 7.7580 (−23.8%) | 5.3126 (−47.8%) | 6.7208 (−34.0%) | 4.6267 (−54.6%) | 0.02767 | 3878.18 |
| c8i.2xlarge | 0.4454 | 0.3394 (−23.8%) | 0.2324 (−47.8%) | 0.2940 (−34.0%) | 0.2024 (−54.6%) | 0.02905 | 169.67 |
| c8i.4xlarge | 0.8908 | 0.6788 (−23.8%) | 0.4648 (−47.8%) | 0.5881 (−34.0%) | 0.4048 (−54.6%) | 0.02905 | 339.33 |
| c8i.8xlarge | 1.7816 | 1.3576 (−23.8%) | 0.9297 (−47.8%) | 1.1761 (−34.0%) | 0.8096 (−54.6%) | 0.02905 | 678.67 |
| c8i.metal-48xl | 10.6896 | 8.1458 (−23.8%) | 5.5781 (−47.8%) | 7.0567 (−34.0%) | 4.8579 (−54.6%) | 0.02905 | 4072.01 |
| c8i.metal-96xl | 21.3792 | 16.2915 (−23.8%) | 11.1562 (−47.8%) | 14.1134 (−34.0%) | 9.7158 (−54.6%) | 0.02905 | 8144.00 |
| m7i.2xlarge | 0.4704 | 0.3568 (−24.2%) | 0.2472 (−47.5%) | 0.3103 (−34.0%) | 0.2143 (−54.4%) | 0.03090 | 180.43 |
| m7i.4xlarge | 0.9408 | 0.7135 (−24.2%) | 0.4944 (−47.5%) | 0.6205 (−34.0%) | 0.4287 (−54.4%) | 0.03090 | 360.88 |
| m7i.8xlarge | 1.8816 | 1.4270 (−24.2%) | 0.9887 (−47.5%) | 1.2410 (−34.0%) | 0.8573 (−54.4%) | 0.03090 | 721.74 |
| m7i.metal-24xl | 5.6448 | 4.2810 (−24.2%) | 2.9661 (−47.5%) | 3.7229 (−34.0%) | 2.5719 (−54.4%) | 0.03090 | 2165.23 |
| m7i.metal-48xl | 11.2896 | 8.5620 (−24.2%) | 5.9322 (−47.5%) | 7.4459 (−34.0%) | 5.1439 (−54.4%) | 0.03090 | 4330.47 |
| m8i.2xlarge | 0.4939 | 0.3746 (−24.2%) | 0.2595 (−47.5%) | 0.3258 (−34.0%) | 0.2250 (−54.4%) | 0.03244 | 189.46 |
| m8i.4xlarge | 0.9878 | 0.7492 (−24.2%) | 0.5191 (−47.5%) | 0.6515 (−34.0%) | 0.4501 (−54.4%) | 0.03244 | 378.91 |
| m8i.8xlarge | 1.9757 | 1.4983 (−24.2%) | 1.0381 (−47.5%) | 1.3030 (−34.0%) | 0.9002 (−54.4%) | 0.03244 | 757.83 |
| m8i.metal-48xl | 11.8541 | 8.9901 (−24.2%) | 6.2287 (−47.5%) | 7.8182 (−34.0%) | 5.4011 (−54.4%) | 0.03244 | 4546.99 |
| m8i.metal-96xl | 23.7082 | 17.9801 (−24.2%) | 12.4575 (−47.5%) | 15.6364 (−34.0%) | 10.8022 (−54.4%) | 0.03244 | 9093.98 |
| r8i.2xlarge | 0.6527 | 0.4723 (−27.6%) | 0.3405 (−47.8%) | 0.4327 (−33.7%) | 0.2950 (−54.8%) | 0.04257 | 248.59 |
| r8i.4xlarge | 1.3054 | 0.9446 (−27.6%) | 0.6811 (−47.8%) | 0.8654 (−33.7%) | 0.5901 (−54.8%) | 0.04257 | 497.19 |
| r8i.8xlarge | 2.6107 | 1.8893 (−27.6%) | 1.3622 (−47.8%) | 1.7309 (−33.7%) | 1.1802 (−54.8%) | 0.04257 | 994.38 |
| r8i.metal-48xl | 15.6643 | 11.3355 (−27.6%) | 8.1730 (−47.8%) | 10.3853 (−33.7%) | 7.0813 (−54.8%) | 0.04257 | 5966.29 |
| r8i.metal-96xl | 31.3286 | 22.6710 (−27.6%) | 16.3460 (−47.8%) | 20.7706 (−33.7%) | 14.1626 (−54.8%) | 0.04257 | 11932.57 |

### Inferences
- Within each family the on-demand rate per vCPU is identical from 2xlarge to metal: for example, c8g is $0.03988 per vCPU-hour at both 2xlarge and metal-48xl. Bare-metal hosts for Firecracker microVMs therefore carry no list-price premium per vCPU. What differs is the Spot price and Advisor band, which vary a lot by size: c8g.4xlarge falls in the >20% interruption band, c8g.metal-48xl in <5%.
- The Spot values at exactly 10% of on-demand look like a floor or placeholder rather than a clearing price. The Advisor's 30-day savings for the same types (70-73% in the us-east-1 examples) is the safer planning figure.
- The 3-year EC2 Instance SP beats the 3-year Compute SP by 4.3-6.8 points in us-east-1 (for example 56.1% vs 51.6% on c7g/c8g) and by 5.8-15.5 points in the EU, where t4g shows the largest gap. The price is locking the instance family and region (computed). In the EU the Compute SP 1-year discount is noticeably weaker than in us-east-1: 17.9% (eu-west-1) and 18.6% (eu-west-3) vs 28.0% on t4g.medium.
- Newer generations cost more per vCPU at list price (c9g +9.0% over c8g, which is +10.0% over c7g). Any cost model should therefore use per-family rates, not a single Graviton rate.

### Gaps
- Spot is a single feed value per type and region with no AZ or per-row timestamp. Per-AZ history from the EC2 DescribeSpotPriceHistory API needs AWS credentials and was not queried.
- The Spot Advisor gives no interruption band for Graviton5 c9g/m9g/r9g, so their Spot risk is unknown.
- Only No Upfront Savings Plans are tabulated, as requested. Partial and All Upfront plans and Reserved Instances are in the same files but were not extracted.
- Network figures are AWS labels such as "Up to 15 Gigabit". The price list does not give baseline-vs-burst bandwidth.
- The general-availability dates of Graviton5 (c9g/m9g/r9g) and of the 8th-generation AMD and Intel families could not be checked once the search budget ran out. Their presence in the price list is confirmed; their launch dates are not.

## 2. AWS Fargate: per vCPU-hour and per GB-hour (x86 vs ARM), Fargate Spot, ephemeral storage

### Takeaway
Fargate Linux costs $0.04048 per vCPU-hour and $0.004445 per GB-hour on x86, and $0.03238 / $0.00356 on ARM (20% less), identically in us-east-1 and eu-west-1. Paris charges $0.0486 / $0.0053 (x86) and $0.03888 / $0.00424 (ARM). A 1 vCPU / 2 GB task costs $36.04 (x86) or $28.84 (ARM) per 730-hour month in us-east-1. Fargate Spot is advertised at "up to 70%" off and is absent from the price list. Compute Savings Plans take 20.0% (1 yr) and 45.0% (3 yr) off x86 in us-east-1, 15.0%/40.0% in the EU, and about 21.2%/46.0% off ARM everywhere.

### Cited Findings
- us-east-1 on-demand: x86 vCPU $0.04048/h and memory $0.004445/GB-h; ARM vCPU $0.03238/h and memory $0.00356/GB-h; ephemeral storage $0.000111/GB-h. ECS offer version 20260911124425, published 2026-09-11: [ECS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/us-east-1/index.json).
- eu-west-1 has the same x86 and ARM rates as us-east-1; ephemeral storage is $0.000122/GB-h: [ECS offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/eu-west-1/index.json).
- eu-west-3: x86 $0.0486/vCPU-h and $0.0053/GB-h; ARM $0.03888/vCPU-h and $0.00424/GB-h; ephemeral storage $0.000129/GB-h: [ECS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/eu-west-3/index.json).
- The pricing page gives matching per-second rates: x86 $0.000011244 per vCPU-second and $0.000001235 per GB-second; ARM $0.0000089944 and $0.0000009889. Linux tasks are billed per second with a 1-minute minimum. "20 GB of ephemeral storage is available for all Fargate Tasks and Pods by default", and extra storage costs $0.0000000308 per GB-second in us-east-1: [Fargate pricing](https://aws.amazon.com/fargate/pricing/).
- Fargate Spot runs tasks "at up to a 70% discount off the regular Fargate price". The page's Spot rate table was not populated in the fetched copy, and the offer files contain no Fargate Spot SKU: [Fargate pricing](https://aws.amazon.com/fargate/pricing/), [ECS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/us-east-1/index.json).
- Compute Savings Plans for Fargate (No Upfront):
  - us-east-1 x86: 1 yr $0.032384/vCPU-h and $0.003556/GB-h (−20.0%); 3 yr $0.022264 and $0.00244475 (−45.0%).
  - us-east-1 ARM: 1 yr $0.0255 and $0.0028 (−21.2% / −21.3%); 3 yr $0.01749 and $0.00192 (−46.0% / −46.1%).
  - eu-west-1 x86: 1 yr $0.034408 and $0.00377825 (−15.0%); 3 yr $0.024288 and $0.002667 (−40.0%). ARM is the same as us-east-1.
  - eu-west-3 x86: 1 yr $0.04131 and $0.004505 (−15.0%); 3 yr $0.02916 and $0.00318 (−40.0%). ARM: 1 yr $0.03062 and $0.00334 (−21.2%); 3 yr $0.021 and $0.00229 (−46.0%).
  - Sources: [SP offer us-east-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/us-east-1/index.json), [SP offer eu-west-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-1/index.json), [SP offer eu-west-3](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-3/index.json).
- Derived monthly cost (730 h, computed) for a 1 vCPU + 2 GB task: us-east-1 and eu-west-1 x86 $0.04937/h = $36.04, ARM $0.0395/h = $28.84; eu-west-3 x86 $0.0592/h = $43.22, ARM $0.04736/h = $34.57. One extra GB of ephemeral storage for a month costs $0.0810 (us-east-1), $0.0891 (eu-west-1) or $0.0942 (eu-west-3). Source rates: [ECS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/us-east-1/index.json), [ECS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/eu-west-3/index.json).
- EKS on Fargate is billed at the same Fargate rates; the EKS offer repeats the USE1-Fargate rows at identical prices: [EKS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/us-east-1/index.json).

### Inferences
- At on-demand list, a Fargate ARM task with 1 vCPU and 2 GB ($0.0395/h) costs 1.0% less per vCPU than a c8g instance ($0.03988 per vCPU-hour, also with 2 GiB per vCPU) and 9.0% more than c7g ($0.03625). With x86, a 1 vCPU / 2 GB task costs 10.6% more than a c7i vCPU ($0.04937 vs $0.04462) (computed). Fargate's premium over EC2 is therefore small per resource-hour. Its real cost lies in bin-packing and the absence of Spot Advisor-style data.
- Because Fargate x86 has the same list price in us-east-1 and eu-west-1 while EC2 is 6.9-12.9% dearer in Ireland, the Fargate-vs-EC2 gap is narrower in eu-west-1.

### Gaps
- No dated Fargate Spot hourly rate could be recorded: Spot rates are dynamic, absent from the price list, and the page's table was empty in the fetched copy. Fargate Spot interruption frequency is not published.

## 3. AWS Lambda (including Managed Instances, SnapStart, Provisioned Concurrency, MicroVMs) and ECS/EKS fees

### Takeaway
Lambda costs $0.0000166667 per GB-second on x86 and $0.0000133334 on Arm, plus $0.20 per million requests, identically in all three regions. Volume tiers bring the rate down to $0.0000133334 (x86, above 15 billion GB-s per month) and $0.0000106667 (Arm, above 18.75 billion). Lambda Managed Instances is launched and verified: it bills the EC2 price plus a 15% management fee plus $0.20 per million requests. A new product, "AWS Lambda MicroVMs" (snapshot suspend and resume), bills $0.0000326557 per vCPU-second (x86) or $0.0000276944 (Arm) plus memory and snapshot charges. ECS orchestration is free, with Managed Instances adding a fee of about 12%. EKS costs $0.10 per cluster-hour ($0.60 on extended support), Auto Mode adds 12% of the instance price, and Provisioned Control Plane tiers cost $1.65-$13.90 per hour.

### Cited Findings
- **Duration tiers (all three regions).**
  - x86: $0.0000166667 per GB-second for the first 6,000,000,000 GB-s per month, $0.0000150000 for 6-15 billion, $0.0000133334 above 15 billion.
  - Arm: $0.0000133334 for the first 7.5 billion, $0.0000120001 for 7.5-18.75 billion, $0.0000106667 above 18.75 billion.
  - Lambda offer version 20261009223122, published 2026-10-09T22:31:22Z: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [Lambda offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-1/index.json), [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json).
  - Tiers apply to aggregate monthly duration per architecture, region and account, or across an AWS Organizations consolidated bill: [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
- **Requests and free tier.** Requests cost $0.20 per million ($0.0000002 each) for x86 and Arm in all three regions. The free tier is 1,000,000 requests and 400,000 GB-seconds per month and does not apply to Provisioned Concurrency: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
- **Derived (computed).** $0.0600 per GB-hour on x86 and $0.0480 on Arm, at first-tier rates. One million invocations at 1 GB for 1 s cost $16.87 on x86 or $13.53 on Arm, including request charges.
- **Provisioned Concurrency.**
  - us-east-1: $0.0000041667 per GB-second provisioned (x86), $0.0000033334 (Arm); duration while provisioned $0.0000097222 (x86), $0.0000077778 (Arm).
  - eu-west-1: $0.0000046460 / $0.0000037168 provisioned; $0.0000108407 / $0.0000086726 duration.
  - eu-west-3: $0.0000048673 / $0.0000038938 provisioned; $0.0000113569 / $0.0000090855 duration.
  - Sources: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [Lambda offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-1/index.json), [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json). Provisioned capacity is billed from enablement to disablement, rounded up to five minutes: [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
- **SnapStart.** The snapshot cache costs $0.0000015046 per GB-second, with a 3-hour minimum per active function version, and restores cost $0.0001397998 per GB, in us-east-1 and eu-west-1. Paris charges $0.0000022468 and $0.0002052964: [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json), [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
- **Ephemeral storage.** 512 MB per function is free. Beyond that it costs $0.0000000309 per GB-second (us-east-1), $0.0000000340 (eu-west-1) or $0.0000000358 (eu-west-3): [Lambda pricing](https://aws.amazon.com/lambda/pricing/), [Lambda offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-1/index.json), [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json).
- **Lambda Managed Instances (verified).**
  - Charges: EC2 instance charges at standard EC2 pricing, reducible with Savings Plans or Reserved Instances, plus a "Compute management fee: 15% on top of the EC2 on-demand price", plus $0.20 per million requests. Duration is not billed per request: [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
  - Price-list fee rows in us-east-1: c7g.2xlarge $0.0435/h, c8g.2xlarge $0.04785/h, m7g.2xlarge $0.04896/h. Request rows are $0.0000002 in all three regions: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json).
- **AWS Lambda MicroVMs (new).**
  - Definition: "a managed, snapshot-based runtime" in which an image is a snapshot of pre-initialized state. Idle MicroVMs can be suspended for up to 8 hours. Memory and vCPU follow a 2:1 ratio (default 2 GB / 1 vCPU, scalable to 4x): [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
  - us-east-1 price list: x86 $0.0000326557 per vCPU-second and $0.0000043235 per GB-second; Arm $0.0000276944 and $0.0000036667. Snapshot write costs $0.0037977138/GB, read $0.0015467699/GB, and storage $0.0001111111 per GB-hour. The page states $0.08 per GB-month with a 1-week minimum retention for images: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
  - eu-west-1: x86 $0.0000340538 per vCPU-second and $0.0000045086 per GB-second; Arm $0.0000291572 and $0.0000038603; snapshot storage $0.0001322222/GB-h, read $0.0016403088/GB, write $0.0040636422/GB. There are no MicroVM rows in eu-west-3: [Lambda offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-1/index.json), [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json).
  - Derived (computed): x86 $0.117561 per vCPU-hour and $0.015565 per GB-hour; Arm $0.0997 and $0.0132. A default 1 vCPU / 2 GB MicroVM costs $0.1261/h ($92.05 per 730-h month) on Arm and $0.14869/h ($108.54) on x86.
- **"Lambda Web functions" (price list only).** Arm-only rows: requests $0.0000003 each ($0.30 per million), $0.0000276944 per vCPU-second and $0.0000036667 per GB-second in us-east-1. eu-west-1 charges $0.0000291572 / $0.0000038603 and eu-west-3 $0.0000401210 / $0.0000053119. The pricing page does not mention this product: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json).
- **Durable Functions.** $8.00 per million operations, $0.25 per GB written and $0.15 per GB-month retained in us-east-1. eu-west-1 charges $0.0000086 per operation, $0.26 and $0.16; eu-west-3 $0.0000125, $0.39 and $0.23: [Lambda pricing](https://aws.amazon.com/lambda/pricing/), [Lambda offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-1/index.json), [Lambda offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/eu-west-3/index.json).
- **Tenant isolation mode.** $0.000166667 per GB of memory per new tenant-isolated execution environment on x86, $0.000133333 on Arm, in all three regions: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [Lambda pricing](https://aws.amazon.com/lambda/pricing/).
- **Other new Lambda SKUs.** Provisioned-mode event source mappings cost $0.185 per Event-Poller-Unit-hour, or $0.00925 for SQS. Response streaming costs $0.008 per GB processed, with the first 100 GB free: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json).
- **Compute Savings Plans for Lambda (1-yr and 3-yr rates are identical).** us-east-1: x86 duration $0.0000147 (−11.8%), Arm $0.0000117 (−12.3%), Provisioned Concurrency $0.00000367 (−11.9%), Provisioned Concurrency duration $0.00000856 (−12.0%). eu-west-1 has the same duration rates, with $0.00000409 and $0.00000954 for Provisioned Concurrency. eu-west-3: $0.000015 (−10.0%) on x86 and $0.0000120001 (−10.0%) on Arm. Requests are not discounted: [SP offer us-east-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/us-east-1/index.json), [SP offer eu-west-1](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-1/index.json), [SP offer eu-west-3](https://pricing.us-east-1.amazonaws.com/savingsPlan/v1.0/aws/AWSComputeSavingsPlan/20261010020024/eu-west-3/index.json).
- **ECS.**
  - "There is no additional charge for Amazon ECS orchestration" on EC2 or Fargate: [ECS pricing](https://aws.amazon.com/ecs/pricing/).
  - ECS Managed Instances charges the EC2 price plus a "per-instance management fee that applies regardless of your EC2 purchase option", billed per second with a 1-minute minimum: [ECS pricing](https://aws.amazon.com/ecs/pricing/). The fee is 12% of on-demand in the price list (see section 1): [ECS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECS/20260911124425/us-east-1/index.json).
  - ECS Anywhere costs $0.01025 per registered instance-hour, $7.48 per 730-h month: [ECS pricing](https://aws.amazon.com/ecs/pricing/).
- **EKS.**
  - Cluster fee: standard support $0.10 per cluster-hour; extended support $0.60, the standard rate plus $0.50: [EKS pricing](https://aws.amazon.com/eks/pricing/). The price list carries the separate $0.10 and $0.50 line items in all three regions. EKS offer version 20260928194255, published 2026-09-28T19:42:55Z: [EKS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/us-east-1/index.json), [EKS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/eu-west-3/index.json).
  - Provisioned Control Plane, added on top of the cluster fee: XL $1.65/h, 2XL $3.40, 4XL $6.90, 8XL $13.90, identical in all three regions: [EKS pricing](https://aws.amazon.com/eks/pricing/), [EKS offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/eu-west-1/index.json).
  - The Auto Mode fee "varies by EC2 instance type", is billed per second with a 1-minute minimum, and works out to about 12% in the page's example: [EKS pricing](https://aws.amazon.com/eks/pricing/). The price list gives exactly 12.0% on every tabulated type: [EKS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/us-east-1/index.json).
  - Hybrid Nodes cost $0.020 per vCPU-hour for the first 576,000 vCPU-hours a month, falling to $0.014, $0.010, $0.008 and $0.006 above 11,520,000: [EKS pricing](https://aws.amazon.com/eks/pricing/).
  - Managed EKS Capabilities in us-east-1: Argo CD $0.03 per capability-hour plus $0.0015 per application-hour; ACK and KRO $0.005 per hour plus $0.00005 per resource-hour: [EKS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEKS/20260928194255/us-east-1/index.json).
  - Derived monthly cost (730 h, computed): EKS standard $73.00 per cluster, extended $438.00; Provisioned Control Plane XL $1,204.50, 2XL $2,482.00, 4XL $5,037.00, 8XL $10,147.00.

### Inferences
- At full utilization, Lambda Managed Instances on m7g.2xlarge (32 GiB) costs $0.3264 + $0.04896 = $0.37536/h, about $0.0117 per GiB-hour. Standard Lambda on x86 costs $0.06 per GB-hour, so standard Lambda is about 5x more per memory-hour. The two converge only at low utilization.
- A default Arm MicroVM (1 vCPU / 2 GB, $0.1261/h) costs about 3.2x a Fargate ARM task of the same size ($0.0395/h) per running hour. Suspend and resume (snapshot storage $0.08 per GB-month) changes the comparison for bursty, session-like workloads such as hosted browsers.
- Savings Plans give Lambda only 10.0-12.3% off, far below the 40-46% available on Fargate and the 41.4-58.2% on EC2 (3-year Compute SP, across the three regions; computed).

### Gaps
- "Lambda Web functions" appears only in the price list. Its product name, behaviour and launch status could not be verified, so it is unverified.
- No launch date was confirmed for Lambda MicroVMs, Durable Functions, tenant isolation mode or Lambda Managed Instances. The pricing page confirms that each is offered; the search budget ran out before launch dates could be checked.
- The ECS Managed Instances 24% outlier (us-east-1 m8a.metal-24xl) could not be explained. It may be a price-list error.

## 4. Storage: S3 (Standard, Intelligent-Tiering, Standard-IA, Express One Zone), EBS gp3, EFS, ECR

### Takeaway
S3 Standard costs $0.023 per GB-month for the first 50 TB in us-east-1 and eu-west-1 ($0.024 in Paris), plus $0.005 per 1,000 PUT/LIST and $0.0004 per 1,000 GET. Express One Zone costs $0.11 per GB-month, $0.00113 per 1,000 PUTs, $0.00003 per 1,000 GETs, $0.0032 per GB uploaded and $0.0006 per GB retrieved. EBS gp3 costs $0.08 per GB-month in us-east-1 ($0.088 in Ireland, $0.0928 in Paris), with 3,000 IOPS and 125 MB/s included. EFS Standard costs $0.30 ($0.33 in the EU) and ECR $0.10 per GB-month everywhere.

### Cited Findings
- **S3 Standard.** $0.023 per GB-month for the first 50 TB, $0.022 for the next 450 TB, $0.021 above 500 TB, in us-east-1 and eu-west-1. eu-west-3 charges $0.024 / $0.023 / $0.022. S3 offer version 20260928230416, published 2026-09-28T23:04:16Z: [S3 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/us-east-1/index.json), [S3 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-1/index.json), [S3 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-3/index.json).
- **Requests (Standard and Intelligent-Tiering).** PUT/COPY/POST/LIST cost $0.005 per 1,000; GET and other requests $0.004 per 10,000 ($0.0004 per 1,000), in us-east-1 and eu-west-1. eu-west-3 charges $0.0053 and $0.00042 per 1,000: [S3 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/us-east-1/index.json), [S3 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-3/index.json).
- **Intelligent-Tiering.** Frequent Access is priced like Standard ($0.023 / $0.022 / $0.021). Infrequent Access costs $0.0125 ($0.0131 in Paris), Archive Instant Access $0.004 ($0.005 in Paris), and monitoring $0.0025 per 1,000 objects per month: [S3 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/us-east-1/index.json), [S3 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-3/index.json).
- **Standard-IA.** $0.0125 per GB-month ($0.0131 in Paris), $0.01 per 1,000 PUTs, $0.01 per 10,000 GETs, and $0.01 per GB retrieved: [S3 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/us-east-1/index.json), [S3 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-3/index.json).
- **Express One Zone.**
  - us-east-1 and eu-west-1: storage $0.11 per GB-month; PUT/COPY/POST/LIST $0.00113 per 1,000; GET and others $0.00003 per 1,000; upload $0.0032/GB; retrieval $0.0006/GB.
  - eu-west-3: $0.115; $0.0012 and $0.0000315 per 1,000; $0.00339/GB upload; $0.00063/GB retrieval.
  - Sources: [S3 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/us-east-1/index.json), [S3 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-1/index.json), [S3 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/eu-west-3/index.json).
- **Buckets.** The global S3 file prices the first 2,000 general purpose buckets per account at $0.00: [S3 offer global](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/20260928230416/aws-other/index.json).
- **EBS** (EC2 offer, published 2026-10-09):
  - gp3 storage: $0.08 per GB-month (us-east-1), $0.088 (eu-west-1), $0.0928 (eu-west-3).
  - gp3 IOPS above baseline: $0.005 per IOPS-month, $0.0055, $0.0058.
  - gp3 throughput above baseline: described as $0.04, $0.044 and $0.0464 per MiB/s-month. The rows are actually priced per GiB/s-month at $40.96, $45.056 and $47.5136.
  - gp2: $0.10, $0.11, $0.116 per GB-month. io2: $0.125 per GB-month plus $0.065 per IOPS-month in us-east-1.
  - Snapshots: $0.05, $0.05, $0.053 per GB-month. Snapshot archive: $0.0125, $0.0125, $0.01325.
  - Sources: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json), [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json), [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json).
- **gp3 baseline.** gp3 includes 3,000 IOPS and 125 MB/s "at no extra charge". The EBS page's worked example prices extra throughput at $0.06 per MB/s-month, which contradicts the $0.04 us-east-1 price-list row: [EBS pricing](https://aws.amazon.com/ebs/pricing/) contradicted by [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json).
- **EFS** (offer version 20260911124425):
  - us-east-1: Standard $0.30 per GB-month; Infrequent Access with Elastic Throughput $0.016; Standard-Infrequent Access $0.025; Archive $0.008; One Zone $0.16; One Zone-IA $0.0133.
  - us-east-1 access and throughput: Elastic Throughput reads $0.03/GB and writes $0.06/GB; IA access $0.01/GB; Archive access $0.03/GB; provisioned throughput $6.00 per MiB/s-month.
  - eu-west-1: Standard $0.33; IA (Elastic Throughput) $0.016; Archive $0.008; One Zone $0.176; Elastic Throughput reads $0.03, writes $0.07; provisioned throughput $6.60.
  - eu-west-3: Standard $0.33; IA (Elastic Throughput) $0.02; Archive $0.01; One Zone-IA $0.0139.
  - Sources: [EFS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEFS/20260911124425/us-east-1/index.json), [EFS offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEFS/20260911124425/eu-west-1/index.json), [EFS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEFS/20260911124425/eu-west-3/index.json).
- **ECR.** Storage costs $0.10 per GB-month in all three regions. An archive storage class costs $0.10 per GB-month for the first 153,600 GB and $0.07 beyond, with retrieval at $0.03/GB. Image signing costs "$0.02 per Count": [ECR offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECR/20260911124425/us-east-1/index.json), [ECR offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonECR/20260911124425/eu-west-3/index.json).

### Inferences
- Express One Zone storage costs about 4.8x Standard ($0.11 vs $0.023), but its GET requests are about 13x cheaper ($0.00003 vs $0.0004 per 1,000) and its PUTs about 4.4x cheaper. For request-heavy, small-object scratch data such as crawl frontiers or page snapshots, request cost dominates.
- EBS gp3 throughput: if the $0.06 example on the EBS page reflects a newer us-east-1 rate, provisioned-throughput costs would be 50% higher than the price list implies. The price list (2026-10-09) is the more recent and more specific source.

### Gaps
- Whether Express One Zone's per-GB upload and retrieval fees apply to all bytes, or only to the part of each request above a size threshold, could not be confirmed from the price list.
- The meaning of ECR's "AsyncActions-ImageSigning" unit (per signature or per image) is unclear. The ECR pricing page was not fetched.

## 5. Databases and queues: DynamoDB, Aurora Serverless v2, Aurora DSQL, RDS PostgreSQL, ElastiCache Serverless for Valkey, SQS, Kinesis, Firehose, MSK Serverless

### Takeaway
DynamoDB on-demand, after the 50% cut on 1 Nov 2024, costs $0.625 per million write units and $0.125 per million read units in us-east-1 ($0.705 / $0.1415 in Ireland, $0.7423 / $0.1487 in Paris). Aurora Serverless v2 PostgreSQL costs $0.12 per ACU-hour ($0.16 I/O-Optimized; $0.14 / $0.19 in the EU). Aurora DSQL costs $8 per million DPUs plus $0.33 per GB-month. ElastiCache Serverless for Valkey costs $0.084 per GB-hour plus $0.0023 per million ECPUs. SQS costs $0.40 per million (standard) and $0.50 per million (FIFO). Kinesis on-demand costs $0.04 per stream-hour, $0.08 per GB in and $0.04 per GB out; the new On-demand Advantage mode costs $0.032 / $0.016 per GB with a 25 MB/s account-level minimum. Firehose costs $0.029 per GB and MSK Serverless $0.75 per cluster-hour.

### Cited Findings
- **DynamoDB, Standard table class.**
  - us-east-1: write request units $0.625 per million, read request units $0.125 per million; provisioned WCU $0.00065/h and RCU $0.00013/h; storage $0.25 per GB-month after 25 free GB.
  - eu-west-1: $0.705 per million writes, $0.1415 per million reads, WCU $0.000735/h, RCU $0.000147/h, storage $0.283.
  - eu-west-3: $0.7423 and $0.1487 per million, WCU $0.000772/h, RCU $0.0001544/h, storage $0.29715.
  - DynamoDB offer version 20260911124422: [DynamoDB offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonDynamoDB/20260911124422/us-east-1/index.json), [DynamoDB offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonDynamoDB/20260911124422/eu-west-1/index.json), [DynamoDB offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonDynamoDB/20260911124422/eu-west-3/index.json).
- **DynamoDB, Standard-IA table class (us-east-1).** $0.78 per million writes, $0.155 per million reads, WCU $0.00081/h, RCU $0.00016/h, storage $0.10 per GB-month: [DynamoDB offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonDynamoDB/20260911124422/us-east-1/index.json).
- **DynamoDB free tier.** 25 WCU, 25 RCU, 25 GB-month of storage and 2.5 million Streams read requests: [DynamoDB offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonDynamoDB/20260911124422/us-east-1/index.json).
- **DynamoDB monthly provisioned cost (730 h, computed).** One WCU costs $0.4745/month and one RCU $0.0949 in us-east-1; $0.5365 / $0.1073 in eu-west-1; $0.5636 / $0.1127 in eu-west-3.
- **DynamoDB price cut.** "On-demand throughput pricing has been reduced by 50%", and global-tables replicated writes fell by up to 67% on on-demand tables (33% on provisioned), "starting November 1, 2024": [AWS Database Blog](https://aws.amazon.com/blogs/database/new-amazon-dynamodb-lowers-pricing-for-on-demand-throughput-and-global-tables/).
- **Aurora PostgreSQL.**
  - Serverless v2: $0.12 per ACU-hour (Aurora Standard) and $0.16 (I/O-Optimized) in us-east-1; $0.14 and $0.19 in eu-west-1 and eu-west-3.
  - Storage: Aurora Standard $0.10 per GB-month plus $0.20 per million I/Os in us-east-1 ($0.11 and $0.22 in the EU). I/O-Optimized storage $0.225 per GB-month in us-east-1 ($0.248 in the EU).
  - Backup beyond the free allocation: $0.021 per GB-month ($0.022 in Paris).
  - RDS offer version 20261006224050: [RDS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/us-east-1/index.json), [RDS offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/eu-west-1/index.json), [RDS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/eu-west-3/index.json).
- **Aurora monthly cost per ACU running continuously (730 h, computed).** $87.60 (Standard) and $116.80 (I/O-Optimized) in us-east-1; $102.20 and $138.70 in the EU.
- **Aurora DSQL.** $0.000008 per DPU ($8.00 per million) and $0.33 per GB-month in us-east-1; $0.0000095 per DPU ($9.50 per million) in eu-west-1 and eu-west-3; storage $0.36 (eu-west-1) and $0.45 (eu-west-3): [DSQL offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AuroraDSQL/20260911124522/us-east-1/index.json), [DSQL offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AuroraDSQL/20260911124522/eu-west-1/index.json), [DSQL offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AuroraDSQL/20260911124522/eu-west-3/index.json).
- **RDS for PostgreSQL, small instances.** On-demand $ per hour, Single-AZ / Multi-AZ, with 730-h monthly cost in brackets (computed):

| Instance (vCPU, GiB) | us-east-1 | eu-west-1 | eu-west-3 |
|---|---|---|---|
| db.t4g.micro (2, 1) | 0.016 / 0.032 ($11.68 / $23.36) | 0.017 / 0.035 ($12.41 / $25.55) | 0.018 / 0.036 ($13.14 / $26.28) |
| db.t4g.small (2, 2) | 0.032 / 0.065 ($23.36 / $47.45) | 0.035 / 0.069 ($25.55 / $50.37) | 0.036 / 0.072 ($26.28 / $52.56) |
| db.t4g.medium (2, 4) | 0.065 / 0.129 ($47.45 / $94.17) | 0.069 / 0.138 ($50.37 / $100.74) | 0.072 / 0.145 ($52.56 / $105.85) |
| db.t4g.large (2, 8) | 0.129 / 0.258 ($94.17 / $188.34) | 0.138 / 0.277 ($100.74 / $202.21) | 0.145 / 0.290 ($105.85 / $211.70) |
| db.m7g.large (2, 8) | 0.168 / 0.337 ($122.64 / $246.01) | 0.186 / 0.372 ($135.78 / $271.56) | 0.195 / 0.390 ($142.35 / $284.70) |
| db.m8g.large (2, 8) | 0.168 / 0.336 ($122.64 / $245.28) | 0.186 / 0.372 ($135.78 / $271.56) | 0.195 / 0.390 ($142.35 / $284.70) |
| db.r8g.large (2, 16) | 0.239 / 0.478 ($174.47 / $348.94) | 0.268 / 0.536 ($195.64 / $391.28) | 0.280 / 0.561 ($204.40 / $409.53) |

  Sources: [RDS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/us-east-1/index.json), [RDS offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/eu-west-1/index.json), [RDS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/eu-west-3/index.json).
- **RDS gp3 storage.** Single-AZ: $0.115 per GB-month, $0.02 per IOPS-month and $0.08 per MiB/s-month in us-east-1; $0.127 / $0.022 / $0.088 in eu-west-1; $0.133 / $0.023 / $0.093 in eu-west-3. Multi-AZ doubles the storage rate ($0.23 in us-east-1). Backup beyond the free allocation costs $0.095 per GB-month ($0.10 in Paris): [RDS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/us-east-1/index.json), [RDS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/20261006224050/eu-west-3/index.json).
- **ElastiCache Serverless.** Valkey: $0.084 per GB-hour of data stored and $0.0023 per million ECPUs (us-east-1); $0.094 and $0.0025 (eu-west-1); $0.098 and $0.0027 (eu-west-3). Redis OSS and Memcached: $0.125 per GB-hour and $0.0034 per million ECPUs (us-east-1). Snapshots: $0.085 per GB-month. ElastiCache offer version 20260914063714: [ElastiCache offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonElastiCache/20260914063714/us-east-1/index.json), [ElastiCache offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonElastiCache/20260914063714/eu-west-1/index.json), [ElastiCache offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonElastiCache/20260914063714/eu-west-3/index.json).
- **Valkey Serverless monthly cost per GB stored (730 h, computed).** $61.32 (us-east-1), $68.62 (eu-west-1), $71.54 (eu-west-3).
- **SQS.**
  - Standard queues: $0.40 per million for the first 100 billion requests a month, $0.30 for 100-200 billion, $0.24 above 200 billion. FIFO: $0.50, $0.40 and $0.35. Same in us-east-1 and eu-west-1.
  - eu-west-3: standard $0.40 / $0.36 / $0.32; FIFO $0.50 / $0.48 / $0.44.
  - "Fair queue" requests: $0.10 per million in all three regions.
  - The first 1,000,000 requests each month are free.
  - SQS offer version 20260911124607: [SQS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSQueueService/20260911124607/us-east-1/index.json), [SQS offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSQueueService/20260911124607/eu-west-3/index.json).
- **Kinesis Data Streams, rates by mode.** Columns are us-east-1 / eu-west-1 / eu-west-3. Kinesis offer version 20260911124443: [Kinesis offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesis/20260911124443/us-east-1/index.json), [Kinesis offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesis/20260911124443/eu-west-1/index.json), [Kinesis offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesis/20260911124443/eu-west-3/index.json).

| Mode and item | us-east-1 | eu-west-1 | eu-west-3 |
|---|---|---|---|
| On-demand Standard: stream-hour | $0.04 | $0.045 | $0.048 |
| On-demand Standard: per GB written | $0.08 | $0.091 | $0.095 |
| On-demand Standard: per GB read (GetRecords) | $0.04 | $0.045 | $0.048 |
| On-demand Standard: per GB read (enhanced fan-out) | $0.05 | $0.057 | $0.06 |
| On-demand Standard: retention 1-7 days, per GB-month | $0.10 | $0.11 | $0.116 |
| On-demand Advantage: per GB in | $0.032 | $0.0364 | $0.038 |
| On-demand Advantage: per GB out | $0.016 | $0.018 | $0.0192 |
| On-demand Advantage: retention, per GB-month | $0.023 | $0.023 | $0.024 |
| Provisioned: shard-hour | $0.015 | $0.017 | $0.0179 |
| Provisioned: per million PUT payload units | $0.014 | $0.0165 | $0.0173 |

- **Kinesis, other provisioned-mode charges (us-east-1).** Extended retention $0.020 per shard-hour; enhanced fan-out $0.015 per consumer-shard-hour plus $0.013 per GB; long-term retention $0.023 per GB-month plus $0.021 per GB read: [Kinesis offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesis/20260911124443/us-east-1/index.json).
- **On-demand Advantage terms.** "A 25 MB/s minimum for ingest and for retrieval, measured at the account level across all on-demand streams", with a shortfall charge below that. There is no per-stream fee and no extra charge for enhanced fan-out. The page positions the mode for workloads of at least 10 MB/s aggregate ingest: [Kinesis Data Streams pricing](https://aws.amazon.com/kinesis/data-streams/pricing/).
- **Kinesis monthly fixed cost (730 h, computed).** An on-demand stream costs $29.20 (us-east-1), $32.85 (eu-west-1) or $35.04 (eu-west-3) per month; a provisioned shard costs $10.95, $12.41 or $13.07.
- **Data Firehose.** Direct PUT ingestion costs $0.029 per GB for the first 500 TB (0-512,000 GB) a month, $0.025 for the next 1.5 PB and $0.020 beyond, in us-east-1. eu-west-1 charges $0.031 / $0.027 / $0.022 and eu-west-3 $0.035 / $0.030 / $0.024. The same tiers apply when the source is Kinesis Data Streams. Firehose offer version 20260911124446: [Firehose offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesisFirehose/20260911124446/us-east-1/index.json), [Firehose offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesisFirehose/20260911124446/eu-west-1/index.json), [Firehose offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonKinesisFirehose/20260911124446/eu-west-3/index.json).
- **MSK Serverless.**
  - us-east-1: $0.75 per cluster-hour, $0.0015 per partition-hour, $0.10 per GB-month of storage, $0.10 per GB in, $0.05 per GB out.
  - eu-west-1: $0.8625, $0.001725, $0.115, $0.115, $0.0575.
  - eu-west-3: $0.875, $0.00175, $0.116, $0.117, $0.0583.
  - Cluster-hours alone cost $547.50, $629.63 and $638.75 per 730-h month (computed).
  - Sources: [MSK offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonMSK/20260911124456/us-east-1/index.json), [MSK offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonMSK/20260911124456/eu-west-1/index.json), [MSK offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonMSK/20260911124456/eu-west-3/index.json).

### Inferences
- In us-east-1, a provisioned WCU running all month ($0.4745) buys 2.628 million writes (3,600 × 730). The same writes cost $1.6425 on-demand ($0.625 per million). Provisioned capacity therefore wins at sustained utilization above 28.9% (0.4745 / 1.6425). This is computed from list prices.
- On-demand Advantage cuts Kinesis per-GB rates to 40% of standard on-demand ($0.032 vs $0.08 in; $0.016 vs $0.04 out) but requires 25 MB/s at account level, 65.7 TB (decimal) per 730-hour month (computed). Below that, shortfall charges apply, so the standard on-demand mode, or SQS, is the relevant comparison for small volumes.

### Gaps
- The minimum ACU for Aurora Serverless v2 and its scale-to-zero behaviour, and the ElastiCache Serverless for Valkey minimum data-storage charge, are not in the price list and could not be verified in this session.
- The meaning of the Firehose "DirectPUT-no-rounding" $0.08/GB row is unclear.
- SQS billing units (for example 64 KB chunks per request) were not re-verified against the SQS pricing page.

## 6. Networking: data transfer, NAT Gateway, public IPv4, VPC endpoints, ALB/NLB, API Gateway, CloudFront, Global Accelerator

### Takeaway
Internet egress costs $0.09 per GB after 100 GB free (up to 10 TB), then $0.085, $0.07 and $0.05 per GB, identically in all three regions. Ingress is free, cross-AZ transfer costs $0.01 per GB, and inter-region transfer $0.02 per GB ($0.01 from us-east-1 to us-east-2). NAT Gateway costs $0.045/h plus $0.045/GB in us-east-1 ($0.048 in Ireland, $0.05 in Paris). Public IPv4 costs $0.005/h, and interface endpoints $0.01 per AZ-hour plus $0.01/GB. ALB and NLB cost $0.0225/h plus $0.008 per LCU-hour or $0.006 per NLCU-hour. API Gateway costs $1.00 per million for HTTP APIs, $3.50 per million for REST, and $1.00 per million WebSocket messages plus $0.25 per million connection-minutes. CloudFront costs $0.085/GB (first 10 TB, US and Europe) plus $0.0100 per 10,000 HTTPS requests in the US, or flat-rate plans from $0 to $1,000 a month, with Premium variants up to $10,000. Global Accelerator costs $0.025/h plus $0.015/GB DT-Premium within and between North America and Europe.

### Cited Findings
- **Internet egress and ingress (AWSDataTransfer offer version 20260916132208).** The first 100 GB a month to the internet is free, aggregated globally. After that: $0.090/GB for the first 10 TB, $0.085 for the next 40 TB, $0.070 for the next 100 TB and $0.050 above 150 TB. These rates are identical in us-east-1, eu-west-1 and eu-west-3. Data transfer in costs $0.000/GB: [DataTransfer offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/us-east-1/index.json), [DataTransfer offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/eu-west-1/index.json), [DataTransfer offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/eu-west-3/index.json).
- **Cross-AZ.** "$0.010 per GB - regional data transfer - in/out/between EC2 AZs or using elastic IPs or ELB", in all three regions: [DataTransfer offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/us-east-1/index.json).
- **Inter-region outbound.** us-east-1 to eu-west-1, eu-west-3, eu-central-1, eu-west-2, us-west-2 or ap-northeast-1 costs $0.02/GB; to us-east-2 $0.01/GB. eu-west-1 and eu-west-3 send to every listed region, including each other and us-east-1, at $0.02/GB: [DataTransfer offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/us-east-1/index.json), [DataTransfer offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/eu-west-1/index.json), [DataTransfer offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/eu-west-3/index.json).
- **NAT Gateway** (EC2 offer):
  - $0.045 per hour plus $0.045 per GB processed (us-east-1); $0.048 and $0.048 (eu-west-1); $0.05 and $0.05 (eu-west-3).
  - New SKUs: NAT Gateway with provisioned bandwidth at $1.076 per Gbps-hour (us-east-1) or $1.164 (eu-west-1) with $0.00 per GB processed; and "Regional NAT Gateway" at the standard hourly and per-GB rates.
  - Sources: [EC2 offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/us-east-1/index.json), [EC2 offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-1/index.json), [EC2 offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonEC2/20261009203134/eu-west-3/index.json).
- **Regional NAT Gateway billing.** It "spans multiple Availability Zones" and is charged per hour "in each availability zone": three AZs cost $0.135/h in the page's Ohio example: [VPC pricing](https://aws.amazon.com/vpc/pricing/).
- **Fixed monthly cost (730 h, computed).** One NAT Gateway costs $32.85 (us-east-1), $35.04 (eu-west-1) or $36.50 (eu-west-3). One public IPv4 address costs $3.65. One interface endpoint costs $7.30 per AZ in us-east-1 and $8.03 in the EU.
- **Public IPv4.** $0.005 per hour, in use or idle, in all three regions; addresses in a contiguous IPv4 block cost $0.008 per hour each: [VPC offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonVPC/20260917190528/us-east-1/index.json), [VPC pricing](https://aws.amazon.com/vpc/pricing/).
- **Interface VPC endpoints.** $0.01 per endpoint-hour in us-east-1 ($0.011 in eu-west-1 and eu-west-3). Data processed costs $0.01/GB up to 1 PB a month, $0.006 from 1 to 5 PB and $0.004 above 5 PB. VPC offer version 20260917190528: [VPC offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonVPC/20260917190528/us-east-1/index.json), [VPC offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonVPC/20260917190528/eu-west-1/index.json).
- **Gateway endpoints.** Gateway-type endpoints for S3 and DynamoDB have "no data processing or hourly charges": [VPC pricing](https://aws.amazon.com/vpc/pricing/).
- **Transit Gateway.** $0.05 per attachment-hour ($0.06 in eu-west-3) plus $0.02 per GB processed: [VPC offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonVPC/20260917190528/us-east-1/index.json), [VPC offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonVPC/20260917190528/eu-west-3/index.json).
- **Load balancers** (ELB offer version 20260911124544). Hourly and capacity-unit rates; a load balancer's hourly charge costs $16.43 a month in us-east-1, $18.40 in eu-west-1 and $19.32 in eu-west-3 (730 h, computed). Sources: [ELB offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSELB/20260911124544/us-east-1/index.json), [ELB offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSELB/20260911124544/eu-west-1/index.json), [ELB offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSELB/20260911124544/eu-west-3/index.json).

| Item | us-east-1 | eu-west-1 | eu-west-3 |
|---|---|---|---|
| ALB per hour | $0.0225 | $0.0252 | $0.02646 |
| ALB per LCU-hour | $0.008 | $0.008 | $0.0084 |
| NLB per hour | $0.0225 | $0.0252 | $0.02646 |
| NLB per NLCU-hour | $0.006 | $0.006 | $0.0063 |
| Gateway Load Balancer per hour | $0.0125 | $0.014 | $0.0147 |
| Classic Load Balancer per hour | $0.025 | $0.028 | $0.0294 |

- **API Gateway** (offer version 20261002231501). Prices per million; columns are us-east-1 / eu-west-1 / eu-west-3. Sources: [API Gateway offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonApiGateway/20261002231501/us-east-1/index.json), [API Gateway offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonApiGateway/20261002231501/eu-west-1/index.json), [API Gateway offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonApiGateway/20261002231501/eu-west-3/index.json).

| Item (per million) | us-east-1 | eu-west-1 | eu-west-3 |
|---|---|---|---|
| HTTP API, first 300 M requests | $1.00 | $1.11 | $1.17 |
| HTTP API, above 300 M | $0.90 | $1.00 | $1.05 |
| REST API, first 333 M | $3.50 | $3.50 | $3.50 |
| REST API, next 667 M | $2.80 | $3.19 | $3.33 |
| REST API, next 19 B | $2.38 | $2.71 | $2.83 |
| REST API, above 20 B | $1.51 | $1.72 | $1.80 |
| WebSocket messages, first 1 B | $1.00 | $1.14 | $1.19 |
| WebSocket messages, above 1 B | $0.80 | $0.94 | $0.99 |
| WebSocket connection-minutes | $0.25 | $0.285 | $0.297 |

- **CloudFront, pay as you go** (global offer version 20261003000426). Data transfer out to the internet from US and Europe edge locations costs $0.085/GB for the first 10 TB, $0.080 for the next 40 TB, $0.060 for the next 100 TB, $0.040 for the next 350 TB, $0.030 for the next 524 TB, $0.025 for the next 4 PB and $0.020 above 5 PB. HTTPS requests cost $0.0100 per 10,000 in the US and $0.0120 in Europe; HTTP requests $0.0075 and $0.0090. Transfer to origin costs $0.020/GB. The free tier is 1 TB and 10 million HTTP/S requests a month: [CloudFront offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudFront/20261003000426/aws-other/index.json).
- **CloudFront flat-rate plans, price-list figures** (CloudFrontPlans offer version 20260911124622, overage policy `NO_OVERAGE_THROTTLE`):
  - Free: $0 for 1 million requests, 100 GB and 5 GB of S3 storage.
  - Pro: $15 for 10 million requests, 50 TB and 50 GB.
  - Business: $200 for 125 million requests, 50 TB and 1 TB.
  - Premium: $1,000 for 500 million requests, 50 TB and 5 TB.
  - Premium variants: $1,450 (750 M / 75 TB), $2,250 (1.25 B / 125 TB), $3,500 (2 B / 200 TB), $6,000 (3.5 B / 350 TB) and $10,000 (6 B / 600 TB).
  - Source: [CloudFrontPlans offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/CloudFrontPlans/20260911124622/aws-other/index.json).
- **CloudFront flat-rate plans, page terms.** Plans are priced "per distribution" with "no additional overage charges". They bundle CDN, Route 53 DNS, DDoS protection, WAF and a TLS certificate. "Blocked DDoS attacks and requests blocked by AWS WAF never count against your usage allowance": [CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/).
- **Global Accelerator.** A fixed fee of $0.025 per accelerator-hour ($18.25 per 730-h month, computed). DT-Premium costs $0.015/GB in the dominant direction for North America to North America, Europe to Europe, and between the two, on top of standard data transfer: [Global Accelerator offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSGlobalAccelerator/20260911124552/aws-other/index.json).

### Inferences
- Egress pricing is region-neutral among these three regions, but fixed and per-request network services are dearer in Europe. Ireland is +6.7% (NAT), +10.0% (interface endpoint), +11.0% (HTTP API), +12.0% (ALB/NLB hour) and +14.0% (WebSocket messages). Paris is +10.0% (endpoint), +11.1% (NAT), +17.0% (HTTP API), +17.6% (ALB/NLB hour) and +19.0% (WebSocket messages). All computed vs us-east-1.
- For an outbound-heavy scraper fleet, the NAT Gateway per-GB charge ($0.045) stacks on top of internet egress ($0.09). Routing through NAT therefore costs 50% more per GB than egress alone, unless public IPv4 at $3.65 per address-month or the provisioned-bandwidth NAT option (no per-GB charge) is used.
- The flat-rate CloudFront plans cap spend at a fixed fee and throttle rather than bill overage, per the price list's `NO_OVERAGE_THROTTLE` policy. Ordinary pay-as-you-go would charge $0.085/GB plus requests.

### Gaps
- That cross-AZ transfer is billed in each direction (sender and receiver) is documented AWS practice but was not re-verified in this session. The price list shows a single $0.01/GB "in/out" rate.
- Pricing for the CloudFront "Custom" plan is "contact sales".
- How CloudFront throttles once a plan's allowance is reached is not detailed on the page.
- The provisioned-bandwidth NAT Gateway SKU appears only in the price list; no AWS page describing it was fetched.

## 7. Observability: CloudWatch Logs, custom metrics, X-Ray

### Takeaway
CloudWatch Logs ingestion costs $0.50/GB (Standard class) and $0.25/GB (Infrequent Access) in us-east-1, $0.57 / $0.285 in Ireland and $0.5985 / $0.299 in Paris. Storage costs $0.03 per GB-month and Logs Insights $0.005 per GB scanned in us-east-1. Custom metrics cost $0.30 per metric-month for the first 10,000, tiering down to $0.02 above 1 million. X-Ray costs $5.00 per million traces recorded and $0.50 per million retrieved.

### Cited Findings
- **Logs.** Custom log ingestion in the Standard class costs $0.50/GB (us-east-1), $0.57 (eu-west-1) or $0.5985 (eu-west-3). The Infrequent Access class costs $0.25, $0.285 or $0.29925; the Paris row is priced at $0.299. Storage costs $0.03 per GB-month ($0.0315 in Paris). CloudWatch offer version 20261007115540: [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json), [CloudWatch offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/eu-west-1/index.json), [CloudWatch offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/eu-west-3/index.json).
- **Logs Insights.** $0.005 per GB scanned (us-east-1), $0.0057 (eu-west-1), $0.0059 (eu-west-3). A separate "Omni-DataScanned" row ("CloudWatch Log Analytics and CloudWatch Logs Insights queries") costs $0.005 in all three regions. Live Tail costs $0.01 per minute: [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json), [CloudWatch offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/eu-west-1/index.json).
- **Vended logs (Standard class, us-east-1).** $0.50/GB for the first 10 TB, $0.25 for the next 20 TB, $0.10 for the next 20 TB and $0.05 above 50 TB. In the Infrequent Access class: $0.25, $0.15, $0.075 and $0.05. OpenTelemetry log ingestion costs $0.50/GB, or $0.25 for the "OtelEss" and IA rows: [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json).
- **Custom metrics** (identical in all three regions). $0.30 per metric-month for the first 10,000, $0.10 for the next 240,000, $0.05 for the next 750,000 and $0.02 above 1,000,000. API requests cost $0.01 per 1,000, and GetMetricData $0.01 per 1,000 metrics requested: [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json), [CloudWatch offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/eu-west-3/index.json).
- **Other metric SKUs.** A "CW:MetricsUsage" row costs $0.07 per metric-month, and "ObservationUsage" costs $0.21 per million observations for the first 1 billion, $0.18 for the next 3 billion and $0.10 above 4 billion: [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json).
- **CloudWatch free tier.** 5 GB of log ingestion, storage and scanning, 10 metrics, 3 basic dashboards and 1,800 Live Tail minutes: [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json), [CloudWatch offer global](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/aws-other/index.json).
- **X-Ray** (offer version 20260911124622). $0.000005 per trace stored ($5.00 per million), $0.0000005 per trace accessed ($0.50 per million) and $0.000001 per Insights trace ($1.00 per million), in us-east-1 and eu-west-1. The free tier is 100,000 traces stored and 1,000,000 traces accessed a month. eu-west-3 lists no "TracesAccessed" row: [X-Ray offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSXRay/20260911124622/us-east-1/index.json), [X-Ray offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSXRay/20260911124622/eu-west-3/index.json).

### Inferences
- At $0.50/GB, logging costs more than internet egress ($0.09/GB). Verbose browser or agent logs shipped to CloudWatch Standard can easily cost more than the bandwidth they describe; the Infrequent Access class halves ingestion.

### Gaps
- What the "CW:MetricsUsage" ($0.07) and "ObservationUsage" metric products are could not be verified against the CloudWatch pricing page. The CloudWatch pricing page was not fetched; all figures come from the price list.

## 8. Amazon Bedrock: on-demand per-million-token prices, batch, service tiers and prompt caching

### Takeaway
Anthropic's current lineup is Claude Fable 5.1, Opus 5.5, Sonnet 5.5 and Haiku 5.5. On Bedrock's global endpoint they cost $10/$50, $4/$20, $2/$10 and $0.10/$0.50 per million input/output tokens. Regional endpoints cost 10% more, and prices are identical in all three regions. Cache reads cost 2.5-10% of the input price. Cache writes cost 1.25x input at the standard rate, which the price list labels "Cache Write Tokens" with no TTL stated, or 2x input for the 1-hour TTL. Open-weight and Amazon models are much cheaper: gpt-oss-120b $0.15/$0.60, Qwen3 32B $0.15/$0.60, Llama 4 Maverick $0.24/$0.97, Nova Micro $0.035/$0.14. They carry Flex (−50%), Priority (+75%) and Batch (−50%) tiers. In Ireland they cost about 14-20% more, and many are not offered in Paris, where Nova costs about 47% more than in us-east-1.

### Cited Findings
- **Price-list sources.** Anthropic models are in the AmazonBedrockFoundationModels offer (version 20261008235637, published 2026-10-08) as AWS Marketplace software usage. Amazon and open-weight models are in the AmazonBedrock offer (version 20261010182242, published 2026-10-10): [Bedrock FM offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/us-east-1/index.json), [Bedrock FM offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-1/index.json), [Bedrock FM offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-3/index.json), [Bedrock offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/us-east-1/index.json), [Bedrock offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-1/index.json), [Bedrock offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-3/index.json).
- **Anthropic lineup.** Anthropic's models overview names Claude Fable 5.1 ($10 / $50), Opus 5.5 ($4 / $20), Sonnet 5.5 ($2 / $10) and Haiku 5.5 ("From $0.10 / input MTok, From $0.50 / output MTok") as the current lineup, all with 1M-token context and 128K max output. The listed legacy models still available are Fable 5, Opus 5, Opus 4.8, 4.7, 4.6 and 4.5, Sonnet 5, Sonnet 4.6 and Haiku 4.5. On Anthropic's API, "Batch API requests are 50% off; prompt cache reads cost 10% of the base input price (2.5% on Claude Fable 5.1 and Claude Mythos 5.1, 5% on Claude Opus 5.5 and Claude Sonnet 5.5)". "Bedrock offers global endpoints (dynamic routing) and regional endpoints (guaranteed data routing) for Claude Sonnet 4.5 and later": [Anthropic models overview](https://platform.claude.com/docs/en/about-claude/models/overview).
- **Bedrock tier rules.** "Priority tier pricing is at 75% premium to Standard tier pricing"; "Flex tier pricing is at 50% discount to Standard tier pricing". Batch inference for select models costs "50% lower price compared to on-demand". Anthropic Sonnet 5.5 "cache read pricing is reduced by 50% from the prior rate", effective 7 October: [Bedrock pricing](https://aws.amazon.com/bedrock/pricing/).
- **Regional ratios (computed).**
  - eu-west-1 open-weight rates are about 14-20% above us-east-1: gpt-oss-120b $0.18/$0.70 vs $0.15/$0.60 (+20.0% / +16.7%); DeepSeek v3.2 $0.74/$2.22 vs $0.62/$1.85 (+19.4% / +20.0%); Mistral Large 3 $0.60/$1.80 (+20.0%).
  - Nova in eu-west-1 is +13.3% to +15.0% (for example Nova Pro $0.92/$3.68). In eu-west-3 it is +46.7% to +48.6% (Nova Pro $1.18/$4.72; Nova Micro $0.052/$0.208).
  - Sources: [Bedrock offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/us-east-1/index.json), [Bedrock offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-1/index.json), [Bedrock offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-3/index.json).
- **Availability.** Not listed in eu-west-1: Llama 3.3 70B, Llama 4 Maverick and Scout, Nova 2.0 Omni, Nova Premier, DeepSeek R1 and Grok 4.3. eu-west-3 lists only the Anthropic models, the Nova family (without Premier), Mistral Large (the original model; the price list gives no version), Pixtral Large 25.02, Grok 4.6/4.7, Kimi K3 and OpenAI GPT-6 Astra among the models below: [Bedrock offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-1/index.json), [Bedrock offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-3/index.json).
- **Same price everywhere.** Grok 4.6/4.7, Kimi K3, OpenAI GPT-6 Astra and Pixtral Large 25.02 are priced the same in all three regions: [Bedrock offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-1/index.json), [Bedrock FM offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-1/index.json).

The tables below were generated from the price-list files. Prices are $ per million tokens. "Global" is the global cross-region endpoint; "Regional" is the in-region or geographic endpoint. "LC" marks long-context rates. Empty cells mean the price list has no row. Where a model has both a regular and a "mantle" SKU with identical prices, only one row is shown; a "(mantle SKU)" marker means only the mantle SKU exists.

#### Anthropic Claude (us-east-1; eu-west-1 and eu-west-3 rows carry identical prices where listed)

Sources: [Bedrock FM offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/us-east-1/index.json), [Bedrock FM offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-1/index.json), [Bedrock FM offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-3/index.json).

| Model | Endpoint | Input | Output | Batch in | Batch out | Cache read | Cache write (standard) | Cache write (1 h TTL) |
|---|---|---|---|---|---|---|---|---|
| Claude Fable 5.1 | Global | 10 | 50 |  |  | 0.25 | 12.5 | 20 |
| Claude Fable 5.1 | Regional | 11 | 55 |  |  | 0.275 | 13.75 | 22 |
| Claude Opus 5.5 | Global | 4 | 20 |  |  | 0.2 | 5 | 8 |
| Claude Opus 5.5 | Regional | 4.4 | 22 |  |  | 0.22 | 5.5 | 8.8 |
| Claude Sonnet 5.5 | Global | 2 | 10 |  |  | 0.1 | 2.5 | 4 |
| Claude Sonnet 5.5 | Regional | 2.2 | 11 |  |  | 0.11 | 2.75 | 4.4 |
| Claude Haiku 5.5 | Global | 0.1 | 0.5 |  |  | 0.01 | 0.125 | 0.2 |
| Claude Haiku 5.5 | Global (LC) | 0.5 | 2.5 |  |  | 0.05 | 0.625 | 1 |
| Claude Haiku 5.5 | Regional | 0.11 | 0.55 |  |  | 0.011 | 0.1375 | 0.22 |
| Claude Haiku 5.5 | Regional (LC) | 0.55 | 2.75 |  |  | 0.055 | 0.6875 | 1.1 |
| Claude Fable 5 | Global | 10 | 50 |  |  | 1 | 12.5 | 20 |
| Claude Fable 5 | Regional | 11 | 55 |  |  | 1.1 | 13.75 | 22 |
| Claude Mythos 5.1 | Global | 10 | 50 |  |  | 0.25 | 12.5 | 20 |
| Claude Mythos 5.1 | Regional | 11 | 55 |  |  | 0.275 | 13.75 | 22 |
| Claude Mythos 5 | Regional | 11 | 55 |  |  | 1.1 | 13.75 | 22 |
| Claude Mythos Preview | Regional | 27.5 | 137.5 |  |  | 2.75 | 34.375 | 55 |
| Claude Opus 5 | Global | 5 | 25 | 2.5 | 12.5 | 0.5 | 6.25 | 10 |
| Claude Opus 5 | Regional | 5.5 | 27.5 | 2.75 | 13.75 | 0.55 | 6.875 | 11 |
| Claude Sonnet 5 | Global | 2 | 10 |  |  | 0.2 | 2.5 | 4 |
| Claude Sonnet 5 | Regional | 2.2 | 11 |  |  | 0.22 | 2.75 | 4.4 |
| Claude Opus 4.8 | Global | 5 | 25 |  |  | 0.5 | 6.25 | 10 |
| Claude Opus 4.8 | Regional | 5.5 | 27.5 |  |  | 0.55 | 6.875 | 11 |
| Claude Opus 4.7 | Global | 5 | 25 |  |  | 0.5 | 6.25 | 10 |
| Claude Opus 4.7 | Regional | 5.5 | 27.5 |  |  | 0.55 | 6.875 | 11 |
| Claude Opus 4.6 | Global | 5 | 25 | 2.5 | 12.5 | 0.5 | 6.25 | 10 |
| Claude Opus 4.6 | Regional | 5.5 | 27.5 | 2.75 | 13.75 | 0.55 | 6.875 | 11 |
| Claude Opus 4.5 | Global | 5 | 25 | 2.5 | 12.5 | 0.5 | 6.25 | 10 |
| Claude Opus 4.5 | Regional | 5.5 | 27.5 | 2.75 | 13.75 | 0.55 | 6.875 | 11 |
| Claude Sonnet 4.6 | Global | 3 | 15 | 1.5 | 7.5 | 0.3 | 3.75 | 6 |
| Claude Sonnet 4.6 | Regional | 3.3 | 16.5 | 1.65 | 8.25 | 0.33 | 4.125 | 6.6 |
| Claude Sonnet 4.5 | Global | 3 | 15 | 1.5 | 7.5 | 0.3 | 3.75 | 6 |
| Claude Sonnet 4.5 | Regional | 3.3 | 16.5 | 1.65 | 8.25 | 0.33 | 4.125 | 6.6 |
| Claude Haiku 4.5 | Global | 1 | 5 | 0.5 | 2.5 | 0.1 | 1.25 | 2 |
| Claude Haiku 4.5 | Regional | 1.1 | 5.5 | 0.55 | 2.75 | 0.11 | 1.375 | 2.2 |

Rows listed in us-east-1 but not in eu-west-1: Claude Fable 5.1 (regional), Claude Mythos 5 (regional), Claude Mythos 5.1 (regional), Claude Mythos Preview (regional). Not in eu-west-3: Claude Fable 5.1 (regional), Claude Mythos 5 (regional), Claude Mythos 5.1 (regional), Claude Mythos Preview (regional).

#### Amazon Nova, Meta, Mistral, DeepSeek, Qwen and OpenAI models (us-east-1)

Sources: [Bedrock offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/us-east-1/index.json), [Bedrock FM offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/us-east-1/index.json).

| Model | Endpoint | Input | Output | Batch in | Batch out | Flex in | Flex out | Priority in | Priority out | Cache read | Cache write (standard) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Nova Micro | Regional | 0.035 | 0.14 | 0.0175 | 0.07 |  |  |  |  | 0.00875 |  |
| Nova Lite | Regional | 0.06 | 0.24 | 0.03 | 0.12 |  |  |  |  | 0.015 |  |
| Nova Pro | Regional | 0.8 | 3.2 | 0.4 | 1.6 | 0.4 | 1.6 | 1.4 | 5.6 | 0.2 |  |
| Nova Premier | Regional | 2.5 | 12.5 | 1.25 | 6.25 | 1.25 | 6.25 | 4.375 | 21.875 | 0.625 |  |
| Nova 2.0 Lite | Global | 0.3 | 2.5 | 0.15 | 1.25 | 0.15 | 1.25 | 0.525 | 4.375 | 0.075 |  |
| Nova 2.0 Lite | Regional | 0.3 | 2.75 | 0.165 | 1.375 | 0.165 | 1.375 | 0.5775 | 4.8125 | 0.0825 |  |
| Nova 2.0 Pro | Global | 1.25 | 10 | 0.625 | 5 | 0.625 | 5 | 2.1875 | 17.5 |  |  |
| Nova 2.0 Pro | Regional | 1.375 | 11 | 0.6875 | 5.5 | 0.6875 | 5.5 | 2.40625 | 19.25 |  |  |
| Nova 2.0 Omni | Global | 0.3 | 2.5 | 0.2 | 1.3 | 0.2 | 1.3 | 0.5 | 4.4 |  |  |
| Nova 2.0 Omni | Regional | 0.3 | 2.8 | 0.2 | 1.4 | 0.2 | 1.4 | 0.6 | 4.8 |  |  |
| Llama 4 Maverick 17B | Regional | 0.24 | 0.97 | 0.12 | 0.485 |  |  |  |  |  |  |
| Llama 4 Scout 17B | Regional | 0.17 | 0.66 | 0.085 | 0.33 |  |  |  |  |  |  |
| Llama 3.3 70B | Regional | 0.72 | 0.72 | 0.36 | 0.36 |  |  |  |  |  |  |
| Mistral Large 3 | Regional | 0.5 | 1.5 | 0.25 | 0.75 | 0.25 | 0.75 | 0.88 | 2.63 |  |  |
| Ministral 3B 3.0 | Regional | 0.1 | 0.1 | 0.05 | 0.05 | 0.05 | 0.05 | 0.17 | 0.17 |  |  |
| Ministral 8B 3.0 | Regional | 0.15 | 0.15 | 0.07 | 0.07 | 0.07 | 0.07 | 0.26 | 0.26 |  |  |
| Ministral 14B 3.0 | Regional | 0.2 | 0.2 | 0.1 | 0.1 | 0.1 | 0.1 | 0.35 | 0.35 |  |  |
| Magistral Small 1.2 | Regional | 0.5 | 1.5 | 0.25 | 0.75 | 0.25 | 0.75 | 0.88 | 2.63 |  |  |
| Devstral | Regional | 0.4 | 2 | 0.2 | 1 | 0.2 | 1 | 0.7 | 3.5 |  |  |
| Pixtral Large 25.02 | Regional | 2 | 6 |  |  |  |  |  |  |  |  |
| Mistral Large | Regional | 4 | 12 |  |  |  |  |  |  |  |  |
| DeepSeek v3.2 | Regional | 0.62 | 1.85 | 0.31 | 0.925 | 0.31 | 0.925 | 1.085 | 3.2375 |  |  |
| DeepSeek V3.1 | Regional (mantle SKU) | 0.58 | 1.68 | 0.29 | 0.84 | 0.29 | 0.84 | 1.015 | 2.94 |  |  |
| R1 | Regional | 1.35 | 5.4 |  |  |  |  |  |  |  |  |
| Qwen3 32B | Regional | 0.15 | 0.6 | 0.075 | 0.3 | 0.075 | 0.3 | 0.2625 | 1.05 |  |  |
| Qwen3 235B A22B 2507 | Regional (mantle SKU) | 0.22 | 0.88 | 0.11 | 0.44 | 0.11 | 0.44 | 0.385 | 1.54 |  |  |
| Qwen3 VL 235B A22B | Regional | 0.53 | 2.66 | 0.26 | 1.33 | 0.26 | 1.33 | 0.93 | 4.66 |  |  |
| Qwen3 Next 80B A3B | Regional (mantle SKU) | 0.14 | 1.2 | 0.07 | 0.6 | 0.07 | 0.6 | 0.26 | 2.1 |  |  |
| Qwen3 Coder 30B A3B | Regional | 0.15 | 0.6 | 0.075 | 0.3 | 0.075 | 0.3 | 0.2625 | 1.05 |  |  |
| Qwen3 Coder 480B A35B | Regional (mantle SKU) | 0.45 | 1.8 | 0.225 | 0.9 | 0.225 | 0.9 | 0.7875 | 3.15 |  |  |
| Qwen3 Coder Next | Regional | 0.5 | 1.2 | 0.25 | 0.6 | 0.25 | 0.6 | 0.875 | 2.1 |  |  |
| gpt-oss-20b | Regional | 0.07 | 0.3 | 0.035 | 0.15 | 0.035 | 0.15 | 0.1225 | 0.525 |  |  |
| gpt-oss-120b | Regional | 0.15 | 0.6 | 0.075 | 0.3 | 0.075 | 0.3 | 0.2625 | 1.05 |  |  |
| GPT OSS Safeguard 20B | Regional | 0.07 | 0.2 | 0.03 | 0.1 | 0.03 | 0.1 | 0.12 | 0.35 |  |  |
| GPT OSS Safeguard 120B | Regional | 0.15 | 0.6 | 0.07 | 0.3 | 0.07 | 0.3 | 0.26 | 1.05 |  |  |
| OpenAI GPT-6 Astra | Global | 10 | 50 |  |  |  |  |  |  | 1 | 12.5 |
| OpenAI GPT-6 Astra | Global (LC) | 20 | 75 |  |  |  |  |  |  | 2 | 25 |
| OpenAI GPT-6 Astra | Regional | 11 | 55 |  |  |  |  |  |  | 1.1 | 13.75 |
| OpenAI GPT-6 Astra | Regional (LC) | 22 | 82.5 |  |  |  |  |  |  | 2.2 | 27.5 |

Other third-party models listed in us-east-1:

| Model | Endpoint | Input | Output | Batch in | Batch out | Flex in | Flex out | Priority in | Priority out | Cache read | Cache write (standard) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| GLM 4.7 | Regional | 0.6 | 2.2 | 0.3 | 1.1 | 0.3 | 1.1 | 1.05 | 3.85 |  |  |
| GLM 4.7 Flash | Regional | 0.07 | 0.4 | 0.035 | 0.2 | 0.035 | 0.2 | 0.1225 | 0.7 |  |  |
| GLM 5 | Regional | 1 | 3.2 | 0.5 | 1.6 | 0.5 | 1.6 | 1.75 | 5.6 |  |  |
| Gemma 3 12B | Regional | 0.09 | 0.29 | 0.05 | 0.15 | 0.05 | 0.15 | 0.16 | 0.51 |  |  |
| Gemma 3 27B | Regional | 0.23 | 0.38 | 0.12 | 0.19 | 0.12 | 0.19 | 0.4 | 0.67 |  |  |
| Gemma 3 4B | Regional | 0.04 | 0.08 | 0.02 | 0.04 | 0.02 | 0.04 | 0.07 | 0.14 |  |  |
| Grok 4.7 | Global (mantle SKU) | 2 | 6 |  |  | 1 | 3 | 3.5 | 10.5 | 0.5 |  |
| Grok 4.7 | Regional (mantle SKU) | 2.2 | 6.6 | 1.1 | 3.3 | 1.1 | 3.3 | 3.85 | 11.55 | 0.55 |  |
| Kimi K2 Thinking | Regional | 0.6 | 2.5 | 0.3 | 1.25 | 0.3 | 1.25 | 1.05 | 4.38 |  |  |
| Kimi K2.5 | Regional | 0.6 | 3 | 0.3 | 1.5 | 0.3 | 1.5 | 1.05 | 5.25 |  |  |
| Kimi K3 | Global (mantle SKU) | 3 | 15 |  |  | 1.5 | 7.5 | 5.25 | 26.25 | 0.3 | 3.75 |
| Kimi K3 | Regional (mantle SKU) | 3.3 | 16.5 | 1.65 | 8.25 | 1.65 | 8.25 | 5.775 | 28.875 | 0.33 | 4.125 |
| MiniMax M2.5 | Regional | 0.3 | 1.2 | 0.15 | 0.6 | 0.15 | 0.6 | 0.525 | 2.1 |  |  |
| Minimax M2 | Regional | 0.3 | 1.2 | 0.15 | 0.6 | 0.15 | 0.6 | 0.52 | 2.1 |  |  |
| Minimax M2.1 | Regional | 0.3 | 1.2 | 0.15 | 0.6 | 0.15 | 0.6 | 0.525 | 2.1 |  |  |
| NVIDIA Nemotron 3 Super 120B A12B | Regional | 0.15 | 0.65 | 0.075 | 0.325 | 0.075 | 0.325 | 0.2625 | 1.1375 |  |  |
| NVIDIA Nemotron Nano 2 | Regional | 0.06 | 0.23 | 0.03 | 0.12 | 0.03 | 0.12 | 0.11 | 0.4 |  |  |
| NVIDIA Nemotron Nano 2 VL | Regional | 0.2 | 0.6 | 0.1 | 0.3 | 0.1 | 0.3 | 0.35 | 1.05 |  |  |
| Nemotron Nano 3 30B | Regional | 0.06 | 0.24 | 0.03 | 0.12 | 0.03 | 0.12 | 0.11 | 0.42 |  |  |
| xai.grok-4.3 | Regional (mantle SKU) | 1.25 | 2.5 | 0.625 | 1.25 | 0.625 | 1.25 | 2.1875 | 4.375 | 0.2 |  |
| xai.grok-4.6 | Global (mantle SKU) | 2 | 6 |  |  | 1 | 3 | 3.5 | 10.5 | 0.5 |  |
| xai.grok-4.6 | Regional (mantle SKU) | 2.2 | 6.6 |  |  | 1.1 | 3.3 | 3.85 | 11.55 | 0.55 |  |

#### Amazon Nova, Meta, Mistral, DeepSeek, Qwen and OpenAI models (eu-west-1)

Sources: [Bedrock offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-1/index.json), [Bedrock FM offer eu-west-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-1/index.json).

| Model | Endpoint | Input | Output | Batch in | Batch out | Flex in | Flex out | Priority in | Priority out | Cache read | Cache write (standard) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Nova Micro | Regional | 0.04 | 0.16 | 0.02 | 0.08 |  |  |  |  | 0.01 |  |
| Nova Lite | Regional | 0.069 | 0.276 | 0.0345 | 0.138 |  |  |  |  | 0.01725 |  |
| Nova Pro | Regional | 0.92 | 3.68 | 0.46 | 1.84 | 0.46 | 1.84 | 1.61 | 6.44 | 0.23 |  |
| Nova 2.0 Lite | Global | 0.34 | 2.87 | 0.17 | 1.435 | 0.17 | 1.435 | 0.595 | 5.0225 | 0.085 |  |
| Nova 2.0 Lite | Regional | 0.374 | 3.157 | 0.187 | 1.5785 | 0.187 | 1.5785 | 0.6545 | 5.52475 | 0.0935 |  |
| Nova 2.0 Pro | Global | 1.43 | 11.47 | 0.715 | 5.735 | 0.715 | 5.735 | 2.5025 | 20.0725 |  |  |
| Mistral Large 3 | Regional (mantle SKU) | 0.6 | 1.8 | 0.3 | 0.9 | 0.3 | 0.9 | 1.05 | 3.15 |  |  |
| Ministral 3B 3.0 | Regional | 0.12 | 0.12 | 0.06 | 0.06 | 0.06 | 0.06 | 0.21 | 0.21 |  |  |
| Ministral 8B 3.0 | Regional | 0.18 | 0.18 | 0.09 | 0.09 | 0.09 | 0.09 | 0.32 | 0.32 |  |  |
| Ministral 14B 3.0 | Regional | 0.23 | 0.23 | 0.12 | 0.12 | 0.12 | 0.12 | 0.4 | 0.4 |  |  |
| Magistral Small 1.2 | Regional | 0.59 | 1.76 | 0.3 | 0.88 | 0.3 | 0.88 | 1.03 | 3.08 |  |  |
| Devstral | Regional | 0.48 | 2.4 | 0.24 | 1.2 | 0.24 | 1.2 | 0.84 | 4.2 |  |  |
| Pixtral Large 25.02 | Regional | 2 | 6 |  |  |  |  |  |  |  |  |
| Mistral Large | Regional | 4.3 | 13 |  |  |  |  |  |  |  |  |
| DeepSeek v3.2 | Regional | 0.74 | 2.22 | 0.37 | 1.11 | 0.37 | 1.11 | 1.295 | 3.885 |  |  |
| DeepSeek V3.1 | Regional (mantle SKU) | 0.696 | 2.016 | 0.348 | 1.008 | 0.348 | 1.008 | 1.218 | 3.528 |  |  |
| Qwen3 32B | Regional | 0.18 | 0.7 | 0.09 | 0.35 | 0.09 | 0.35 | 0.315 | 1.225 |  |  |
| Qwen3 235B A22B 2507 | Regional (mantle SKU) | 0.264 | 1.056 | 0.132 | 0.528 | 0.132 | 0.528 | 0.462 | 1.848 |  |  |
| Qwen3 VL 235B A22B | Regional | 0.62 | 3.12 | 0.31 | 1.56 | 0.31 | 1.56 | 1.09 | 5.46 |  |  |
| Qwen3 Next 80B A3B | Regional (mantle SKU) | 0.168 | 1.44 | 0.084 | 0.72 | 0.09 | 0.71 | 0.32 | 2.47 |  |  |
| Qwen3 Coder 30B A3B | Regional | 0.18 | 0.7 | 0.09 | 0.35 | 0.09 | 0.35 | 0.315 | 1.225 |  |  |
| Qwen3 Coder 480B A35B | Regional (mantle SKU) | 0.54 | 2.16 | 0.27 | 1.08 | 0.27 | 1.08 | 0.945 | 3.78 |  |  |
| Qwen3 Coder Next | Regional | 0.6 | 1.44 | 0.3 | 0.72 | 0.3 | 0.72 | 1.05 | 2.52 |  |  |
| gpt-oss-20b | Regional | 0.08 | 0.35 | 0.04 | 0.18 | 0.04 | 0.175 | 0.14 | 0.6125 |  |  |
| gpt-oss-120b | Regional | 0.18 | 0.7 | 0.09 | 0.35 | 0.09 | 0.35 | 0.315 | 1.225 |  |  |
| GPT OSS Safeguard 20B | Regional | 0.08 | 0.23 | 0.04 | 0.12 | 0.04 | 0.12 | 0.14 | 0.4 |  |  |
| GPT OSS Safeguard 120B | Regional | 0.18 | 0.7 | 0.09 | 0.35 | 0.09 | 0.35 | 0.32 | 1.22 |  |  |
| OpenAI GPT-6 Astra | Global | 10 | 50 |  |  |  |  |  |  | 1 | 12.5 |
| OpenAI GPT-6 Astra | Global (LC) | 20 | 75 |  |  |  |  |  |  | 2 | 25 |

Other third-party models listed in eu-west-1:

| Model | Endpoint | Input | Output | Batch in | Batch out | Flex in | Flex out | Priority in | Priority out | Cache read | Cache write (standard) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| GLM 4.7 | Regional | 0.72 | 2.64 | 0.36 | 1.32 | 0.36 | 1.32 | 1.26 | 4.62 |  |  |
| GLM 4.7 Flash | Regional | 0.08 | 0.48 | 0.04 | 0.24 | 0.04 | 0.24 | 0.14 | 0.84 |  |  |
| GLM 5 | Regional | 1.2 | 3.84 | 0.6 | 1.92 | 0.6 | 1.92 | 2.1 | 6.72 |  |  |
| Gemma 3 12B | Regional | 0.11 | 0.34 | 0.06 | 0.17 | 0.06 | 0.17 | 0.19 | 0.6 |  |  |
| Gemma 3 27B | Regional | 0.27 | 0.45 | 0.14 | 0.22 | 0.14 | 0.22 | 0.47 | 0.79 |  |  |
| Gemma 3 4B | Regional | 0.05 | 0.09 | 0.03 | 0.05 | 0.03 | 0.05 | 0.09 | 0.16 |  |  |
| Grok 4.7 | Global (mantle SKU) | 2 | 6 |  |  | 1 | 3 | 3.5 | 10.5 | 0.5 |  |
| Grok 4.7 | Regional (mantle SKU) | 2.2 | 6.6 | 1.1 | 3.3 | 1.1 | 3.3 | 3.85 | 11.55 | 0.55 |  |
| Kimi K2 Thinking | Regional (mantle SKU) | 0.72 | 3 | 0.36 | 1.5 | 0.36 | 1.5 | 1.26 | 5.25 |  |  |
| Kimi K2.5 | Regional | 0.72 | 3.6 | 0.36 | 1.8 | 0.36 | 1.8 | 1.26 | 6.3 |  |  |
| Kimi K3 | Global (mantle SKU) | 3 | 15 |  |  | 1.5 | 7.5 | 5.25 | 26.25 | 0.3 | 3.75 |
| Kimi K3 | Regional (mantle SKU) | 3.3 | 16.5 | 1.65 | 8.25 | 1.65 | 8.25 | 5.775 | 28.875 | 0.33 | 4.125 |
| MiniMax M2.5 | Regional | 0.36 | 1.44 | 0.18 | 0.72 | 0.18 | 0.72 | 0.63 | 2.52 |  |  |
| Minimax M2 | Regional | 0.35 | 1.41 | 0.17 | 0.71 | 0.17 | 0.71 | 0.61 | 2.47 |  |  |
| Minimax M2.1 | Regional | 0.36 | 1.44 | 0.18 | 0.72 | 0.18 | 0.72 | 0.63 | 2.52 |  |  |
| NVIDIA Nemotron 3 Super 120B A12B | Regional | 0.18 | 0.78 | 0.09 | 0.39 | 0.09 | 0.39 | 0.315 | 1.365 |  |  |
| NVIDIA Nemotron Nano 2 | Regional | 0.07 | 0.27 | 0.03 | 0.14 | 0.03 | 0.14 | 0.12 | 0.47 |  |  |
| NVIDIA Nemotron Nano 2 VL | Regional | 0.23 | 0.7 | 0.12 | 0.35 | 0.12 | 0.35 | 0.4 | 1.22 |  |  |
| Nemotron Nano 3 30B | Regional | 0.07 | 0.28 | 0.03 | 0.14 | 0.03 | 0.14 | 0.12 | 0.49 |  |  |
| xai.grok-4.6 | Global (mantle SKU) | 2 | 6 |  |  | 1 | 3 | 3.5 | 10.5 | 0.5 |  |
| xai.grok-4.6 | Regional (mantle SKU) | 2.2 | 6.6 |  |  | 1.1 | 3.3 | 3.85 | 11.55 | 0.55 |  |

#### Amazon Nova, Meta, Mistral, DeepSeek, Qwen and OpenAI models (eu-west-3)

Sources: [Bedrock offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrock/20261010182242/eu-west-3/index.json), [Bedrock FM offer eu-west-3](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonBedrockFoundationModels/20261008235637/eu-west-3/index.json).

| Model | Endpoint | Input | Output | Batch in | Batch out | Flex in | Flex out | Priority in | Priority out | Cache read | Cache write (standard) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Nova Micro | Regional | 0.052 | 0.208 | 0.026 | 0.104 |  |  |  |  | 0.013 |  |
| Nova Lite | Regional | 0.088 | 0.352 | 0.044 | 0.176 |  |  |  |  | 0.022 |  |
| Nova Pro | Regional | 1.18 | 4.72 | 0.59 | 2.36 | 0.59 | 2.36 | 2.065 | 8.26 | 0.295 |  |
| Nova 2.0 Lite | Global | 0.44 | 3.69 | 0.22 | 1.845 | 0.22 | 1.845 | 0.77 | 6.4575 | 0.11 |  |
| Nova 2.0 Lite | Regional | 0.484 | 4.059 | 0.242 | 2.0295 | 0.242 | 2.0295 | 0.847 | 7.10325 | 0.121 |  |
| Nova 2.0 Pro | Global | 1.84 | 14.74 | 0.92 | 7.37 | 0.92 | 7.37 | 3.22 | 25.795 |  |  |
| Nova 2.0 Omni | Global | 0.4 | 3.7 | 0.2 | 1.8 | 0.2 | 1.8 | 0.8 | 6.5 |  |  |
| Pixtral Large 25.02 | Regional | 2 | 6 |  |  |  |  |  |  |  |  |
| Mistral Large | Regional | 5.2 | 15.6 |  |  |  |  |  |  |  |  |
| OpenAI GPT-6 Astra | Global | 10 | 50 |  |  |  |  |  |  | 1 | 12.5 |
| OpenAI GPT-6 Astra | Global (LC) | 20 | 75 |  |  |  |  |  |  | 2 | 25 |

Other third-party models listed in eu-west-3:

| Model | Endpoint | Input | Output | Batch in | Batch out | Flex in | Flex out | Priority in | Priority out | Cache read | Cache write (standard) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Grok 4.7 | Global (mantle SKU) | 2 | 6 |  |  | 1 | 3 | 3.5 | 10.5 | 0.5 |  |
| Grok 4.7 | Regional (mantle SKU) | 2.2 | 6.6 | 1.1 | 3.3 | 1.1 | 3.3 | 3.85 | 11.55 | 0.55 |  |
| Kimi K3 | Global (mantle SKU) | 3 | 15 |  |  | 1.5 | 7.5 | 5.25 | 26.25 | 0.3 | 3.75 |
| Kimi K3 | Regional (mantle SKU) | 3.3 | 16.5 | 1.65 | 8.25 | 1.65 | 8.25 | 5.775 | 28.875 | 0.33 | 4.125 |
| xai.grok-4.6 | Global (mantle SKU) | 2 | 6 |  |  | 1 | 3 | 3.5 | 10.5 | 0.5 |  |
| xai.grok-4.6 | Regional (mantle SKU) | 2.2 | 6.6 |  |  | 1.1 | 3.3 | 3.85 | 11.55 | 0.55 |  |

### Inferences
- For extraction-style calls with large cached prompts, the Claude 5.5 generation cuts cost through cache-read rates of 5% (Opus 5.5 and Sonnet 5.5: $0.20 and $0.10 per million) and 10% (Haiku 5.5: $0.01 per million). Haiku 5.5's long-context rates are 5x its standard rates ($0.50/$2.50 vs $0.10/$0.50), so prompts beyond the long-context threshold are much more expensive.
- Choosing Bedrock's global endpoint over a regional one is a flat 10% saving on every Anthropic token type, but regional endpoints guarantee data routing. That trade-off matters for EU data-residency commitments.
- The price list shows no Bedrock batch rows for the 5.5-generation Claude models, Fable, Mythos, Sonnet 5 or Opus 4.7/4.8, unlike Opus 5, Opus 4.5/4.6, Sonnet 4.5/4.6 and Haiku 4.5. Batch discounts for the current models on Bedrock should not be assumed.

### Gaps
- What "mantle" SKUs are is not explained on the fetched pages. Their prices equal the regular SKUs wherever both exist.
- The long-context threshold for Haiku 5.5 and GPT-6 Astra is not stated in the price list.
- The Bedrock pricing page was truncated in the fetch, and its Anthropic tables were not readable there; the price list was used instead.
- Model capabilities and quality were not assessed. "Claude Mythos 5 / 5.1 / Preview" appear in the price list (Mythos Preview at $27.50/$137.50, regional only) but not in Anthropic's current-lineup table, apart from a pricing footnote naming Mythos 5.1.

## 9. AWS Free Tier (credits-based), AWS Support plan pricing, AWS Marketplace SaaS listing fees

### Takeaway
New AWS customers get $100 in credits at sign-up and can earn up to $100 more. On the Free plan the account closes after 6 months or when credits run out; on the Paid plan usage beyond credits is billed. More than 30 always-free allowances remain. The current Support plans are Business Support+ (greater of $29 or 9/7/5/3% of usage), Enterprise Support (greater of $5,000 or 10/7/5/3%) and Unified Operations (greater of $50,000 or 10/6/5%). AWS Marketplace charges 3% on SaaS public offers, 3%/2%/1.5% on private offers by contract value, 1.5% on renewals and 0.5% extra for channel-partner private offers.

### Cited Findings
- **Free Tier credits and plans.** "You get $100 in credits when you open a Free Tier account, and you can earn up to $100 more by exploring key services". The Free plan "closes on its own 6 months after you open it" or when credits run out, whichever is first; Paid plan customers are also eligible for up to $200 in credits. "30+ AWS services are always free within monthly usage limits": [AWS Free Tier](https://aws.amazon.com/free/).
- **Free Tier documentation.** New users "receive USD $100 in credits after you create an account, regardless of your account plan". Free account plans "don't have access to certain AWS services that would rapidly consume the entire AWS Free Tier credit amount": [Billing user guide, Free Tier](https://docs.aws.amazon.com/awsaccountbilling/latest/aboutv2/free-tier.html).
- **Free Tier FAQs (inconsistent).** The FAQs refer to accounts opened "prior to the launch of the new sign-up experience in October 2026". They also contradict each other on credit expiry: one answer says credits "expire 12 months after you create your account", others say 6 months: [AWS Free Tier FAQs](https://aws.amazon.com/free/free-tier-faqs/).
- **Always-free allowances in the price list.** Lambda 1M requests and 400,000 GB-s; SQS 1M requests; DynamoDB 25 GB, 25 WCU and 25 RCU; CloudFront 1 TB and 10M requests; 100 GB of internet egress aggregated globally; X-Ray 100,000 traces stored and 1M accessed; CloudWatch 5 GB of logs and 10 metrics: [Lambda offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSLambda/20261009223122/us-east-1/index.json), [SQS offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSQueueService/20260911124607/us-east-1/index.json), [DynamoDB offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonDynamoDB/20260911124422/us-east-1/index.json), [CloudFront offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudFront/20261003000426/aws-other/index.json), [DataTransfer offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/20260916132208/us-east-1/index.json), [X-Ray offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSXRay/20260911124622/us-east-1/index.json), [CloudWatch offer us-east-1](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonCloudWatch/20261007115540/us-east-1/index.json).
- **Current Support plans** ([AWS Support pricing](https://aws.amazon.com/premiumsupport/pricing/)):
  - Business Support+: "Minimum of $29/month per account", otherwise 9% of monthly AWS charges up to $10K, 7% from $10K to $80K, 5% from $80K to $250K and 3% above $250K.
  - Enterprise Support: minimum $5K a month, otherwise 10% up to $150K, 7% from $150K to $500K, 5% from $500K to $1M and 3% above $1M. A regional variant for qualified LATAM countries, India and Mainland China uses 10% / 7% / 5% / 3% with breakpoints at $150K, $250K and $500K.
  - Unified Operations: minimum $50K a month, otherwise 10% up to $1M, 6% from $1M to $5M and 5% above $5M.
  - Basic Support is included for all customers.
- **Support offers in the price list.**
  - The current plans match the page: "AWSSupportEssential" ($29 recurring fee; 9 / 7 / 5 / 3%), "AWSSupportEnterprise" ($5,000; 10 / 7 / 5 / 3%) and "AWSUnifiedOperations" ($50,000; 10 / 6 / 5%).
  - Three older plans still have offers, although the current pricing page no longer shows them. "AWSSupportBusiness": $100 recurring fee; 10% up to $10K, 7% to $80K, 5% to $250K, 3% above. "AWSEnterpriseOnRamp": $5,500; 10%. "AWSDeveloperSupport": $29; 3%.
  - Sources: [Support Essential offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSSupportEssential/20260911124616/aws-other/index.json), [Support Enterprise offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSSupportEnterprise/20260911124616/aws-other/index.json), [Unified Operations offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSUnifiedOperations/20260923205907/aws-other/index.json), [Support Business offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSSupportBusiness/20260911124616/aws-other/index.json), [Enterprise On-Ramp offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSEnterpriseOnRamp/20260911124549/aws-other/index.json), [Developer Support offer](https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDeveloperSupport/20260911124540/aws-other/index.json).
- **Marketplace listing fees** (effective since January 5, 2024; fees are charged on the pre-tax total contract value) ([AWS Marketplace Seller Guide, listing fees](https://docs.aws.amazon.com/marketplace/latest/userguide/listing-fees.html)):
  - Public offers: SaaS 3%, Server products (AMI, container, ML) 20%, AWS Data Exchange 3%.
  - Private offers: "Less than $1M – 3%; Between $1M and less than $10M – 2%; Equal to or greater than $10M – 1.5%; All renewals – 1.5%".
  - Channel partner private offers: an extra 0.5%.
  - Professional services: 0.5% on private offers, or 0% within qualifying multi-product offer sets.
  - South Korea buyers: an extra 1% regional fee from 04/01/2025.

### Inferences
- Under Business Support+, a $20,000 monthly AWS bill would cost 9% × $10,000 + 7% × $10,000 = $1,600 a month. Under Enterprise Support the same bill would cost the $5,000 minimum, because 10% of $20,000 is $2,000. This is computed from the page's formula.
- Selling Goodfetch Cloud as a SaaS public listing would give up 3% of revenue to AWS Marketplace. Large private deals would give up 1.5-2%.

### Gaps
- The exact date the credits-based Free Tier took effect for new accounts could not be confirmed. The fetched pages give no day, and the FAQ's mention of an October 2026 "new sign-up experience" suggests a further change that could not be investigated after the search budget ran out.
- Whether Developer, Business (classic) and Enterprise On-Ramp Support can still be bought, and their retirement dates, could not be verified. They exist only as price-list offers, not on the current pricing page.
