import { describe, expect, test } from "vitest";
import { versionsConfig } from "./versions.ts";

describe("versionsConfig", () => {
  test("before the first release, only next is built, with its banner", () => {
    expect(versionsConfig([])).toEqual({
      lastVersion: "current",
      onlyIncludeVersions: ["current"],
      versions: { current: { label: "next", banner: "unreleased" } },
    });
  });

  test("keeps stable, next and the last minor of each previous major", () => {
    expect(versionsConfig(["2.1", "2.0", "1.9", "1.8", "0.9", "0.8"])).toEqual({
      lastVersion: "2.1",
      onlyIncludeVersions: ["current", "2.1", "1.9", "0.9", "0.8"],
      versions: {
        current: { label: "next", path: "next", banner: "unreleased" },
        "2.1": { label: "2.1", banner: "none" },
        "1.9": { label: "1.9", path: "1.9", banner: "unmaintained" },
        "0.9": { label: "0.9", path: "0.9", banner: "unmaintained" },
        "0.8": { label: "0.8", path: "0.8", banner: "unmaintained" },
      },
    });
  });

  test("during previews each minor counts as a major", () => {
    expect(versionsConfig(["0.5", "0.4"]).onlyIncludeVersions).toEqual(["current", "0.5", "0.4"]);
  });
});
