// The program's side of file saving (ADR 0116) against the real
// filesystem: a file lands whole or not at all, and a failure is
// `path_not_writable` or `file_write_failed` with side `program`, its OS
// error chained as the cause. The save helper over a Connection whose
// engine the test plays: the engine writes the path, or streams the bytes
// for the SDK to write.
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { ClientConnection } from "./client-connection.ts";
import { FileSaving, saveFile, writeFileWhole } from "./file-saving.ts";
import { encodeMessage } from "./framing.ts";
import {
  collieErrorClasses,
  FileWriteFailedError,
  PathNotWritableError,
} from "./generated/errors.ts";
import type { Transport } from "./rpc.ts";

let folder: string;

beforeEach(async () => {
  folder = await mkdtemp(join(tmpdir(), "file-saving-"));
});

afterEach(async () => {
  await chmod(folder, 0o700).catch(() => undefined);
  await rm(folder, { recursive: true, force: true });
});

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the write succeeded");
}

/** The names in `at`, the temporary files among them. */
async function namesIn(at: string): Promise<string[]> {
  return (await readdir(at)).toSorted();
}

const saveAs = { call: "saveAs" } as const;

/** A small CSV file in two parts. */
async function* csvParts(): AsyncIterable<Uint8Array> {
  yield bytes("a,b\n");
  yield bytes("1,2\n");
}
const runsAsRoot = process.getuid?.() === 0;

describe("writeFileWhole", () => {
  test("makes the missing folders and leaves only the file", async () => {
    const path = join(folder, "a", "b", "report.pdf");

    await writeFileWhole(path, bytes("%PDF"), saveAs);

    expect(await readFile(path, "utf8")).toBe("%PDF");
    expect(await namesIn(join(folder, "a", "b"))).toEqual(["report.pdf"]);
  });

  test("resolves a relative path against the working directory", async () => {
    const before = process.cwd();
    process.chdir(folder);
    try {
      await writeFileWhole(join("out", "page.html"), bytes("<html>"), saveAs);
    } finally {
      process.chdir(before);
    }

    expect(await readFile(join(folder, "out", "page.html"), "utf8")).toBe("<html>");
  });

  test("replaces a file at the path whole, from parts", async () => {
    const path = join(folder, "data.csv");
    await writeFile(path, "what it held, and longer than the new file");

    await writeFileWhole(path, csvParts(), saveAs);

    expect(await readFile(path, "utf8")).toBe("a,b\n1,2\n");
    expect(await namesIn(folder)).toEqual(["data.csv"]);
  });

  test.skipIf(runsAsRoot || process.platform === "win32")(
    "raises path_not_writable in a folder the user cannot write, its OS error the cause",
    async () => {
      const locked = join(folder, "locked");
      await mkdir(locked);
      await chmod(locked, 0o500);
      const path = join(locked, "report.pdf");

      const error = await failure(writeFileWhole(path, bytes("%PDF"), saveAs));

      expect(error).toBeInstanceOf(PathNotWritableError);
      expect(error).toMatchObject({
        code: "path_not_writable",
        retryable: false,
        fix: "Give `saveAs` a path in a folder this user can write.",
        details: { reason: "permission_denied", os_code: "EACCES", path },
        cause: { code: "EACCES" },
      });
      await chmod(locked, 0o700);
      expect(await namesIn(locked)).toEqual([]);
    },
  );

  test("raises path_not_writable naming a folder at the path", async () => {
    const path = join(folder, "taken");
    await mkdir(path);

    const error = await failure(writeFileWhole(path, bytes("x"), saveAs));

    expect(error).toBeInstanceOf(PathNotWritableError);
    expect(error).toMatchObject({
      fix: "Give `saveAs` a file name, not a folder.",
      details: { reason: "is_a_directory", path },
    });
  });

  test("raises path_not_writable when a parent folder is a file", async () => {
    await writeFile(join(folder, "plain"), "a file");
    const path = join(folder, "plain", "inner", "report.pdf");

    const error = await failure(writeFileWhole(path, bytes("x"), saveAs));

    expect(error).toBeInstanceOf(PathNotWritableError);
    expect(error).toMatchObject({
      fix: "Give `saveAs` a path whose parent folders are folders, not files.",
      details: { reason: "not_a_directory", path },
    });
    expect(error).toHaveProperty("cause.code");
  });

  test("raises path_not_writable for a path that names no file", async () => {
    const error = await failure(writeFileWhole(`${folder}/`, bytes("x"), saveAs));

    expect(error).toBeInstanceOf(PathNotWritableError);
    expect(error).toMatchObject({ details: { reason: "invalid_filename", os_code: null } });
  });

  test("leaves the path as it was when the signal aborts mid-write", async () => {
    const path = join(folder, "data.csv");
    await writeFile(path, "what it held");
    const controller = new AbortController();

    async function* parts(): AsyncIterable<Uint8Array> {
      yield bytes("first part\n");
      controller.abort(new Error("deadline"));
      yield bytes("second part\n");
    }
    const error = await failure(
      writeFileWhole(path, parts(), { ...saveAs, signal: controller.signal }),
    );

    expect(error).toEqual(new Error("deadline"));
    expect(await readFile(path, "utf8")).toBe("what it held");
    expect(await namesIn(folder)).toEqual(["data.csv"]);
  });

  test("names the call given in the fix", async () => {
    const path = join(folder, "taken");
    await mkdir(path);

    const error = await failure(writeFileWhole(path, bytes("x"), { call: "screenshot()" }));

    expect(error).toMatchObject({ fix: "Give `screenshot()` a file name, not a folder." });
  });
});

