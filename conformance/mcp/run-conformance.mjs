#!/usr/bin/env node
/**
 * Runs `@modelcontextprotocol/conformance` against Goodfetch's shipped Collie
 * executable (`goodfetch-collie mcp`) and the test-only fixture server
 * (`goodfetch-mcp-fixture-server`) for each supported MCP revision, plus the
 * implemented extensions (`sep-2640-skills-*`) and `StrictStdout` stdio checks
 * (ADR 0070, ticket #414).
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../..");
const EXPECTED_DIR = path.join(__dirname, "expected-failures");

const REVISIONS = [
  { version: "2025-03-26", mode: "spec-version" },
  { version: "2025-06-18", mode: "spec-version" },
  { version: "2025-11-25", mode: "requirements" },
  { version: "2026-07-28", mode: "requirements" },
];

const IMPLEMENTED_EXTENSIONS = [
  "sep-2640-skills-directory",
  "sep-2640-skills-enumeration",
  "sep-2640-skills-manifest",
];

function parseArgs(argv) {
  let revision = null;
  let target = "both"; // "collie" | "fixture" | "both"
  let skipStdio = false;
  let skipExtensions = false;
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--target" && argv[i + 1]) {
      target = argv[i + 1];
      i += 1;
    } else if (arg === "--skip-stdio") {
      skipStdio = true;
    } else if (arg === "--skip-extensions") {
      skipExtensions = true;
    } else if (!arg.startsWith("-")) {
      revision = arg;
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  return { revision, target, skipStdio, skipExtensions };
}

function binaryPath(name) {
  const targetDir = process.env.CARGO_TARGET_DIR || path.join(REPO_ROOT, "target");
  const ext = process.platform === "win32" ? ".exe" : "";
  return path.join(targetDir, "debug", `${name}${ext}`);
}

function cleanEnv(home) {
  const env = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (v !== undefined && !k.startsWith("GOODFETCH_")) {
      env[k] = v;
    }
  }
  env.GOODFETCH_HOME = home;
  return env;
}

/**
 * Starts a child MCP server with `--http 127.0.0.1:0` in an isolated temp home
 * and returns `{ child, address, token, home, close }`.
 */
