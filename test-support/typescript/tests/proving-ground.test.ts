// Seam B (spec #325): the `provingGround` fixture, used exactly as a product
// test uses it, against the real `goodfetch-test-env --proving-ground` and
// the commit's Proving ground, which the tool manifest names. A test reaches
// its pages through the `resolve` mapping, its forward proxies and its
// connection record with no setup of its own.
import { get, type IncomingHttpHeaders } from "node:http";
import { readFileSync } from "node:fs";
import { describe, expect } from "vitest";
import { provingGroundOf, test } from "@goodfetch/test-support";

/** An answer's status and body. */
interface Answer {
  readonly status: number;
  readonly body: string;
}

/** Sends `GET <path>` to `address` (`<ip>:<port>`) with these headers. */
function getFrom(address: string, path: string, headers: IncomingHttpHeaders): Promise<Answer> {
  const end = address.lastIndexOf(":");
  return new Promise((resolve, reject) => {
    const request = get(
      {
        host: address.slice(0, end),
        port: Number(address.slice(end + 1)),
        path,
        headers: { ...headers, connection: "close" },
      },
      (response) => {
        let body = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => {
          body += chunk;
        });
        response.on("end", () => {
          resolve({ status: response.statusCode ?? 0, body });
        });
      },
    );
    request.on("error", reject);
  });
}

describe("provingGround", () => {
  test.override("provingGroundSeed", 341);

  test("answers on its address with the seed asked for", async ({ provingGround }) => {
    const answer = await getFrom(provingGround.address, "/tasks", {});

    expect(provingGround.seed).toBe(341);
    expect(JSON.parse(answer.body)).toMatchObject({ proving_ground: { seed: 341 } });
  });

  test("reaches catalog.test through its resolve mapping", async ({ provingGround }) => {
    const port = provingGround.address.slice(provingGround.address.lastIndexOf(":") + 1);
    const ip = provingGround.resolve["catalog.test"];

    const answer = await getFrom(`${ip}:${port}`, "/catalog/1", {
      host: `catalog.test:${port}`,
    });

    expect(answer.status).toBe(200);
    expect(answer.body).toContain("/products/P-0001");
  });

  test("reaches catalog.test through the open forward proxy, whose relay the connection record holds", async ({
    provingGround,
  }) => {
    const port = provingGround.address.slice(provingGround.address.lastIndexOf(":") + 1);
    const url = `http://catalog.test:${port}/catalog/1`;
    const proxy = provingGround.fixtures.proxies["open"] ?? "";

    const answer = await getFrom(proxy, url, { host: `catalog.test:${port}` });

    const record = await fetch(provingGround.fixtures.connections).then((response) =>
      response.json(),
    );
    expect(answer.status).toBe(200);
    expect(record).toMatchObject({ relays: [{ proxy: "open", target: url, outcome: "relayed" }] });
  });

  test("names the file of its certificate authority", ({ provingGround }) => {
    const authority = readFileSync(provingGround.ca, "utf8");

    expect(authority).toMatch(/^-----BEGIN CERTIFICATE-----\n/);
  });
});

describe("provingGround without a seed", () => {
  test("fails the fixture with proving_ground_unseeded", ({ testEnv }) => {
    expect(() => provingGroundOf(testEnv)).toThrow(
      expect.objectContaining({ code: "proving_ground_unseeded" }),
    );
  });
});
