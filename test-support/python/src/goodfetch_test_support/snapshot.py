"""The shared snapshot redactions and filters (ADR 0062).

They are read from `test-support/fixtures/snapshot-filters.json`, the one
list insta and Vitest apply too, so inline-snapshot's snapshots redact as
theirs do. A test asserts `redact(output, ...) == snapshot(...)`.
"""

import os
import re
from typing import cast

from goodfetch_test_support.schemas import shared_fixture


def redact(
    text: str,
    *,
    folder: str | os.PathLike[str] | None = None,
    product_version: str | None = None,
    commit: str | None = None,
) -> str:
    """`text` with the shared rules applied.

    The run's values go first, matched literally, then each committed
    pattern, in the file's order.

    Args:
        text: The output to snapshot.
        folder: The test's folder, `test_env.folder`.
        product_version: The Product version the test's executable reports.
        commit: The commit the test's executable reports.

    Returns:
        The redacted text.

    Raises:
        SchemaError: The committed rules fail their schema.
    """
    filters = cast(
        "dict[str, list[dict[str, str]]]",
        shared_fixture("snapshot-filters.json", "snapshot-filters.schema.json"),
    )
    values = {
        "folder": None if folder is None else os.fspath(folder),
        "product_version": product_version,
        "commit": commit,
    }
    redacted = text
    for rule in filters["values"]:
        value = values[rule["value"]]
        if value is not None:
            redacted = redacted.replace(value, rule["replacement"])
    for rule in filters["patterns"]:
        redacted = re.sub(rule["pattern"], rule["replacement"], redacted)
    return redacted
