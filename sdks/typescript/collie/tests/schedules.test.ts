// A Connection's Schedules (spec 589, ADR 0099), the `schedules-crud`
// client conformance case, on an Attached engine of the executable of the
// same commit: `schedules.create` answers a `Schedule` under the ID given,
// or one it made when none was, `schedules.list` lists both,
// `schedules.get` answers the handle of a held ID and fails an unknown one
// with `schedule_not_found`, and a handle's `info()` reads what the store
// holds and its `delete()` answers whether it deleted it.
import { Collie } from "@goodfetch/collie";
import { failure, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("a Connection's Schedules on an Attached engine", () => {
  test(
    "creates, lists, reads and deletes a Schedule, and fails an unknown ID with schedule_not_found",
    {
      tags: [
        "client_case/schedules-crud",
        "attached",
        "story:10-2",
        "operation:schedule.create",
        "operation:schedule.list",
        "operation:schedule.get",
        "operation:schedule.info",
        "operation:schedule.delete",
        "error:schedule_not_found",
      ],
    },
    async ({ env, clientCase }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const named = await collie.schedules.create({
        id: "nightly-prices",
        target: { recipe: "shop-prices" },
        trigger: { every: 3_600_000 },
        timeZone: "Europe/Paris",
      });
      const made = await collie.schedules.create(
        { target: { recipe: "shop-prices" }, trigger: { every: 60_000 } },
        { timeout: 10_000 },
      );
      const listed = await collie.schedules.list();
      const got = await collie.schedules.get("nightly-prices", { timeout: null });
      const info = await got.info();
      const unknown = await failure(collie.schedules.get("never-created"));
      const deleted = await named.schedule.delete();
      const again = await got.delete();
      const gone = await failure(got.info());
      await collie.close();

      await clientCase.check(
        "create answers the Schedule under the ID given, with no warnings",
        () => {
          expect(named.schedule.id()).toBe("nightly-prices");
          expect(named.warnings).toEqual([]);
        },
      );
      await clientCase.check("create makes an ID when the definition names none", () =>
        expect(made.schedule.id()).toMatch(/^s[0-9a-z]{26}$/u),
      );
      await clientCase.check("list holds both Schedules", () =>
        expect(listed.map((schedule) => schedule.scheduleId).toSorted()).toEqual(
          [made.schedule.id(), "nightly-prices"].toSorted(),
        ),
      );
      await clientCase.check("get answers the handle, whose info reads the stored Schedule", () => {
        expect(got.id()).toBe("nightly-prices");
        expect(info).toMatchObject({
          scheduleId: "nightly-prices",
          target: { recipe: "shop-prices" },
          trigger: { every: 3_600_000 },
          timeZone: "Europe/Paris",
          version: 1,
          state: "on",
        });
      });
      await clientCase.check("get of an unknown ID fails with schedule_not_found", () =>
        expect(unknown).toMatchObject({
          code: "schedule_not_found",
          details: { scheduleId: "never-created" },
        }),
      );
      await clientCase.check("delete answers whether it deleted, and info then fails", () => {
        expect([deleted, again]).toEqual([true, false]);
        expect(gone).toMatchObject({ code: "schedule_not_found" });
      });
    },
  );
});
