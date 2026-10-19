import { describe, expect, test } from "vitest";
import { provingGroundOf } from "./proving-ground.ts";
import type { TestEnvDescription } from "./test-env.ts";

const DESCRIPTION: TestEnvDescription = {
  folder: "/tmp/a test é x",
  home: "/tmp/a test é x/home",
  env: { GOODFETCH_HOME: "/tmp/a test é x/home", TZ: "UTC" },
  seed: 1,
  start_time: "2026-01-05T09:00:00Z",
  tools: {},
};

const PROVING_GROUND = {
  address: "127.0.0.1:4000",
  version: "0.0.0+0",
  seed: 341,
  resolve: { "catalog.test": "127.0.0.1" },
  ca: "/tmp/a test é x/proving-ground-1-1/certificate-authority.pem",
  fixtures: {
    proxies: { open: "127.0.0.1:4001" },
    proxy_credentials: { username: "pg1", password: "secret" },
    dns: "127.0.0.1:4002",
    target: "127.0.0.1:4003",
    connections: "http://127.0.0.1:4000/observations",
  },
} as const;

describe("provingGroundOf", () => {
  test("fails when the description names no Proving ground", () => {
    expect(() => provingGroundOf(DESCRIPTION)).toThrow(
      expect.objectContaining({ code: "proving_ground_unseeded" }),
    );
  });

  test("is the Proving ground the description names", () => {
    const provingGround = provingGroundOf({ ...DESCRIPTION, proving_ground: PROVING_GROUND });

    expect(provingGround).toBe(PROVING_GROUND);
  });
});
