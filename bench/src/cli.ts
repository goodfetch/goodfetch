// The `bench` command line, the harness's output writer: it prints one JSON
// line naming the run's folder and status, or `error[<code>]: <message>` on
// stderr and exits 1. Usage:
//
//   node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>
import { parseArgs } from "node:util";
import { BenchError } from "./errors.ts";
import { runCampaign } from "./runner.ts";

const usage =
  "usage: node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>\n";

async function main(argv: readonly string[]): Promise<number> {
  const { values, positionals } = parseArgs({
    args: [...argv],
    allowPositionals: true,
    options: {
      "proving-ground": { type: "string" },
      out: { type: "string" },
    },
  });
  const [command, campaignPath] = positionals;
  const address = values["proving-ground"];
  const out = values.out;
  if (
    command !== "run" ||
    campaignPath === undefined ||
    address === undefined ||
    out === undefined
  ) {
    process.stderr.write(usage);
    return 2;
  }
  try {
    const { directory, record } = await runCampaign({ campaignPath, provingGround: address, out });
    process.stdout.write(`${JSON.stringify({ run: directory, status: record.status })}\n`);
    return 0;
  } catch (error) {
    if (error instanceof BenchError) {
      process.stderr.write(`error[${error.code}]: ${error.message}\n`);
      return 1;
    }
    throw error;
  }
}

process.exitCode = await main(process.argv.slice(2));
