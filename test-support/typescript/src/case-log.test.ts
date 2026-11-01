import { describe, expect, test } from "vitest";
import {
  checkedCaseLogEntry,
  clientCaseRecord,
  clientCaseTags,
  parseClientCases,
  rustPlatform,
  type CaseRecording,
} from "./case-log.ts";

describe("rustPlatform", () => {
  test.for([
    ["darwin", "arm64", { os: "macos", arch: "aarch64" }],
    ["darwin", "x64", { os: "macos", arch: "x86_64" }],
    ["linux", "x64", { os: "linux", arch: "x86_64" }],
    ["linux", "arm64", { os: "linux", arch: "aarch64" }],
    ["win32", "x64", { os: "windows", arch: "x86_64" }],
    ["win32", "arm64", { os: "windows", arch: "aarch64" }],
  ] as const)("names Node's %s on %s as Rust's std::env::consts do", ([platform, arch, rust]) => {
    expect(rustPlatform(platform, arch)).toEqual(rust);
  });
});

const CASE_LIST = `
[suite]
name = "clients"
version = "1"

[[capability]]
name = "hello"

[[capability.case]]
id = "hello-refused"
covers = ["story:11-1"]
surfaces = ["sdk-typescript", "cli"]

[[capability]]
name = "cancel"

[[capability.case]]
id = "cancel-before-start"
covers = ["story:11-4"]
surfaces = ["sdk-typescript"]

[[capability.case]]
id = "cancel-mid-run"
covers = ["story:11-4"]
surfaces = ["sdk-typescript"]
modes = ["attached"]
`;

const entry = {
  case: "hello-refused",
  capability: "hello",
  mode: "attached",
  platform: { os: "linux", arch: "x86_64" },
  runtime: "node-24",
  checks: [{ name: "the Client raises protocol_unsupported", passed: true }],
  evidence: { messages: [{ method: "hello" }] },
};

describe("parseClientCases", () => {
  test("gives each declared case with its capability, in the list's order", () => {
    expect(parseClientCases(CASE_LIST)).toEqual([
      { id: "hello-refused", capability: "hello" },
      { id: "cancel-before-start", capability: "cancel" },
      { id: "cancel-mid-run", capability: "cancel" },
    ]);
  });

  test.for([
    ["is no TOML", "[[capability"],
    ["has a case with no ID", '[[capability]]\nname = "hello"\n[[capability.case]]\ncovers = []\n'],
    [
      "has a capability with no name",
      '[[capability]]\n[[capability.case]]\nid = "hello-refused"\n',
    ],
  ] as const)("refuses a case list that %s", ([, text]) => {
    expect(() => parseClientCases(text)).toThrow(
      expect.objectContaining({ code: "case_list_invalid" }),
    );
  });
});

describe("clientCaseTags", () => {
  test("gives a tag per case and one per mode a test runs a case in", () => {
    expect(clientCaseTags(parseClientCases(CASE_LIST)).map((tag) => tag.name)).toEqual([
      "client_case/hello-refused",
      "client_case/cancel-before-start",
      "client_case/cancel-mid-run",
      "attached",
      "daemon",
    ]);
  });
});

describe("checkedCaseLogEntry", () => {
  test("gives back an entry that matches the bundle's schema", () => {
    expect(checkedCaseLogEntry(entry)).toEqual(entry);
  });

  test.for([
    ["sets an outcome", { ...entry, outcome: "passed" }],
    ["has no checks", { ...entry, checks: undefined }],
    ["names no capability", { ...entry, capability: undefined }],
  ])("refuses an entry that %s", ([, value]) => {
    expect(() => checkedCaseLogEntry(value)).toThrow(
      expect.objectContaining({ code: "case_log_entry_invalid" }),
    );
  });
});

describe("clientCaseRecord", () => {
  test("records each check's result in the test's meta and rethrows a failed check's error", async () => {
    const task: { name: string; tags: string[]; meta: { clientCase?: CaseRecording } } = {
      name: "cancels a run mid-way",
      tags: ["client_case/cancel-mid-run", "attached"],
      meta: {},
    };
    const record = clientCaseRecord(task);
    await record.check("the call sends $/cancel", () => {});
    record.message({ method: "$/cancel" });

    const failed = record.check("the run ends cancelled", () => {
      throw new Error("the run completed");
    });

    await expect(failed).rejects.toThrow("the run completed");
    expect(task.meta.clientCase).toEqual({
      checks: [
        { name: "the call sends $/cancel", passed: true },
        { name: "the run ends cancelled", passed: false },
      ],
      evidence: { oracle_answers: [], observations: [], messages: [{ method: "$/cancel" }] },
    });
  });

  test("refuses a test that carries no client_case tag", () => {
    expect(() =>
      clientCaseRecord({ name: "carries no case", tags: ["attached"], meta: {} }),
    ).toThrow(expect.objectContaining({ code: "client_case_untagged" }));
  });
});
