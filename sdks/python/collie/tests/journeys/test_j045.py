"""J-045, develop against the Response cache, then test offline from a recording.

"Develop against the Response cache, then test offline from a recording", on
the `sdk-python` surface (ADRs 0046, 0063 and 0105; ticket #808): a developer
runs an automation against a live Proving ground product page under
`cache.mode: "replay"`, re-runs it against the same Response cache with zero
requests to the Proving ground, records a Response recording, and runs it again
offline with `recording: { mode: "replay" }` with zero requests and no miss
listed in its Run report. Rust's `tests/it/journeys/j045.rs` and TypeScript's
`tests/journeys/J-045.test.ts` are its twins.
"""

from typing import TYPE_CHECKING, cast

import anyio.to_thread
from goodfetch_collie import AsyncCollie
from goodfetch_collie_client.generated.models import (
    PageField,
    Recording,
    RecordingMode,
    RunReport,
)
from goodfetch_collie_client.generated.settings import RunSettings
from goodfetch_test_support import JourneyRun, journey

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence

PRODUCT_FIELDS = {
    "id": PageField.model_validate({"locator": {"kind": "css", "selector": "span.id"}}),
    "name": PageField.model_validate(
        {"locator": {"kind": "css", "selector": "h1.name"}}
    ),
    "price": PageField.model_validate(
        {
            "locator": {"kind": "css", "selector": "span.price"},
            "attribute": "data-price",
            "transform": {"kind": "number", "locale": "en"},
        }
    ),
}
"""The product's ID, name and price, as task `catalog/2` names them."""


async def run_program(
    collie: AsyncCollie,
    url: str,
    *,
    settings: RunSettings | None = None,
    recording: Recording | None = None,
) -> tuple[dict[str, object], RunReport]:
    """Runs the program once, returning the product values and Run report."""
    started = await collie.start(settings=settings, recording=recording)
    page = await started.new_page()
    await page.goto(url)
    read = await page.read(PRODUCT_FIELDS)
    report = await started.finish()
    return dict(read.values), report


@journey("J-045", "sdk-python")
async def test_j045(run: JourneyRun) -> None:
    """Develop against cache replay, then record and replay offline."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-045 has a Proving ground"
    product_url = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/2")
    warc_path = str(run.test_env.folder / "product.warc")
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    live_hosts = list(proving_ground.resolve)

    async def request_count() -> int:
        observations = await anyio.to_thread.run_sync(proving_ground.observations)
        requests = cast("Sequence[Mapping[str, object]]", observations["requests"])
        return len(requests)

    first_values: dict[str, object] | None = None
    second_values: dict[str, object] | None = None
    offline_values: dict[str, object] | None = None
    offline_report: RunReport | None = None
    requests_after_first = 0
    requests_after_second = 0
    requests_after_record = 0
    requests_after_offline = 0

    collie = await AsyncCollie.connect(env=run.env)
    try:

        async def first_run() -> None:
            nonlocal first_values, requests_after_first
            first_values, _ = await run_program(
                collie,
                product_url,
                settings={
                    "network": {"resolve": resolve},
                    "egress": {"private_ranges": live_hosts},
                    "cache": {"mode": "replay"},
                },
            )
            requests_after_first = await request_count()

        async def second_run() -> None:
            nonlocal second_values, requests_after_second
            second_values, _ = await run_program(
                collie,
                product_url,
                settings={
                    "network": {"resolve": resolve},
                    "egress": {"private_ranges": live_hosts},
                    "cache": {"mode": "replay"},
                },
            )
            requests_after_second = await request_count()

        async def record_run() -> None:
            nonlocal requests_after_record
            await run_program(
                collie,
                product_url,
                settings={
                    "network": {"resolve": resolve},
                    "egress": {"private_ranges": live_hosts},
                },
                recording=Recording(mode=RecordingMode.record, path=warc_path),
            )
            requests_after_record = await request_count()

        async def offline_run() -> None:
            nonlocal offline_values, offline_report, requests_after_offline
            offline_values, offline_report = await run_program(
                collie,
                product_url,
                recording=Recording(mode=RecordingMode.replay, path=warc_path),
            )
            requests_after_offline = await request_count()

        await run.step(
            "Run the program against a product page of the site under cache replay",
            first_run,
        )
        await run.step(
            "Run the same program a second time under cache replay "
            "on the same Response cache",
            second_run,
        )
        await run.step(
            "Record a Response recording of the program against the site",
            record_run,
        )
        await run.step(
            "Run the same program offline from the Response recording",
            offline_run,
        )
    finally:
        await collie.close()

    await run.expect_task(
        "The oracle verifies the product the first run reads",
        {
            "status": "success",
            "records": [] if first_values is None else [first_values],
        },
    )
    await run.expect_task(
        "The oracle verifies the product the second run reads from the Response cache",
        {
            "status": "success",
            "records": [] if second_values is None else [second_values],
        },
    )

    async def second_run_sent_no_request() -> None:
        assert requests_after_first > 0
        assert requests_after_second == requests_after_first

    await run.check(
        "The second run under cache replay sends the Proving ground no request",
        second_run_sent_no_request,
    )
    await run.expect_task(
        "The oracle verifies the product the offline run reads "
        "from the Response recording",
        {
            "status": "success",
            "records": [] if offline_values is None else [offline_values],
        },
    )

    async def offline_run_sent_no_request_and_no_miss() -> None:
        assert requests_after_record > requests_after_second
        assert requests_after_offline == requests_after_record
        assert offline_report is not None
        assert offline_report.recording is not None
        assert offline_report.recording.mode == "replay"
        assert offline_report.recording.responses_used > 0
        assert (offline_report.recording.misses or []) == []

    await run.check(
        "The offline run from the Response recording sends the Proving ground "
        "no request and lists no miss in its Run report",
        offline_run_sent_no_request_and_no_miss,
    )
