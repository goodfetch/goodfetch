// The install harness's npm side (ADR 0069; TESTING.md, Installed packages):
// a pinned Verdaccio with no uplink, started on port 0, serves packed
// packages, and npm, pnpm, Yarn, Bun or Deno installs them into a temporary
// project whose path holds a space and a non-ASCII letter. Each install
// keeps its manager's release-age gate on, at one day where the manager
// offers one that is off by default, and passes it through ADR 0030's
// exclusion snippet for that manager, read from the source the release
// notes read (`docs/release-age-exclusions.toml`).
//
// Each manager runs with a home, caches and configuration of the test's own,
// and with none of the outer run's variables but those that find programs.
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { Server } from "node:http";
import { createRequire } from "node:module";
import { basename, dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "smol-toml";
import { runServer } from "verdaccio";
import { TestSupportError } from "./errors.ts";
import { ProcessRecord, type Exit } from "./process-record.ts";

/** A package manager the harness installs with. */
export type PackageManager = "npm" | "pnpm" | "yarn" | "bun" | "deno";

/** Every package manager the harness installs with, in ADR 0069's order. */
export const PACKAGE_MANAGERS: readonly PackageManager[] = ["npm", "pnpm", "yarn", "bun", "deno"];

/** ADR 0030's exclusion snippet for one manager: the file it goes in, from the project's root, and its text. */
export interface ExclusionSnippet {
  readonly file: string;
  readonly text: string;
}

const EXCLUSIONS = fileURLToPath(
  new URL("../../../docs/release-age-exclusions.toml", import.meta.url),
);

/**
 * ADR 0030's exclusion snippet for `manager`, read from the source the
 * release notes read; `exclusions_invalid` when that source cannot be read
 * or lacks the manager's `file` or `text`.
 */
export function exclusionSnippet(manager: PackageManager): ExclusionSnippet {
  let source: unknown;
  try {
    source = parse(readFileSync(EXCLUSIONS, "utf8"));
  } catch (cause) {
    throw new TestSupportError("exclusions_invalid", `cannot read ${EXCLUSIONS}`, { cause });
  }
  const entry: unknown = isRecord(source) ? source[manager] : undefined;
  if (!isRecord(entry) || typeof entry["file"] !== "string" || typeof entry["text"] !== "string") {
    throw new TestSupportError(
      "exclusions_invalid",
      `${EXCLUSIONS} has no \`file\` and \`text\` for ${manager}`,
    );
  }
  return { file: entry["file"], text: entry["text"] };
}

/** What a command the harness ran gave: how it ended, and its output. */
export interface CommandOutput {
  readonly exit: Exit;
  readonly stdout: string;
  readonly stderr: string;
}

/** A release the registry publishes: package folders, packed as pnpm packs them. */
export interface Release {
  /** The package folders, dependencies before their dependents. */
  readonly packages: readonly string[];
  /** How many days ago the release reads as published; 0, the default, for now. */
  readonly ageDays?: number | undefined;
}

/** A running Verdaccio with no uplink, serving what the test published. */
export interface Registry {
  /** Its URL, `http://127.0.0.1:<port>/`. */
  readonly url: string;
  /** Packs and publishes each package of `release`; `publish_failed` when one fails. */
  publish(release: Release): Promise<void>;
  /** Stops the registry. */
  close(): Promise<void>;
}

/**
 * Starts a Verdaccio with no uplink on port 0, keeping its storage under
 * `folder`; `publish_failed` when it does not start.
 */
export async function startRegistry(folder: string): Promise<Registry> {
  const storage = join(folder, "storage");
  mkdirSync(storage, { recursive: true });
  // No uplink and no middleware: the audit middleware would forward to
  // npmjs.org, which the outbound deny refuses.
  const config: VerdaccioConfig = {
    self_path: join(folder, "config.yaml"),
    storage,
    uplinks: {},
    packages: { "**": { access: "$all", publish: "$authenticated", unpublish: "$authenticated" } },
    auth: { htpasswd: { file: join(folder, "htpasswd") } },
    middlewares: {},
    web: { enable: false },
    log: { type: "stdout", format: "json", level: "fatal" },
  };
  const server = await verdaccio(config);
  const port = await listen(server);
  const url = `http://127.0.0.1:${port}/`;
  const publisher = await publisherConfig(folder, url);
  return {
    url,
    publish: (release) => publish(release, { folder, storage, url, publisher }),
    close: () =>
      new Promise((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
        server.closeAllConnections();
      }),
  };
}

/** Verdaccio's configuration, as `runServer` takes it inline. */
type VerdaccioConfig = Exclude<Parameters<typeof runServer>[0], string | undefined>;

async function verdaccio(config: VerdaccioConfig): Promise<Server> {
  let server: unknown;
  try {
    server = await runServer(config);
  } catch (cause) {
    throw new TestSupportError("publish_failed", "Verdaccio did not start", { cause });
  }
  if (!(server instanceof Server)) {
    throw new TestSupportError("publish_failed", "Verdaccio's runServer gave no HTTP server");
  }
  return server;
}

/** Listens on a free loopback port, and gives it. */
async function listen(server: Server): Promise<number> {
  try {
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    });
  } catch (cause) {
    throw new TestSupportError("publish_failed", "Verdaccio did not listen", { cause });
  }
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new TestSupportError(
      "publish_failed",
      `Verdaccio listens on no port: ${String(address)}`,
    );
  }
  return address.port;
}

