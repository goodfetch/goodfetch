// The Goodfetch Guard (ADR 0019) over WebDriver BiDi: the engine's own check and disarm scripts, read from the page
// crate unchanged, called with `script.callFunction` in a named sandbox the page cannot see, `awaitPromise` set. One
// call checks and arms, waiting in the page at most a slice; the loop below calls it again until the action's window
// passes, as the engine's `guard::attempt` does, so nothing polls the page from outside and nothing runs in its main world.
import { readFileSync } from 'node:fs';
import { Bidi, BidiError, fromRemote, toLocal } from './bidi.ts';

const source = (name: string) => readFileSync(new URL(`../../../../crates/goodfetch-page/src/guard/${name}`, import.meta.url), 'utf8');
export const CHECK = source('check.js');
export const DISARM = source('disarm.js');
// The engine's `guard::SETTLED`: a call that does nothing, one round trip into the page once a key press's input was sent.
const SETTLED = 'function settled() {}';
// The sandbox the Guard and the driver's reads run in.
export const SANDBOX = 'goodfetch-guard';
// The longest one check waits in the page before it answers, as the engine's `guard::SLICE`.
const SLICE_MS = 1000;

export type Kind = 'pointer' | 'fill' | 'text';
// What one check answered (the engine's `check::Seen`).
type Seen =
  | { state: 'armed'; x: number; y: number }
  | { state: 'focused'; set_value?: boolean }
  | { state: 'waiting'; step: string; observed: string; matches?: string[] }
  | { state: 'invalid'; message: string };
type Waiting = Extract<Seen, { state: 'waiting' }>;
type Landed = { state: 'landed' } | { state: 'intercepted'; observed: string };
export type Point = { x: number; y: number };

// The realm a call targets went away with its document: the call is made again, as the engine's `WorldGone`. An
// exception the script threw is never one, whatever its text: the engine fails the action on it.
export class WorldGone extends Error {}
export function isWorldGone(error: unknown): boolean {
  if (error instanceof WorldGone) return true;
  if (!(error instanceof BidiError) || error.code === 'javascript error') return false;
  return error.code === 'no such frame' || /discarded|destroyed|dead object|navigat/i.test(error.message);
}

// Calls `functionDeclaration` with `argument` in the Guard's sandbox of `context` and answers its value.
export async function callInSandbox(bidi: Bidi, context: string, functionDeclaration: string, argument?: unknown): Promise<unknown> {
  const answer = await bidi.send('script.callFunction', { functionDeclaration, arguments: argument === undefined ? [] : [toLocal(argument)], awaitPromise: true, resultOwnership: 'none', target: { context, sandbox: SANDBOX } });
  if (answer.type === 'exception') {
    const details = answer.exceptionDetails as { text?: string } | undefined;
    throw new BidiError('script.callFunction', 'javascript error', details?.text ?? 'exception');
  }
  return fromRemote(answer.result as Record<string, unknown>);
}

function readSeen(value: unknown): Seen {
  const seen = value as Partial<Record<string, unknown>> | null;
  switch (seen?.state) {
    case 'armed': if (typeof seen.x === 'number' && typeof seen.y === 'number') return { state: 'armed', x: seen.x, y: seen.y }; break;
    case 'focused': return { state: 'focused', set_value: seen.set_value === true };
    case 'waiting': return { state: 'waiting', step: String(seen.step), observed: String(seen.observed), ...(Array.isArray(seen.matches) ? { matches: seen.matches.map(String) } : {}) };
    case 'invalid': return { state: 'invalid', message: String(seen.message) };
  }
  throw new Error(`the Guard answered no check: ${JSON.stringify(value)}`);
}

function readLanded(value: unknown): Landed {
  const landed = value as Partial<Record<string, unknown>> | null;
  if (landed?.state === 'landed') return { state: 'landed' };
  if (landed?.state === 'intercepted') return { state: 'intercepted', observed: String(landed.observed) };
  throw new Error(`the Guard answered no disarm: ${JSON.stringify(value)}`);
}

