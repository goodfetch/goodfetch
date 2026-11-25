"""Foxhound's testing kit (ADRs 0069 and 0090).

Where the main types live (ADR 0117): `goodfetch_collie` holds
`AsyncCollie` and the sync `Collie`; the Collie client package,
`goodfetch_collie_client`, holds `AsyncRun` and `Run`, `AsyncSession` and
`Session`, `AsyncPage` and `Page`, `AsyncLocator` and `Locator`;
`goodfetch_foxhound` holds `AsyncFoxhound` and `Foxhound`, `Crawl`,
`Label`, `Dataset` and `CrawlContext`.

Opens Foxhound's Connection over a transport the test holds, such as the
Scripted engine's in-memory player, so a program's tests run its code with
no engine process: the same Connection as `connect()`'s. Importing it reads,
patches and logs nothing (ADR 0078). `goodfetch_collie.testing` and
TypeScript's `@goodfetch/foxhound/testing` are its twins.
"""

from goodfetch_foxhound.foxhound import async_connect_over, connect_over

__all__ = ["async_connect_over", "connect_over"]
