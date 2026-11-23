// J-033's program (issue 2887), which Collie's `cli` Journey test runs
// with Node on Collie's executable of the same commit: `read <start>
// <runId>` reads every product of the catalog from its first listing page
// `start`, under the Run ID `runId`, until the catalog ends or a Limit
// stops the Run, and `raise` asks to raise the install's monthly requests
// Limit itself. Each prints what it saw as one JSON line on stdout.
import { Collie, LimitReachedError } from "@goodfetch/collie";

/** A product as its product page shows it, the price without its currency sign. */
interface Product {
  readonly id: string;
  readonly name: string;
  readonly price: string;
}

/** What a listing page links to: its products' pages and the next listing page. */
interface Listing {
  readonly products: readonly string[];
  readonly next: string | null;
}

const LISTING =
  "() => ({ products: [...document.querySelectorAll('li.product a')].map((link) => link.href), next: document.querySelector('a[rel=next]')?.href ?? null })";
const PRODUCT =
  "() => { const product = document.querySelector('article.product'); return { id: product.querySelector('.id').textContent, name: product.querySelector('.name').textContent, price: product.querySelector('.price').dataset.price }; }";

/** Prints `value` as the program's one line. */
function print(value: unknown): void {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

/** The code of the Goodfetch error `error`, or rethrows it. */
function codeOf(error: unknown): string {
  if (error instanceof Error && "code" in error && typeof error.code === "string") {
    return error.code;
  }
  throw error;
}

/** Reads the catalog from `start` in a Run `runId`, until it ends or a Limit stops the Run. */
async function read(collie: Collie, start: string, runId: string): Promise<void> {
  const run = await collie.start({ runId });
  const page = await run.newPage();
  const products: Product[] = [];
  let stoppedBy: string | null = null;
  try {
    for (let listing: string | null = start; listing !== null;) {
      await page.goto(listing);
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the page function answers the listing's links, which the catalog's markup fixes.
      const { products: links, next } = (await page.evaluate(LISTING)) as unknown as Listing;
      for (const link of links) {
        await page.goto(link);
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the page function answers the product's fields, which the catalog's markup fixes.
        products.push((await page.evaluate(PRODUCT)) as unknown as Product);
      }
      listing = next;
    }
  } catch (error) {
    if (!(error instanceof LimitReachedError)) throw error;
    stoppedBy = error.code;
  }
  const report = await run.finish();
  print({
    startedAs: run.startedAs(),
    stoppedBy,
    lifecycle: report.lifecycle,
    completeness: report.completeness,
    products,
  });
}

/** Asks the engine to raise the install's monthly requests Limit, as a program. */
async function raise(collie: Collie): Promise<void> {
  const refused = await collie
    .call("usage.raise", {
      scope: "install",
      usage_kind: "requests",
      amount: 1_000_000,
      period: { every: "month", time_zone: "UTC" },
    })
    .then(
      () => null,
      (error: unknown) => codeOf(error),
    );
  print({ refused });
}

const [command, start, runId] = process.argv.slice(2);
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect();
try {
  if (command === "read" && start !== undefined && runId !== undefined) {
    await read(collie, start, runId);
  } else if (command === "raise") {
    await raise(collie);
  } else {
    throw new TypeError(`usage: catalog-reader.ts read <start> <runId> | raise, not ${command}`);
  }
} finally {
  await collie.close();
}