// Where the Guard runs: a call into its isolated world, which throws an error `isWorldGone` takes when the document
// went away, and the log of every command the session sent.
export type GuardWorld = { call(functionDeclaration: string, argument?: unknown): Promise<unknown>; sent(): readonly string[] };
// The Guard's world in `context` over WebDriver BiDi: its sandbox.
export function sandboxWorld(bidi: Bidi, context: string): GuardWorld {
  return { call: (functionDeclaration, argument) => callInSandbox(bidi, context, functionDeclaration, argument), sent: () => bidi.sent.map(command => command.method) };
}
// What one guarded action asks: its target, its kind, the events its arm watches and its window.
export type Request = { selector: string; kind: Kind; events: readonly string[]; timeoutMs: number };
// How a guarded action went: its checks, inputs and disarms, counted on the session's own log.
export type Acted = { attempts: number; commands: number; methods: string[] };

// Waits for one round trip into the page once a key press's input was sent, as the engine's `settled`: the events
// the input caused, such as a navigation, arrive before its answer. A document gone with the input sent them already.
export async function settled(world: GuardWorld): Promise<void> {
  try { await world.call(SETTLED); } catch (error) { if (!isWorldGone(error)) throw error; }
}

// Performs `request` through the Guard: `perform` sends the input once the target is ready, at the armed point for a
// pointer action. A gesture that landed elsewhere is replayed until the window passes; at the deadline the last
// observation decides, as the engine's `target_not_unique` and `deadline_exceeded`. A check answers within its slice
// of the window, so past the window and one slice the page is not answering, as the engine's `guard::act` bounds it.
export async function act(world: GuardWorld, request: Request, perform: (point: Point) => Promise<void>): Promise<Acted> {
  let timer: NodeJS.Timeout | undefined;
  const unanswered = new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error(`deadline_exceeded: ${request.selector}: attached: the page answered none of the action's checks in time`)), request.timeoutMs + SLICE_MS); });
  const attempted = attempt(world, request, perform);
  try { return await Promise.race([attempted, unanswered]); }
  finally { clearTimeout(timer); attempted.catch(() => { /* answered after the bound: the action already failed */ }); }
}

async function attempt(world: GuardWorld, request: Request, perform: (point: Point) => Promise<void>): Promise<Acted> {
  const firstCommand = world.sent().length; const end = performance.now() + request.timeoutMs; let attempts = 0;
  let last: Waiting = { state: 'waiting', step: 'attached', observed: 'the action has not checked its target yet' };
  for (;;) {
    const remaining = end - performance.now();
    if (remaining <= 0) {
      if (last.step === 'unique') throw new Error(`target_not_unique: ${request.selector}: ${(last.matches ?? []).join(', ')}`);
      throw new Error(`deadline_exceeded: ${request.selector}: ${last.step}: ${last.observed}`);
    }
    attempts++;
    let seen: Seen;
    try { seen = readSeen(await world.call(CHECK, { selector: request.selector, action: request.kind, events: request.events, slice: Math.max(1, Math.min(SLICE_MS, Math.round(remaining))) })); }
    catch (error) { if (isWorldGone(error)) continue; throw error; }
    if (seen.state === 'invalid') throw new Error(`invalid_params: \`${request.selector}\` is no CSS selector the page parses: ${seen.message}`);
    if (seen.state === 'waiting') { last = seen; continue; }
    if (seen.state === 'focused') {
      if (request.kind === 'pointer') throw new Error(`the Guard focused a pointer target: ${JSON.stringify(seen)}`);
      if (seen.set_value) throw new Error('a picked value is not in the matrix: the BiDi route sets no picked value');
      await perform({ x: 0, y: 0 });
      return done();
    }
    if (request.kind !== 'pointer') throw new Error(`the Guard armed a text target: ${JSON.stringify(seen)}`);
    await perform({ x: seen.x, y: seen.y });
    let landed: Landed;
    // A document gone with the gesture means it landed: nothing was left to cancel it.
    try { landed = readLanded(await world.call(DISARM)); }
    catch (error) { if (!isWorldGone(error)) throw error; landed = { state: 'landed' }; }
    if (landed.state === 'landed') return done();
    last = { state: 'waiting', step: 'receives_events', observed: landed.observed };
  }
  function done(): Acted {
    const methods = world.sent().slice(firstCommand);
    return { attempts, commands: methods.length, methods };
  }
}
