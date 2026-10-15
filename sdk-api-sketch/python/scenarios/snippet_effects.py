"""Snippet for Effects a program declares in code (#500, ADR 0106). No golden scenario places an order, so these
programs are the sketch's own."""

import logging

from pydantic import BaseModel, EmailStr

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    AsyncPage,
    AsyncRun,
    Effect,
    EffectDone,
    EffectReconciled,
    EffectUncertainError,
    Foxhound,
    seed,
)

log = logging.getLogger(__name__)


async def place_order(sku: str) -> str | None:
    """A Collie program safe to run again after a crash: the Run is named, so a start under the same ID resumes it, and
    the order placed before the crash comes back from the Effect journal."""
    async with AsyncFoxhound.connect() as foxhound, await foxhound.start(id=f"order-{sku}") as run:
        page = await run.new_page()
        await page.goto(f"https://shop.example/products/{sku}")
        await page.get_by_role("button", name="Add to cart").click()  # not declared: adding twice is harmless here
        await page.goto("https://shop.example/checkout")

        async def place(effect: Effect) -> str:
            await page.get_by_role("button", name="Place order").click()
            return (await page.get_by_test_id("order-number").text_content()).strip()  # kept in the journal

        placed = await run.effect(f"place-order-{sku}", place)
        await run.finish()
        match placed:
            case EffectDone(output=number):  # `performed` now, or `journal`: placed before a crash, not pressed again
                return number
            case EffectReconciled(reconciled_by=person):  # a person said it happened; its number is not known
                log.warning("order placed, number unknown, reconciled by %s", person)
                return None
            case _:
                return None


async def subscribe(run: AsyncRun, page: AsyncPage, plan: str) -> int:
    """A body that sends the site's own idempotency key is safe to repeat: declare it so."""

    async def post(effect: Effect) -> int:
        response = await page.request.post(
            "https://shop.example/api/subscriptions",
            headers={"Idempotency-Key": effect.idempotency_key},
            data={"plan": plan},
        )
        return response.status

    result = await run.effect(f"subscribe-{plan}", post, repeatable=True)
    return result.output if isinstance(result, EffectDone) else 0


# In a crawl handler: one form submitted per request, never twice across retries, escalations, sample comparisons or a
# resume. The key's namespace is the crawl request.
class Seller(BaseModel):
    email: EmailStr


crawl = AsyncCrawl()
listing = crawl.label("listing", payload=Seller)


@crawl.handle(listing)
async def contact_seller(ctx: AsyncCrawlContext[Seller]) -> None:
    await ctx.page.get_by_label("Email").fill(ctx.request.payload.email)  # preparing the page is no part of the Effect

    async def send(effect: Effect) -> None:
        await ctx.page.get_by_role("button", name="Contact seller").click()
        await ctx.page.get_by_text("Message sent").wait_for()

    try:
        sent = await ctx.run.effect("contact-seller", send)
    except EffectUncertainError as error:
        # The click went out and the confirmation never showed: a person reconciles it in the Console.
        log.warning("contact uncertain: %s (%s)", error.details.key, error.details.why)
        raise  # the request ends `failed` with `effect_uncertain`, never retried
    if not (isinstance(sent, EffectDone) and sent.source == "performed"):
        log.info("already contacted: %s", ctx.request.url)


async def contact_sellers(foxhound: AsyncFoxhound, emails: list[str]) -> None:
    run = await foxhound.start_crawl(
        crawl,
        id="contact-2026-10-15",
        seeds=[seed(f"https://market.example/listings/{i}", listing, Seller(email=email)) for i, email in enumerate(emails)],
    )
    async with run:
        report = await run.finish()
        for effect in report.uncertain_effects:
            log.warning("needs a person: %s on %s", effect.key, effect.request.url if effect.request else None)


def place_order_sync(sku: str) -> str | None:
    """The sync client: the body is a plain function, run on the calling thread."""
    with Foxhound.connect() as foxhound, foxhound.start(id=f"order-{sku}") as run:
        page = run.new_page()
        page.goto("https://shop.example/checkout")

        def place(effect: Effect) -> str:
            page.get_by_role("button", name="Place order").click()
            return page.get_by_test_id("order-number").text_content().strip()

        placed = run.effect(f"place-order-{sku}", place)
        run.finish()
        return placed.output if isinstance(placed, EffectDone) else None


async def mistakes(run: AsyncRun) -> None:
    """Typed mistakes, caught by the type checkers before anything runs."""

    async def not_json(effect: Effect) -> object:
        return object()

    await run.effect("k", not_json)  # type: ignore[type-var]  # pyright: ignore[reportArgumentType]
