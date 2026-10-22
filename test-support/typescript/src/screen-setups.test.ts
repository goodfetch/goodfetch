import { describe, expect, test } from "vitest";
import { parseScreenSetups } from "./screen-setups.ts";

const ENTRY = `[[screen]]
feature = "screen:/runs/$run"
route = "/runs/r-1"
sign_in = "signed-in"
walk = ["link Runs", "button Start a run"]
`;

describe("parseScreenSetups", () => {
  test("reads each entry's route and sign-in state by its Feature", () => {
    const setups = parseScreenSetups(ENTRY);

    expect([...setups.values()]).toEqual([
      {
        feature: "screen:/runs/$run",
        route: "/runs/r-1",
        signIn: "signed-in",
        walk: ["link Runs", "button Start a run"],
      },
    ]);
  });

  test("reads a file of comments only as no entry", () => {
    const setups = parseScreenSetups("# No screen yet.\n");

    expect(setups.size).toBe(0);
  });

  test("reads an entry without its route, sign-in state or walk, for its case to fail by name", () => {
    const setups = parseScreenSetups('[[screen]]\nfeature = "screen:/runs"\n');

    expect(setups.get("screen:/runs")).toEqual({
      feature: "screen:/runs",
      route: undefined,
      signIn: undefined,
      walk: undefined,
    });
  });

  test.for<[string, string]>([
    ["no TOML", "[[screen]\n"],
    ["a key it does not define", `${ENTRY}[other]\n`],
    ["an entry without a Feature", ENTRY.replace('feature = "screen:/runs/$run"\n', "")],
    [
      "an entry naming another kind of Feature",
      ENTRY.replace("screen:/runs/$run", "command:crawl"),
    ],
    ["an entry with a field it does not define", `${ENTRY}skip = true\n`],
    ["a route that is no path", ENTRY.replace('"/runs/r-1"', '"runs/r-1"')],
    ["an unknown sign-in state", ENTRY.replace('"signed-in"', '"guest"')],
    [
      "a walk that is no list of stops",
      ENTRY.replace('["link Runs", "button Start a run"]', '"link Runs"'),
    ],
    ["a walk with an empty stop", ENTRY.replace('"button Start a run"', '""')],
    ["two entries of one Feature", `${ENTRY}\n${ENTRY}`],
  ])("refuses %s with screen_setups_invalid", ([, text]) => {
    const parsing = () => parseScreenSetups(text);

    expect(parsing).toThrow(expect.objectContaining({ code: "screen_setups_invalid" }));
  });
});