describe("FileSaving", () => {
  test("abort removes the temporary file and leaves the path as it was", async () => {
    const path = join(folder, "data.csv");
    await writeFile(path, "what it held");

    const saving = await FileSaving.create(path, saveAs);
    await saving.write(bytes("half a file"));
    expect((await namesIn(folder)).filter((name) => name.endsWith(".partial"))).toHaveLength(1);
    await saving.abort();

    expect(await readFile(path, "utf8")).toBe("what it held");
    expect(await namesIn(folder)).toEqual(["data.csv"]);
  });
});

/** A tmpfs of `size` bytes over a folder, mounted in a user and mount namespace of a `sleep` the test kills. */
interface SmallFilesystem {
  /** The folder as this process reaches it, through the sleeper's root. */
  readonly inside: string;
  readonly sleeper: ChildProcess;
}

/** The test-only `unshare` that `ci/os-services.sh setup` installs. */
const TEST_UNSHARE = "/usr/local/libexec/goodfetch-test-tools/unshare";

async function smallFilesystem(size: number): Promise<SmallFilesystem> {
  const small = join(folder, "small");
  await mkdir(small);
  const sleeper = spawn(
    existsSync(TEST_UNSHARE) ? TEST_UNSHARE : "unshare",
    [
      "--user",
      "--map-root-user",
      "--mount",
      "sh",
      "-c",
      'mount -t tmpfs -o size="$2" tmpfs "$1" && touch "$1/.mounted" && exec sleep 600',
      "sh",
      small,
      String(size),
    ],
    { stdio: "ignore" },
  );
  const inside = join("/proc", String(sleeper.pid), "root", small);
  for (let tries = 0; tries < 500; tries += 1) {
    if (existsSync(join(inside, ".mounted"))) {
      await unlink(join(inside, ".mounted"));
      return { inside, sleeper };
    }
    await new Promise((settle) => setTimeout(settle, 10));
  }
  sleeper.kill();
  throw new Error(
    `\`${TEST_UNSHARE}\` mounted no tmpfs over ${small}: run \`ci/os-services.sh setup\``,
  );
}

describe.runIf(process.platform === "linux")("on a full filesystem", () => {
  let mounted: SmallFilesystem | undefined;

  afterEach(() => {
    mounted?.sleeper.kill();
    mounted = undefined;
  });

  test("raises file_write_failed with side program, and keeps the file it replaced", async () => {
    mounted = await smallFilesystem(64 * 1024);
    const path = join(mounted.inside, "data.bin");
    await writeFile(path, "what it held");

    const error = await failure(writeFileWhole(path, new Uint8Array(256 * 1024), saveAs));

    expect(error).toBeInstanceOf(FileWriteFailedError);
    expect(error).toMatchObject({
      code: "file_write_failed",
      retryable: false,
      details: { reason: "storage_full", os_code: "ENOSPC", path, side: "program" },
      cause: { code: "ENOSPC" },
    });
    expect(await readFile(path, "utf8")).toBe("what it held");
    expect(await namesIn(mounted.inside)).toEqual(["data.bin"]);
  });
});

/** The engine's side of an in-memory transport: what the Client wrote, and what it reads next. */
interface PlayedEngine {
  readonly transport: Transport;
  /** Each message the Client wrote, parsed, as it arrives. */
  readonly written: Record<string, unknown>[];
  /** Sends the Client one message. */
  send(message: Readonly<Record<string, unknown>>): void;
  /** The next message the Client writes, once it has. */
  next(): Promise<Record<string, unknown>>;
}

