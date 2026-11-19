// A program with no logger that connects to the Attached engine its
// environment names and closes once the engine has said what it says. It
// writes nothing to stdout or stderr itself, so anything there is the SDK's.
// Run by Node alone, from the package's own source.
import { Collie } from "@goodfetch/collie";

const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);
// oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
const collie = await Collie.connect({ env });
await collie.close();
