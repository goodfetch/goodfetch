// The error a reply raises: its code's generated class with the fields every
// error carries, or its group's class keeping the code when the SDK does not
// know it; and the reply an error is answered with. Python's
// `test_errors.py` is its twin.
import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { errorOfReply, replyOfError } from "./errors.ts";
import { isGoodfetchError } from "./goodfetch-error.ts";
import {
  BlockedError,
  collieErrorClasses,
  errorGroupClasses,
  InternalError,
  isCollieError,
  PageError,
  ProgramError,
  SettingInvalidError,
  type ErrorGroup,
} from "./generated/index.ts";

interface CatalogCode {
  readonly name: string;
  readonly group: ErrorGroup;
  readonly retryable: boolean;
}

// oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the committed catalog file lists each Error code with its group and retryable flag.
const catalog = JSON.parse(
  readFileSync(
    new URL("../../../../crates/goodfetch-protocol/generated/collie.json", import.meta.url),
    "utf8",
  ),
) as { readonly error_codes: readonly CatalogCode[] };

/** The JSON-RPC error of a `setting_invalid` refusal carrying every common field. */
const refusal = {
  code: -32001,
  message: "page.goto's timeout of 0 ms is below the least, 1 ms",
  data: {
    code: "setting_invalid",
    retryable: false,
    fix: "Pass a timeout of 1 ms or more.",
    operation: "page.goto",
    call_id: "call-7",
    run_id: "run-3",
    target: "page.getByRole('button', { name: 'Buy' })",
    url: "https://shop.example/cart",
    observed: "a button that is disabled",
    limit: { key: "timeout", value: 0, layer: "run", origin: "argument" },
    call_log: ["waiting for getByRole('button')", "  locator resolved to <button>"],
    usage: {
      calls: 1,
      input_tokens: 10,
      output_tokens: 2,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
    },
    agent_task: { steps: [] },
    reason: "below_least",
  },
};

describe("errorOfReply", () => {
  // ADR 0079: the codes JSON-RPC 2.0 predefines map to groups, invalid
  // params and method not found to `program`, the others to `internal`.
  test.for([
    { wireCode: -32602, code: "invalid_params", groupClass: ProgramError, group: "program" },
    { wireCode: -32601, code: "method_not_found", groupClass: ProgramError, group: "program" },
    { wireCode: -32700, code: "parse_error", groupClass: InternalError, group: "internal" },
    { wireCode: -32600, code: "invalid_request", groupClass: InternalError, group: "internal" },
    { wireCode: -32603, code: "internal_error", groupClass: InternalError, group: "internal" },
  ])(
    "raises the protocol failure $code in its group's class, keeping its code",
    ({ wireCode, code, groupClass, group }) => {
      const error = errorOfReply(
        { code: wireCode, message: "refused", data: { code, retryable: false } },
        {},
      );

      expect({
        isGroupClass: error instanceof groupClass,
        code: error.code,
        group: error.group,
      }).toEqual({
        isGroupClass: true,
        code,
        group,
      });
    },
  );

  // ADR 0079: a code a newer engine adds raises the class of the group its
  // JSON-RPC error code names, keeping the code as received.
  test("raises a code it does not know in its group's class, keeping the code", () => {
    const error = errorOfReply(
      {
        code: -32003,
        message: "the page went away",
        data: { code: "a_code_from_a_newer_engine", retryable: true },
      },
      collieErrorClasses,
    );

    expect({
      isGroupClass: error instanceof PageError,
      code: error.code,
      group: error.group,
      retryable: error.retryable,
    }).toEqual({
      isGroupClass: true,
      code: "a_code_from_a_newer_engine",
      group: "page",
      retryable: true,
    });
  });

  test(
    "raises the code's class with the fields every error carries, leaving the code's own details",
    { tags: ["story:11-30"] },
    () => {
      const error = errorOfReply(refusal, collieErrorClasses);

      expect(error).toBeInstanceOf(SettingInvalidError);
      expect({
        name: error.name,
        message: error.message,
        code: error.code,
        group: error.group,
        retryable: error.retryable,
        fix: error.fix,
        operation: error.operation,
        callId: error.callId,
        runId: error.runId,
        target: error.target,
        url: error.url,
        observed: error.observed,
        limit: error.limit,
        callLog: error.callLog,
        usage: error.usage,
        agentTask: error.agentTask,
        details: error.details,
      }).toEqual({
        name: "SettingInvalidError",
        message: refusal.message,
        code: "setting_invalid",
        group: "program",
        retryable: false,
        fix: refusal.data.fix,
        operation: "page.goto",
        callId: "call-7",
        runId: "run-3",
        target: refusal.data.target,
        url: refusal.data.url,
        observed: refusal.data.observed,
        limit: refusal.data.limit,
        callLog: refusal.data.call_log,
        usage: refusal.data.usage,
        agentTask: refusal.data.agent_task,
        details: { reason: "below_least" },
      });
    },
  );

  // A code's own details may use a common field's name for something else,
  // such as `response_limit_exceeded`'s byte count or a redirect cap's
  // `{kind, name, count}`: a value without the field's shape stays there.
  test.for([
    { key: "limit", value: 10_485_760 },
    { key: "limit", value: { kind: "option", name: "max_redirects", count: 3 } },
    { key: "url", value: 404 },
    { key: "call_log", value: "one line" },
    { key: "usage", value: 7 },
  ])("leaves a $key of another shape in the code's details", ({ key, value }) => {
    const error = errorOfReply(
      {
        code: -32003,
        message: "too big",
        data: { code: "blocked", retryable: false, [key]: value },
      },
      collieErrorClasses,
    );

    expect({
      limit: error.limit,
      url: error.url,
      usage: error.usage,
      details: error.details,
    }).toEqual({
      limit: undefined,
      url: undefined,
      usage: undefined,
      details: { [key]: value },
    });
  });

  // ADRs 0092 and 0118: `blocked`'s `retry_after` is the time the site's
  // `Retry-After` asks for, sent in any offset and read as a `Date`; `null`
  // when it asks for none. The details keep it as sent.
  test.for([
    { sent: "2026-11-21T10:00:00+02:00", retryAfter: new Date("2026-11-21T08:00:00Z") },
    { sent: null, retryAfter: null },
    { sent: undefined, retryAfter: null },
  ])("reads blocked's retry_after $sent as a point in time", ({ sent, retryAfter }) => {
    const error = errorOfReply(
      {
        code: -32003,
        message: "blocked",
        data: { code: "blocked", retryable: false, cause: "rate_limit", retry_after: sent },
      },
      collieErrorClasses,
    );

    expect(error).toBeInstanceOf(BlockedError);
    expect({
      retryAfter: error instanceof BlockedError ? error.retryAfter : "not blocked",
      sent: error.details["retry_after"],
    }).toEqual({ retryAfter, sent });
  });

  test("gives an error with no call log an empty one", () => {
    const error = errorOfReply(
      { code: -32003, message: "blocked", data: { code: "blocked", retryable: false } },
      collieErrorClasses,
    );

    expect(error.callLog).toEqual([]);
  });
});

