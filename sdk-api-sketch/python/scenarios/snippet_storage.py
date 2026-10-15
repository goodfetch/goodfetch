"""Snippet for A disk that fills under a long Run, and log volume (#553, ADR 0122), on scenario 07's crawl."""

import logging

from goodfetch_foxhound import AsyncCrawl, AsyncCrawlRun, AsyncFoxhound, CrawlRunReport, ResourcesExhaustedError, seed

log = logging.getLogger(__name__)

books = AsyncCrawl()
home = books.label("home")


async def crawl_on_a_small_disk() -> CrawlRunReport | None:
    """A long crawl on a small disk: below `storage.critical_free` it stops resumably, never half-written, and this
    same program run again under its ID carries on once space is freed."""
    async with AsyncFoxhound.connect() as foxhound:
        try:
            run = await foxhound.start_crawl(
                books,
                seeds=[seed("https://books.toscrape.com/", home)],
                run_id="books-2026-10-15",
            )
            async with run:
                report = await run.finish()
                reason = report.stop_reason
                if reason is not None and reason.kind == "resources" and reason.resource == "storage":
                    log.warning("stopped before the disk filled: free space at %s, then run again", reason.path)
                for warning in report.warnings:
                    if warning.code == "storage_low":
                        log.warning("%s", warning.message)
                return report
        except ResourcesExhaustedError as error:
            # Only where no report says it: a start refused on a full filesystem, or a stop it could not record.
            log.error("storage full at %s (%s bytes free)", error.details.path, error.details.free_bytes)
            return None


async def watch_storage(run: AsyncCrawlRun) -> None:
    """While it runs: the worst storage level among the filesystems it writes to."""
    async for progress in run.progress():
        if progress.storage.level != "normal":
            log.warning("storage %s at %s: %s bytes free", progress.storage.level, progress.storage.path,
                        progress.storage.free_bytes)
