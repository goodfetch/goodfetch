// The project file `connect()` reads (ADR 0083; story 11-75), read as the
// engine reads one: found upward from the working directory, named by
// `project` from it, or none for `null`; refused before any engine is
// reached when it is not there, does not read as a project file, is of a
// format version this Goodfetch cannot read, or sets a `timeout.connect`
// that is no whole number of 1 ms or more.
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { readProject } from "./project-file.ts";

let folder: string;

beforeEach(async () => {
  folder = await realpath(await mkdtemp(join(tmpdir(), "project-file-")));
});

afterEach(async () => {
  await rm(folder, { recursive: true, force: true });
});

/** Writes `text` as `name` in `parent`, creating it, and returns the file's path. */
async function write(parent: string, name: string, text: string): Promise<string> {
  await mkdir(parent, { recursive: true });
  const file = join(parent, name);
  await writeFile(file, text);
  return file;
}

/** A project file's text that sets `timeout.navigation` to `navigation` milliseconds. */
function navigation(value: number): string {
  return `version = "0.1"\n\n[settings.timeout]\nnavigation = ${value}\n`;
}

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the promise resolved");
}

describe("readProject", { tags: ["story:11-75"] }, () => {
  test("finds the project file in the nearest folder above the working directory", async () => {
    const file = await write(join(folder, "project"), "goodfetch.toml", navigation(45_000));
    const deeper = join(folder, "project", "src", "deeper");
    await mkdir(deeper, { recursive: true });
    await write(folder, "goodfetch.toml", navigation(60_000));

    expect(await readProject(undefined, () => deeper)).toEqual({
      carried: { path: file, settings: { timeout: { navigation: 45_000 } } },
      connectTimeout: undefined,
    });
  });

  test("reads the file `project` names, by its absolute path or from the working directory, and none for null", async () => {
    await write(join(folder, "work"), "goodfetch.toml", navigation(45_000));
    const other = await write(join(folder, "elsewhere"), "other.toml", navigation(60_000));
    const work = () => join(folder, "work");
    const settings = { timeout: { navigation: 60_000 } };

    expect([
      (await readProject(other, work))?.carried,
      (await readProject("../elsewhere/./other.toml", work))?.carried,
      await readProject(null, work),
    ]).toEqual([
      { path: other, settings },
      // As the Rust SDK's `std::path::absolute`: `..` kept for the file
      // system to resolve, `.` dropped.
      {
        path: process.platform === "win32" ? other : `${folder}/work/../elsewhere/other.toml`,
        settings,
      },
      undefined,
    ]);
  });

  test("carries a TOML date as its text and a value written as one dotted key unchanged", async () => {
    const file = await write(
      folder,
      "goodfetch.toml",
      'version = "0.1"\n\n[settings]\n"timeout.connect" = 200\nday = 2026-01-02\n',
    );

    expect(await readProject(file, () => folder)).toEqual({
      carried: { path: file, settings: { "timeout.connect": 200, day: "2026-01-02" } },
      connectTimeout: 200,
    });
  });

  test("refuses a working directory that cannot be read with setting_invalid naming `project`", async () => {
    const error = await failure(
      readProject(undefined, () => {
        throw new Error("ENOENT: the folder was removed");
      }),
    );

    expect(error).toMatchObject({
      code: "setting_invalid",
      details: { key: "project", layer: "project_configuration" },
    });
  });

  test.for([
    { name: "a named file that is not there", text: undefined, reason: "the file is not there" },
    { name: "a file that is no TOML", text: 'version = "0.1"\n[settings\n', reason: undefined },
    {
      name: "a file that names no version",
      text: "[settings.timeout]\nnavigation = 45000\n",
      reason: "it names no `version`",
    },
    {
      name: "a version that is no major.minor",
      text: 'version = "0.01"\n',
      reason: "`0.01` is no `major.minor` version",
    },
    {
      name: "a lowest_minor that is no minor",
      text: 'version = "0.1"\nlowest_minor = -1\n',
      reason: "`lowest_minor` is no minor",
    },
    {
      name: "a lowest_minor written as a float",
      text: 'version = "0.1"\nlowest_minor = 1.0\n',
      reason: "`lowest_minor` is no minor",
    },
    {
      name: "an integer past what a number holds exactly",
      text: 'version = "0.1"\n\n[settings]\nseed = 9007199254740993\n',
      reason: "the integer `9007199254740993` cannot be carried exactly",
    },
    {
      name: "settings that are not a table",
      text: 'version = "0.1"\nsettings = 3\n',
      reason: "`settings` is not a table",
    },
    {
      name: "a top-level key besides the settings",
      text: 'version = "0.1"\ntimeout = 3\n',
      reason: "`timeout` is no part of this file; settings go in `[settings]`",
    },
  ])("refuses $name with setting_invalid at project_configuration", async ({ text, reason }) => {
    const file =
      text === undefined
        ? join(folder, "goodfetch.toml")
        : await write(folder, "goodfetch.toml", text);

    const error = await failure(readProject(file, () => folder));

    expect(error).toMatchObject({
      code: "setting_invalid",
      fix: expect.stringContaining("`project: null`"),
      details: {
        key: file,
        layer: "project_configuration",
        ...(reason === undefined ? {} : { reason }),
      },
    });
  });

  test.for([
    {
      name: "a newer major",
      version: "9.0",
      fix: "Upgrade Goodfetch to a version that reads user-configuration 9.0.",
    },
    {
      name: "a newer 0.y line",
      version: "0.2",
      fix: "Upgrade Goodfetch to a version that reads user-configuration 0.2.",
    },
    { name: "an older 0.y line", version: "0.0", fix: "Run `upgrade-files`, then try again." },
  ])("refuses a file of $name with format_version_unsupported", async ({ version, fix }) => {
    const file = await write(folder, "goodfetch.toml", `version = "${version}"\n`);

    const error = await failure(readProject(file, () => folder));

    expect(error).toMatchObject({
      code: "format_version_unsupported",
      fix,
      details: { format: "user-configuration", found: version, supported: "0.1" },
    });
  });

  test.for([
    { name: "0", value: "0", reason: "a timeout of 0 ms would end the opening at once" },
    { name: "text", value: '"30s"', reason: "it is no whole number of milliseconds" },
    { name: "a fraction", value: "1.5", reason: "it is no whole number of milliseconds" },
    { name: "a whole float", value: "200.0", reason: "it is no whole number of milliseconds" },
  ])(
    "refuses a timeout.connect of $name with setting_invalid naming the key",
    async ({ value, reason }) => {
      const file = await write(
        folder,
        "goodfetch.toml",
        `version = "0.1"\n\n[settings.timeout]\nconnect = ${value}\n`,
      );

      const error = await failure(readProject(file, () => folder));

      expect(error).toMatchObject({
        code: "setting_invalid",
        limit: { key: "timeout.connect", layer: "project_configuration" },
        details: { key: "timeout.connect", layer: "project_configuration", reason },
      });
    },
  );
});