async function startHttpServer(bin, args) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "goodfetch-mcp-conf-"));
  const child = spawn(bin, args, {
    env: cleanEnv(home),
    stdio: ["ignore", "pipe", "pipe"],
  });

  let stderrBuf = "";
  child.stderr.on("data", (chunk) => {
    stderrBuf += chunk.toString("utf8");
  });

  const startup = await new Promise((resolve, reject) => {
    const rl = readline.createInterface({ input: child.stdout });
    const timer = setTimeout(() => {
      reject(new Error(`timed out waiting for ${bin} startup JSON; stderr:\n${stderrBuf}`));
    }, 15_000);
    rl.once("line", (line) => {
      clearTimeout(timer);
      rl.close();
      try {
        resolve(JSON.parse(line));
      } catch (err) {
        reject(new Error(`invalid startup JSON from ${bin}: ${line} (${err})`));
      }
    });
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      reject(new Error(`${bin} exited early (${code ?? signal}); stderr:\n${stderrBuf}`));
    });
  });

  const close = async () => {
    child.kill("SIGTERM");
    await new Promise((resolve) => {
      const timer = setTimeout(() => {
        child.kill("SIGKILL");
        resolve();
      }, 3_000);
      child.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
    fs.rmSync(home, { recursive: true, force: true });
  };

  return { child, address: startup.address, token: startup.token, home, close };
}

/**
 * Thin loopback proxy that injects `Authorization: Bearer <token>` and rewrites
 * loopback `Host`/`Origin` to the target server's port while forwarding any
 * non-loopback `Host`/`Origin` untouched so `dns-rebinding-protection` hits the
 * server's real header checks.
 */
async function startAuthProxy(targetUrl, token) {
  const upstream = new URL(targetUrl);
  const upstreamHost = upstream.host;

  let proxyPort = 0;
  const isLoopbackAuthority = (authority) => {
    const lower = authority.toLowerCase();
    return (
      lower === `127.0.0.1:${proxyPort}` ||
      lower === `localhost:${proxyPort}` ||
      lower === `[::1]:${proxyPort}`
    );
  };

  const server = http.createServer((req, res) => {
    const headers = { ...req.headers };
    if (token && !headers.authorization) {
      headers.authorization = `Bearer ${token}`;
    }
    if (typeof headers.host === "string" && isLoopbackAuthority(headers.host)) {
      headers.host = upstreamHost;
    }
    if (typeof headers.origin === "string") {
      try {
        const originUrl = new URL(headers.origin);
        if (isLoopbackAuthority(originUrl.host)) {
          headers.origin = `${upstream.protocol}//${upstreamHost}`;
        }
      } catch {
        // Forward malformed Origin untouched so the server rejects it.
      }
    }

    const upstreamReq = http.request(
      {
        hostname: upstream.hostname,
        port: upstream.port,
        method: req.method,
        path: req.url,
        headers: { ...headers, connection: "close" },
        agent: false,
      },
      (upstreamRes) => {
        res.writeHead(upstreamRes.statusCode ?? 502, upstreamRes.headers);
        upstreamRes.pipe(res);
      },
    );
    upstreamReq.on("error", (err) => {
      if (!res.headersSent) {
        res.writeHead(502, { "content-type": "text/plain" });
      }
      res.end(`proxy error: ${err.message}`);
    });
    req.pipe(upstreamReq);
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address();
  if (!addr || typeof addr === "string") {
    throw new Error("failed to bind loopback proxy");
  }
  proxyPort = addr.port;
  const proxyUrl = `http://127.0.0.1:${proxyPort}${upstream.pathname}`;

  const close = () =>
    new Promise((resolve) => {
      server.close(() => resolve());
    });

  return { proxyUrl, close };
}

/**
 * Verifies stdio framing strictly under `trace` logging: every stdout line must
 * be a valid JSON-RPC 2.0 object, with no stray text or whitespace.
 */
async function checkStrictStdio(bin, args) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "goodfetch-mcp-stdio-"));
  fs.mkdirSync(path.join(home, "config"), { recursive: true });
  fs.writeFileSync(path.join(home, "config", "config.toml"), '[log]\nlevel = "trace"\n');

  const child = spawn(bin, args, {
    env: cleanEnv(home),
    stdio: ["pipe", "pipe", "pipe"],
  });

  let stderrBuf = "";
  child.stderr.on("data", (chunk) => {
    stderrBuf += chunk.toString("utf8");
  });

  const rl = readline.createInterface({ input: child.stdout });
  const lines = [];
  let lineResolver = null;
  rl.on("line", (line) => {
    if (lineResolver) {
      const resolve = lineResolver;
      lineResolver = null;
      resolve(line);
    } else {
      lines.push(line);
    }
  });

  const nextLine = () =>
    new Promise((resolve, reject) => {
      if (lines.length > 0) {
        resolve(lines.shift());
        return;
      }
      const timer = setTimeout(() => {
        reject(
          new Error(`timed out waiting for stdio JSON-RPC line from ${bin}; stderr:\n${stderrBuf}`),
        );
      }, 15_000);
      lineResolver = (line) => {
        clearTimeout(timer);
        resolve(line);
      };
    });

  const assertJsonRpcLine = (line) => {
    if (!line || line !== line.trim()) {
      throw new Error(
        `non-JSON-RPC whitespace/empty stdout line from ${bin}: ${JSON.stringify(line)}`,
      );
    }
    const parsed = JSON.parse(line);
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed) ||
      parsed.jsonrpc !== "2.0"
    ) {
      throw new Error(`stdout line from ${bin} is not JSON-RPC 2.0: ${line}`);
    }
    return parsed;
  };

  try {
    child.stdin.write(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: {
          protocolVersion: "2025-11-25",
          capabilities: {},
          clientInfo: { name: "strict-stdout-conformance", version: "1.0.0" },
        },
      })}\n`,
    );
    const init = assertJsonRpcLine(await nextLine());
    if (init.id !== 1 || !init.result) {
      throw new Error(`unexpected initialize reply from ${bin}: ${JSON.stringify(init)}`);
    }
    child.stdin.write(
      `${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`,
    );
    child.stdin.write(
      `${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} })}\n`,
    );
    const tools = assertJsonRpcLine(await nextLine());
    if (tools.id !== 2 || !Array.isArray(tools.result?.tools)) {
      throw new Error(`unexpected tools/list reply from ${bin}: ${JSON.stringify(tools)}`);
    }
    child.stdin.end();
    await new Promise((resolve, reject) => {
      child.once("exit", (code) => {
        if (code === 0) resolve();
        else reject(new Error(`${bin} stdio exited with ${code}; stderr:\n${stderrBuf}`));
      });
    });
  } finally {
    rl.close();
    child.kill("SIGTERM");
    fs.rmSync(home, { recursive: true, force: true });
  }
}

async function runConformanceCli(args) {
  const bin = path.join(__dirname, "node_modules", ".bin", "conformance");
  return new Promise((resolve) => {
    const child = spawn(bin, args, {
      cwd: REPO_ROOT,
      stdio: "inherit",
    });
    child.once("exit", (code) => resolve(code ?? 1));
  });
}

async function main() {
  const { revision, target, skipStdio, skipExtensions } = parseArgs(process.argv.slice(2));
  const selectedRevisions = revision ? REVISIONS.filter((r) => r.version === revision) : REVISIONS;
  if (selectedRevisions.length === 0) {
    console.error(`unknown MCP revision: ${revision}`);
    process.exit(2);
  }

  const outputRoot = fs.mkdtempSync(path.join(os.tmpdir(), "goodfetch-mcp-conf-out-"));
  let failures = 0;

  try {
    const runServerTarget = async (label, bin, stdioArgs, httpArgs, expectedFileFor) => {
      if (!fs.existsSync(bin)) {
        throw new Error(`missing binary ${bin}; build it first`);
      }
      if (!skipStdio) {
        console.log(`\n=== [${label}] StrictStdout stdio check ===`);
        await checkStrictStdio(bin, stdioArgs);
        console.log(`[${label}] StrictStdout stdio check passed`);
      }

      const srv = await startHttpServer(bin, httpArgs);
      const proxy = await startAuthProxy(srv.address, srv.token);
      try {
        for (const rev of selectedRevisions) {
          const flag = rev.mode === "requirements" ? "--requirements" : "--spec-version";
          const expectedFile = expectedFileFor(rev.version);
          const outDir = path.join(outputRoot, `${label}-${rev.version}`);
          console.log(`\n=== [${label}] conformance ${flag} ${rev.version} ===`);
          const code = await runConformanceCli([
            "server",
            "--url",
            proxy.proxyUrl,
            flag,
            rev.version,
            "--expected-failures",
            expectedFile,
            "--output-dir",
            outDir,
          ]);
          if (code !== 0) {
            console.error(`FAILED: [${label}] ${rev.version} (exit ${code})`);
            failures += 1;
          }
        }

        if (!skipExtensions && (!revision || revision === "2026-07-28")) {
          const extFile = expectedFileFor("extensions");
          for (const scenario of IMPLEMENTED_EXTENSIONS) {
            const outDir = path.join(outputRoot, `${label}-ext-${scenario}`);
            console.log(`\n=== [${label}] extension --scenario ${scenario} ===`);
            const code = await runConformanceCli([
              "server",
              "--url",
              proxy.proxyUrl,
              "--scenario",
              scenario,
              "--expected-failures",
              extFile,
              "--output-dir",
              outDir,
            ]);
            if (code !== 0) {
              console.error(`FAILED: [${label}] extension ${scenario} (exit ${code})`);
              failures += 1;
            }
          }
        }
      } finally {
        await proxy.close();
        await srv.close();
      }
    };

    if (target === "collie" || target === "both") {
      await runServerTarget(
        "goodfetch-collie",
        binaryPath("goodfetch-collie"),
        ["mcp", "--attached"],
        ["mcp", "--attached", "--http", "127.0.0.1:0"],
        (name) => path.join(EXPECTED_DIR, `${name}.yaml`),
      );
    }

    if (target === "fixture" || target === "both") {
      await runServerTarget(
        "goodfetch-mcp-fixture-server",
        binaryPath("goodfetch-mcp-fixture-server"),
        ["--stdio"],
        ["--http", "127.0.0.1:0"],
        () => path.join(EXPECTED_DIR, "fixture.yaml"),
      );
    }
  } finally {
    fs.rmSync(outputRoot, { recursive: true, force: true });
  }

  if (failures > 0) {
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
