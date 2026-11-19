import { describe, expect, test } from "vitest";
import { unicodeRecord } from "./unicode.ts";

describe("unicodeRecord", () => {
  test("holds ADR 0073's hard text", () => {
    const record = unicodeRecord();

    expect(record.fields).toEqual([
      {
        name: "bidi",
        text: "\u05e9\u05dc\u05d5\u05dd \u200fworld\u200e \u0645\u0631\u062d\u0628\u0627 \u2067\u05e2\u05d1\u05e8\u05d9\u05ea\u2069 end",
      },
      { name: "nfc", text: "caf\u00e9" },
      { name: "nfd", text: "cafe\u0301" },
      { name: "zwj_emoji", text: "\u{1f469}\u200d\u{1f469}\u200d\u{1f467}" },
      { name: "astral", text: "\u{1d11e}" },
      { name: "zero_width_joiner", text: "a\u200db" },
      { name: "bom", text: "\ufeffleads with a byte order mark" },
      { name: "tab_and_newline", text: "one\ttwo\nthree" },
    ]);
  });
});
