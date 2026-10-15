"""Scenario s04: a person logs in once; the kept login is reused headless.

Usage (from the working folder):

    python -m scenarios.s04_login_human_pause login
    python -m scenarios.s04_login_human_pause check

`login` opens a visible browser at the login page, fills the username and the password, and waits for
the person to press Login. It keeps the logged-in state as a Saved identity named LOGIN_NAME. There is
no time limit while waiting; Ctrl-C cancels, and then nothing is kept.

`check` reuses that Saved identity in a headless run, opens the home page and confirms that a Logout
link is shown. When the kept login has expired, it says so and exits without waiting for a person.

The password is never held by this program. It is named by a Secret placeholder that is bound to the
Secret reference `env:QUOTES_PASSWORD` for the site's domain, and the engine types it into the field.
To read it from the system keychain instead, change SECRET_REFERENCE to a `keychain:` reference.
Logs, traces and the Saved identity never carry the value.
"""

import argparse
import asyncio
import logging
import sys
from collections.abc import Coroutine, Sequence
from typing import Final

from goodfetch_collie import (
    AsyncCollie,
    GoodfetchError,
    HumanPauseDeclinedError,
    LoggedOutError,
    ReferenceBinding,
    SavedIdentitySource,
    SecretsSettings,
    SecretUnavailableError,
    SessionSettings,
    Settings,
    secret,
)

logger = logging.getLogger("scenarios.s04_login_human_pause")

LOGIN_URL: Final = "https://quotes.toscrape.com/login"
HOME_URL: Final = "https://quotes.toscrape.com/"
SITE_DOMAIN: Final = "toscrape.com"  # The registrable domain of quotes.toscrape.com (ADR 0004).
USERNAME: Final = "reader"
LOGIN_NAME: Final = "quotes-toscrape-login"
SECRET_NAME: Final = "quotes_password"
SECRET_REFERENCE: Final = "env:QUOTES_PASSWORD"
LOGIN_PAGE_GLOB: Final = "**/login"

EXIT_OK: Final = 0
EXIT_FAILED: Final = 1
EXIT_NO_LOGIN: Final = 2
EXIT_EXPIRED: Final = 3
EXIT_CANCELLED: Final = 130


def connection_settings(*, headless: bool) -> Settings:
    """Settings of one connection: the visibility of its browser and the password's binding."""
    secrets: SecretsSettings = {
        SECRET_NAME: ReferenceBinding(reference=SECRET_REFERENCE, domains=[SITE_DOMAIN]),
    }
    return {"browser": {"headless": headless}, "secrets": secrets}


def check_session_settings() -> SessionSettings:
    """A reused login never waits for a person or calls an agent; a lost login is reported, not repaired."""
    return {
        "session": {
            "page_check": {"login_page": {"url": LOGIN_PAGE_GLOB}},
            "recovery": {"human_pause": False, "agent": False},
        },
    }


def saved_login_source() -> SavedIdentitySource:
    return {"kind": "saved_identity", "name": LOGIN_NAME}


async def login() -> int:
    async with AsyncCollie.connect(settings=connection_settings(headless=False)) as collie:
        run = await collie.start()
        async with run:
            session = await run.new_session()
            page = await session.new_page()
            await page.goto(LOGIN_URL)
            await page.get_by_label("Username").fill(USERNAME)
            await page.get_by_label("Password").fill(secret(SECRET_NAME))
            logger.info(
                "A visible browser is open at the login page. Check the form and press Login. "
                "The program continues once the Logout link shows. Press Ctrl-C to cancel."
            )
            result = await page.pause(
                reason="Check the username and password, then press Login.",
                until=page.get_by_role("link", name="Logout"),
                timeout=None,
            )
            logger.info("The person finished the login step (%s).", result.resolved_by)
            identity = await session.save_identity(LOGIN_NAME)
            logger.info("Kept the logged-in state as %s.", identity.name)
            await run.finish()
    return EXIT_OK


async def check() -> int:
    async with AsyncCollie.connect(settings=connection_settings(headless=True)) as collie:
        identities = await collie.identities.list()
        if not any(identity.name == LOGIN_NAME for identity in identities):
            logger.error("No kept login named %s. Run the login command first.", LOGIN_NAME)
            return EXIT_NO_LOGIN
        run = await collie.start()
        async with run:
            try:
                session = await run.new_session(source=saved_login_source(), settings=check_session_settings())
                page = await session.new_page()
                await page.goto(HOME_URL)
            except LoggedOutError:
                logger.error("The kept login %s has expired. Run the login command again.", LOGIN_NAME)
                return EXIT_EXPIRED
            logout = page.get_by_role("link", name="Logout")
            if await logout.count() == 0:
                logger.error(
                    "The kept login %s has expired: the home page shows no Logout link. Run the login command again.",
                    LOGIN_NAME,
                )
                return EXIT_EXPIRED
            if not await logout.is_visible():
                logger.error("The home page has a Logout link, but it is not visible.")
                return EXIT_FAILED
            logger.info("Logged in: the home page shows a Logout link.")
            await run.finish()
    return EXIT_OK


async def run_login() -> int:
    try:
        return await login()
    except HumanPauseDeclinedError:
        logger.error("The person declined the login step. No login was kept.")
        return EXIT_FAILED
    except SecretUnavailableError as exc:
        logger.error("The secret %s could not be read (%s). Set QUOTES_PASSWORD and run again.", SECRET_NAME, exc.code)
        return EXIT_FAILED
    except GoodfetchError as exc:
        logger.error("Login failed: %s during %s.", exc.code, exc.operation)
        return EXIT_FAILED


async def run_check() -> int:
    try:
        return await check()
    except GoodfetchError as exc:
        logger.error("Check failed: %s during %s.", exc.code, exc.operation)
        return EXIT_FAILED


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Log in to quotes.toscrape.com once, then reuse the kept login.")
    parser.add_argument(
        "command",
        choices=("login", "check"),
        help="login: a person logs in in a visible browser; check: reuse the kept login headless",
    )
    args = parser.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    work: Coroutine[object, object, int] = run_login() if args.command == "login" else run_check()
    try:
        return asyncio.run(work)
    except KeyboardInterrupt:
        logger.warning("Cancelled with Ctrl-C.")
        return EXIT_CANCELLED


if __name__ == "__main__":
    sys.exit(main())
