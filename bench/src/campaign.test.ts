import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
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
  expect(loaded.declared).toEqual({ decisions: [], claims: [] });
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

/** A campaign of adapters `a` and `b` declaring `D-example`, whose registry holds `decision` and `claim`. */
async function declaring(decision: Record<string, unknown>, claim: Record<string, unknown>) {
  const path = await written({
    ...campaign,
    adapters: ["a", "b"].map((id) => ({ id, command: ["node", "stub-adapter.ts"] })),
    registry: { folder: "registry", decisions: ["D-example"] },
  });
  const root = join(path, "..", "registry");
  await mkdir(join(root, "decisions"), { recursive: true });
  await mkdir(join(root, "claims"));
  await writeFile(join(root, "decisions", "D-example.json"), JSON.stringify(decision));
  await writeFile(join(root, "claims", "C-example.json"), JSON.stringify(claim));
  return path;
}

const declaredAt = "2026-11-01T00:00:00.000Z";
const decision = {
  id: "D-example",
  title: "Example",
  question: "Which option?",
  status: "open",
  declared_at: declaredAt,
  options: [
    { id: "a", description: "A", adapter: "a" },
    { id: "b", description: "B", adapter: "b" },
  ],
  baseline: "a",
  primary_metric: "task_p50_ms_verified",
  secondary_metrics: [],
  contract_gates: ["oracle_exact"],
  delta: 0.5,
  rule: { kind: "superiority", alpha: 0.05, power: 0.8, noisy_blocks: "exclude" },
  host_class: { decision: "local-dev", publication: "reference" },
  evidence: [],
  depends_on: [],
  notes: "",
};
const claim = {
  id: "C-example",
  statement: "b answers faster than a",
  decision: "D-example",
  metric: "task_p50_ms_verified",
  comparator: { option: "b", against: ["a"] },
  direction: "lower",
  delta: 0.5,
  publication_host_class: "reference",
  status: "candidate",
  declared_at: declaredAt,
  evidence: [],
  notes: "",
};

test("reads the decisions a campaign declares, with their claims, as they stand before the run", async () => {
  const loaded = await loadCampaign(await declaring(decision, claim));

  expect(loaded.declared).toEqual({
    decisions: [{ id: "D-example", declared_at: declaredAt }],
    claims: [{ id: "C-example", declared_at: declaredAt }],
  });
});

test("refuses, before any measurement, an unknown decision, an option it cannot run or a late claim", async () => {
  const cases = [
    { decision: { ...decision, id: "D-other" }, claim: { ...claim, decision: "D-other" } },
    {
      decision: {
        ...decision,
        options: [...decision.options, { id: "c", description: "C", adapter: "c" }],
      },
      claim,
    },
    { decision, claim: { ...claim, declared_at: "2999-01-01T00:00:00.000Z" } },
  ];

  const messages = await Promise.all(
    cases.map(async (one) =>
      loadCampaign(await declaring(one.decision, one.claim)).then(
        () => "read",
        (error: unknown) => `${errorCode(error)}: ${error instanceof Error ? error.message : ""}`,
      ),
    ),
  );

  expect(messages).toEqual([
    expect.stringMatching(/^registry_refused: .*the campaign declares unknown decision D-example$/),
    expect.stringMatching(
      /^registry_refused: .*D-example: option c's adapter c is not one of the campaign's$/,
    ),
    expect.stringMatching(
      /^registry_refused: .*C-example: declared at 2999-01-01T00:00:00.000Z, after now$/,
    ),
  ]);
});

test("refuses a published configuration that does not declare both defaults", async () => {
  const path = await written(matchedAgainst("tool", { configuration: "tool.json" }));
  await writeFile(
    join(path, "..", "tool.json"),
    JSON.stringify({ defaults: { filter_list_blocking: false } }),
  );

  await expect(loadCampaign(path)).rejects.toMatchObject({ code: "invalid_document" });
});
