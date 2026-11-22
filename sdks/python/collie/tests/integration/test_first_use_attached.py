"""A Connection's first page on an empty Browser cache, on the real engine.

The engine is Collie's executable of the same commit (#3100), three ways:
async on asyncio, async on Trio and through the sync `Collie` (the download
not yet on Trio, #3372). Its Browser
cache is empty and its `browser.mirror` is the Proving ground's
`browser-builds.test`, never the internet (`goodfetch-test-env
--browser-download`): the first page downloads the pinned build and opens,
and a cache the engine cannot write, `egress.offline` or a mirror that
answers 404 raises `BrowserNotInstalledError`,
`ArtifactUnavailableOfflineError` or `ArtifactDownloadFailedError`, while
`connect()` succeeds; the cache the engine cannot write is the
`browser-cache-not-writable` case's, on Unix.
`test_product_and_provisioning_errors.py` plays the same case on the
Scripted engine; TypeScript's
`product-and-provisioning-errors.test.ts` is its twin.
"""

import json
import tomllib
import urllib.request
from collections.abc import Mapping
from pathlib import Path
from typing import cast

import anyio
import pytest
from collie_calls import failure, open_collie
from goodfetch_collie import (
    ArtifactDownloadFailedError,
    ArtifactUnavailableOfflineError,
    BrowserNotInstalledError,
    GoodfetchError,
    RefusedError,
    RunSettings,
    UnavailableError,
)
from goodfetch_test_support import (
    WORKSPACE,
    BrowserDownload,
    ClientCaseRecord,
    ProvingGround,
    Way,
    proving_ground_routes,
    three_ways,
)

PINNED_BUILD = cast(
    "dict[str, dict[str, str]]",
    json.loads(
        (WORKSPACE / "crates/goodfetch-collie/src/pins.json").read_text("utf-8")
    ),
)["chrome_for_testing"]["build"]
"""The Chrome for Testing build Collie pins, from its pins."""

BUILD = {"kind": "browser", "name": "chrome-for-testing", "build": PINNED_BUILD}
"""The pinned build, as each provisioning error names it."""

ENGINE_VERSION = cast(
    "dict[str, dict[str, dict[str, str]]]",
    tomllib.loads((WORKSPACE / "Cargo.toml").read_text("utf-8")),
)["workspace"]["package"]["version"]
"""The real engine's Product version, the workspace's version."""

PINNED_BY = {"product": "collie", "product_version": ENGINE_VERSION}
"""The Product version that pins it."""

DESCRIBED = (
    f"the browser build chrome-for-testing {PINNED_BUILD}, pinned by Collie"
    f" {ENGINE_VERSION}"
)
"""The build and what pins it, as each provisioning error's message names them."""

BUILDS_HOST = "browser-builds.test"
"""The Proving ground's host that serves the pinned builds, the `browser.mirror`."""

DOWNLOAD_FIX = (
    "Retry, or run `goodfetch-collie browser install` beforehand, or point"
    " GOODFETCH_BROWSERS_PATH at a prepared Browser cache"
)
"""What a failed download's error says to do, whatever failed."""

CASE = pytest.mark.client_case(id="product-and-provisioning-errors", mode="attached")
"""The case each test plays, and its mode."""


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground whose `browser-builds.test` is the engine's mirror."""
    return 3100


def cache_of(env: Mapping[str, str]) -> str:
    """The empty Browser cache `goodfetch-test-env` points the engine at."""
    return env["GOODFETCH_BROWSERS_PATH"]


def task_start_url(proving_ground: ProvingGround, task: str) -> str:
    """The URL the Proving ground's task `task` starts at, from its task list."""
    tasks = f"http://{proving_ground.address}{proving_ground_routes().tasks}"
    with urllib.request.urlopen(tasks) as answer:
        listed = cast("dict[str, list[dict[str, str]]]", json.load(answer))
    return next(entry["start_url"] for entry in listed["tasks"] if entry["id"] == task)


async def first_page_failure(
    way: Way, env: Mapping[str, str], settings: RunSettings | None = None
) -> GoodfetchError:
    """What the first page of a Run started with `settings` raises.

    Once `connect()` and the Run's start succeeded.
    """
    collie = await open_collie(way, env)
    run = await collie.start(settings=settings)
    failed = await failure(run.new_page())
    await run.finish()
    await collie.close()
    return failed