function playedEngine(): PlayedEngine {
  const decoder = new TextDecoder();
  const written: Record<string, unknown>[] = [];
  const waiting: ((message: Record<string, unknown>) => void)[] = [];
  let read = 0;
  let engine: ReadableStreamDefaultController<Uint8Array> | undefined;
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      engine = controller;
    },
  });
  const writable = new WritableStream<Uint8Array>({
    write(chunk) {
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the Client writes one JSON-RPC object per write.
      written.push(JSON.parse(decoder.decode(chunk)) as Record<string, unknown>);
      while (read < written.length && waiting.length > 0) {
        const message = written[read];
        read += 1;
        if (message !== undefined) {
          waiting.shift()?.(message);
        }
      }
    },
    // The Client's close ends its input; the engine's output ends with it.
    close() {
      engine?.close();
    },
  });
  return {
    transport: { readable, writable },
    written,
    send: (message) => engine?.enqueue(encodeMessage(message)),
    next: () => {
      const message = written[read];
      if (message !== undefined) {
        read += 1;
        return Promise.resolve(message);
      }
      return new Promise((settle) => waiting.push(settle));
    },
  };
}

/** A Connection to `engine`, past its hello. */
async function connected(engine: PlayedEngine): Promise<ClientConnection> {
  const opening = ClientConnection.open({
    transport: engine.transport,
    needs: "collie",
    errorClasses: collieErrorClasses,
  });
  const hello = await engine.next();
  engine.send({
    jsonrpc: "2.0",
    id: hello.id,
    result: {
      protocol_version: "0.1",
      product: "collie",
      product_version: "0.1.0",
      engine_instance: "0123456789abcdef",
    },
  });
  return opening;
}

describe("saveFile", () => {
  test("gives a local engine the absolute path, and writes nothing when it wrote the file", async () => {
    const engine = playedEngine();
    const connection = await connected(engine);
    const path = join(folder, "report.pdf");

    const saving = saveFile(connection, "download.save_as", { download_id: "d1" }, path, saveAs);
    const call = await engine.next();
    engine.send({ jsonrpc: "2.0", id: call.id, result: { written: true } });

    expect(await saving).toEqual({ written: true });
    expect(call).toMatchObject({
      method: "download.save_as",
      params: { download_id: "d1", path },
    });
    expect(await namesIn(folder)).toEqual([]);
    await connection.close();
  });

  test("writes the chunks the engine streams when it did not write the file, granting credits", async () => {
    const engine = playedEngine();
    const connection = await connected(engine);
    const path = join(folder, "out", "data.csv");

    const saving = saveFile(connection, "download.save_as", { download_id: "d1" }, path, saveAs);
    const call = await engine.next();
    const credit = await engine.next();
    for (const [offset, part] of [
      [0, "a,b\n"],
      [4, "1,2\n"],
    ] as const) {
      engine.send({
        jsonrpc: "2.0",
        method: "$/chunk",
        params: { id: call.id, offset, data: Buffer.from(part).toString("base64") },
      });
    }
    engine.send({ jsonrpc: "2.0", id: call.id, result: { written: false } });

    expect(await saving).toEqual({ written: false });
    expect(credit).toMatchObject({ method: "$/credit", params: { id: call.id, credits: 16 } });
    expect(await readFile(path, "utf8")).toBe("a,b\n1,2\n");
    expect(await namesIn(join(folder, "out"))).toEqual(["data.csv"]);
    await connection.close();
  });

  test("writes an empty file when the engine streams none and did not write it", async () => {
    const engine = playedEngine();
    const connection = await connected(engine);
    const path = join(folder, "empty.txt");

    const saving = saveFile(connection, "download.save_as", { download_id: "d1" }, path, saveAs);
    const call = await engine.next();
    engine.send({ jsonrpc: "2.0", id: call.id, result: { written: false } });

    await saving;
    expect(await readFile(path, "utf8")).toBe("");
    await connection.close();
  });

  test("leaves the path as it was when the engine fails the call mid-stream", async () => {
    const engine = playedEngine();
    const connection = await connected(engine);
    const path = join(folder, "data.csv");
    await writeFile(path, "what it held");

    const saving = saveFile(connection, "download.save_as", { download_id: "d1" }, path, saveAs);
    const call = await engine.next();
    engine.send({
      jsonrpc: "2.0",
      method: "$/chunk",
      params: { id: call.id, offset: 0, data: Buffer.from("half").toString("base64") },
    });
    engine.send({
      jsonrpc: "2.0",
      id: call.id,
      error: {
        code: -32000,
        message: "the page crashed",
        data: { code: "page_crashed", retryable: true },
      },
    });

    const error = await failure(saving);

    expect(error).toMatchObject({ code: "page_crashed" });
    expect(await readFile(path, "utf8")).toBe("what it held");
    expect(await namesIn(folder)).toEqual(["data.csv"]);
    await connection.close();
  });
});
