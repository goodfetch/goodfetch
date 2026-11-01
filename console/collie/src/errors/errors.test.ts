// Each Error code's message from the Console's catalog (ADRs 0073 and
// 0079): every code of Collie's catalog maps to one, read from the error's
// fields, an unknown code keeps the engine's English message, and the
// program's own exception is always named. The catalog's English text is
// asserted here, in the `unit` project; screens assert where it shows.
// Collie's Error codes are read from its catalog's generated projection.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { errorOfReply } from "@goodfetch/collie-client/browser";
import { collieErrorClasses } from "@goodfetch/collie-client/generated";
import { describe, expect, test } from "vitest";
import { i18n } from "#i18n.tsx";
import { describeError } from "./errors.ts";

/** The error the Console's Client raises for an engine's failure with `code` and `details`. */
function engineError(code: string, details: Readonly<Record<string, unknown>> = {}): unknown {
  return errorOfReply(
    {
      code: -32001,
      message: `the engine's English message for ${code}`,
      data: { code, retryable: false, ...details },
    },
    collieErrorClasses,
  );
}

/** Every Error code of Collie's catalog, as `xtask codegen` projects it. */
const collieCodes: readonly string[] = codeNames(
  JSON.parse(
    readFileSync(
      join(import.meta.dirname, "../../../../crates/goodfetch-protocol/generated/collie.json"),
      "utf8",
    ),
  ),
);

/** The names of the Error codes of `catalog`, a Product's catalog projection. */
function codeNames(catalog: unknown): string[] {
  if (
    typeof catalog !== "object" ||
    catalog === null ||
    !("error_codes" in catalog) ||
    !Array.isArray(catalog.error_codes)
  ) {
    throw new Error("invariant: a catalog projection lists its Error codes");
  }
  return catalog.error_codes.map((code: unknown) => {
    if (
      typeof code !== "object" ||
      code === null ||
      !("name" in code) ||
      typeof code.name !== "string"
    ) {
      throw new Error("invariant: each Error code of a catalog projection has its name");
    }
    return code.name;
  });
}

/** The Error codes whose message always names the program's own exception (ADRs 0079, 0082, 0100 and 0104). */
const PROGRAM_EXCEPTION_CODES = [
  "handler_failed",
  "hook_failed",
  "model_handler_failed",
  "agent_tool_failed",
  "success_check_threw",
  "schema_validator_failed",
];

describe("an Error code", { tags: ["story:12-133"] }, () => {
  test("of Collie's catalog are all there", () => {
    expect(collieCodes).toEqual(expect.arrayContaining(Object.keys(collieErrorClasses)));
  });

  test.each([...collieCodes, ...PROGRAM_EXCEPTION_CODES])(
    "%s maps to a message of the catalog",
    (code) => {
      const described = describeError(
        engineError(code, { exception_type: "TypeError", exception_message: "x is undefined" }),
        i18n,
      );

      expect(described).toMatchObject({ kind: "catalog", code });
      expect(described.kind === "catalog" && described.summary).not.toMatch(/English message/u);
    },
  );

  test.each([
    { code: "run_not_found", summary: "No run has this Run ID." },
    { code: "page_crashed", summary: "The page crashed." },
    {
      code: "deadline_exceeded",
      summary: "The call's deadline passed before what it waited for came.",
    },
  ])("$code reads “$summary”", ({ code, summary }) => {
    expect(describeError(engineError(code), i18n)).toMatchObject({ summary });
  });

  test("is rendered from the error's fields", () => {
    const described = describeError(
      engineError("deadline_exceeded", {
        operation: "locator.click",
        target: "page.getByRole('button', { name: 'Next' })",
        url: "https://shop.test/cart",
        observed: "the button is disabled",
        limit: { key: "timeout.action", value: 30_000, layer: "user_configuration" },
        fix: "raise `timeout.action`",
        call_log: ["waiting for the button", "the button is disabled"],
      }),
      i18n,
    );

    expect(described).toEqual({
      kind: "catalog",
      code: "deadline_exceeded",
      summary: "The call's deadline passed before what it waited for came.",
      operation: "locator.click",
      target: "page.getByRole('button', { name: 'Next' })",
      url: "https://shop.test/cart",
      observed: "the button is disabled",
      limit: { key: "timeout.action", value: 30_000, layer: "User configuration" },
      fix: "raise `timeout.action`",
      callLog: ["waiting for the button", "the button is disabled"],
      exception: undefined,
    });
  });

  test("the catalog does not know keeps the engine's English message", () => {
    expect(describeError(engineError("no_such_code", { fix: "upgrade" }), i18n)).toEqual({
      kind: "engine",
      code: "no_such_code",
      message: "the engine's English message for no_such_code",
    });
  });

  test("a failure that is no engine error keeps its own message", () => {
    expect(describeError(new Error("the body could not be fetched"), i18n)).toEqual({
      kind: "engine",
      code: undefined,
      message: "the body could not be fetched",
    });
  });
});

describe("the program's own exception", { tags: ["story:12-133"] }, () => {
  test.each(PROGRAM_EXCEPTION_CODES)("is always named by %s", (code) => {
    const described = describeError(
      engineError(code, {
        exception_type: "TypeError",
        exception_message: "Cannot read properties of undefined (reading 'price')",
      }),
      i18n,
    );

    expect(described).toMatchObject({
      kind: "catalog",
      exception: {
        type: "TypeError",
        message: "Cannot read properties of undefined (reading 'price')",
      },
    });
  });

  test.each(PROGRAM_EXCEPTION_CODES)(
    "falls back to the engine's message when %s's details do not name it",
    (code) => {
      expect(describeError(engineError(code), i18n)).toEqual({
        kind: "engine",
        code,
        message: `the engine's English message for ${code}`,
      });
    },
  );
});
