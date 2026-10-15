"""Snippet for Sessions, logins, the Human pause and secrets in code (#476), on golden scenario 04."""

import logging

from goodfetch_collie import AsyncCollie, Collie, HumanPauseDeclinedError, LoggedOutError, SecretRequest, secret

log = logging.getLogger(__name__)


async def read_vault(name: str) -> str:
    """The program's own secret store."""
    raise NotImplementedError


async def log_in_once() -> None:
    """First run: a person at the keyboard logs in once, and the login is kept under a name."""
    async with AsyncCollie.connect() as collie:
        # A Secret placeholder: the program names it, and the engine types its value, only into a field on toscrape.com.
        async with await collie.start(
            settings={"secrets": {"password": {"reference": "env:QUOTES_PASSWORD", "domains": ["toscrape.com"]}}}
        ) as run:
            # A visible window for the person; headless, they would act in the Live view instead.
            async with await run.new_session(settings={"browser": {"headless": False}}) as session:
                page = await session.new_page()
                await page.goto("https://quotes.toscrape.com/login")
                await page.get_by_label("Username").fill("alice")
                await page.get_by_label("Password").fill(secret("password"))
                try:
                    # The Human pause: no clock runs while it waits, and it ends once the Logout link shows.
                    await page.pause(
                        reason="Check the form, then press Login",
                        until=page.get_by_role("link", name="Logout"),
                        timeout=None,
                    )
                except HumanPauseDeclinedError as error:
                    log.warning("Login declined: %s", error.details.note)
                    raise
                # Kept encrypted per install, never as a clear-text file (ADRs 0035, 0056).
                await session.save_identity("quotes")


def reuse_login() -> bool:
    """Second run, sync: headless and unattended, it reuses the login and says when it has expired."""
    with Collie.connect() as collie, collie.start() as run:
        with run.new_session(
            source={"kind": "saved_identity", "name": "quotes"},
            settings={
                "session": {
                    # A page showing the Login link is a lost login; nobody is there to log in again.
                    "page_check": {"login_page": {"selector": "a[href='/login']"}},
                    "recovery": {"human_pause": False},
                }
            },
        ) as session:
            page = session.new_page()
            try:
                page.goto("https://quotes.toscrape.com/")
            except LoggedOutError as error:
                log.warning("The saved login has expired: run log_in_once again (%d steps tried)", len(error.details.steps))
                return False
            return page.get_by_role("link", name="Logout").is_visible()


async def answer_secret(request: SecretRequest) -> str:
    return await read_vault(request.name)


async def cookies_and_proxy() -> int:
    """Cookies, a proxy and secrets the program answers from its own store."""
    async with AsyncCollie.connect(resolve_secret=answer_secret) as collie:
        async with await collie.start(
            settings={
                "proxy": {"server": "http://proxy.example:3128", "username": "crawler", "password": "caller:proxy_password"},
                "session": {
                    "http_credentials": [{"username": "reader", "password": "caller:basic_password", "domains": ["httpbin.org"]}]
                },
                "secrets": {"token": {"reference": "caller:api_token", "domains": ["toscrape.com"]}},
            }
        ) as run:
            # The Run's default Session, which every `run.new_page()` shares.
            page = await run.new_page()
            session = page.session
            await session.add_cookies([{"name": "theme", "value": "dark", "url": "https://quotes.toscrape.com"}])
            await page.goto("https://quotes.toscrape.com/")
            await page.request.get("https://quotes.toscrape.com/api/quotes", headers={"authorization": secret("token")})
            cookies = await session.cookies("https://quotes.toscrape.com")
            # Playwright's storage API, for the page's current origin; kept across runs by a Saved identity.
            await page.local_storage.set_item("theme", "dark")

            # No field to check: keyboard input takes no Secret placeholder.
            await page.keyboard.type(secret("token"))  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]

            for identity in await collie.identities.list():
                log.info("Saved identity %s, cookies expire %s", identity.name, identity.cookies_expire)
            return len(cookies)
