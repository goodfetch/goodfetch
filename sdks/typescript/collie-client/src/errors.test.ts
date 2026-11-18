// The error a reply raises when its Error code is none the SDK knows: its
// group's class, keeping the code. Python's `test_errors.py` is its twin.
import { describe, expect, test } from "vitest";
import { errorOfReply } from "./errors.ts";
import { InternalError, ProgramError } from "./generated/index.ts";

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
});
