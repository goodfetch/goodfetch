// The types of AI work's result (ADR 0117): the answer sits in how the work
// ended, so reading `output` before narrowing on `ended.kind` fails `tsc`,
// and a stopped task's `partial` is typed as partial: `Partial<T>` for an
// object, the elements for a list (story 11-34).
import { expectTypeOf } from "vitest";
import type { AIWorkResult, PartialOf } from "./ai-work.ts";
import type { StopReason } from "./generated/index.ts";

interface Author {
  readonly name: string;
  readonly born: number;
}

declare const result: AIWorkResult<Author>;

// @ts-expect-error the result has no `output` of its own
void result.output;
// @ts-expect-error `ended` holds an output only once narrowed to `finished`
void result.ended.output;

if (result.ended.kind === "finished") {
  expectTypeOf(result.ended.output).toEqualTypeOf<Author>();
} else {
  expectTypeOf(result.ended.reason).toEqualTypeOf<StopReason>();
  expectTypeOf(result.ended.partial).toEqualTypeOf<Partial<Author>>();
}

expectTypeOf(result.requireVerified()).toEqualTypeOf<Author>();
expectTypeOf<PartialOf<readonly Author[]>>().toEqualTypeOf<Author[]>();
