"""Log in to quotes.toscrape.com with a person at the keyboard, keep the session, reuse it headless.

Usage:
  python s04_login_human_pause_playwright.py login   visible browser; the username is filled, the
                                                    person checks the form and presses Login
  python s04_login_human_pause_playwright.py check   headless; reuses the kept state and confirms
                                                    that the home page shows a Logout link

Secrets: the password comes from the QUOTES_PASSWORD environment variable or, on macOS, from the
keychain item (service KEYCHAIN_SERVICE, account = username). It is read only when the login step
starts, handed straight to Playwright's fill(), and never printed, traced or saved. Tracing is
never enabled. The kept state is Playwright's storage state (cookies and local storage): it holds
the session, not the password, and is written with owner-only permissions.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

from playwright.sync_api import Browser, BrowserContext, Error as PlaywrightError, sync_playwright

BASE_URL = "https://quotes.toscrape.com"
LOGIN_URL = f"{BASE_URL}/login"
HOME_URL = f"{BASE_URL}/"

STATE_NAME = "quotes-toscrape"
STATE_DIR_ENV = "QUOTES_STATE_DIR"
DEFAULT_STATE_DIR = Path.home() / ".local" / "state" / "s04-login-human-pause"

USERNAME_ENV = "QUOTES_USERNAME"
DEFAULT_USERNAME = "admin"
PASSWORD_ENV = "QUOTES_PASSWORD"
KEYCHAIN_SERVICE = "quotes.toscrape.com"

EXIT_OK = 0
EXIT_CANCELLED = 1
EXIT_ERROR = 2
EXIT_NO_STATE = 3
EXIT_EXPIRED = 4


def state_path() -> Path:
    """Return the file that keeps the logged-in state under STATE_NAME."""
    configured = os.environ.get(STATE_DIR_ENV)
    directory = Path(configured).expanduser() if configured else DEFAULT_STATE_DIR
    return directory / f"{STATE_NAME}.storage-state.json"


def read_username() -> str:
    return os.environ.get(USERNAME_ENV) or DEFAULT_USERNAME


def read_password(username: str) -> str:
    """Return the password from the environment or the macOS keychain, without echoing it."""
    from_env = os.environ.get(PASSWORD_ENV)
    if from_env:
        return from_env
    if sys.platform == "darwin":
        found = subprocess.run(
            ["security", "find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", username, "-w"],
            capture_output=True,
            text=True,
            check=False,
        )
        if found.returncode == 0 and found.stdout.strip():
            return found.stdout.rstrip("\r\n")
    raise RuntimeError(
        f"No password found. Set {PASSWORD_ENV} or add a keychain item with service "
        f"'{KEYCHAIN_SERVICE}' and account '{username}'."
    )


def save_state(context: BrowserContext) -> Path:
    """Write the context's storage state to the named file with owner-only permissions."""
    path = state_path()
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    data = context.storage_state()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(data, handle)
    os.chmod(path, 0o600)
    return path


def _close(browser: Browser) -> None:
    try:
        browser.close()
    except PlaywrightError:
        pass


def _login_in_browser(browser: Browser, username: str, password: str) -> int:
    context = browser.new_context()
    page = context.new_page()
    page.goto(LOGIN_URL)
    page.locator("input[name='username']").fill(username)
    page.locator("input[name='password']").fill(password)
    del password  # the value now lives only in the page's form field
    print("The form is filled. Check it in the browser and press Login.")
    print("There is no time limit. To cancel, press Ctrl+C here or close the browser window.")
    # Logout is only shown once the person's Login has succeeded; timeout=0 waits without limit.
    page.get_by_role("link", name="Logout", exact=True).wait_for(state="visible", timeout=0)
    page.wait_for_load_state()
    path = save_state(context)
    print(f"Logged in. Kept the login state as '{STATE_NAME}' at {path}.")
    return EXIT_OK


def run_login() -> int:
    username = read_username()
    try:
        password = read_password(username)
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return EXIT_ERROR
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=False)
        try:
            return _login_in_browser(browser, username, password)
        except KeyboardInterrupt:
            print("Login cancelled. No login state was saved.")
            return EXIT_CANCELLED
        except PlaywrightError:
            print("Login did not complete: the browser was closed or the page failed. No state was saved.")
            return EXIT_CANCELLED
        finally:
            _close(browser)


def run_check() -> int:
    path = state_path()
    if not path.is_file():
        print(f"No kept login state at {path}. Run the login step first.")
        return EXIT_NO_STATE
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True)
        try:
            context = browser.new_context(storage_state=str(path))
            page = context.new_page()
            page.goto(HOME_URL)
            page.wait_for_load_state()
            if page.get_by_role("link", name="Logout", exact=True).count() > 0:
                print(f"Logged in: the Logout link is shown on {HOME_URL}.")
                return EXIT_OK
            print(
                "The kept login state has expired or is no longer valid: no Logout link is shown. "
                "Run the login step again."
            )
            return EXIT_EXPIRED
        except PlaywrightError:
            print("Check did not complete: the page could not be loaded. The kept state was not judged.")
            return EXIT_ERROR
        finally:
            _close(browser)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Human-paused login to quotes.toscrape.com, then headless reuse of the kept state."
    )
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("login", help="visible browser: fill the username and password, person presses Login")
    commands.add_parser("check", help="headless: reuse the kept state and confirm the Logout link")
    args = parser.parse_args(argv)
    command: str = args.command
    if command == "login":
        return run_login()
    return run_check()


if __name__ == "__main__":
    raise SystemExit(main())
