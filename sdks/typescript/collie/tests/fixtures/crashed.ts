// A host program whose Attached engine, the one its environment names,
// crashes while its Run starts (ADRs 0078 and 0079). It connects with a
// logger, which the engine's stderr lines reach, lets the start fail with
// `engine_exited`, closes and exits 0. It writes nothing to stdout or
// stderr itself, so anything there is the SDK's. Run by Node alone, from
// the package's own source.
import { Collie, type Logger } from "@goodfetch/collie";

const ignored = (): void => {};
// It asks for none of the engine's log events: `hello` says `off`, as the script expects.
const logger: Logger = {
  level: "off",
  debug: ignored,
  info: ignored,
  warn: ignored,
  error: ignored,
};
const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect({ env, logger });
const started = await collie.start().then(
  () => "started",
  (error: unknown) => (error instanceof Error && "code" in error ? error.code : error),
);
await collie.close();
if (started !== "engine_exited") {
  process.exitCode = 1;
}
