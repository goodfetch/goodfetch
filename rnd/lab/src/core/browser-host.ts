// Chromium launched by the harness (or by a CDP driver in standalone mode) with one fixed flag set,
// exposing its DevTools WebSocket so every driver connects to the same browser build and configuration.
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// The Rust CDP driver launches with the same list (adapters/cdp-rust/src/browser.rs).
export const browserFlags = ['--headless', '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--disable-background-timer-throttling', '--disable-backgrounding-occluded-windows', '--disable-renderer-backgrounding', '--mute-audio', '--window-size=1280,720'];

export type HostedBrowser = { pid: number; wsEndpoint: string; port: number; close(): Promise<void> };

export async function launchBrowser(executablePath: string, timeoutMs = 30_000, extraFlags: string[] = []): Promise<HostedBrowser> {
  const profile = await mkdtemp(join(tmpdir(), 'goodfetch-chrome-'));
  const child = spawn(executablePath, [...browserFlags, ...extraFlags, `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  const exited = new Promise<void>(resolve => child.once('exit', () => resolve()));
  const close = async () => { if (child.exitCode === null && child.signalCode === null) { child.kill('SIGTERM'); const timer = setTimeout(() => child.kill('SIGKILL'), 5000); await exited; clearTimeout(timer); } await rm(profile, { recursive: true, force: true }); };
  try {
    const wsEndpoint = await new Promise<string>((resolve, reject) => {
      let buffer = '';
      const timer = setTimeout(() => reject(new Error('browser did not expose a DevTools endpoint in time')), timeoutMs);
      child.stderr!.setEncoding('utf8');
      child.stderr!.on('data', (chunk: string) => { buffer += chunk; const match = buffer.match(/DevTools listening on (ws:\/\/\S+)/); if (match) { clearTimeout(timer); resolve(match[1]!); } });
      child.once('error', error => { clearTimeout(timer); reject(error); });
      child.once('exit', code => { clearTimeout(timer); reject(new Error(`browser exited before ready (code ${code})`)); });
    });
    child.stderr!.resume();
    return { pid: child.pid!, wsEndpoint, port: Number(new URL(wsEndpoint).port), close };
  } catch (error) { await close(); throw error; }
}

// The same endpoint reached through another host and port, such as a latency proxy in front of the browser.
export const throughProxy = (wsEndpoint: string, proxy: { host: string; port: number }) => { const url = new URL(wsEndpoint); url.hostname = proxy.host; url.port = String(proxy.port); return url.href; };

// Lightpanda (AGPL-3.0, so always out of process): a CDP server without rendering, pinned under .toolchains.
export const lightpandaPath = (root: string) => join(root, '.toolchains/lightpanda/1.0.0/lightpanda-aarch64-macos');
export async function launchLightpanda(binary: string, timeoutMs = 15_000, extraArgs: string[] = []): Promise<HostedBrowser> {
  const { createServer } = await import('node:net');
  const port = await new Promise<number>(resolve => { const probe = createServer(); probe.listen(0, '127.0.0.1', () => { const free = (probe.address() as { port: number }).port; probe.close(() => resolve(free)); }); });
  const child = spawn(binary, ['serve', '--host', '127.0.0.1', '--port', String(port), '--disable-metrics', ...extraArgs], { stdio: ['ignore', 'ignore', 'ignore'] });
  const exited = new Promise<void>(resolve => child.once('exit', () => resolve()));
  const close = async () => { if (child.exitCode === null && child.signalCode === null) { child.kill('SIGTERM'); const timer = setTimeout(() => child.kill('SIGKILL'), 5000); await exited; clearTimeout(timer); } };
  const deadline = performance.now() + timeoutMs;
  while (performance.now() < deadline) {
    try { const response = await fetch(`http://127.0.0.1:${port}/json/version`); if (response.ok) { const { webSocketDebuggerUrl } = await response.json() as { webSocketDebuggerUrl?: string }; return { pid: child.pid!, wsEndpoint: webSocketDebuggerUrl ?? `ws://127.0.0.1:${port}/`, port, close }; } } catch { /* not listening yet */ }
    if (child.exitCode !== null) break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  await close(); throw new Error('Lightpanda did not expose a CDP endpoint in time');
}
