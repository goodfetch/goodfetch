import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { type UnicodeField, unicodeRecord } from "./unicode.ts";

const folders: string[] = [];

afterEach(() => {
  for (const folder of folders.splice(0)) rmSync(folder, { recursive: true, force: true });
});

/** Writes each field to its own file and reads it back through `decode`. */
function roundTrip(decode: (bytes: Uint8Array) => string): UnicodeField[] {
  const folder = mkdtempSync(join(tmpdir(), "unicode record \u00e9 "));
  folders.push(folder);
  return unicodeRecord().fields.map(({ name, text }) => {
    const path = join(folder, `${name}.txt`);
    writeFileSync(path, text, "utf8");
    return { name, text: decode(readFileSync(path)) };
  });
}

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

  test("round-trips byte for byte through a file", () => {
    const read = roundTrip((bytes) => new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes));

    expect(read).toEqual(unicodeRecord().fields);
  });

  test("fails the round trip through a decoder that drops a leading byte order mark", () => {
    const read = roundTrip((bytes) => new TextDecoder("utf-8").decode(bytes));

    expect(read).not.toEqual(unicodeRecord().fields);
  });
});
