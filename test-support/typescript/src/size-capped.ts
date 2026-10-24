// A size-capped filesystem for one served test (TESTING.md, Storage,
// network and OS services; ADR 0122), as the Rust Storage cases use: on
// Linux, a tmpfs mounted over a folder of the test's in a user and mount
// namespace that `goodfetch-test-env` is started in, so the test's whole
// folder, its Goodfetch home and its Daemon's files, is on it. The test's
// own processes stay outside: they reach the filesystem through
// `/proc/<pid>/root` of `goodfetch-test-env`, run the executable's commands
// in its namespaces through `nsenter`, and the tmpfs goes when the last
// process inside exits, never filling the machine's disk. `ci/os-services.sh
// setup` installs the `unshare` that may make the namespace.
import { existsSync } from "node:fs";
import { mkdtemp, open, rm, rmdir, statfs } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TestSupportError } from "./errors.ts";
import type { Program } from "./test-env.ts";

/** The test-only `unshare` that `ci/os-services.sh setup` installs, which AppArmor lets make user namespaces. */
const TEST_UNSHARE = "/usr/local/libexec/goodfetch-test-tools/unshare";

/** Mounts a tmpfs of `$2` bytes over the folder `$1`, makes it the temporary folder, then runs the rest of the arguments. */
const MOUNT =
  'set -e; folder="$1"; size="$2"; shift 2; mount -t tmpfs -o "size=$size" tmpfs "$folder"; TMPDIR="$folder"; export TMPDIR; exec "$@"';

/** The filler a test fills the filesystem with. */
const FILLER = "filler";

/** One write of the filler, in bytes. */
const BLOCK = 1 << 20;

/** A size-capped filesystem a served test's folder is on. */
export interface SizeCapped {
  /** Its size, in bytes. */
  readonly size: number;
  /** `path`, a path inside the namespace, as the test's own processes reach it. */
  outside(path: string): string;
  /** Fills it, keeping `room` bytes free, with a file of zeros in `folder`, a path inside the namespace. */
  fill(folder: string, room: number): Promise<void>;
  /** Frees what the fills took. */
  free(): Promise<void>;
}

/** A folder made for the mount, and the program that makes it and runs `goodfetch-test-env` on it. */
export interface Mounting {
  /** Runs `testEnv` on a tmpfs of `size` bytes. */
  readonly program: Program;
  /** The filesystem, once `goodfetch-test-env` runs as `pid`. */
  mounted(pid: number): SizeCapped;
  /** Runs `command` with `args` in `pid`'s namespaces, as `nsenter` does. */
  inside(pid: number, command: string, args: readonly string[]): Program;
  /** Removes the folder, which the tmpfs left empty. */
  close(): Promise<void>;
}

/**
 * Prepares a tmpfs of `size` bytes for `goodfetch-test-env` at `testEnv`.
 *
 * @throws {TestSupportError} `size_capped_unsupported` off Linux, where a small disk image stands in for it, which test support does not make yet.
 */
export async function sizeCapped(size: number, testEnv: string): Promise<Mounting> {
  if (process.platform !== "linux") {
    throw new TestSupportError(
      "size_capped_unsupported",
      `a size-capped filesystem is a tmpfs on Linux only; ${process.platform}'s small disk image is not made yet`,
    );
  }
  const folder = await mkdtemp(join(tmpdir(), "goodfetch-size-capped-"));
  const unshare = existsSync(TEST_UNSHARE) ? TEST_UNSHARE : "unshare";
  return {
    program: {
      command: unshare,
      args: [
        "--user",
        "--map-root-user",
        "--mount",
        "sh",
        "-c",
        MOUNT,
        "sh",
        folder,
        String(size),
        testEnv,
      ],
    },
    mounted: (pid) => mountedAt(pid, size),
    inside: (pid, command, args) => ({
      command: "nsenter",
      args: ["-t", String(pid), "-U", "-m", "--preserve-credentials", "--", command, ...args],
    }),
    close: async () => {
      await rmdir(folder).catch(async () => rm(folder, { recursive: true, force: true }));
    },
  };
}

function mountedAt(pid: number, size: number): SizeCapped {
  const outside = (path: string): string => join("/proc", String(pid), "root", path);
  const fillers: string[] = [];
  return {
    size,
    outside,
    fill: async (folder, room) => {
      const path = outside(join(folder, `${FILLER}-${String(fillers.length)}`));
      fillers.push(path);
      const file = await open(path, "w");
      try {
        const block = Buffer.alloc(BLOCK);
        let left = (await available(path)) - room;
        while (left > 0) {
          const length = Math.min(BLOCK, left);
          await file.write(block, 0, length);
          left -= length;
        }
        await file.sync();
      } finally {
        await file.close();
      }
    },
    free: async () => {
      for (const filler of fillers.splice(0)) {
        await rm(filler, { force: true });
      }
    },
  };
}

/** The bytes left to an unprivileged user on the filesystem holding `path`. */
async function available(path: string): Promise<number> {
  const stats = await statfs(path);
  return stats.bavail * stats.bsize;
}
