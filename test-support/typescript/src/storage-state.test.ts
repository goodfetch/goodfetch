import { describe, expect, test } from "vitest";
import { refuseStorageState, type StorageStateOptions } from "./storage-state.ts";

describe("refuseStorageState", () => {
  test.for<[string, StorageStateOptions]>([
    ["a storage state file", { storageState: "signed-in.json" }],
    ["a storage state's cookies", { storageState: { cookies: [], origins: [] } }],
    ["contextOptions' storage state", { contextOptions: { storageState: "signed-in.json" } }],
  ])("refuses %s with storage_state_refused", ([, options]) => {
    const refusal = () => refuseStorageState(options);

    expect(refusal).toThrow(expect.objectContaining({ code: "storage_state_refused" }));
  });

  test.for<[string, StorageStateOptions]>([
    ["no option", {}],
    ["no storage state", { storageState: undefined, contextOptions: {} }],
  ])("allows %s", ([, options]) => {
    const refusal = () => refuseStorageState(options);

    expect(refusal).not.toThrow();
  });
});
