import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "vitest";
import { loadCampaign } from "./campaign.ts";
import { errorCode } from "./errors.ts";

const campaign = {
  id: "stub-catalog",
  host_class: { requested: "local-dev", allow_downgrade: false },
  order_seed: 7,
  blocks: 2,
  warmup: 1,
  iterations: 4,
  concurrency: 2,
  timeout_ms: 5000,
  max_duration_ms: 60_000,
  max_restarts: 1,
  adapters: [{ id: "stub", command: ["node", "stub-adapter.ts"] }],
  tasks: ["catalog/2"],
};

async function written(document: unknown): Promise<string> {
  const path = join(await mkdtemp(join(tmpdir(), "goodfetch-campaign-")), "campaign.json");
  await writeFile(path, JSON.stringify(document));
  return path;
}

test("reads a campaign with the folder its adapters start in", async () => {
  const path = await written(campaign);

  const loaded = await loadCampaign(path);

  expect(loaded.campaign).toEqual(campaign);
  expect(loaded.folder).toBe(join(path, ".."));
  expect(loaded.sha256).toMatch(/^[0-9a-f]{64}$/);
});

test("refuses a campaign that does not have the file's shape", async () => {
  const cases = [
    { ...campaign, retries: 0 },
    { ...campaign, adapters: [] },
    { ...campaign, host_class: { requested: "cloud", allow_downgrade: false } },
  ];

  const codes = await Promise.all(
    cases.map(async (document) =>
      loadCampaign(await written(document)).then(
        () => "read",
        (error: unknown) => errorCode(error),
      ),
    ),
  );

  expect(codes).toEqual(["invalid_document", "invalid_document", "invalid_document"]);
});

/** A campaign whose `goodfetch` adapter is matched against `against`, beside the tool `tool`. */
function matchedAgainst(against: string, tool: Record<string, unknown>): unknown {
  return {
    ...campaign,
    adapters: [
      { id: "tool", command: ["node", "tool.ts"], ...tool },
      {
        id: "goodfetch",
        command: ["node", "goodfetch.ts"],
        matched: { against, settings: { "fetch.tier": "http" } },
      },
    ],
  };
}

test("gives each adapter its row: default, or matched with the settings it names and the tool's published defaults", async () => {
  const path = await written(matchedAgainst("tool", { configuration: "tool.json" }));
  await writeFile(
    join(path, "..", "tool.json"),
    JSON.stringify({
      tool: "a tool",
      defaults: { filter_list_blocking: false, consent_prompt_handling: true },
    }),
  );

  const loaded = await loadCampaign(path);

  expect(Object.fromEntries(loaded.rows)).toEqual({
    tool: { kind: "default" },
    goodfetch: {
      kind: "matched",
      against: "tool",
      settings: { "fetch.tier": "http" },
      competitor_defaults: { filter_list_blocking: false, consent_prompt_handling: true },
    },
  });
});

test("refuses a matched row against no adapter of the campaign, against itself, or against a tool with no published configuration", async () => {
  const cases = [
    matchedAgainst("absent", { configuration: "tool.json" }),
    matchedAgainst("goodfetch", { configuration: "tool.json" }),
    matchedAgainst("tool", {}),
  ];

  const codes = await Promise.all(
    cases.map(async (document) => {
      const path = await written(document);
      await writeFile(
        join(path, "..", "tool.json"),
        JSON.stringify({ defaults: { filter_list_blocking: true, consent_prompt_handling: true } }),
      );
      return loadCampaign(path).then(
        () => "read",
        (error: unknown) => errorCode(error),
      );
    }),
  );

  expect(codes).toEqual(["invalid_document", "invalid_document", "invalid_document"]);
});

test("refuses a published configuration that does not declare both defaults", async () => {
  const path = await written(matchedAgainst("tool", { configuration: "tool.json" }));
  await writeFile(
    join(path, "..", "tool.json"),
    JSON.stringify({ defaults: { filter_list_blocking: false } }),
  );

  await expect(loadCampaign(path)).rejects.toMatchObject({ code: "invalid_document" });
});
