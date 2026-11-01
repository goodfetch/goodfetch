// Commands of a served test's Daemon's executable, run in the test's home and
// cleared environment, and the Product and version it reports, which the
// Journey helper and the Console conformance suite write into their result
// documents.
import type { Executable } from "./generated/result-document.ts";
import type { ProcessRecord } from "./process-record.ts";
import type { ServedDaemon } from "./served-daemon.ts";

/** What a command of the executable gave. */
export interface CliOutput {
  readonly code: number | null;
  readonly stdout: string;
  readonly stderr: string;
}

/** Runs the Daemon's executable with `args` in the test's home and cleared environment. */
export async function daemonCommand(
  processes: ProcessRecord,
  daemon: Pick<ServedDaemon, "executable" | "env">,
  args: readonly string[],
): Promise<CliOutput> {
  const started = processes.spawn(daemon.executable, args, { env: daemon.env });
  started.child.stdin.end();
  let stdout = "";
  let stderr = "";
  started.child.stdout.setEncoding("utf8");
  started.child.stderr.setEncoding("utf8");
  started.child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  started.child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });
  const { code } = await started.exit;
  return { code, stdout, stderr };
}

/** The Product and Product version the executable reports through `--version`, `goodfetch-<product> <version>`. */
export async function reportedVersion(
  cli: (args: readonly string[]) => Promise<CliOutput>,
  executable: string,
): Promise<Executable> {
  const output = await cli(["--version"]);
  const [binary, version] = output.stdout.trim().split(" ");
  const product = binary?.replace(/^goodfetch-/, "");
  if (
    output.code !== 0 ||
    version === undefined ||
    (product !== "collie" && product !== "foxhound")
  ) {
    throw new Error(
      `the executable ${executable} reports no \`goodfetch-<product> <version>\` through \`--version\`: \`${output.stdout}\``,
    );
  }
  return { product, version };
}
