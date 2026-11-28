// Unit tests for `serveSecretResolver` (ADRs 0067 and 0092, issue 1281):
// parses `secrets.resolve` params into `SecretRequest`, returns `{ value }`
// straight through, and redacts any thrown error so no Secret canary can reach
// the JSON-RPC error reply.
import { describe, expect, test } from "vitest";
import { replyOfError } from "./errors.ts";
import { SecretUnavailableError } from "./generated/errors.ts";
import { serveSecretResolver, type SecretRequest } from "./secrets.ts";

const CANARY = "gf-canary-secret-value-9f8e7d6c";

describe("serveSecretResolver", () => {
  test("passes name, runId and domain and returns the resolved value", async () => {
    const seen: SecretRequest[] = [];
    const served = serveSecretResolver(async (request) => {
      seen.push(request);
      return CANARY;
    });

    const controller = new AbortController();
    const withDomain = await served(
      { name: "shop_password", run_id: "nightly", domain: "shop.test" },
      controller.signal,
    );
    const withoutDomain = await served({ name: "api_key", run_id: "nightly" }, controller.signal);

    expect(withDomain).toEqual({ value: CANARY });
    expect(withoutDomain).toEqual({ value: CANARY });
    expect(seen).toEqual([
      { name: "shop_password", runId: "nightly", domain: "shop.test" },
      { name: "api_key", runId: "nightly" },
    ]);
  });

  test("a thrown error becomes a redacted secret_unavailable with no canary in the JSON-RPC error", async () => {
    const served = serveSecretResolver(async () => {
      throw new Error(`vault failed while reading ${CANARY}`);
    });

    const controller = new AbortController();
    let caught: unknown;
    try {
      await served({ name: "shop_password", run_id: "nightly" }, controller.signal);
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(SecretUnavailableError);
    const reply = replyOfError(caught);
    expect(reply.data["code"]).toBe("secret_unavailable");
    expect(reply.data["failure"]).toBe("resolver_failed");
    expect(JSON.stringify(reply)).not.toContain(CANARY);
    expect(String(caught)).not.toContain(CANARY);
  });

  test("a thrown SecretUnavailableError keeps its failure tag and strips its message", async () => {
    const served = serveSecretResolver(async () => {
      throw new SecretUnavailableError(`missing ${CANARY}`, {
        details: { failure: "missing", leaked: CANARY },
      });
    });

    const controller = new AbortController();
    let caught: unknown;
    try {
      await served({ name: "shop_password", run_id: "nightly" }, controller.signal);
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(SecretUnavailableError);
    const reply = replyOfError(caught);
    expect(reply.data["code"]).toBe("secret_unavailable");
    expect(reply.data["failure"]).toBe("missing");
    expect(JSON.stringify(reply)).not.toContain(CANARY);
  });

  test("malformed params fail without echoing them", async () => {
    const served = serveSecretResolver(async () => CANARY);

    const controller = new AbortController();
    let caught: unknown;
    try {
      await served({ name: 123, run_id: CANARY }, controller.signal);
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(SecretUnavailableError);
    const reply = replyOfError(caught);
    expect(reply.data["failure"]).toBe("resolver_failed");
    expect(JSON.stringify(reply)).not.toContain(CANARY);
  });
});