/** Adds a user to the registry and writes the npm configuration that publishes as it. */
async function publisherConfig(folder: string, url: string): Promise<string> {
  const name = "install-harness";
  const answer = await fetch(new URL(`-/user/org.couchdb.user:${name}`, url), {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name, password: "install-harness" }),
  });
  const body: unknown = await answer.json().catch(() => undefined);
  if (!answer.ok || !isRecord(body) || typeof body["token"] !== "string") {
    throw new TestSupportError(
      "publish_failed",
      `Verdaccio refused the publishing user: ${answer.status} ${JSON.stringify(body)}`,
    );
  }
  const config = join(folder, "publisher.npmrc");
  const host = url.replace(/^http:/, "");
  writeFileSync(config, `registry=${url}\n${host}:_authToken=${body["token"]}\n`);
  return config;
}

interface RegistryPlace {
  readonly folder: string;
  readonly storage: string;
  readonly url: string;
  readonly publisher: string;
}

async function publish(release: Release, place: RegistryPlace): Promise<void> {
  const home = mkdtempSync(join(place.folder, "publish-"));
  const env = { ...isolatedEnv(home), npm_config_userconfig: place.publisher };
  const processes = new ProcessRecord();
  try {
    for (const folder of release.packages) {
      const destination = mkdtempSync(join(home, "pack-"));
      const pnpm = managerProgram("pnpm");
      succeed(
        await runCommand(
          processes,
          pnpm.command,
          [...pnpm.args, "pack", "--pack-destination", destination],
          {
            env,
            cwd: folder,
          },
        ),
        `pnpm pack of ${folder}`,
      );
      const [tarball] = readdirSync(destination);
      if (tarball === undefined) {
        throw new TestSupportError("publish_failed", `pnpm pack wrote no tarball for ${folder}`);
      }
      const npm = managerProgram("npm");
      succeed(
        await runCommand(
          processes,
          npm.command,
          [
            ...npm.args,
            "publish",
            join(destination, tarball),
            "--ignore-scripts",
            "--access",
            "public",
          ],
          { env, cwd: home },
        ),
        `npm publish of ${folder}`,
      );
    }
  } finally {
    await processes.close();
  }
  const ageDays = release.ageDays ?? 0;
  if (ageDays > 0) {
    const published = new Date(Date.now() - ageDays * 86_400_000).toISOString();
    for (const folder of release.packages) {
      backdate(place.storage, readManifest(folder), published);
    }
  }
}

/**
 * Sets the publication time Verdaccio serves for one package version, which
 * the managers' release-age gates read: the registry has no way to publish
 * a version as older than now.
 */
function backdate(
  storage: string,
  manifest: { name: string; version: string },
  published: string,
): void {
  const file = join(storage, ...manifest.name.split("/"), "package.json");
  const document: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!isRecord(document) || !isRecord(document["time"])) {
    throw new TestSupportError(
      "publish_failed",
      `Verdaccio's record of ${manifest.name} has no \`time\``,
    );
  }
  document["time"][manifest.version] = published;
  document["time"]["created"] = published;
  writeFileSync(file, JSON.stringify(document, null, 2));
}

function readManifest(folder: string): { name: string; version: string } {
  const manifest: unknown = JSON.parse(readFileSync(join(folder, "package.json"), "utf8"));
  if (
    !isRecord(manifest) ||
    typeof manifest["name"] !== "string" ||
    typeof manifest["version"] !== "string"
  ) {
    throw new TestSupportError(
      "publish_failed",
      `${folder}'s package.json names no package and version`,
    );
  }
  return { name: manifest["name"], version: manifest["version"] };
}

/** What `installPacked` installs, and how. */
export interface InstallOptions {
  readonly manager: PackageManager;
  readonly registry: Registry;
  /** The folder the project and its home are made in, one per install. */
  readonly folder: string;
  /** The packages the project adds, by name, as a user adds them. */
  readonly packages: readonly string[];
  /** Whether the project carries ADR 0030's exclusion snippet; `true` unless a test shows the gate holding a release back. */
  readonly exclusion?: boolean | undefined;
  /** Installs with npm's `--omit=optional`, which leaves the platform packages out (ADR 0024); npm only. */
  readonly omitOptional?: boolean | undefined;
  /** The test's process record, which every command the project runs is started through. */
  readonly processes: ProcessRecord;
}

