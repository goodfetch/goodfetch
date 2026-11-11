# Every costly safety or reliability control can be switched off, and the results say so

ADR 0016 made the costly security controls switchable. At the owner's request the same rule now covers reliability: a control that costs speed or resources is on by default and can be switched off per run, or per call where that makes sense. Each switch-off is recorded in the Audit log, and the results it touches say what was skipped, so the engine never claims a guarantee it did not apply. Controls with no run-time cost stay locked, as in ADR 0016: secrets never reach the model, endpoints require a token, releases are signed. Decided in [Adopt site-access decisions](https://github.com/goodfetch/goodfetch/issues/76).

## Considered Options

- **Fixed guarantees**: rejected. Users trade checks for speed on pages they trust: Playwright skips its checks with `force: true`, and Stagehand v4 checks nothing before acting; in the lab, native Stagehand ran a 50-click task in 945 ms against 2,715 ms for native Playwright ([qualification](https://github.com/goodfetch/goodfetch/blob/main/rnd/lab/reports/qualification-2026-10-07.md)).
- **Switches without marks in the results**: rejected. A fast run and a checked run would look the same, which is the false success ADR 0013 exists to prevent.

## Consequences

| Control | When switched off |
| --- | --- |
| Actionability guard (ADR 0019) | actions are marked unguarded in Provenance |
| Page check (ADR 0006) | pages are reported as unchecked |
| Sample comparison of fetch tiers (ADR 0020) | Provenance records the forced tier |
| Browser-fetch for identity consistency (ADR 0020) | the Session's HTTP goes through reqwest or wreq, and Provenance records the identity change |
| Backoff on 429, 503 and `Retry-After` (ADR 0021) | the run records that it ignored them |
| Target fingerprint check before the first Effect (ADR 0009) | Effects are marked unverified in the Effect journal |
| Success checks (ADR 0013) | results are `needs-review`, never a success |

> Amended by [ADR 0086](0086-a-crawl-obeys-robots-txt-and-sends-one-request-at-a-time-per-domain-by-default-and-pressing-harder-is-recorded.md): the backoff row also covers 502, 504 and connections refused or timed out, and switching the backoff off, or shortening its waits, is recorded in the Audit log and the run's result.

> Amended by [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md): the sample comparison row's switch is the setting `fetch.tier`, which forces a tier at any scope down to a crawl's Label, besides per run.

> Note (agent decision, pending owner review), 2026-11-06, [#1577](https://github.com/goodfetch/goodfetch/issues/1577) ([ADR 0138](0138-a-page-is-analysed-once-costly-features-are-settings-and-run-state-commits-in-groups-at-a-chosen-durability-level.md)): switching a control off removes its work as well as its effect. As built, the table gains these rows:
>
> | Control | When switched off |
> | --- | --- |
> | Durability of run state (ADR 0002), `run_state.durability` below `full` | the run report's `weakened` and `config explain` name the level: `fast` and `periodic` may lose recent commits to a power cut, and `off` discards the run file a crash leaves ([#1927](https://github.com/goodfetch/goodfetch/issues/1927)) |
> | Learned URL rules and the Page signature (ADR 0005), `frontier.learning: false` | no rule is learned, so the Run report's `learned_rules` is empty, with the warning `learning_off` ([#1909](https://github.com/goodfetch/goodfetch/issues/1909)) |
> | Foxhound's Provenance log (ADR 0047), `provenance.log: false` | the Run trace holds no Provenance log, with the warning `provenance_off`; each Record keeps its own Provenance ([#1921](https://github.com/goodfetch/goodfetch/issues/1921)) |
> | Foxhound's Records (ADR 0103), `dataset.records: false` | no Record is kept: each succeeded request's outcome says `records_off`, with the warning `records_off` ([#1921](https://github.com/goodfetch/goodfetch/issues/1921)) |
> | Foxhound's Block classification (ADR 0035), `fetch.block_classification: false` | no request moves to the browser for a Challenge or ends `blocked`, with the warning `block_classification_off` ([#1921](https://github.com/goodfetch/goodfetch/issues/1921)) |
>
> Each of the last four is recorded in the Audit log, `frontier.learning` as `learning_switched_off` and the other three as `feature_switched_off`, with its value and its default, and listed in the report's `weakened`. A Durability level below `full` writes no Audit log entry.

- ADR 0028 lets an Operator policy or a Recipe cap or lock these switches; where nothing locks them, a run can still switch them.
- The V1 benchmark publishes what each control costs, as it does for the security controls of ADR 0016.
  > Note (owner's decisions, 2026-11-10, grilling of decision G-08 of the consolidated review: Q2 and Q4 answered "yes", each on the grilling's recommended option as put; the wording is the grilling session's, and the owner did not read it): `just compare`'s report states each control's cost before the benchmark does. For Foxhound, a table of the instructions per page each default adds when switched on alone. For Collie, the gap between its Guard-on and Guard-off rows on the same tasks.
- Later decisions that add a costly control state its switch and its mark.
