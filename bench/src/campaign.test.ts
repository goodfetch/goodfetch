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
    { ...campaign, retries: 3 },
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
