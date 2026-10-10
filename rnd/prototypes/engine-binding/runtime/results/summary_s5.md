### S5 medians across blocks (each process journals its pages; 1 KB pages, concurrency 8, synchronous=FULL)

| SDK | Topology | Procs | Inspector | Rows/s total | Commit p50 (ms) | Commit p99, worst process (ms) | SQLITE_BUSY after 5 s | Lock wait total (ms) | Commits | WAL at end (MB) | Inspector query p99 (ms) | Failures | Blocks |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| node | daemon | 1 | off | 2,532 | 0.39 | 4.85 | 0 | 5 | 328 | 3.00 | - | 0 | 5 |
| node | daemon | 1 | on | 3,042 | 0.42 | 3.14 | 0 | 5 | 354 | 3.20 | 1.48 | 0 | 5 |
| node | daemon | 2 | off | 4,235 | 0.55 | 7.60 | 0 | 7 | 526 | 3.93 | - | 0 | 5 |
| node | daemon | 2 | on | 1,576 | 0.61 | 27.13 | 0 | 12 | 495 | 3.94 | 3.57 | 0 | 5 |
| node | daemon | 4 | off | 2,889 | 0.93 | 20.11 | 0 | 24 | 565 | 3.94 | - | 0 | 5 |
| node | daemon | 4 | on | 2,164 | 0.80 | 44.66 | 0 | 31 | 631 | 3.94 | 6.27 | 0 | 5 |
| node | daemon | 8 | off | 2,673 | 1.38 | 78.67 | 0 | 58 | 757 | 3.95 | - | 0 | 5 |
| node | daemon | 8 | on | 3,539 | 1.38 | 26.47 | 0 | 31 | 867 | 3.96 | 28.82 | 0 | 5 |
| node | embedded | 1 | off | 1,439 | 0.72 | 62.40 | 0 | 7 | 359 | 3.27 | - | 0 | 5 |
| node | embedded | 1 | on | 1,743 | 0.46 | 14.68 | 0 | 6 | 381 | 3.42 | 3.91 | 0 | 5 |
| node | embedded | 2 | off | 2,995 | 0.77 | 46.80 | 0 | 339 | 752 | 3.93 | - | 0 | 5 |
| node | embedded | 2 | on | 2,442 | 0.92 | 25.52 | 0 | 309 | 669 | 3.94 | 2.66 | 0 | 5 |
| node | embedded | 4 | off | 3,313 | 0.59 | 89.23 | 0 | 1,309 | 1,437 | 3.99 | - | 0 | 5 |
| node | embedded | 4 | on | 3,719 | 0.68 | 97.92 | 0 | 2,024 | 1,459 | 4.08 | 4.52 | 0 | 5 |
| node | embedded | 8 | off | 3,620 | 1.34 | 189.07 | 0 | 10,189 | 2,790 | 4.11 | - | 0 | 5 |
| node | embedded | 8 | on | 3,209 | 1.78 | 444.20 | 0 | 11,052 | 2,687 | 4.07 | 24.34 | 0 | 5 |
| python | daemon | 1 | off | 1,628 | 0.45 | 7.91 | 0 | 10 | 422 | 3.72 | - | 0 | 5 |
| python | daemon | 1 | on | 1,871 | 0.40 | 4.05 | 0 | 7 | 421 | 3.73 | 1.59 | 0 | 5 |
| python | daemon | 2 | off | 3,329 | 0.52 | 7.80 | 0 | 11 | 576 | 3.93 | - | 0 | 5 |
| python | daemon | 2 | on | 2,583 | 0.53 | 23.56 | 0 | 19 | 625 | 3.93 | 1.15 | 0 | 5 |
| python | daemon | 4 | off | 1,401 | 1.42 | 25.85 | 0 | 44 | 646 | 3.94 | - | 0 | 5 |
| python | daemon | 4 | on | 2,176 | 0.77 | 32.48 | 0 | 31 | 722 | 3.94 | 8.38 | 0 | 5 |
| python | daemon | 8 | off | 2,590 | 1.26 | 45.05 | 0 | 44 | 1,071 | 3.96 | - | 0 | 5 |
| python | daemon | 8 | on | 3,091 | 1.37 | 32.46 | 0 | 27 | 724 | 3.96 | 9.47 | 0 | 5 |
| python | embedded | 1 | off | 1,194 | 0.47 | 7.31 | 0 | 9 | 338 | 3.08 | - | 0 | 5 |
| python | embedded | 1 | on | 1,533 | 0.43 | 8.08 | 0 | 7 | 441 | 3.87 | 7.00 | 0 | 5 |
| python | embedded | 2 | off | 1,310 | 0.62 | 21.75 | 0 | 368 | 764 | 3.96 | - | 0 | 5 |
| python | embedded | 2 | on | 3,188 | 0.41 | 37.07 | 0 | 308 | 854 | 3.94 | 2.72 | 0 | 5 |
| python | embedded | 4 | off | 2,046 | 1.51 | 118.48 | 0 | 2,007 | 1,303 | 3.96 | - | 0 | 5 |
| python | embedded | 4 | on | 3,202 | 0.94 | 80.81 | 0 | 2,212 | 1,460 | 3.99 | 3.66 | 0 | 5 |
| python | embedded | 8 | off | 3,280 | 1.53 | 134.37 | 0 | 8,158 | 2,839 | 4.21 | - | 0 | 5 |
| python | embedded | 8 | on | 3,369 | 1.65 | 250.99 | 0 | 8,023 | 2,901 | 4.01 | 12.95 | 0 | 5 |

### S5 paired ratios

| Comparison | Blocks | Ratio (median) | 95% CI |
| --- | ---: | ---: | --- |
| node, 1 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **1.34** | 0.68–23.40 |
| node, 1 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.09** | 0.01–3.75 |
| node, 2 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **1.08** | 0.71–2.77 |
| node, 2 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.31** | 0.04–1.20 |
| node, 4 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **0.75** | 0.62–1.58 |
| node, 4 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.30** | 0.07–0.41 |
| node, 8 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **0.91** | 0.30–1.61 |
| node, 8 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.17** | 0.10–0.88 |
| python, 1 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **1.10** | 0.89–3.32 |
| python, 1 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.57** | 0.17–2.41 |
| python, 2 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **1.79** | 0.49–8.95 |
| python, 2 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.45** | 0.05–1.25 |
| python, 4 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **1.02** | 0.22–2.06 |
| python, 4 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.54** | 0.09–0.76 |
| python, 8 procs: throughput daemon/embedded (>1: daemon faster) | 5 | **0.79** | 0.71–1.10 |
| python, 8 procs: commit p99 daemon/embedded (<1: daemon lower) | 5 | **0.26** | 0.07–0.55 |
| node embedded, 1 procs: throughput with/without inspector | 5 | **0.82** | 0.66–35.95 |
| node embedded, 8 procs: throughput with/without inspector | 5 | **1.07** | 0.48–2.01 |
| node daemon, 1 procs: throughput with/without inspector | 5 | **1.55** | 0.11–1.96 |
| node daemon, 8 procs: throughput with/without inspector | 5 | **1.23** | 0.51–3.57 |
| python embedded, 1 procs: throughput with/without inspector | 5 | **1.12** | 0.22–6.74 |
| python embedded, 8 procs: throughput with/without inspector | 5 | **1.15** | 1.03–1.80 |
| python daemon, 1 procs: throughput with/without inspector | 5 | **0.90** | 0.29–9.64 |
| python daemon, 8 procs: throughput with/without inspector | 5 | **1.10** | 0.22–2.45 |
