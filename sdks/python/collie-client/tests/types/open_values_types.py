"""A newer engine's values (ADR 0079), checked by pyright and mypy.

An open enumeration's alias types any value this package does not know, so a
Verdict a newer engine adds, or a Usage kind of Foxhound's own (ADR 0095),
fits where Collie's alias is expected. TypeScript's twin is the `toExtend`
test of Foxhound's `tests/open-values.test.ts`.
"""

from goodfetch_collie_client.generated.open_values import Verdict
from goodfetch_collie_client.generated.shared_types import UsageKind

newer_verdict: Verdict = "a_newer_verdict"
foxhounds_usage: UsageKind = "paid_fetch"
