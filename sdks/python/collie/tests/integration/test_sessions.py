"""A program's Sessions (ADRs 0092 and 0078): each opens from its own source.

On an Attached engine that plays a Scripted engine's script (ADR 0069), three
ways: async on asyncio, async on Trio and through the sync `Collie`.
`run.new_session` sends its `source` beside its `settings`, each Session's
pages are its own, and closing one closes its pages while the others open
pages still. The script fails on a message it does not expect and on a step
left unused, and the engine writes that failure to its stderr, which reaches
the `goodfetch.client` logger at warning: no such line says the SDK sent
exactly what the script holds. TypeScript's `sessions.test.ts` is its twin.
"""

import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_collie import ClosedError, ProgramError, SettingInvalidError
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

SHOP_URL = "https://shop.test/"


@pytest.mark.covers("story:11-92")
@pytest.mark.client_case(id="sessions-open-from-their-source", mode="attached")
@three_ways
async def test_opens_sessions_from_their_source_and_closes_each_with_its_pages(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Two Sessions open apart; the closed one refuses, the others still open pages."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/sessions-open-from-their-source.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start()
    saved = await run.new_session(
        source={"kind": "saved_identity", "name": "shop"},
        settings={"browser": {"access_mode": "standard"}},
    )
    saved_page = await saved.new_page()
    fresh = await run.new_session(source=None)
    await fresh.new_page()

    await saved.close()
    after_close = [
        await failure(saved_page.goto(SHOP_URL)),
        await failure(saved.new_page()),
    ]
    await fresh.new_page()
    await run.new_page()
    not_registrable = await failure(
        run.new_session(
            source={
                "kind": "cdp_url",
                "url": "http://127.0.0.1:9222",
                "domains": ["www.example.com"],
            }
        )
    )
    await run.finish()
    await collie.close()

    with client_case.check(
        "the closed Session's page and the Session itself raise ClosedError"
        " without a message"
    ):
        assert [(type(error), error.code, error.group) for error in after_close] == [
            (ClosedError, "closed", "program")
        ] * 2
    with client_case.check(
        "a domain that is no registrable one raises SettingInvalidError whose"
        " fix names the registrable domain"
    ):
        assert isinstance(not_registrable, SettingInvalidError)
        assert isinstance(not_registrable, ProgramError)
        assert (
            not_registrable.code,
            not_registrable.group,
            not_registrable.retryable,
        ) == ("setting_invalid", "program", False)
        assert not_registrable.fix == "Give `example.com` in `domains`"
        assert not_registrable.details == {
            "key": "session.source",
            "layer": "run",
            "reason": "`www.example.com` in `domains` is no registrable domain,"
            " whose is `example.com`",
        }
    with client_case.check(
        "each call sent its source, settings and Session as the script holds"
        " them, and the others opened pages still"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-92")
@pytest.mark.client_case(id="a-page-reaches-its-session", mode="attached")
@three_ways
async def test_a_page_reaches_its_session_without_a_call(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The Run's pages share its default Session; closing it closes them."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/a-page-reaches-its-session.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start()
    first = await run.new_page()
    second = await run.new_page()
    own = await run.new_session(source=None)
    own_page = await own.new_page()

    # A property is the sync API's own value, which `way.api` hands with
    # awaited methods.
    default_session = first.session
    await way.api(default_session, default_session).close()
    after_close = await failure(second.goto(SHOP_URL))
    reopened = await failure(run.new_page())
    own_session = own_page.session
    await way.api(own_session, own_session).new_page()
    await run.finish()
    await collie.close()

    with client_case.check(
        "the Run's two pages reach one default Session, and a page of the"
        " program's Session reaches that Session"
    ):
        assert second.session is default_session
        assert own_session is not default_session
    with client_case.check(
        "closing the default Session through a page closes the other page,"
        " and a later run.new_page() raises ClosedError"
    ):
        assert [
            (type(error), error.code, error.group) for error in (after_close, reopened)
        ] == [(ClosedError, "closed", "program")] * 2
    with client_case.check(
        "no call reached a page's Session but those the program made, as the"
        " script holds them"
    ):
        assert engine_warnings(caplog) == []
