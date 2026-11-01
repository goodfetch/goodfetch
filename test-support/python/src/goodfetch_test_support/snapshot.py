"""The shared snapshot redactions and filters (ADR 0062).

They are read from `test-support/fixtures/snapshot-filters.json`, the one
list insta and Vitest apply too, so inline-snapshot's snapshots redact as
theirs do. A test asserts `redact(output, ...) == snapshot(...)`.
"""

import json
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

    The run's values go first, matched literally (the folder in each form an
    output prints it in), then each committed pattern, in the file's order. A
    word boundary is ASCII's, as in Rust and JavaScript.

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
        if value is None:
            continue
        forms = _folder_forms(value) if rule["value"] == "folder" else [value]
        for form in forms:
            redacted = redacted.replace(form, rule["replacement"])
    for rule in filters["patterns"]:
        redacted = re.sub(
            rule["pattern"], rule["replacement"], redacted, flags=re.ASCII
        )
    return redacted


def _folder_forms(folder: str) -> list[str]:
    """The forms an output prints a folder in, longest first.

    The path, its canonical path when it has another, and each written
    inside a JSON string as the engine writes one, so a longer form is
    replaced before a shorter one inside it.
    """
    paths = [folder, os.path.realpath(folder)]
    forms = {
        form
        for path in paths
        for form in (path, json.dumps(path, ensure_ascii=False)[1:-1])
    }
    return sorted(forms, key=lambda form: (-len(form), form))