/** A project `installPacked` installed into. */
export interface PackedProject {
  /** The project's folder, whose name holds a space and a non-ASCII letter. */
  readonly folder: string;
  /** What the manager's install command gave; it failed when the gate held every version back. */
  readonly install: CommandOutput;
  /** Runs an executable the installed packages declare, through the manager. */
  runExecutable(
    packageName: string,
    executable: string,
    args: readonly string[],
  ): Promise<CommandOutput>;
  /** Copies `script` into the project and runs it with the manager's runtime, as a user's script runs. */
  runScript(script: string, args: readonly string[]): Promise<CommandOutput>;
}

/**
 * Installs `packages` from `registry` with `manager` into a new project
 * under `folder`, with the manager's release-age gate on and, unless
 * `exclusion` is `false`, ADR 0030's snippet passing Goodfetch's packages
 * through it. It returns the install's output rather than failing on it, so
 * a test can show the gate holding a release back. `manager_missing` when
 * the manager is not installed.
 */
export async function installPacked(options: InstallOptions): Promise<PackedProject> {
  const { manager, registry, processes } = options;
  const project = join(options.folder, `${manager} café`);
  const home = join(options.folder, "home");
  mkdirSync(project, { recursive: true });
  mkdirSync(home, { recursive: true });
  writeProject(manager, project, registry.url, options.exclusion ?? true);
  const env = isolatedEnv(home);
  const program = managerProgram(manager);
  const run = (args: readonly string[]): Promise<CommandOutput> =>
    runCommand(processes, program.command, [...program.args, ...args], { env, cwd: project });
  const install = await run(installArgs(manager, options.packages, options.omitOptional ?? false));
  if (install.exit.error !== undefined) {
    throw new TestSupportError(
      "manager_missing",
      `${manager} did not start: ${install.exit.error.message}`,
      {
        cause: install.exit.error,
      },
    );
  }
  return {
    folder: project,
    install,
    runExecutable: (packageName, executable, args) =>
      run(execArgs(manager, packageName, executable, args)),
    runScript: (script, args) => {
      const copy = join(project, basename(script));
      copyFileSync(script, copy);
      const runtime = scriptRuntime(manager, program);
      return runCommand(processes, runtime.command, [...runtime.args, copy, ...args], {
        env,
        cwd: project,
      });
    },
  };
}

/** Writes the project's manifest and the configuration that names the registry, turns the gate on and holds the snippet. */
function writeProject(
  manager: PackageManager,
  project: string,
  url: string,
  exclusion: boolean,
): void {
  const files = new Map<string, string>();
  if (manager !== "deno") {
    // Deno adds to `package.json` when there is one; without it, it keeps
    // its own `deno.json`, as a Deno project does.
    files.set(
      "package.json",
      `${JSON.stringify({ name: "install-project", private: true }, null, 2)}\n`,
    );
  }
  if (manager === "yarn") {
    const { hostname } = new URL(url);
    files.set(
      ".yarnrc.yml",
      `npmRegistryServer: "${url}"\nunsafeHttpWhitelist:\n  - "${hostname}"\nenableTelemetry: false\n`,
    );
  } else {
    // npm's gate is off by default: one day turns it on. pnpm's, Yarn's and
    // Deno's are on at one day by default; Bun's is turned on at its
    // command line.
    files.set(".npmrc", `registry=${url}\n${manager === "npm" ? "min-release-age=1\n" : ""}`);
  }
  if (exclusion) {
    const snippet = exclusionSnippet(manager);
    const before = files.get(snippet.file);
    files.set(snippet.file, before === undefined ? snippet.text : `${before}${snippet.text}`);
  }
  for (const [file, text] of files) {
    writeFileSync(join(project, file), text);
  }
}

function installArgs(
  manager: PackageManager,
  packages: readonly string[],
  omitOptional: boolean,
): string[] {
  switch (manager) {
    case "npm":
      return [
        "install",
        "--no-audit",
        "--no-fund",
        ...(omitOptional ? ["--omit=optional"] : []),
        ...packages,
      ];
    case "pnpm":
    case "yarn":
      return ["add", ...packages];
    case "bun":
      return ["add", "--minimum-release-age", String(86_400), ...packages];
    case "deno":
      return ["add", ...packages.map((name) => `npm:${name}`)];
    default: {
      const exhaustive: never = manager;
      return exhaustive;
    }
  }
}

function execArgs(
  manager: PackageManager,
  packageName: string,
  executable: string,
  args: readonly string[],
): string[] {
  switch (manager) {
    case "npm":
      return ["exec", "--no", "--", executable, ...args];
    case "pnpm":
      return ["exec", executable, ...args];
    case "yarn":
    case "bun":
      return ["run", executable, ...args];
    case "deno":
      return ["run", "-A", `npm:${packageName}/${executable}`, ...args];
    default: {
      const exhaustive: never = manager;
      return exhaustive;
    }
  }
}

