"""What a Collie engine lacks, and a browser build it cannot fetch.

On the Scripted engine as a Collie engine (ADRs 0087, 0090, 0095, 0098 and
0115), three ways: async on asyncio, async on Trio and through the sync
`Collie`. A Foxhound operation or setting raises `NotInProductError`, naming both
Products and Foxhound's install command on the engine's channel, and a page
whose build is missing raises the provisioning code's class, naming the
build, the Product version that pins it and its `browser install`.
TypeScript's `product-and-provisioning-errors.test.ts` is its twin.
"""

import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_collie import (
    ArtifactDownloadFailedError,
    ArtifactUnavailableOfflineError,
    BrowserNotInstalledError,
    NotInProductError,
    RefusedError,
    UnavailableError,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

BUILD = {
    "kind": "browser",
    "name": "chrome-for-testing",
    "build": "141.0.7390.54-linux64",
}
"""The build every refused page needs."""

PINNED_BY = {"product": "collie", "product_version": "0.0.0"}
"""The Product version that pins it."""

CACHE = "/srv/goodfetch/browsers"
"""The Browser cache the engine reads."""

INSTALL = "pip install goodfetch-foxhound"
"""Foxhound's install command on the engine's channel, PyPI."""


@pytest.mark.covers(
    "story:11-10",
    "story:11-13",
    "story:11-189",
    "story:11-294",
    "error:not_in_product",
    "error:browser_not_installed",
    "error:artifact_unavailable_offline",
    "error:artifact_download_failed",
)
@pytest.mark.client_case(id="product-and-provisioning-errors", mode="attached")
@three_ways
async def test_raises_not_in_product_and_each_missing_builds_class(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """For Foxhound's operation and setting, then for each missing build."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/product-and-provisioning-errors.json"
    )
    collie = await open_collie(way, env)

    operation = await failure(
        collie.call("crawl.start", {"seeds": ["https://shop.test/"]})
    )
    setting = await failure(
        collie.call("run.start", {"settings": {"crawl": {"max_requests": 10}}})
    )
    run = await collie.start()
    not_installed = await failure(run.new_page())
    offline = await failure(run.new_page())
    download_failed = await failure(run.new_page())
    await run.finish()
    await collie.close()

    with client_case.check(
        "a Foxhound operation raises NotInProductError, an UnavailableError"
        " naming both Products and Foxhound's install command"
    ):
        assert isinstance(operation, NotInProductError)
        assert isinstance(operation, UnavailableError)
        assert (operation.code, operation.group, operation.retryable) == (
            "not_in_product",
            "unavailable",
            False,
        )
        assert operation.fix == (
            "Run it with Foxhound, which contains everything Collie does"
            f" (`{INSTALL}`), or connect to a Foxhound engine."
        )
        assert operation.details == {
            "product": "collie",
            "product_version": "0.0.0",
            "needs": "foxhound",
            "requested": "crawl.start",
            "install": INSTALL,
        }
    with client_case.check(
        "a Foxhound setting raises NotInProductError naming its key"
    ):
        assert isinstance(setting, NotInProductError)
        assert not setting.retryable
        assert setting.details["needs"] == "foxhound"
        assert setting.details["requested"] == "crawl.max_requests"
        assert setting.details["install"] == INSTALL
    with client_case.check(
        "a build absent from a cache the engine cannot write raises"
        " BrowserNotInstalledError, not retryable, naming the build, what pins"
        " it and its browser install"
    ):
        assert isinstance(not_installed, BrowserNotInstalledError)
        assert isinstance(not_installed, UnavailableError)
        assert (
            not_installed.code,
            not_installed.group,
            not_installed.retryable,
        ) == ("browser_not_installed", "unavailable", False)
        assert not_installed.fix == (
            "Run `goodfetch-collie browser install` where the Browser cache is"
            " writable, such as in the image's build, or point"
            " GOODFETCH_BROWSERS_PATH at a cache that holds it"
        )
        assert not_installed.details == {
            "missing": {
                "kind": "pinned",
                "artifact": BUILD,
                "pinned_by": PINNED_BY,
                "cache": CACHE,
            }
        }
    with client_case.check(
        "a download offline refuses raises ArtifactUnavailableOfflineError, a"
        " RefusedError whose limit names egress.offline"
    ):
        assert isinstance(offline, ArtifactUnavailableOfflineError)
        assert isinstance(offline, RefusedError)
        assert (offline.code, offline.group, offline.retryable) == (
            "artifact_unavailable_offline",
            "refused",
            False,
        )
        assert offline.fix == (
            "Run `goodfetch-collie browser install` beforehand where the Browser"
            " cache is writable, or lift `egress.offline` where it was set"
        )
        assert offline.limit is not None
        assert (offline.limit.key, offline.limit.value, offline.limit.layer) == (
            "egress.offline",
            True,
            "user_configuration",
        )
        assert offline.details == {
            "artifact": BUILD,
            "pinned_by": PINNED_BY,
            "cache": CACHE,
        }
    with client_case.check(
        "a failed download raises ArtifactDownloadFailedError, retryable,"
        " naming its host and failure"
    ):
        assert isinstance(download_failed, ArtifactDownloadFailedError)
        assert isinstance(download_failed, UnavailableError)
        assert (
            download_failed.code,
            download_failed.group,
            download_failed.retryable,
        ) == ("artifact_download_failed", "unavailable", True)
        assert download_failed.fix == (
            "Retry, or run `goodfetch-collie browser install` beforehand, or point"
            " GOODFETCH_BROWSERS_PATH at a prepared Browser cache"
        )
        assert download_failed.details == {
            "artifact": BUILD,
            "pinned_by": PINNED_BY,
            "host": "storage.googleapis.com",
            "failure": "http_status",
            "status": 404,
        }
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []
