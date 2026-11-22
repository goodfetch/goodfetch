// A program where `@opentelemetry/api` does not resolve (ADR 0132). A
// resolve hook refuses that package, as a program that does not install it
// sees, and records each attempt and the stage it came at: import or
// connect. The program then imports `@goodfetch/collie`, connects to the
// Attached engine its environment names, makes one call and closes. It
// writes, into the file its argument names, the attempts seen. The Scripted
// engine it reaches refuses a call with any `_meta`, so the call passing
// says it sent none. Run by Node alone, from the package's own source.
import { writeFileSync } from "node:fs";
import { registerHooks } from "node:module";

const out = process.argv[2];
if (out === undefined) {
  throw new TypeError("the program takes the file it writes as its argument");
}

/** The package the program has not installed. */
const PACKAGE = "@opentelemetry/api";

type Stage = "import" | "connect";
let stage: Stage = "import";
const tried: { readonly stage: Stage; readonly specifier: string }[] = [];

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === PACKAGE || specifier.startsWith(`${PACKAGE}/`)) {
      tried.push({ stage, specifier });
      throw Object.assign(new Error(`Cannot find package '${specifier}'`), {
        code: "ERR_MODULE_NOT_FOUND",
      });
    }
    return nextResolve(specifier, context);
  },
});

const { Collie } = await import("@goodfetch/collie");
stage = "connect";
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect();
await collie.explainSettings();
await collie.close();

writeFileSync(out, JSON.stringify({ tried }));
