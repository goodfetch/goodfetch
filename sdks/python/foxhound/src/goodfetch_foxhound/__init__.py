"""Foxhound's Python SDK, built on the Collie client package (ADR 0095).

Where the main types live (ADR 0117): `goodfetch_collie` holds
`AsyncCollie` and the sync `Collie`; the Collie client package,
`goodfetch_collie_client`, holds `AsyncRun` and `Run`, `AsyncSession` and
`Session`, `AsyncPage` and `Page`, `AsyncLocator` and `Locator`, with the
results and the errors; this package, `goodfetch_foxhound`, holds
`AsyncFoxhound` and `Foxhound`, and a crawl's `Crawl`, `Label`, `Dataset`
and `CrawlContext` belong to it.

This package, `goodfetch_foxhound`, holds `AsyncFoxhound` and the sync
`Foxhound`, which carry every member of Collie's Connection under the same
names and extend the Collie client package's `AsyncConnection` and
`Connection`. Its Runs, results and error classes are the client package's
own, `goodfetch_collie_client`'s, so a helper written for Collie takes them
and an `except` on a Collie error class catches them. It re-exports
`ExperimentalWarning`, the category of an Experimental member's warning
(ADR 0030), which a program's filter names. `generated` holds the
layer `xtask codegen` writes from Foxhound's own entries of the catalog, and
`goodfetch_foxhound.testing` opens a Connection over a transport a test
holds.

The Connection reaches a Daemon or finds Foxhound's executable, reading
`GOODFETCH_URL`, `GOODFETCH_TOKEN`, `GOODFETCH_HOME` and
`GOODFETCH_FOXHOUND_ENGINE_PATH`, never Collie's variable. Importing it
reads, starts, patches and logs nothing (ADR 0078).
"""

from goodfetch_collie_client.markers import ExperimentalWarning

from goodfetch_foxhound.foxhound import AsyncFoxhound, Connecting, Foxhound

__all__ = ["AsyncFoxhound", "Connecting", "ExperimentalWarning", "Foxhound"]
