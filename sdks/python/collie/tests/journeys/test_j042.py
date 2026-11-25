"""J-042, nothing leaves the machine beyond the site and pinned downloads.

"Drive one page and see nothing leave the machine beyond its site and
pinned downloads", on the `sdk-python` surface (ADRs 0043 and 0063): a
program drives one product page of the catalog through the Python SDK, on
an Attached engine of Collie's executable whose Browser cache holds no
browser and with nothing configured to leave the machine, so the first page
downloads the pinned Chrome for Testing build from the Proving ground's
mirror. The Proving ground's record then holds the page's requests to its
site and that download, and nothing else. A connection to any other host
would leave that record, which CI's outbound deny refuses and each test
job's `check` step fails on (ADR 0043). It runs on asyncio, as `journey`
does. Rust's `tests/it/journeys/j042.rs` and TypeScript's
`tests/journeys/J-042.test.ts` are its twins.
"""

import re
from collections.abc import Mapping, Sequence
from typing import TypeVar, cast
from urllib.parse import urlsplit

import anyio.to_thread
from goodfetch_collie import AsyncCollie
from goodfetch_collie_client.generated.models import PageField
from goodfetch_test_support import BrowserDownload, JourneyRun, journey

BROWSER_BUILDS_HOST = "browser-builds.test"
"""The host the Proving ground serves the browser builds' mirror on."""

PINNED_ARCHIVE = re.compile(
    r"/\d+\.\d+\.\d+\.\d+/(linux64|mac-arm64|mac-x64|win64)/chrome-\1\.zip"
)
"""The pinned path of a Chrome for Testing build's archive."""

RECORD_PARTS = ("relays", "dns_queries", "udp_datagrams")
"""The parts of the connection record that list what reached it other than its sites."""

UNRECORDED = (
    "unrecorded_requests",
    "unrecorded_connections",
    "unrecorded_relays",
    "unrecorded_dns_queries",
    "unrecorded_udp_datagrams",
)
"""The counts of entries past the record's bound."""

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

Entry = Mapping[str, object]
"""An entry of the record: a request, a connection, a relay."""

_T = TypeVar("_T")


def present(value: _T | None) -> _T:
    """The value an earlier step gave: a step runs once those before it passed."""
    assert value is not None, "an earlier step gave it"
    return value


def entries(observations: Mapping[str, object], part: str) -> Sequence[Entry]:
    """The entries the observations list under `part`."""
    return cast("Sequence[Entry]", observations[part])


def download(request: Entry) -> dict[str, object]:
    """A request to the mirror, as the check reads it: its headers by name."""
    target = cast("str", request["target"])
    headers = cast("Sequence[Sequence[str]]", request["headers"])
    return {
        "method": request["method"],
        "target": "<the pinned archive>"
        if PINNED_ARCHIVE.fullmatch(target)
        else target,
        "over_tls": request.get("tls") is not None,
        "headers": sorted([name, value] for name, value in headers),
        "cookies": request["cookies"],
    }


def beyond_the_site(observations: Mapping[str, object], site: str) -> list[object]:
    """What the record holds beyond the page's requests to `site` and the downloads.

    Each request to another host, each connection a listener other than the
    sites' accepted, each relay, DNS query and UDP datagram, and each entry
    past the record's bound.
    """
    beyond: list[object] = [
        ["request", request["host"], request["target"]]
        for request in entries(observations, "requests")
        if request["host"] not in {site, BROWSER_BUILDS_HOST}
    ]
    beyond += [
        ["connection", connection]
        for connection in entries(observations, "connections")
        if connection["listener"] != "site"
    ]
    beyond += [
        [part, entry] for part in RECORD_PARTS for entry in entries(observations, part)
    ]
    beyond += [
        [count, observations[count]] for count in UNRECORDED if observations[count] != 0
    ]
    return beyond


@journey("J-042", "sdk-python", browser_download=BrowserDownload.SERVED)
async def test_j042(run: JourneyRun) -> None:
    """Covers: story:4-80, story:4-81, row:SC-10."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-042 has a Proving ground"
    product = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/2")
    port = proving_ground.address.rpartition(":")[2]
    collie = await AsyncCollie.connect(env=run.env)
    try:
        started = await run.step(
            "Start a run with nothing configured to leave the machine, "
            "on a Browser cache that holds no browser",
            collie.start,
        )
        page = await run.step(
            "Open a page, whose browser is downloaded on first use",
            lambda: present(started).new_page(),
        )
        await run.step(
            "Go to a product page of the site",
            lambda: present(page).goto(product),
        )
        read = await run.step(
            "Read the product's ID, name and price from the page",
            lambda: present(page).read(PRODUCT_FIELDS),
        )

        async def finish() -> Mapping[str, object]:
            await present(started).finish()
            return await anyio.to_thread.run_sync(proving_ground.observations)

        observations = await run.step("Finish the run", finish)
    finally:
        await collie.close()

    await run.expect_task(
        "The oracle verifies the product the page's fields answer",
        {"status": "success", "records": [] if read is None else [read.values]},
    )

    async def downloaded_once() -> None:
        assert observations is not None, "the run finished"
        downloads = [
            download(request)
            for request in entries(observations, "requests")
            if request["host"] == BROWSER_BUILDS_HOST
        ]
        assert downloads == [
            {
                "method": "GET",
                "target": "<the pinned archive>",
                "over_tls": True,
                "headers": [
                    ["host", f"{BROWSER_BUILDS_HOST}:{port}"],
                    ["user-agent", "goodfetch-download"],
                ],
                "cookies": [],
            }
        ]

    async def nothing_beyond_the_site() -> None:
        assert observations is not None, "the run finished"
        assert beyond_the_site(observations, urlsplit(product).hostname or "") == []

    await run.check(
        "The browser build was downloaded as one plain GET of its pinned path, "
        "with no cookie, no query and no header but its host and the user agent "
        "`goodfetch-download`",
        downloaded_once,
    )
    await run.check(
        "Every request the Proving ground received went to the product page's host "
        "or was that download: no telemetry, no update check and no browser "
        "background traffic",
        nothing_beyond_the_site,
    )
