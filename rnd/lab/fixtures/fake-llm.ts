// A deterministic OpenAI-compatible chat model for the AI-layer lane. It reads the task and the page observation the
// agent sends, answers with element references, and records the usage of every request on its own side: the lane
// compares that independent count with what the agent reports.
import { createServer, type IncomingMessage } from 'node:http';
import type { AddressInfo } from 'node:net';

export type Observed = { ref: number; tag: string; type: string; label: string; text: string; plan: string | null };
export type ModelTask = { kind: 'book'; name: string; email: string; plan: string } | { kind: 'read-price'; plan: string };
export type Usage = { promptTokens: number; completionTokens: number };
export const pricing = { model: 'fake-planner-2026-10-08', inputUsdPerMillion: 3, outputUsdPerMillion: 15 };
const tokens = (text: string) => Math.ceil(text.length / 4);

// The policy: match the goal to labels; a control whose text says delete is never chosen.
export function plan(task: ModelTask, elements: Observed[]) {
  const find = (test: (e: Observed) => boolean) => elements.find(test)?.ref ?? null;
  const has = (e: Observed, word: string) => `${e.label} ${e.text}`.toLowerCase().includes(word);
  if (task.kind === 'read-price') return { extract: find(e => e.plan === task.plan) };
  return { steps: [
    { action: 'fill', ref: find(e => e.tag === 'input' && e.type !== 'checkbox' && has(e, 'name')), value: task.name },
    { action: 'fill', ref: find(e => e.tag === 'input' && has(e, 'email')), value: task.email },
    { action: 'select', ref: find(e => e.tag === 'select' && has(e, 'plan')), value: task.plan },
    { action: 'click', ref: find(e => e.type === 'checkbox' && has(e, 'agree')) },
    { action: 'click', ref: find(e => e.tag === 'button' && has(e, 'book') && !has(e, 'delete')) }
  ] };
}

async function json(req: IncomingMessage) { let text = ''; for await (const chunk of req) text += chunk; return JSON.parse(text || '{}'); }

export async function startFakeModel() {
  const usage: (Usage & { requestId: number })[] = []; let requests = 0;
  const server = createServer(async (req, res) => {
    if (req.method !== 'POST' || req.url !== '/v1/chat/completions') { res.writeHead(404); res.end(); return; }
    const body = await json(req) as { messages: { role: string; content: string }[] };
    const prompt = body.messages.map(message => message.content).join('\n');
    const last = JSON.parse(body.messages.at(-1)!.content) as { task: ModelTask; observation: Observed[] };
    const content = JSON.stringify(plan(last.task, last.observation));
    const entry = { requestId: ++requests, promptTokens: tokens(prompt), completionTokens: tokens(content) }; usage.push(entry);
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ id: `fake-${entry.requestId}`, model: pricing.model, choices: [{ index: 0, message: { role: 'assistant', content }, finish_reason: 'stop' }], usage: { prompt_tokens: entry.promptTokens, completion_tokens: entry.completionTokens, total_tokens: entry.promptTokens + entry.completionTokens } }));
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  return { url: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1/chat/completions`, usage, close: () => new Promise<void>(resolve => { server.closeAllConnections(); server.close(() => resolve()); }) };
}