describe("replyOfError", () => {
  test("answers with every field the error carries, which the reply raises back", () => {
    const error = errorOfReply(refusal, collieErrorClasses);

    expect(replyOfError(error)).toEqual(refusal);
  });
});

describe("the generated error classes", () => {
  // ADR 0099: every Error code of the catalog has its class under its
  // group; a code named like its group is raised as its group's class.
  test.for(catalog.error_codes)(
    "raise $name under its group, with its code, group and retryable flag",
    { tags: ["story:11-28", "story:11-37"] },
    ({ name, group, retryable }) => {
      const known = collieErrorClasses[name];
      const error =
        known === undefined
          ? new errorGroupClasses[group]("failed", { code: name, group, retryable })
          : new known("failed");

      expect({
        isGroupClass: error instanceof errorGroupClasses[group],
        hasOwnClass: known !== undefined || name === group,
        name: error.name === (known ?? errorGroupClasses[group]).name,
        code: error.code,
        group: error.group,
        retryable: error.retryable,
        isGoodfetchError: isGoodfetchError(error),
        isCollieError: isCollieError(error),
      }).toEqual({
        isGroupClass: true,
        hasOwnClass: true,
        name: true,
        code: name,
        group,
        retryable,
        isGoodfetchError: true,
        isCollieError: true,
      });
    },
  );

  // The codes areas 5, 7 and 8 add, and the CLI's `person_required`, each
  // with its own class under the group its ADRs fix (ADRs 0033, 0035, 0039,
  // 0079 and 0092). Windows' refusal to start a file is
  // `executable_blocked_by_smart_app_control` (ADR 0039's note of 2026-10-20).
  test.for([
    ["browser_host_unreachable", "BrowserHostUnreachableError", "unavailable", true],
    ["browser_host_outdated", "BrowserHostOutdatedError", "unavailable", false],
    [
      "executable_blocked_by_smart_app_control",
      "ExecutableBlockedBySmartAppControlError",
      "refused",
      false,
    ],
    ["confinement_unavailable", "ConfinementUnavailableError", "refused", false],
    ["challenge_unsolved", "ChallengeUnsolvedError", "page", false],
    ["plugin_crashed", "PluginCrashedError", "unavailable", true],
    ["person_required", "PersonRequiredError", "refused", false],
  ] as const)(
    "raise %s as %s under its group",
    { tags: ["story:11-190"] },
    ([code, className, group, retryable]) => {
      const known = collieErrorClasses[code];
      const error = known === undefined ? undefined : new known("failed");

      expect({
        name: error?.name,
        isGroupClass: error instanceof errorGroupClasses[group],
        group: error?.group,
        retryable: error?.retryable,
      }).toEqual({ name: className, isGroupClass: true, group, retryable });
    },
  );

  test("tell a code Collie does not know from Collie's own, whatever the class", () => {
    const error = new ProgramError("failed", {
      code: "some_future_code",
      group: "program",
      retryable: false,
    });

    expect({
      isGoodfetchError: isGoodfetchError(error),
      isCollieError: isCollieError(error),
      isPlainError: isGoodfetchError(new Error("failed")),
    }).toEqual({ isGoodfetchError: true, isCollieError: false, isPlainError: false });
  });
});
