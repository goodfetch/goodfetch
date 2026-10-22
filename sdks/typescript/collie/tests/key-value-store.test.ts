// A Run's Key-value store through the SDK, on an Attached engine of the
// executable of the same commit (ADRs 0069 and 0103): `get` on an absent
// key answers null, a set value comes back, `delete` answers whether the
// key existed, and `set(key, null)` fails `tsc`, while an untyped caller's
// null fails in the engine with `value_invalid`, naming `delete`.
import { Collie } from "@goodfetch/collie";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}

describe("a Run's Key-value store on an Attached engine", () => {
  test(
    "answers null for an absent key, a set value, and whether a deleted key existed",
    {
      tags: [
        "client_case/key-value-store-get-absent-answers-null",
        "attached",
        "story:6-28",
        "story:6-31",
      ],
    },
    async ({ env, clientCase }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const run = await collie.start({ runId: "stored" });

      const absent = await run.keyValueStore.get("cursor");
      await run.keyValueStore.set("cursor", { page: 3, seen: [1, null] });
      const stored = await run.keyValueStore.get("cursor");
      const deleted = await run.keyValueStore.delete("cursor");
      const again = await run.keyValueStore.delete("cursor");
      await run.finish();
      await collie.close();

      await clientCase.check("get on an absent key answers null", () => expect(absent).toBeNull());
      await clientCase.check("a set value comes back with its nested null", () =>
        expect(stored).toEqual({ page: 3, seen: [1, null] }),
      );
      await clientCase.check("delete answers whether the key existed", () =>
        expect([deleted, again]).toEqual([true, false]),
      );
    },
  );

  test(
    "refuses a null value: tsc rejects it, and the engine fails it with value_invalid",
    {
      tags: [
        "client_case/key-value-store-set-null-refused",
        "attached",
        "story:6-29",
        "error:value_invalid",
      ],
    },
    async ({ env, clientCase }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const run = await collie.start({ runId: "nulled" });

      // @ts-expect-error -- `set` takes no top-level null (G3); this untyped call reaches the engine's refusal.
      const refused = await failure(run.keyValueStore.set("cursor", null));
      const after = await run.keyValueStore.get("cursor");
      await run.finish();
      await collie.close();

      await clientCase.check("the engine refused the null with value_invalid", () =>
        expect(refused).toMatchObject({
          code: "value_invalid",
          group: "program",
          retryable: false,
          details: { key: "cursor", refused: "value", reason: "value_null" },
        }),
      );
      await clientCase.check("its fix names delete", () =>
        expect(refused).toHaveProperty("fix", expect.stringContaining("`delete`")),
      );
      await clientCase.check("nothing was stored", () => expect(after).toBeNull());
    },
  );
});
