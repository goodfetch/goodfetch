# 2200-defaults-b: commit b715958c3, Hetzner Cloud CCX43, 16 dedicated vCPUs, AMD EPYC-Milan Processor, 16 threads, load at start 1.37, seed 973780065458093915
# 2200-percore512-b: commit b715958c3, Hetzner Cloud CCX43, 16 dedicated vCPUs, AMD EPYC-Milan Processor, 16 threads, load at start 0.62, seed 3004701457845617403

| Cell | 2200-defaults-b: median s [min-max] n | 2200-percore512-b: median s [min-max] n | wall Δ | p | CPU s 2200-defaults-b → 2200-percore512-b | peak MiB 2200-defaults-b → 2200-percore512-b (p) | threads | max start load |
|---|---|---|---|---|---|---|---|---|
| durable/foxhound full | 1.564 [1.375-1.686] 10 | 1.585 [1.466-1.753] 10 | +1.4% | 0.436 | 1.07 → 1.26 | 31.3 → 39.5 (0.000) | 12 → 31 | 1.1 / 1.16 |
| durable/foxhound periodic | 1.300 [1.180-1.419] 10 | 1.352 [1.165-1.663] 10 | +4.0% | 0.190 | 0.99 → 1.18 | 31.5 → 39.5 (0.000) | 12 → 31 | 1.64 / 0.98 |
| polite/foxhound | 77.360 [76.979-77.696] 10 | 77.028 [76.792-77.690] 10 | -0.4% | 0.043 | 26.60 → 30.31 | 69.7 → 92.4 (0.000) | 27 → 44 | 1.64 / 1.38 |

instructions 2200-defaults-b: []
instructions 2200-percore512-b: []