@pytest.mark.covers("story:11-13", "setting:browser.mirror")
@pytest.mark.parametrize("browser_download", [BrowserDownload.SERVED])
@CASE
# Not on Trio until #3372: Trio starts the engine from a worker thread that
# ends after 10 idle seconds, and on Linux the engine's parent-death signal
# follows that thread, so it dies mid-download.
@pytest.mark.parametrize(
    ("anyio_backend", "way"),
    [
        pytest.param("asyncio", Way.ASYNCIO, id="asyncio"),
        pytest.param("asyncio", Way.SYNC, id="sync"),
    ],
)
@pytest.mark.anyio
async def test_downloads_the_pinned_build_from_the_mirror_and_opens_the_page(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """The first page downloads the build into `GOODFETCH_BROWSERS_PATH`."""
    start = await anyio.to_thread.run_sync(task_start_url, proving_ground, "statuses/1")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()
    navigated = await page.goto(start)
    await run.finish()
    await collie.close()

    with client_case.check("the first page opened on the downloaded build"):
        assert navigated.response is not None
        assert navigated.response.status == 200
    with client_case.check("the build is in the cache GOODFETCH_BROWSERS_PATH names"):
        entry = (
            Path(cache_of(env))
            / "browser"
            / "chrome-for-testing"
            / PINNED_BUILD
            / "entry.json"
        )
        assert entry.is_file()


@pytest.mark.covers("story:11-294", "error:browser_not_installed")
@pytest.mark.parametrize("browser_download", [BrowserDownload.READ_ONLY_CACHE])
@pytest.mark.client_case(id="browser-cache-not-writable", mode="attached")
@three_ways
async def test_a_cache_the_engine_cannot_write_raises_browser_not_installed(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Not retryable, naming the build, what pins it, the cache, `browser install`."""
    refused = await first_page_failure(way, env)
    cache = cache_of(env)

    with client_case.check("the real engine raises BrowserNotInstalledError, whole"):
        assert isinstance(refused, BrowserNotInstalledError)
        assert isinstance(refused, UnavailableError)
        assert (refused.code, refused.group, refused.retryable) == (
            "browser_not_installed",
            "unavailable",
            False,
        )
        assert str(refused) == (
            f"{DESCRIBED} is not in the Browser cache at {cache}, which this"
            " engine cannot write"
        )
        assert refused.fix == (
            "Run `goodfetch-collie browser install` where the Browser cache is"
            " writable, such as in the image's build, or point"
            " GOODFETCH_BROWSERS_PATH at a cache that holds it"
        )
        assert refused.details == {
            "missing": {
                "kind": "pinned",
                "artifact": BUILD,
                "pinned_by": PINNED_BY,
                "cache": cache,
            }
        }


@pytest.mark.covers(
    "story:11-294", "error:artifact_unavailable_offline", "setting:egress.offline"
)
@pytest.mark.parametrize("browser_download", [BrowserDownload.NOT_FOUND])
@CASE
@three_ways
async def test_egress_offline_raises_artifact_unavailable_offline(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """A refusal whose limit names the Run's `egress.offline`, before any download."""
    refused = await first_page_failure(way, env, {"egress": {"offline": True}})
    cache = cache_of(env)

    with client_case.check(
        "the real engine raises ArtifactUnavailableOfflineError, whole"
    ):
        assert isinstance(refused, ArtifactUnavailableOfflineError)
        assert isinstance(refused, RefusedError)
        assert (refused.code, refused.group, refused.retryable) == (
            "artifact_unavailable_offline",
            "refused",
            False,
        )
        assert str(refused) == (
            f"{DESCRIBED} is not in the Browser cache at {cache}, and"
            " `egress.offline` refuses its download"
        )
        assert refused.fix == (
            "Run `goodfetch-collie browser install` beforehand where the Browser"
            " cache is writable, or lift `egress.offline` where it was set"
        )
        assert refused.limit is not None
        assert (refused.limit.key, refused.limit.value, refused.limit.layer) == (
            "egress.offline",
            True,
            "run",
        )
        assert refused.details == {
            "artifact": BUILD,
            "pinned_by": PINNED_BY,
            "cache": cache,
        }


@pytest.mark.covers(
    "story:11-294", "error:artifact_download_failed", "setting:browser.mirror"
)
@pytest.mark.parametrize("browser_download", [BrowserDownload.NOT_FOUND])
@CASE
@three_ways
async def test_a_mirror_that_answers_404_raises_artifact_download_failed(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Retryable, naming the mirror's host and the status it answered."""
    refused = await first_page_failure(way, env)

    with client_case.check("the real engine raises ArtifactDownloadFailedError, whole"):
        assert isinstance(refused, ArtifactDownloadFailedError)
        assert isinstance(refused, UnavailableError)
        assert (refused.code, refused.group, refused.retryable) == (
            "artifact_download_failed",
            "unavailable",
            True,
        )
        assert str(refused) == (
            f"downloading {DESCRIBED} from {BUILDS_HOST} failed: {BUILDS_HOST}"
            " answered 404"
        )
        assert refused.fix == DOWNLOAD_FIX
        assert refused.details == {
            "artifact": BUILD,
            "pinned_by": PINNED_BY,
            "host": BUILDS_HOST,
            "failure": "http_status",
            "status": 404,
        }