/** A program and the arguments before its own. */
interface Program {
  readonly command: string;
  readonly args: readonly string[];
}

const require = createRequire(import.meta.url);

/**
 * The manager's program: npm as the running Node carries it, the pnpm
 * running the tests (`pnpm exec`, as every suite runs Vitest), Yarn from
 * test support's pinned dependency, and Bun and Deno from `PATH`, as the
 * runners install them (`ci/host/runtimes.env`).
 */
function managerProgram(manager: PackageManager): Program {
  switch (manager) {
    case "npm": {
      const prefix = dirname(process.execPath);
      const cli =
        process.platform === "win32"
          ? join(prefix, "node_modules", "npm", "bin", "npm-cli.js")
          : join(prefix, "..", "lib", "node_modules", "npm", "bin", "npm-cli.js");
      return { command: process.execPath, args: [cli] };
    }
    case "pnpm": {
      // pnpm's standalone build is an executable; installed from npm, it is
      // a script Node runs.
      const pnpm = process.env["npm_execpath"];
      if (pnpm === undefined || !basename(pnpm).startsWith("pnpm")) {
        throw new TestSupportError(
          "manager_missing",
          "the tests do not run under `pnpm exec`, so no pnpm is known",
        );
      }
      return [".js", ".cjs", ".mjs"].includes(extname(pnpm))
        ? { command: process.execPath, args: [pnpm] }
        : { command: pnpm, args: [] };
    }
    case "yarn":
      return {
        command: process.execPath,
        args: [require.resolve("@yarnpkg/cli-dist/bin/yarn.js")],
      };
    case "bun":
    case "deno":
      return { command: manager, args: [] };
    default: {
      const exhaustive: never = manager;
      return exhaustive;
    }
  }
}

/** The runtime a user's script runs with in a project of `manager`. */
function scriptRuntime(manager: PackageManager, program: Program): Program {
  switch (manager) {
    case "npm":
    case "pnpm":
      return { command: process.execPath, args: [] };
    case "yarn":
      return { command: program.command, args: [...program.args, "node"] };
    case "bun":
      return { command: "bun", args: [] };
    case "deno":
      return { command: "deno", args: ["run", "-A"] };
    default: {
      const exhaustive: never = manager;
      return exhaustive;
    }
  }
}

/** Variables that find programs and the system's folders, the only ones a manager takes from the outer run. */
const PASSED_VARIABLES = [
  "PATH",
  "Path",
  "PATHEXT",
  "SystemRoot",
  "SYSTEMROOT",
  "WINDIR",
  "ComSpec",
  "TEMP",
  "TMP",
];

/** An environment whose home, configuration and caches are all under `home`. */
function isolatedEnv(home: string): Record<string, string> {
  const passed = Object.fromEntries(
    PASSED_VARIABLES.flatMap((name) => {
      const value = process.env[name];
      return value === undefined ? [] : [[name, value]];
    }),
  );
  return {
    ...passed,
    HOME: home,
    USERPROFILE: home,
    APPDATA: join(home, "AppData", "Roaming"),
    LOCALAPPDATA: join(home, "AppData", "Local"),
    XDG_CONFIG_HOME: join(home, ".config"),
    XDG_CACHE_HOME: join(home, ".cache"),
    XDG_DATA_HOME: join(home, ".local", "share"),
    XDG_STATE_HOME: join(home, ".local", "state"),
    npm_config_cache: join(home, ".npm"),
    npm_config_update_notifier: "false",
    BUN_INSTALL_CACHE_DIR: join(home, ".bun", "install", "cache"),
    DENO_DIR: join(home, ".deno"),
    DENO_NO_UPDATE_CHECK: "1",
    YARN_ENABLE_TELEMETRY: "0",
    NO_COLOR: "1",
  };
}

async function runCommand(
  processes: ProcessRecord,
  command: string,
  args: readonly string[],
  options: { readonly env: Readonly<Record<string, string>>; readonly cwd: string },
): Promise<CommandOutput> {
  const { child, exit } = processes.spawn(command, args, options);
  child.stdin.end();
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
    stdout += chunk;
  });
  child.stderr.setEncoding("utf8").on("data", (chunk: string) => {
    stderr += chunk;
  });
  return { exit: await exit, stdout, stderr };
}

function succeed(output: CommandOutput, what: string): void {
  if (output.exit.code !== 0) {
    throw new TestSupportError(
      "publish_failed",
      `${what} failed (${output.exit.error?.message ?? `exit ${String(output.exit.code)}`}): ${output.stderr}`,
      { cause: output.exit.error },
    );
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
