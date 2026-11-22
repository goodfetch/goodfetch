"""Foxhound's Python SDK, built on the Collie client package (ADR 0095).

This package, `goodfetch_foxhound`, holds `AsyncFoxhound` and the sync
`Foxhound`, which carry every member of Collie's Connection under the same
names and extend the Collie client package's `AsyncConnection` and
`Connection`. Its Runs, results and error classes are the client package's
own, `goodfetch_collie_client`'s, so a helper written for Collie takes them
and an `except` on a Collie error class catches them. `generated` holds the
layer `xtask codegen` writes from Foxhound's own entries of the catalog, and
`goodfetch_foxhound.testing` opens a Connection over a transport a test
holds.

The Connection reaches a Daemon or finds Foxhound's executable, reading
`GOODFETCH_URL`, `GOODFETCH_TOKEN`, `GOODFETCH_HOME` and
`GOODFETCH_FOXHOUND_ENGINE_PATH`, never Collie's variable. Importing it
reads, starts, patches and logs nothing (ADR 0078).
"""

from goodfetch_foxhound.foxhound import AsyncFoxhound, Connecting, Foxhound

__all__ = ["AsyncFoxhound", "Connecting", "Foxhound"]
