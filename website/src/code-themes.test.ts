import { themes, type PrismTheme } from "prism-react-renderer";
import { describe, expect, test } from "vitest";
import { codeThemes, contrast, legible } from "./code-themes.ts";

/** Each coloured token style's colour, by its token types. */
function colours(theme: PrismTheme): Map<string, string> {
  return new Map(
    theme.styles.flatMap(({ types, style }) =>
      style.color === undefined ? [] : [[types.join(","), style.color] as const],
    ),
  );
}

describe("codeThemes", { tags: ["story:1-116"] }, () => {
  for (const [name, theme] of Object.entries(codeThemes)) {
    test(`every token of the ${name} theme meets 4.5:1 on its background`, () => {
      const background = theme.plain.backgroundColor ?? "";
      const under = [...colours(theme)].filter(([, colour]) => contrast(colour, background) < 4.5);
      expect(under).toEqual([]);
      expect(contrast(theme.plain.color ?? "", background)).toBeGreaterThanOrEqual(4.5);
    });
  }
});

describe("legible", () => {
  test("darkens only the colours under the ratio, toward the text colour", () => {
    const theme = legible(themes.github);
    const before = colours(themes.github);
    const after = colours(theme);

    // #36acaa is 2.58:1 on #f6f8fa; #6f42c1 already 6.12:1.
    const property = "entity,url,symbol,number,boolean,variable,constant,property,regex,inserted";
    expect(contrast(after.get(property) ?? "", "#f6f8fa")).toBeGreaterThanOrEqual(4.5);
    expect(after.get(property)).not.toBe(before.get(property));
    expect(after.get("function-variable")).toBe(before.get("function-variable"));
  });

  test("keeps a theme's other styles", () => {
    const theme = legible(themes.github);

    expect(theme.plain).toEqual(themes.github.plain);
    expect(theme.styles.map(({ types }) => types)).toEqual(
      themes.github.styles.map(({ types }) => types),
    );
    expect(theme.styles[0]?.style.fontStyle).toBe("italic");
  });
});

describe("contrast", () => {
  test("reads hex and rgb() colours as WCAG 2.2 does", () => {
    expect(contrast("#000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("rgb(255, 255, 255)", "#fff")).toBeCloseTo(1, 5);
  });
});
