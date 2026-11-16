// A Run's pages on the Scripted engine (ADR 0069): each test plays a script
// of the shared corpus over the in-memory transport. The script fails the
// test on a message it does not expect and on a step left unused, so a
// member that sent a second request, or none, fails here.
import type { Page, PageField } from "@goodfetch/collie-client";
import { failure, playScript, protocolCounts, test } from "@goodfetch/test-support";
import { describe, expect, expectTypeOf } from "vitest";
import { fillTheForm } from "../tests/programs/form.ts";
import type { Collie } from "./collie.ts";
import { connectOver } from "./testing.ts";

/** The Scripted engine playing `script`, and a Collie Connection opened over it. */
async function collieOn(
  script: string,
): Promise<{ collie: Collie; played: ReturnType<typeof playScript> }> {
  const played = playScript(script);
  const collie = await connectOver(played.transport);
  return { collie, played };
}

describe("a Run's page", () => {
  test(
    "runs the form program with one request a member called",
    { tags: ["story:11-77"] },
    async () => {
      const { collie, played } = await collieOn("page/form-filled.json");

      const filled = await fillTheForm(collie, "https://20.actionability.test/");

      expect(filled).toEqual({
        status: 200,
        markdown: "done\n\nName\n\nSave",
        lifecycle: "finished",
      });
      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 11,
          "events": 0,
          "replies": 0,
          "requests": [
            "hello",
            "run.start",
            "run.new_page",
            "page.goto",
            "locator.fill",
            "locator.click",
            "locator.press",
            "locator.wait_for",
            "page.formats",
            "page.close",
            "run.finish",
          ],
          "served": [],
        }
      `);
    },
  );

  test(
    "refuses a call on a closed page with closed, sending nothing",
    { tags: ["story:11-77", "error:closed"] },
    async () => {
      const { collie, played } = await collieOn("page/closed.json");
      const run = await collie.start();
      const page = await run.newPage();
      await page.close();

      const navigation = await failure(page.goto("https://20.actionability.test/"));

      expect(navigation).toMatchObject({ code: "closed", group: "program" });
      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses an action of a Locator built on a page since closed with closed",
    { tags: ["story:11-77", "error:closed"] },
    async () => {
      const { collie, played } = await collieOn("page/closed.json");
      const run = await collie.start();
      const page = await run.newPage();
      const button = page.locator("#submit");
      await page.close();

      const click = await failure(button.click());

      expect(click).toMatchObject({ code: "closed", group: "program" });
      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("closes a page twice with one request", { tags: ["story:11-77"] }, async () => {
    const { collie, played } = await collieOn("page/closed.json");
    const run = await collie.start();
    const page = await run.newPage();
    await page.close();

    await page.close();

    await run.finish();
    await collie.close();
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test(
    "closes a page of a closed Session without a request",
    { tags: ["story:11-77"] },
    async () => {
      const { collie, played } = await collieOn("page/session-closed.json");
      const run = await collie.start();
      const session = await run.newSession();
      const page = await session.newPage();
      await session.close();

      await page.close();

      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );
});

/** A CSS locator of `selector`. */
const css = (selector: string) => ({ kind: "css", selector }) as const;

/** The Value transform that reads an English number. */
const NUMBER = { kind: "number", locale: "en" } as const;

/** The twenty fields `page/read-twenty-fields.json` reads. */
async function readTwentyFields(page: Page) {
  return page.read({
    title: { locator: css("h1") },
    price: { locator: css(".price"), transform: NUMBER },
    currency: { locator: css(".price"), attribute: "data-currency" },
    sku: { locator: { kind: "xpath", selector: "//span[@itemprop='sku']" } },
    brand: { locator: css(".brand") },
    rating: { locator: css(".rating"), transform: NUMBER },
    reviews: {
      locator: css(".reviews"),
      match: { pattern: String.raw`(\d+) reviews`, group: 1 },
      transform: NUMBER,
    },
    released: { locator: css("time.released"), transform: { kind: "date", locale: "en" } },
    updated: {
      locator: css("time.updated"),
      transform: { kind: "date_time", locale: "en", time_zone: "Europe/Paris" },
    },
    image: { locator: css("img.main"), attribute: "src" },
    canonical: { locator: css("link[rel=canonical]"), attribute: "href" },
    description: { locator: css(".description") },
    availability: { locator: css(".stock") },
    weight: { locator: css(".weight"), transform: NUMBER },
    color: { locator: css(".color") },
    material: { locator: css(".material") },
    warranty: { locator: css(".warranty") },
    tags: { list: { locator: css(".tag") } },
    thumbnails: { list: { locator: css("img.thumb"), attribute: "src" } },
    offers: {
      rows: css(".offer"),
      fields: {
        seller: { locator: css(".seller") },
        price: { locator: css(".offer-price"), transform: NUMBER },
      },
    },
  });
}

/** A text field and a number field of `page`. */
async function readTitleAndPrice(page: Page) {
  return page.read({
    title: { locator: css("h1") },
    price: { locator: css(".price"), transform: NUMBER },
  });
}

/** `field`, declared as any field, of `page`. */
async function readAnyField(page: Page, field: PageField) {
  return page.read({ field });
}

describe("a page's read", () => {
  test(
    "reads twenty fields with one request, each null with why",
    { tags: ["story:11-77"] },
    async () => {
      const { collie, played } = await collieOn("page/read-twenty-fields.json");
      const run = await collie.start();
      const page = await run.newPage();

      const read = await readTwentyFields(page);

      expect(read.values).toMatchObject({
        title: "Blue kettle",
        price: 12.34,
        released: "2026-01-15",
        warranty: null,
        tags: ["kitchen", "kettle"],
        offers: [
          { seller: "Shop A", price: 12.34 },
          { seller: "Shop B", price: null },
        ],
      });
      expect(read.unread).toEqual([
        { field: "/warranty", reason: "no_element" },
        { field: "/offers/1/price", reason: "not_parsed", text: "call us" },
      ]);
      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 5,
          "events": 0,
          "replies": 0,
          "requests": [
            "hello",
            "run.start",
            "run.new_page",
            "page.read",
            "run.finish",
          ],
          "served": [],
        }
      `);
    },
  );

  test("types a text field as a string and a number field as a number", () => {
    expectTypeOf<Awaited<ReturnType<typeof readTitleAndPrice>>["values"]>().toEqualTypeOf<{
      title: string | null;
      price: number | null;
    }>();
  });

  test("takes a signal and no timeout, as every query", () => {
    expectTypeOf<Parameters<Page["read"]>[1]>().toEqualTypeOf<
      { readonly signal?: AbortSignal | undefined } | undefined
    >();
  });

  test("types a list as an array and rows as an array of objects", () => {
    expectTypeOf<Awaited<ReturnType<typeof readTwentyFields>>["values"]>().toEqualTypeOf<{
      title: string | null;
      price: number | null;
      currency: string | null;
      sku: string | null;
      brand: string | null;
      rating: number | null;
      reviews: number | null;
      released: string | null;
      updated: string | null;
      image: string | null;
      canonical: string | null;
      description: string | null;
      availability: string | null;
      weight: number | null;
      color: string | null;
      material: string | null;
      warranty: string | null;
      tags: (string | null)[];
      thumbnails: (string | null)[];
      offers: { seller: string | null; price: number | null }[];
    }>();
  });

  test("types a field declared as any field as any value", () => {
    expectTypeOf<Awaited<ReturnType<typeof readAnyField>>["values"]["field"]>().toEqualTypeOf<
      | string
      | number
      | null
      | (string | number | null)[]
      | { [name: string]: string | number | null }[]
    >();
  });
});
