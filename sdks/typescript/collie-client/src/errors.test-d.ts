// The types of the errors a Connection raises (ADR 0079): `code` is a
// literal union, so checking it narrows an error to its code's class
// without `instanceof`, which fails across two copies of the package in one
// bundle (stories 11-28 and 11-37).
import { expectTypeOf } from "vitest";
import {
  isCollieError,
  type BlockedError,
  type CollieError,
  type CollieErrorCode,
  type DeadlineExceededError,
  type GoodfetchError,
  type SettingInvalidError,
  type ErrorLimit,
} from "./generated/index.ts";

declare const caught: unknown;

if (isCollieError(caught)) {
  expectTypeOf(caught.code).toEqualTypeOf<CollieErrorCode>();
  if (caught.code === "setting_invalid") {
    expectTypeOf(caught).toEqualTypeOf<SettingInvalidError>();
    expectTypeOf(caught.group).toEqualTypeOf<"program">();
    expectTypeOf(caught.limit).toEqualTypeOf<ErrorLimit | undefined>();
  }
  if (caught.code === "deadline_exceeded") {
    expectTypeOf(caught).toEqualTypeOf<DeadlineExceededError<"deadline_exceeded">>();
  }
  switch (caught.code) {
    case "blocked":
      expectTypeOf(caught).toEqualTypeOf<BlockedError>();
      expectTypeOf(caught.group).toEqualTypeOf<"page">();
      break;
    default:
      expectTypeOf(caught).not.toEqualTypeOf<BlockedError>();
  }
  // @ts-expect-error: no Collie Error code is spelled so.
  expectTypeOf(caught.code === "setting-invalid").toBeBoolean();
}

declare const error: CollieError;
expectTypeOf(error).toExtend<GoodfetchError>();
expectTypeOf(error.callLog).toEqualTypeOf<readonly string[]>();
expectTypeOf(error.details).toEqualTypeOf<Readonly<Record<string, unknown>>>();
expectTypeOf<"setting_invalid">().toExtend<CollieErrorCode>();
expectTypeOf<"some_future_code">().not.toExtend<CollieErrorCode>();
