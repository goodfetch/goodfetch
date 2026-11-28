"""Browser setup as typed `SessionSettings` where a Session opens (ADRs 0083, 0121).

- Scripted engine (attached and daemon, three ways): all 12 browser setup keys
  serialize in their exact wire shape (`session-browser-setup-settings`).
- Proving ground (attached, three ways): viewport, device scale factor, locale,
  time zone, colour scheme, permissions and network throttle are observed on a
  real Proving ground page (`session-browser-setup-observed-on-a-page`).
- No `BrowserContext` exists in `goodfetch_collie`'s public items.
"""

from collections.abc import Mapping

import anyio.to_thread
import goodfetch_collie
import pytest
from collie_calls import engine_warnings, exit_code, failure, open_collie
from goodfetch_collie import SessionSettings
from goodfetch_test_support import (
    ClientCaseRecord,
    ProvingGround,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

ALL_TWELVE_SETTINGS: SessionSettings = {
    "browser": {
        "viewport": {"width": 800, "height": 600},
        "device_scale_factor": 1.5,
        "locale": "fr-FR",
        "timezone_id": "Europe/Paris",
        "color_scheme": "dark",
        "java_script_enabled": False,
        "geolocation": {"latitude": 48.8566, "longitude": 2.3522, "accuracy": 10.5},
        "permissions": ["geolocation"],
        "headless": True,
        "block_resources": ["image"],
        "service_workers": "block",
    },
    "network": {
        "throttle": {
            "offline": False,
            "latency": 40,
            "download_bytes_per_second": 500_000,
            "upload_bytes_per_second": 250_000,
        }
    },
}

COVERS_ALL = pytest.mark.covers(
    "story:11-89",
    "setting:browser.viewport",
    "setting:browser.device_scale_factor",
    "setting:browser.locale",
    "setting:browser.timezone_id",
    "setting:browser.color_scheme",
    "setting:browser.java_script_enabled",
    "setting:browser.geolocation",
    "setting:browser.permissions",
    "setting:browser.headless",
    "setting:browser.block_resources",
    "setting:browser.service_workers",
    "setting:network.throttle",
)


@COVERS_ALL
@pytest.mark.client_case(id="session-browser-setup-settings", mode="attached")
@three_ways
async def test_browser_setup_settings_attached(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """All 12 browser setup keys serialize in their wire shape on an Attached engine."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/session-browser-setup-settings.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start()
    session = await run.new_session(settings=ALL_TWELVE_SETTINGS)
    await session.close()
    await run.finish()
    await collie.close()

    with client_case.check("all 12 browser setup keys matched the script wire shape"):
        assert engine_warnings(caplog) == []
    with client_case.check("no BrowserContext exists in the SDK's public items"):
        assert "BrowserContext" not in goodfetch_collie.__all__
        assert not hasattr(goodfetch_collie, "BrowserContext")


@COVERS_ALL
@pytest.mark.client_case(id="session-browser-setup-settings", mode="daemon")
@three_ways
async def test_browser_setup_settings_daemon(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
) -> None:
    """All 12 browser setup keys serialize in their wire shape against a Daemon."""
    daemon = start_scripted_daemon(
        test_env,
        "sdk-python/run/session-browser-setup-settings-from-a-daemon.json",
        "socket",
    )
    collie = await open_collie(way, test_env.env, engine="daemon")
    run = await collie.start()
    session = await run.new_session(settings=ALL_TWELVE_SETTINGS)
    await session.close()
    await run.finish()
    await collie.close()
    code = await exit_code(daemon.wait)

    with client_case.check("the Daemon played its script through"):
        assert code == 0
    with client_case.check("no BrowserContext exists in the SDK's public items"):
        assert "BrowserContext" not in goodfetch_collie.__all__
        assert not hasattr(goodfetch_collie, "BrowserContext")


class TestBrowserSetupOnProvingGround:
    """Browser setup settings observed on a real Proving ground page."""

    @pytest.fixture
    def proving_ground_seed(self) -> int:
        """The Proving ground's own seed for these tests."""
        return 1305

    @pytest.fixture
    def with_browser(self) -> bool:
        """The engine opens pages on the Browser cache laid out in the test's home."""
        return True

    @pytest.mark.covers(
        "story:11-89",
        "setting:browser.viewport",
        "setting:browser.device_scale_factor",
        "setting:browser.locale",
        "setting:browser.timezone_id",
        "setting:browser.color_scheme",
        "setting:browser.geolocation",
        "setting:browser.permissions",
        "setting:browser.headless",
        "setting:browser.block_resources",
        "setting:browser.service_workers",
        "setting:network.throttle",
    )
    @pytest.mark.client_case(
        id="session-browser-setup-observed-on-a-page", mode="attached"
    )
    @three_ways
    async def test_browser_setup_observed_on_a_proving_ground_page(
        self,
        way: Way,
        env: Mapping[str, str],
        proving_ground: ProvingGround,
        client_case: ClientCaseRecord,
    ) -> None:
        """Viewport, scale, locale, zone, scheme, permissions and throttle apply."""
        product = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/2")
        collie = await open_collie(way, env)
        run = await collie.start()
        session = await run.new_session(
            settings={
                "browser": {
                    "viewport": {"width": 800, "height": 600},
                    "device_scale_factor": 2,
                    "locale": "fr-FR",
                    "timezone_id": "Europe/Paris",
                    "color_scheme": "dark",
                    "geolocation": {
                        "latitude": 48.8566,
                        "longitude": 2.3522,
                        "accuracy": 10,
                    },
                    "permissions": ["geolocation"],
                    "headless": True,
                    "block_resources": ["image"],
                    "service_workers": "block",
                },
                "network": {
                    "throttle": {
                        "offline": False,
                        "latency": 40,
                        "download_bytes_per_second": 500_000,
                        "upload_bytes_per_second": 250_000,
                    }
                },
            }
        )
        page = await session.new_page()
        await page.goto(product)
        observed = await page.evaluate(
            """async () => ({
              width: window.innerWidth,
              height: window.innerHeight,
              dpr: window.devicePixelRatio,
              locale: navigator.language,
              timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
              dark: window.matchMedia("(prefers-color-scheme: dark)").matches,
              geoPermission: (
                await navigator.permissions.query({ name: "geolocation" })
              ).state,
            })"""
        )

        offline_session = await run.new_session(
            settings={"network": {"throttle": {"offline": True, "latency": 0}}}
        )
        offline_page = await offline_session.new_page()
        offline_error = await failure(offline_page.goto(product))

        await offline_session.close()
        await session.close()
        await run.finish()
        await collie.close()

        with client_case.check(
            "viewport, scale, locale, zone, colour scheme and permissions are observed"
        ):
            assert observed == {
                "width": 800,
                "height": 600,
                "dpr": 2,
                "locale": "fr-FR",
                "timeZone": "Europe/Paris",
                "dark": True,
                "geoPermission": "granted",
            }
        with client_case.check("network.throttle offline prevents navigation"):
            assert isinstance(offline_error, Exception)
        with client_case.check("no BrowserContext exists in the SDK's public items"):
            assert "BrowserContext" not in goodfetch_collie.__all__
            assert not hasattr(goodfetch_collie, "BrowserContext")
