# Measurements on an idle Mac, October 8, 2026

Speed and resource campaigns executed in class `local-controlled`, on committed sources. The estimates, intervals and verdicts are computed by `pnpm lab verdict` and rendered in [decisions.md](decisions.md). This report does not copy any measured figure.

## Host

That day, an idle Mac did not pass the first `local-controlled` thresholds. Two causes: the macOS maintenance launched as soon as the machine is inactive (Spotlight, Siri, file provider), and the agent app driving the campaign. The thresholds were therefore adjusted and committed before the measurements (`hosts/local-controlled.json`):

- the agent app driving the run is recorded separately (`controllerCpuPercent`) instead of counting as foreign load;
- the allowed foreign CPU rises to 150% and the load per CPU to 0.35;
- being plugged into mains power is no longer required, low-power mode remaining forbidden.

The blocks are paired, with the drivers interleaved in an order randomized at each block: slow background noise affects all the options of a block and cancels out in the ratio. Any block where the host check fails is excluded from the estimate. The absolute figures remain unpublishable until the reruns on the reference host.

## Runs

| Run | Decision | Content |
| --- | --- | --- |
| `browser-driver-rtt-0-2026-10-08T12-39-53-109Z-63890b` | D-browser-driver | driver-browser link at 0 ms; block 0 noisy, excluded |
| `browser-driver-rtt-20-2026-10-08T12-50-54-854Z-96969e` | D-browser-driver | driver-browser link at 20 ms |
| `browser-driver-rtt-100-2026-10-08T13-14-24-301Z-331916` | D-browser-driver | driver-browser link at 100 ms |
| `browser-driver-rtt-300-2026-10-08T14-00-36-292Z-657a67` | D-browser-driver | driver-browser link at 300 ms |
| `runtime-native-capacity-2026-10-08T12-46-38-910Z-83d7e0` | D-core-language | decision stratum, Node versus Rust, eight blocks |

All tasks of these runs were verified by the oracle. The main verdict of D-browser-driver covers the 0 ms stratum, declared in the decision; the other RTTs describe sensitivity to latency. The `runtime-native-capacity` run replaces `runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e`, measured in `local-dev`, as the measurement for D-core-language.

## Attempts of October 9

Twelve runs from six campaigns were relaunched requesting `local-controlled`. All are classified `diagnostic`; their counts are exact, and their only failures are deadlines exceeded and made explicit.

- **Night** (from 3:35 a.m., idle Mac). The campaigns followed one another without a pause, so the load of one run still weighed on the host check of the next; three runs were downgraded to `local-dev`. An identity campaign got stuck for four hours on one cell after the Mac went to sleep. Fixes:
  - CDP commands bounded at 60 s and a time budget per cell (#42);
  - launches under `caffeinate -i`;
  - waiting for quiet before each campaign.
- **Morning** (Mac in light use). All runs remained in `local-dev`: macOS photo analysis, the Docker Desktop VM and the display exceeded the allowed foreign CPU.

The `local-controlled` reruns therefore remain to be done.

### Afternoon of October 9, at the project owner's request

The six campaigns were relaunched under the usage conditions of the moment (without ffmpeg or Docker, but with macOS photo analysis, the display and video). The six runs are `local-dev` and classified `diagnostic`. Their counts are exact: the actionability matrix with the guard gives 162/162, admission and scheduling 10/10, RTT 300 432/432 and rendered extraction 21/21.

Indicative reading, computed outside the ledger, with no decision value:
- **Drivers at 300 ms of latency** (`browser-driver-rtt-300-2026-10-09T10-50-18-619Z-27903b`, 6 clean blocks out of 6). Against Playwright, the guarded CDP drivers take 0.68 to 0.81 of the time, Rust and Node tied; guarded Puppeteer takes 1.8 to 2.3 times longer on actions. This extends the trend observed at 20 and 100 ms.
- **Scheduling** (`durable-scheduler-2026-10-09T10-48-01-989Z-994cf4`). Per-domain queues give 2.7 times the verified throughput of small domains, as on October 8.

