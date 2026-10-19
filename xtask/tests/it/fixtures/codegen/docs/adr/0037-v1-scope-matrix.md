# V1 scope matrix

## Status counts

| Status | Parity rows | Pains |
|---|---:|---:|
| `decided` | 2 | 1 |
| `excluded` | 1 | 0 |

## Parity rows

| ID | Capability | Status | Source | Spec area | Note |
|---|---|---|---|---|---|
| RT-01 | Concurrency autoscaled | `decided` | ADR 0040 | 3 Engine runtime | One governor. |
| RT-02 | A request cap | `spec` | ADR 0004 | 3 Engine runtime; 11 Client surfaces | Spec. |
| RT-03 | A replaceable store | `excluded` | ADR 0002 | 7 Plugin protocol | Excluded. |

## Pains

| Rank | Pain | Status | Source | Spec areas | Note |
|---:|---|---|---|---|---|
| 1 | `durability-resume` | `decided` | ADR 0002 | 3 Engine runtime | Per-run SQLite. |
