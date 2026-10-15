"""Golden scenario 04, "Log in once with a person, reuse the login headless": the leader, Playwright 1.64.0 (Python),
with `page.pause()` and `storage_state`.

`python s04_login_human_pause_playwright.py` logs in with a person; `--reuse` checks the kept login headless.
"""

# The sync API, which Playwright's Python docs show first in every example.
import os
import sys
from typing import Literal

from playwright.sync_api import sync_playwright

# The authentication guide's location; its advice is to keep the folder out of version control.
STATE = "playwright/.auth/quotes.json"


def log_in_once(username: str) -> None:
    password = os.environ["QUOTES_PASSWORD"]
    with sync_playwright() as p:
        # Ctrl+C: Playwright's own SIGINT handler closes the browser (`handle_sigint`, on by default).
        browser = p.chromium.launch(headless=False)
        context = browser.new_context()
        page = context.new_page()
        page.goto("https://quotes.toscrape.com/login")
        page.get_by_label("Username").fill(username)
        # The page's "Password" label points at the username field (`for="username"`): the field is found by its id.
        page.locator("#password").fill(password)
        # The person checks the form, presses Login, then Resume in the Playwright Inspector; no timeout applies.
        page.pause()
        page.get_by_role("link", name="Logout").wait_for()
        context.storage_state(path=STATE)
        browser.close()


def confirm_login() -> Literal["logged_in", "expired"]:
    with sync_playwright() as p:
        browser = p.chromium.launch()
        context = browser.new_context(storage_state=STATE)
        page = context.new_page()
        page.goto("https://quotes.toscrape.com/")
        logout = page.get_by_role("link", name="Logout")
        # A logged-out page shows the Login link instead.
        logout.or_(page.get_by_role("link", name="Login")).wait_for()
        state: Literal["logged_in", "expired"] = (
            "logged_in" if logout.is_visible() else "expired"
        )
        browser.close()
        return state


if __name__ == "__main__":
    if "--reuse" in sys.argv:
        if confirm_login() == "logged_in":
            print("Logged in: the page shows Logout.")
        else:
            print("The kept login has expired: log in again.")
    else:
        log_in_once("alice")
