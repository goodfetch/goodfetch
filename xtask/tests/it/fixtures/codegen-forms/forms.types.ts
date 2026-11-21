// What a program can and cannot write against the handles `xtask codegen`
// generates from the forms catalog (ADR 0099): `tsc` fails on a wrong one,
// and oxlint's `no-deprecated` on a deprecated use without its directive.
import { expectTypeOf } from "vitest";
import type { Locator, Page, PageKeyboard, Run } from "./generated/handles.ts";
import type { Acted, Navigated, PageField, Readiness, ReadyState } from "./generated/types.ts";

declare const run: Run;
declare const page: Page;
declare const otherKeyboard: PageKeyboard;
declare const anyState: ReadyState;
declare const anyPoll: number | undefined;

// A round trip: its required texts are positional in the catalog's order,
// which is not the alphabet's, its required boolean and its optional
// parameter are named, and it can be cancelled.
expectTypeOf(page.goto).parameters.toEqualTypeOf<
  [
    url: string,
    referrer: string,
    options: {
      readonly fresh: boolean;
      readonly timeout?: number | null | undefined;
      readonly signal?: AbortSignal | undefined;
    },
  ]
>();
expectTypeOf(page.goto).returns.toEqualTypeOf<Promise<Navigated["status"]>>();
// @ts-expect-error a boolean is never positional
void page.goto("https://example.test/", "https://example.test/from", true);

// A query takes nothing to wait with: no options at all here.
expectTypeOf(page.isReady).parameters.toEqualTypeOf<[]>();
expectTypeOf(page.isReady).returns.toEqualTypeOf<Promise<Readiness["ready"]>>();
// @ts-expect-error a query has no timeout
void page.isReady({ timeout: 1 });

// A result typed by the fields its call declares: each field's value under
// its name, a number for a number field, an array for a list.
const read = page.read({
  title: { selector: "h1" },
  price: { selector: ".price", transform: { kind: "number" } },
  tags: { list: { selector: ".tag" } },
});
expectTypeOf(read).resolves.toEqualTypeOf<{
  values: { title: string | null; price: number | null; tags: (string | null)[] };
  unread: string[];
}>();
declare const field: PageField;
expectTypeOf(page.read({ field })).resolves.toHaveProperty("values").toEqualTypeOf<{
  field: string | number | null | (string | number | null)[];
}>();
// @ts-expect-error a field is declared as its type says
void page.read({ title: { selector: 1 } });

// A waiter keeps its timeout.
// oxlint-disable-next-line typescript/no-deprecated -- oxlint reads a method with a deprecated overload, named without a call, as deprecated
expectTypeOf(page.waitForReady).toBeCallableWith({ timeout: 1 });

// A deprecated option and a deprecated value each have an overload of their
// own, which TSDoc marks: oxlint's `no-deprecated` reports a call that passes
// them, and no other (ADR 0099), since `tsc` reports no deprecation.
void page.waitForReady({ state: "complete", timeout: 1 });
// oxlint-disable-next-line typescript/no-deprecated -- the deprecated option the fixture proves reported
void page.waitForReady({ poll: 100, state: null });
// oxlint-disable-next-line typescript/no-deprecated -- the deprecated value the fixture proves reported
void page.waitForReady({ state: "loaded" });
// @ts-expect-error a deprecated option keeps its type
void page.waitForReady({ poll: "often" });
// A value only its type gives, a union with a deprecated value or an
// optional one, resolves to the last overload, which takes every form.
void page.waitForReady({ state: anyState });
void page.waitForReady({ poll: anyPoll });

// A `new` member returns the value it opens, which closes with its scope.
expectTypeOf(run.newPage).returns.toEqualTypeOf<Promise<Page>>();
expectTypeOf<Page>().toExtend<AsyncDisposable>();

// A handle's own close answers nothing: it is idempotent (ADR 0078).
expectTypeOf(page.close).returns.toEqualTypeOf<Promise<void>>();

// A value built without a call is returned at once, by a sync method.
expectTypeOf(page.locator).parameters.toEqualTypeOf<[selector: string]>();
expectTypeOf(page.locator).returns.toEqualTypeOf<Locator>();
expectTypeOf(page.locator("button").click).returns.toEqualTypeOf<Promise<Acted>>();

// A sub-object is a property the program cannot replace.
expectTypeOf(page.keyboard).toEqualTypeOf<PageKeyboard>();
// @ts-expect-error a sub-object is read-only
page.keyboard = otherKeyboard;
// oxlint-disable-next-line typescript/no-deprecated -- the deprecated member the fixture proves reported
expectTypeOf(page.keyboard.press).parameters.toEqualTypeOf<
  [key: string, options?: { readonly signal?: AbortSignal | undefined } | undefined]
>();
