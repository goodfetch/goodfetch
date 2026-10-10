# Every costly safety or reliability control can be switched off, and the results say so

ADR 0016 made the costly security controls switchable. At the owner's request the same rule now covers reliability: a control that costs speed or resources is on by default and can be switched off per run, or per call where that makes sense. Each switch-off is recorded in the Audit log, and the results it touches say what was skipped, so the engine never claims a guarantee it did not apply. Controls with no run-time cost stay locked, as in ADR 0016: secrets never reach the model, endpoints require a token, releases are signed. Decided in [Adopt site-access decisions](https://github.com/Jbmanllr/automan/issues/76).

## Considered Options

- **Fixed guarantees**: rejected. Users trade checks for speed on pages they trust: Playwright skips its checks with `force: true`, and Stagehand v4 checks nothing before acting; in the lab, native Stagehand ran a 50-click task in 945 ms against 2,715 ms for native Playwright ([qualification](https://github.com/Jbmanllr/automan/blob/main/rnd/lab/reports/qualification-2026-10-07.md)).
- **Switches without marks in the results**: rejected. A fast run and a checked run would look the same, which is the false success ADR 0013 exists to prevent.

## Consequences

| Control | When switched off |
| --- | --- |
| Actionability guard (ADR 0018) | actions are marked unguarded in Provenance |
| Page check (ADR 0006) | pages are reported as unchecked |
| Sample comparison of fetch tiers (ADR 0019) | Provenance records the forced tier |
| Browser-fetch for identity consistency (ADR 0019) | the Session's HTTP goes through reqwest or wreq, and Provenance records the identity change |
| Backoff on 429, 503 and `Retry-After` (ADR 0020) | the run records that it ignored them |
| Target fingerprint check before the first Effect (ADR 0009) | Effects are marked unverified in the Effect journal |
| Success checks (ADR 0013) | results are `needs-review`, never a success |

- The V1 benchmark publishes what each control costs, as it does for the security controls of ADR 0016.
- Later decisions that add a costly control state its switch and its mark.
