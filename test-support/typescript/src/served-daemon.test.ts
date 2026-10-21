import { describe, expect, test } from "vitest";
import { ProcessRecord } from "./process-record.ts";
import { startServedDaemon } from "./served-daemon.ts";

describe("startServedDaemon", () => {
  test("fails with console_not_served when nothing serves the Console", async () => {
    const processes = new ProcessRecord();

    const started = startServedDaemon({
      processes,
      testName: "a served test",
      toolManifest: undefined,
      product: "collie",
      provingGroundSeed: undefined,
      consoleServer: undefined,
      oidc: {
        issuer: "http://127.0.0.1:1",
        clientId: "console",
        clientSecret: "secret",
        redirectPath: "/sso/callback",
      },
    });

    await expect(started).rejects.toMatchObject({ code: "console_not_served" });
    await processes.close();
  });
});
