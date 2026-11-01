// The Console's own Client as an ordinary Client (spec #592; ADRs 0068 and
// 0069; story 12-138): `createConsoleClient` of the target's build, the
// protocol code the TypeScript SDK runs, reaches the target's `/protocol`
// with the page's sign-in, its endpoint and authentication given as
// configuration, and passes the JSON-RPC protocol suite's cases (#172) and
// the TypeScript SDK's lifecycle cases (#364) a browser's Client has. Each
// message either way is recorded, so a case reads what crossed the wire.
import type { BrowserContext, Page } from "@playwright/test";
import { expect, test } from "@goodfetch/test-support/conformance";
import type { ServedFixtures } from "@goodfetch/test-support/playwright";
import { command } from "@goodfetch/test-support/daemon-calls";
import type { ConsoleClient } from "@goodfetch/console/client";
import {
  connected,
  consoleClient,
  cookieHeader,
  failureCode,
  field,
  startRun,
  type Carriage,
} from "./target.ts";

/** What signing a Client in as the page takes. */
interface Signing extends Pick<ServedFixtures, "daemon" | "signIn"> {
  readonly page: Page;
  readonly context: BrowserContext;
}

/** The capability every case of this file proves. */
const CLIENT = { type: "capability", description: "client" };

/** Calls `method`, which no catalog has, as a screen never could. */
async function callUnknown(client: ConsoleClient, method: string): Promise<unknown> {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the case sends a method outside the catalog on purpose, which the operation map's types refuse.
  return client.call(method as "engine.info", {});
}

/** The engine's answers to the Client's request `id`. */
function answersTo(carried: readonly Carriage[], id: unknown): unknown[] {
  return carried
    .filter(({ from, message }) => from === "engine" && field(message, "id") === id)
    .map(({ message }) => message);
}

/** The id of the Client's last request of `method`. */
function requestId(carried: readonly Carriage[], method: string): unknown {
  return field(
    carried.findLast(
      ({ from, message }) => from === "client" && field(message, "method") === method,
    )?.message,
    "id",
  );
}

