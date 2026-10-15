"""Log in to quotes.toscrape.com once with a person at the keyboard, keep the login, and reuse it headless.

    python -m scenarios.s04_login_human_pause login    # visible browser, person presses Login
    python -m scenarios.s04_login_human_pause verify   # headless, reuses the kept state

The password is never held by this program: the engine reads it from the environment variable QUOTES_PASSWORD
(a Secret reference, `env:QUOTES_PASSWORD`; `keychain:goodfetch/quotes` would read the system keychain instead)
and types it into the field. The program only names it with a Secret placeholder, so it appears in no log, no
trace and no saved file; the kept login is a Saved identity, which the engine stores encrypted.
"""

from __future__ import annotations

import argparse
import asyncio
import logging
import re
import sys
from collections.abc import Sequence
from datetime import timedelta

from goodfetch_collie import (
    AsyncCollie,
    AsyncPage,
    GoodfetchError,
    HumanPauseDeclinedError,
    LoggedOutError,
    RunSettings,
    SecretsSettings,
    SecretUnavailableError,
    SessionSettings,
    secret,
)

log = logging.getLogger("scenarios.s04")

LOGIN_URL = "https://quotes.toscrape.com/login"
HOME_URL = "https://quotes.toscrape.com/"
USERNAME = "demo"
IDENTITY_NAME = "quotes-toscrape-demo"
PASSWORD_REFERENCE = "env:QUOTES_PASSWORD"
PASSWORD_NAME = "quotes_password"

EXIT_OK = 0
EXIT_FAILED = 1
EXIT_EXPIRED = 3
EXIT_CANCELLED = 130


def _secrets() -> SecretsSettings:
    # The reference names where the engine finds the value; the domains are the only ones that may receive it.
    return {PASSWORD_NAME: {"reference": PASSWORD_REFERENCE, "domains": ["toscrape.com"]}}


def _logout_link(page: AsyncPage):  # noqa: ANN202 - the locator type is not exported
    return page.get_by_role("link", name="Logout")


async def login_with_person() -> int:
    """First run: a visible browser, the form filled, then wait for the person to check it and press Login."""
    run_settings: RunSettings = {"browser": {"headless": False}, "secrets": _secrets()}
    async with AsyncCollie.connect() as collie:
        async with await collie.start(settings=run_settings) as run:
            page = await run.new_page()
            await page.goto(LOGIN_URL)
            await page.locator("#username").fill(USERNAME)
            await page.locator("#password").fill(secret(PASSWORD_NAME))

            print("The form is filled. Check it in the browser window and press Login.")
            print("The program waits without a time limit; press Ctrl+C or Decline in the Live view to cancel.")
            try:
                # Ends by itself when the Logout link shows, which only a successful login produces.
                result = await page.pause(
                    reason="Check the login form and press Login",
                    until=_logout_link(page),
                    timeout=None,
                )
            except HumanPauseDeclinedError as error:
                print(f"Cancelled by {error.details.person or 'the person'}; nothing was kept.")
                return EXIT_CANCELLED
            log.info("human pause ended by %s after %s", result.resolved_by, result.waited)

            if not await _logout_link(page).is_visible():
                print("The page shows no Logout link, so the login did not succeed; nothing was kept.")
                return EXIT_FAILED

            identity = await page.session.save_identity(IDENTITY_NAME)
            print(f"Login kept as {identity.name!r}.")
            await run.finish()
    return EXIT_OK


async def verify_kept_login() -> int:
    """Second run: headless, reuse the kept state and confirm the Logout link."""
    run_settings: RunSettings = {"browser": {"headless": True}}
    session_settings: SessionSettings = {
        "session": {
            "page_check": {"login_page": {"url": re.compile(r"/login")}},
            # A lost login is reported, not repaired by asking a person or a model.
            "recovery": {"human_pause": False, "agent": False},
        }
    }
    async with AsyncCollie.connect() as collie:
        kept = {identity.name for identity in await collie.identities.list()}
        if IDENTITY_NAME not in kept:
            print(f"No kept login named {IDENTITY_NAME!r}: run the login step first.")
            return EXIT_EXPIRED

        async with await collie.start(settings=run_settings) as run:
            session = await run.new_session(
                source={"kind": "saved_identity", "name": IDENTITY_NAME},
                settings=session_settings,
            )
            page = await session.new_page()
            try:
                await page.goto(HOME_URL)
            except LoggedOutError:
                print("The kept login has expired: the site sent the browser back to its login page.")
                return EXIT_EXPIRED

            logout = _logout_link(page)
            try:
                await logout.wait_for(state="visible", timeout=timedelta(seconds=5))
            except GoodfetchError:
                if await page.get_by_role("link", name="Login").is_visible():
                    print("The kept login has expired: the page shows a Login link, not a Logout link.")
                else:
                    print("The page shows no Logout link, and no Login link either; the kept login cannot be confirmed.")
                return EXIT_EXPIRED

            print("Logged in: the page shows a Logout link.")
            await run.finish()
    return EXIT_OK


async def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("step", choices=["login", "verify"])
    args = parser.parse_args(argv)
    step: str = args.step

    try:
        return await (login_with_person() if step == "login" else verify_kept_login())
    except SecretUnavailableError as error:
        # Names the reference and why, never a value.
        print(f"The password secret {error.details.reference!r} could not be read ({error.details.failure}).")
        return EXIT_FAILED
    except GoodfetchError as error:
        print(f"Failed: {error}")
        return EXIT_FAILED


def run_main() -> int:
    logging.basicConfig(level=logging.INFO)
    try:
        return asyncio.run(main(sys.argv[1:]))
    except (KeyboardInterrupt, asyncio.CancelledError):
        print("Cancelled; nothing was kept.")
        return EXIT_CANCELLED


if __name__ == "__main__":
    sys.exit(run_main())
