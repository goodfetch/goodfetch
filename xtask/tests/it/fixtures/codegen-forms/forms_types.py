"""What a program can and cannot write against the generated handles.

The handles `xtask codegen` generates from the forms catalog (ADR 0099), in
their async form and their sync twin: pyright and mypy fail on a wrong one,
and each `# type: ignore[...]` that no longer catches an error fails too.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from datetime import timedelta
from typing import Literal, assert_type

from goodfetch_collie_client.generated.handles import (
    AsyncLocator,
    AsyncPage,
    AsyncPageKeyboard,
    AsyncRun,
    Locator,
    Page,
    PageKeyboard,
    Run,
)
from goodfetch_collie_client.handle import NotGiven
from goodfetch_collie_client.generated.models import (
    Acted,
    Navigated,
    PageField,
    Read,
    Readiness,
)

URL = "https://example.test/"
REFERRER = "https://example.test/from"


async def async_forms(
    run: AsyncRun,
    page: AsyncPage,
    other_keyboard: AsyncPageKeyboard,
    fields: dict[str, PageField],
) -> None:
    """The async handles: each round trip, query and waiter is awaited."""
    # A round trip: its required texts are positional in the catalog's
    # order, which is not the alphabet's; its required boolean and its
    # optional parameter are keyword-only.
    assert_type(await page.goto(URL, REFERRER, fresh=True), Navigated)
    assert_type(
        await page.goto(URL, REFERRER, fresh=False, timeout=timedelta(seconds=1)),
        Navigated,
    )
    assert_type(await page.goto(url=URL, referrer=REFERRER, fresh=True), Navigated)
    # `None` is a value, no timeout; leaving it out takes the setting's
    # default (ADR 0080).
    assert_type(
        await page.goto(URL, REFERRER, fresh=True, timeout=None), Navigated
    )
    # A timeout is a `timedelta`, never a number of milliseconds (ADR 0080).
    await page.goto(URL, REFERRER, fresh=False, timeout=1)  # type: ignore[arg-type]
    # A boolean is never positional.
    await page.goto(URL, REFERRER, True)  # type: ignore[call-arg]
    # A required boolean has no default.
    await page.goto(URL, REFERRER)  # type: ignore[call-arg]

    # A query takes nothing to wait with: no timeout.
    assert_type(await page.is_ready(), Readiness)
    await page.is_ready(timeout=1)  # type: ignore[call-arg]

    # A result typed by the fields its call declares is the result's model
    # in Python, whose values hold each field's value under its name.
    assert_type(await page.read(fields), Read)

    # A waiter keeps its timeout.
    assert_type(await page.wait_for_ready(timeout=timedelta(seconds=1)), Acted)
    assert_type(await page.wait_for_ready(expected={"ready": [1, "a"]}), Acted)
    assert_type(await page.wait_for_ready(state="complete"), Acted)
    await page.wait_for_ready(state="ready")  # type: ignore[call-overload]
    # A wrapped choice that admits null takes `None` unwrapped.
    assert_type(await page.wait_for_ready(state=None), Acted)
    # A deprecated keyword and a deprecated value each have an overload of
    # their own, which PEP 702 marks: a checker reports a call that passes
    # them, and no other (ADR 0099).
    assert_type(await page.wait_for_ready(poll=100), Acted)  # type: ignore[deprecated]
    assert_type(await page.wait_for_ready(state="loaded"), Acted)  # type: ignore[deprecated]
    assert_type(
        await page.wait_for_ready(state="complete", timeout=timedelta(seconds=1)), Acted
    )

    # A `new` member returns the value it opens, which an `async with`
    # block closes.
    assert_type(await run.new_page(), AsyncPage)
    assert_type(await run.new_page(viewport="1280x720"), AsyncPage)
    async with await run.new_page() as opened:
        assert_type(opened, AsyncPage)

    # A handle's own close answers nothing: it is idempotent (ADR 0078).
    close: Callable[[], Awaitable[None]] = page.close
    await close()

    # A value built without a call is returned at once, by a sync method.
    assert_type(page.locator("button"), AsyncLocator)
    assert_type(await page.locator("button").click(), Acted)

    # A sub-object is a property the program cannot replace.
    assert_type(page.keyboard, AsyncPageKeyboard)
    page.keyboard = other_keyboard  # type: ignore[misc]
    assert_type(await page.keyboard.press("Enter"), Acted)  # type: ignore[deprecated]


def sync_forms(
    run: Run,
    page: Page,
    other_keyboard: PageKeyboard,
    fields: dict[str, PageField],
    any_state: Literal["interactive", "complete", "loaded"] | None,
    any_poll: int | None | NotGiven,
) -> None:
    """The sync twins: the same members under the same names, each plain."""
    assert_type(page.goto(URL, REFERRER, fresh=True), Navigated)
    assert_type(
        page.goto(URL, REFERRER, fresh=False, timeout=timedelta(seconds=1)), Navigated
    )
    page.goto(URL, REFERRER, fresh=False, timeout=1)  # type: ignore[arg-type]
    page.goto(URL, REFERRER, True)  # type: ignore[call-arg]

    assert_type(page.is_ready(), Readiness)
    page.is_ready(timeout=1)  # type: ignore[call-arg]

    assert_type(page.read(fields), Read)
    assert_type(page.wait_for_ready(timeout=timedelta(seconds=1)), Acted)
    assert_type(page.wait_for_ready(expected=None), Acted)
    assert_type(page.wait_for_ready(state="interactive"), Acted)
    assert_type(page.wait_for_ready(poll=100, state=None), Acted)  # type: ignore[deprecated]
    assert_type(page.wait_for_ready(state="loaded"), Acted)  # type: ignore[deprecated]
    page.wait_for_ready(poll="often")  # type: ignore[call-overload]
    # A value only its type gives, a union with a deprecated value or one
    # forwarded as given or not, resolves to the last overload, which takes
    # every form.
    assert_type(page.wait_for_ready(state=any_state), Acted)
    assert_type(page.wait_for_ready(poll=any_poll), Acted)
    page.keyboard.press(1)  # type: ignore[arg-type,deprecated]

    assert_type(run.new_page(), Page)
    with run.new_page() as opened:
        assert_type(opened, Page)

    close: Callable[[], None] = page.close
    close()

    assert_type(page.locator("button"), Locator)
    assert_type(page.locator("button").click(), Acted)

    assert_type(page.keyboard, PageKeyboard)
    page.keyboard = other_keyboard  # type: ignore[misc]
    assert_type(page.keyboard.press("Enter"), Acted)  # type: ignore[deprecated]