test.describe("the Console's Client on its target", () => {
  const clients: ConsoleClient[] = [];
  test.afterEach(() => {
    for (const client of clients.splice(0)) {
      client.close();
    }
  });

  /** The Console's Client, signed in as the page is, closed after the test. */
  async function signedIn(
    { page, context, daemon, signIn }: Signing,
    carried: Carriage[] = [],
  ): Promise<ConsoleClient> {
    await signIn.withLink(page);
    const client = consoleClient(daemon, await cookieHeader(context, daemon.url), carried);
    clients.push(client);
    await connected(client);
    return client;
  }

  test(
    "hello-is-answered-with-the-engines-product",
    { annotation: [CLIENT, { type: "covers", description: "story:12-138" }] },
    async ({ page, context, daemon, signIn }) => {
      const carried: Carriage[] = [];
      const client = await signedIn({ page, context, daemon, signIn }, carried);

      const hello = carried.find(
        ({ from, message }) => from === "client" && field(message, "method") === "hello",
      )?.message;
      expect(field(hello, "params")).toMatchObject({
        surface: "console",
        form: "catalog",
        needs: daemon.product,
      });
      const version = (await command(daemon, ["--version"])).trim();
      expect(client.engine?.product).toBe(daemon.product);
      expect(version).toContain(client.engine?.productVersion ?? "no version");
    },
  );

  test(
    "engine-info-names-the-engine-hello-named",
    { annotation: [CLIENT, { type: "covers", description: "story:12-138" }] },
    async ({ page, context, daemon, signIn }) => {
      const client = await signedIn({ page, context, daemon, signIn });

      const info = await client.call("engine.info", {});

      expect(info).toMatchObject({
        product: client.engine?.product,
        product_version: client.engine?.productVersion,
      });
    },
  );

  test(
    "an-unknown-method-is-method-not-found",
    { annotation: [CLIENT, { type: "covers", description: "story:12-138" }] },
    async ({ page, context, daemon, signIn }) => {
      const client = await signedIn({ page, context, daemon, signIn });

      expect(await failureCode(callUnknown(client, "conformance.unknown_method"))).toBe(
        "method_not_found",
      );
      // The Connection still answers.
      await expect(client.call("engine.info", {})).resolves.toMatchObject({
        product: daemon.product,
      });
    },
  );

  test(
    "params-the-operation-does-not-take-are-invalid-params",
    { annotation: [CLIENT, { type: "covers", description: "story:12-138" }] },
    async ({ page, context, daemon, signIn }) => {
      const client = await signedIn({ page, context, daemon, signIn });

      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the case sends a param the operation does not take on purpose, which its type refuses.
      const unknown = { unknown: true } as unknown as Record<string, never>;

      expect(await failureCode(client.call("engine.info", unknown))).toBe("invalid_params");
    },
  );

  test(
    "a-call-in-flight-cancelled-ends-with-call-cancelled",
    {
      annotation: [
        CLIENT,
        { type: "covers", description: "story:12-138" },
        { type: "covers", description: "error:call_cancelled" },
      ],
    },
    async ({ page, context, daemon, signIn }) => {
      const carried: Carriage[] = [];
      const client = await signedIn({ page, context, daemon, signIn }, carried);
      const subscription = client.subscribe("events");
      await expect.poll(() => requestId(carried, "events.subscribe")).toBeDefined();
      const id = requestId(carried, "events.subscribe");

      subscription.close();

      await expect
        .poll(() =>
          carried.some(
            ({ from, message }) =>
              from === "client" &&
              field(message, "method") === "$/cancel" &&
              field(field(message, "params"), "id") === id,
          ),
        )
        .toBe(true);
      await expect
        .poll(() => answersTo(carried, id).map((answer) => field(field(answer, "error"), "data")))
        .toEqual([expect.objectContaining({ code: "call_cancelled" })]);
    },
  );

  test(
    "cancelling-an-answered-call-leaves-the-connection-answering",
    { annotation: [CLIENT, { type: "covers", description: "story:12-138" }] },
    async ({ page, context, daemon, signIn }) => {
      const carried: Carriage[] = [];
      const client = await signedIn({ page, context, daemon, signIn }, carried);
      const abort = new AbortController();
      await client.call("engine.info", {}, { signal: abort.signal });
      const id = requestId(carried, "engine.info");

      abort.abort();

      await expect(client.call("engine.info", {})).resolves.toMatchObject({
        product: daemon.product,
      });
      expect(answersTo(carried, id)).toHaveLength(1);
      expect(client.state).toBe("connected");
    },
  );

  test(
    "two-connections-at-once",
    { annotation: [CLIENT, { type: "covers", description: "story:12-138" }] },
    async ({ page, context, daemon, signIn }) => {
      const first = await signedIn({ page, context, daemon, signIn });
      const second = consoleClient(daemon, await cookieHeader(context, daemon.url));
      clients.push(second);
      await connected(second);
      await startRun(daemon, "both");

      const lists = await Promise.all(
        [first, second].map(async (client) => client.call("run.list", {})),
      );

      for (const list of lists) {
        expect(JSON.stringify(list)).toContain('"both"');
      }
      expect(first.engine?.engineInstance).toBe(second.engine?.engineInstance);
    },
  );

  test(
    "connect-after-close",
    {
      annotation: [
        CLIENT,
        { type: "covers", description: "story:12-138" },
        { type: "covers", description: "error:connection_lost" },
      ],
    },
    async ({ page, context, daemon, signIn }) => {
      const first = await signedIn({ page, context, daemon, signIn });
      first.close();

      const second = consoleClient(daemon, await cookieHeader(context, daemon.url));
      clients.push(second);
      await connected(second);

      await expect(second.call("engine.info", {})).resolves.toMatchObject({
        product: daemon.product,
      });
      expect(await failureCode(first.call("engine.info", {}))).toBe("connection_lost");
    },
  );
});
