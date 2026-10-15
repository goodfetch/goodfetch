"""Golden scenario 04, "Log in once with a person, reuse the login headless" (Collie, Python).

`python s04_login_human_pause.py` logs in with a person; `--reuse` confirms the kept login headless.
"""

# The sync client, as the leader's program uses Playwright's sync API. Ctrl+C cancels the call in flight (ADR 0081).
import sys
from typing import Literal

from goodfetch_collie import Collie, LoggedOutError, secret

IDENTITY = "quotes"  # the name the login is kept under


def log_in_once(username: str) -> None:
    """First run: a person checks the form and presses Login; the login is kept as a Saved identity."""
    # The Secret placeholder `password`: the engine reads QUOTES_PASSWORD and types it only into a field on toscrape.com.
    # The program never holds it, and every log, trace, screenshot and returned value shows `%password%` instead.
    with (
        Collie.connect() as collie,
        collie.start(
            settings={
                "secrets": {
                    "password": {
                        "reference": "env:QUOTES_PASSWORD",
                        "domains": ["toscrape.com"],
                    }
                }
            }
        ) as run,
    ):
        session = run.new_session(settings={"browser": {"headless": False}})
        page = session.new_page()
        page.goto("https://quotes.toscrape.com/login")
        page.get_by_label("Username").fill(username)
        # The page's "Password" label points at the username field (`for="username"`): the field is found by its id.
        page.locator("#password").fill(secret("password"))
        # The Human pause: no time limit and no other clock runs; it ends once the Logout link shows, and a person's
        # "Decline" raises `HumanPauseDeclinedError`.
        page.pause(
            reason="Check the form, then press Login",
            until=page.get_by_role("link", name="Logout"),
            timeout=None,
        )
        # Kept encrypted in the install's store under that name, never as a clear-text file (ADR 0056).
        session.save_identity(IDENTITY)
        run.finish()


def confirm_login() -> Literal["logged_in", "expired"]:
    """Second run, headless by default: `expired` when the kept login no longer logs in."""
    with Collie.connect() as collie, collie.start() as run:
        session = run.new_session(
            source={"kind": "saved_identity", "name": IDENTITY},
            settings={
                "session": {
                    # A page showing the Login link is a lost login, and no one is there to log in again.
                    "page_check": {"login_page": {"selector": "a[href='/login']"}},
                    "recovery": {"human_pause": False, "agent": False},
                }
            },
        )
        page = session.new_page()
        state: Literal["logged_in", "expired"] = "logged_in"
        try:
            page.goto("https://quotes.toscrape.com/")
            page.get_by_role("link", name="Logout").wait_for()
        except LoggedOutError:
            # An outcome, named by the Page check, not a missing element.
            state = "expired"
        run.finish()
        return state


if __name__ == "__main__":
    if "--reuse" in sys.argv:
        if confirm_login() == "logged_in":
            print("Logged in: the page shows Logout.")
        else:
            print("The kept login has expired: log in again.")
    else:
        log_in_once("alice")
