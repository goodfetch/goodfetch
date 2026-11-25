// The program's test, on Collie's testing kit: a test engine whose Runs
// reach only the hosts `network.resolve` maps to loopback, here the page
// this test serves itself, so it needs no network.
import { strict as assert } from "node:assert";
import { createServer } from "node:http";
import { test } from "node:test";
import { TestEngine } from "@goodfetch/collie/testing";
import { readTitle } from "../src/title.ts";

await test("reads the page's title", async () => {
  const server = createServer((_request, response) => {
    response.setHeader("content-type", "text/html; charset=utf-8");
    response.end("<!doctype html><title>Hello from the starter</title>");
  });
  await new Promise<void>((listening) => server.listen(0, "127.0.0.1", listening));
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error(`the page's server listens on no port: ${String(address)}`);
  }
  const { port } = address;
  try {
    await using engine = await TestEngine.start({
      settings: { network: { resolve: { "starter.test": ["127.0.0.1"] } } },
    });

    const title = await readTitle(engine.connection, `http://starter.test:${port}/`);

    assert.equal(title, "Hello from the starter");
  } finally {
    server.close();
  }
});
