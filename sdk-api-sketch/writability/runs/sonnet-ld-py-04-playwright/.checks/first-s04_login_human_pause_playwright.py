"""Log in to quotes.toscrape.com once with a person at the keyboard, keep the
session under a name, then reuse it in a headless run.

    python s04_login_human_pause_playwright.py login --name quotes --username alice
    python s04_login_human_pause_playwright.py check --name quotes

The password is read from the environment variable QUOTES_PASSWORD, or from the
macOS keychain when --keychain-service is given. It is read only at the moment
it is typed into the form: it is never printed, logged, traced or written to a
file. The saved state holds cookies and local storage, never the password.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import signal
import subprocess
import sys
from pathlib import Path

from playwright.async_api import Page, async_playwright

BASE_URL = "https://quotes.toscrape.com/"
LOGIN_URL = BASE_URL + "login"
STATE_DIR = Path(".auth")
PASSWORD_ENV = "QUOTES_PASSWORD"
USERNAME_ENV = "QUOTES_USERNAME"
NAME_PATTERN = re.compile(r"[A-Za-z0-9_-]{1,64}")

EXIT_OK = 0
EXIT_FAILED = 1
EXIT_EXPIRED = 2
EXIT_CANCELLED = 130


def say(message: str) -> None:
    print(message, flush=True)


def state_path(name: str) -> Path:
    if not NAME_PATTERN.fullmatch(name):
        raise SystemExit("The state name may only use letters, digits, '-' and '_'.")
    return STATE_DIR / f"{name}.json"


def read_password(keychain_service: str | None, keychain_account: str | None) -> str:
    """Fetch the secret from a keychain or the environment, never from argv."""
    if keychain_service is not None:
        account = keychain_account or os.environ.get("USER", "")
        result = subprocess.run(
            ["security", "find-generic-password", "-s", keychain_service, "-a", account, "-w"],
            capture_output=True,
            text=True,
            check=False,
        )
        secret = result.stdout.rstrip("\n")
        if result.returncode != 0 or not secret:
            raise SystemExit("No password found in the keychain for that service and account.")
        return secret
    secret = os.environ.get(PASSWORD_ENV, "")
    if not secret:
        raise SystemExit(f"Set {PASSWORD_ENV} (or use --keychain-service) to supply the password.")
    return secret


def write_private_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(data, handle, indent=2)
    os.chmod(path, 0o600)


async def fill_form(page: Page, username: str, password: str) -> None:
    """Type the credentials. Errors are reported by type only, so no call log
    that might echo the typed value ever reaches the output."""
    try:
        await page.locator("#username").fill(username)
        await page.locator("#password").fill(password)
    except Exception as error:
        raise SystemExit(f"Could not fill the login form ({type(error).__name__}).") from None


async def run_login(args: argparse.Namespace) -> int:
    path = state_path(args.name)
    username: str = args.username or os.environ.get(USERNAME_ENV, "")
    if not username:
        raise SystemExit(f"Give --username or set {USERNAME_ENV}.")
    password = read_password(args.keychain_service, args.keychain_account)

    loop = asyncio.get_running_loop()
    cancelled = asyncio.Event()
    page_closed = asyncio.Event()
    loop.add_signal_handler(signal.SIGINT, cancelled.set)
    loop.add_signal_handler(signal.SIGTERM, cancelled.set)

    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=False)
        try:
            context = await browser.new_context()
            page = await context.new_page()
            page.on("close", lambda _page: page_closed.set())
            browser.on("disconnected", lambda _browser: page_closed.set())

            await page.goto(LOGIN_URL)
            await fill_form(page, username, password)
            del password

            say("The form is filled in. Check it in the browser window and press Login.")
            say("Waiting without a time limit. Press Ctrl-C or close the window to cancel.")

            logout_link = page.get_by_role("link", name="Logout")
            logged_in = asyncio.ensure_future(logout_link.wait_for(state="visible", timeout=0))
            stop = asyncio.ensure_future(_first_of(cancelled, page_closed))
            done, pending = await asyncio.wait({logged_in, stop}, return_when=asyncio.FIRST_COMPLETED)
            for task in pending:
                task.cancel()
            await asyncio.gather(*pending, return_exceptions=True)

            if logged_in not in done or logged_in.exception() is not None:
                say("Cancelled: no state was saved.")
                return EXIT_CANCELLED

            state = await context.storage_state()
            write_private_json(path, state)
            say(f"Logged in. State kept as '{args.name}' in {path}.")
            return EXIT_OK
        finally:
            try:
                await browser.close()
            except Exception:
                pass
            loop.remove_signal_handler(signal.SIGINT)
            loop.remove_signal_handler(signal.SIGTERM)


async def _first_of(*events: asyncio.Event) -> None:
    waiters = [asyncio.ensure_future(event.wait()) for event in events]
    try:
        await asyncio.wait(waiters, return_when=asyncio.FIRST_COMPLETED)
    finally:
        for waiter in waiters:
            waiter.cancel()


async def run_check(args: argparse.Namespace) -> int:
    path = state_path(args.name)
    if not path.is_file():
        say(f"No kept state named '{args.name}' ({path}). Run the login step first.")
        return EXIT_FAILED

    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=True)
        try:
            context = await browser.new_context(storage_state=path)
            page = await context.new_page()
            await page.goto(BASE_URL)

            if await page.get_by_role("link", name="Logout").count() > 0:
                say(f"Logged in: the page shows a Logout link (state '{args.name}').")
                return EXIT_OK
            if await page.get_by_role("link", name="Login").count() > 0:
                say(f"The kept state '{args.name}' has expired: the page shows a Login link. Log in again.")
                return EXIT_EXPIRED
            say("Neither a Logout nor a Login link was found; the page did not look as expected.")
            return EXIT_FAILED
        finally:
            await browser.close()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    login = commands.add_parser("login", help="visible browser; a person presses Login")
    login.add_argument("--name", required=True, help="name to keep the logged-in state under")
    login.add_argument("--username", help=f"username (default: ${USERNAME_ENV})")
    login.add_argument("--keychain-service", help="read the password from this macOS keychain service")
    login.add_argument("--keychain-account", help="keychain account (default: $USER)")

    check = commands.add_parser("check", help="headless; reuse the kept state")
    check.add_argument("--name", required=True, help="name the state was kept under")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.command == "login":
            return asyncio.run(run_login(args))
        return asyncio.run(run_check(args))
    except KeyboardInterrupt:
        say("Cancelled.")
        return EXIT_CANCELLED


if __name__ == "__main__":
    sys.exit(main())
