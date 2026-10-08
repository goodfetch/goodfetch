// AI-layer lane (D-ai-layer): sessions of warm runs on the unchanged page, then one run on a drifted page, for an agent
// that plans on every run and for a compiled plan replayed without the model. Model usage comes from the fake model's
// own accounting and is compared with what the agent reports.
import { startFixtures } from '../fixtures/server.ts';
import { startFakeModel, pricing, type ModelTask } from '../fixtures/fake-llm.ts';
import { launchBrowser } from './core/browser-host.ts';
import { browserPath } from './environment.ts';
import { CdpConnection, CdpPage } from '../adapters/cdp-node/cdp.ts';
import { waitScript, condition } from '../adapters/browser-scripts/scripts.ts';
import { runTask, type AgentMode, type CompiledPlan, type ModelEndpoint } from './ai/agent.ts';
import { runInterface, type Interface } from './ai/interfaces.ts';
import { quantiles } from './statistics.ts';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';

// Real models are reached through an OpenAI-compatible gateway; the key comes from the environment and is never recorded.
export type AiConfig = { modes: AgentMode[]; variants: string[]; warmRuns: number; blocks: number; seed: number; timeoutMs: number; models?: string[]; gateway?: string; maxUsd?: number; interfaces?: Interface[] };
// With interfaces, a cell is one model reaching the page through one interface; otherwise one agent mode.
export type AiCell = { mode?: AgentMode; interface?: Interface; variant: string; model: string };
export const aiCombinations = (config: AiConfig): AiCell[] => (config.models ?? [pricing.model]).flatMap((model): AiCell[] => config.interfaces
  ? config.interfaces.flatMap(kind => config.variants.map(variant => ({ interface: kind, variant, model })))
  : config.modes.flatMap(mode => config.variants.map(variant => ({ mode, variant, model }))));
export const gatewayKey = () => process.env.AUTOMAN_AI_GATEWAY_KEY;
type Price = { input: number; output: number };
// A dated snapshot of the gateway's price per token for the campaign's models, kept with the run as the independent cost oracle.
export async function priceSnapshot(config: AiConfig): Promise<Record<string, Price> | null> {
  if (!config.models) return null;
  const response = await fetch(`${config.gateway}/models`); if (!response.ok) throw new Error(`gateway models HTTP ${response.status}`);
  const { data } = await response.json() as { data: { id: string; pricing?: { input?: string; output?: string } }[] };
  return Object.fromEntries(config.models.map(id => { const entry = data.find(m => m.id === id); if (!entry?.pricing?.input || !entry.pricing.output) throw new Error(`no price for ${id}`); return [id, { input: Number(entry.pricing.input), output: Number(entry.pricing.output) }]; }));
}
// Spend of this campaign process; a cell that would start beyond the cap is refused, never run.
let spentUsd = 0;
const taskFor = (variant: string): ModelTask => variant === 'price-change' ? { kind: 'read-price', plan: 'pro' } : { kind: 'book', name: 'Ada Lovelace', email: 'ada@example.test', plan: 'pro' };

export async function runAiCell(config: AiConfig, cell: AiCell, block: number, directory?: string, prices?: Record<string, Price> | null): Promise<Record<string, unknown>> {
  const real = cell.model !== pricing.model;
  if (real && !gatewayKey()) return { ...cell, block, status: 'unsupported', reason: 'AUTOMAN_AI_GATEWAY_KEY is not set' };
  if (real && config.maxUsd !== undefined && spentUsd >= config.maxUsd) return { ...cell, block, status: 'rejected', errorCode: 'E_BUDGET', reason: `campaign spend ${spentUsd.toFixed(4)} USD reached the cap of ${config.maxUsd} USD` };
  if (real && prices && directory) await writeFile(join(directory, 'model-prices.json'), JSON.stringify({ fetchedFrom: `${config.gateway}/models`, unit: 'USD per token', prices }, null, 2) + '\n');
  if (cell.interface) return runInterfaceCell(config, cell as AiCell & { interface: Interface }, block);
  const [fixtures, model, browser] = await Promise.all([startFixtures(), startFakeModel(), launchBrowser(browserPath())]);
  const endpoint: ModelEndpoint = real ? { url: `${config.gateway}/chat/completions`, model: cell.model, apiKey: gatewayKey() } : { url: model.url, model: pricing.model };
  const connection = await CdpConnection.connect(browser.wsEndpoint); const store = new Map<string, CompiledPlan>();
  const sequence = [...Array.from({ length: config.warmRuns }, () => 'baseline'), ...(cell.variant === 'baseline' ? [] : [cell.variant])];
  let verified = 0, falseSuccess = 0, failedExplicitly = 0, modelCalls = 0, repairs = 0, agentTokens = 0, gatewayUsd = 0, pricedUsd = 0; const reasons: string[] = [];
  try {
    for (const variant of sequence) {
      const task = taskFor(cell.variant); const fixture = await fixtures.createRun({ scenario: 'agent', seed: config.seed + block, variant: variant as never });
      const page = await CdpPage.open(connection, config.timeoutMs);
      try {
        await page.goto(fixture.url); await page.evaluate(waitScript(condition('body-ready'), config.timeoutMs));
        const result = await runTask(page, cell.mode!, task, store, endpoint, config.timeoutMs);
        modelCalls += result.modelCalls; repairs += result.repairs; agentTokens += result.usage.reduce((sum, u) => sum + u.promptTokens + u.completionTokens, 0);
        if (real) for (const u of result.usage) { gatewayUsd += u.costUsd ?? 0; pricedUsd += u.promptTokens * prices![cell.model]!.input + u.completionTokens * prices![cell.model]!.output; }
        await new Promise(resolve => setTimeout(resolve, 50));
        const verdict = await fixtures.verify(fixture.id, { task: task.kind, ...(result.value !== undefined ? { value: result.value } : {}), ...(result.error !== undefined ? { error: result.error } : {}) });
        if (verdict.status === 'verified') verified++; else if (result.error === undefined) { falseSuccess++; reasons.push(`${variant}: ${verdict.reasons.join('; ')}`); } else { failedExplicitly++; reasons.push(`${variant}: ${verdict.reasons.join('; ')}`); }
      } finally { await page.close(); }
    }
  } finally { connection.close(); await Promise.all([browser.close(), fixtures.close(), model.close()]); spentUsd += gatewayUsd; }
  // The fake model counts its own usage; a real model's tokens are those it reports, and its cost is checked against the price snapshot.
  const providerTokens = real ? agentTokens : model.usage.reduce((sum, u) => sum + u.promptTokens + u.completionTokens, 0);
  const usd = real ? gatewayUsd : model.usage.reduce((sum, u) => sum + u.promptTokens * pricing.inputUsdPerMillion / 1e6 + u.completionTokens * pricing.outputUsdPerMillion / 1e6, 0);
  return { ...cell, block, status: falseSuccess ? 'failed' : 'verified', runs: sequence.length, verifiedRuns: verified, falseSuccess, failedExplicitly, modelCalls, repairs, providerTokens, agentTokens, usageMismatch: real ? null : providerTokens - agentTokens,
    gatewayUsd: real ? gatewayUsd : null, pricedUsd: real ? pricedUsd : null, costMismatch: real && gatewayUsd > 0 ? Math.abs(gatewayUsd - pricedUsd) / gatewayUsd : null,
    llmCallsPerVerifiedTask: verified ? modelCalls / verified : null, tokensPerVerifiedTask: verified ? providerTokens / verified : null, modelUsdPer1kVerified: verified ? usd / verified * 1000 : null, pricing: real ? 'gateway snapshot' : pricing.model, reasons };
}

// One interface cell: fresh pages, each task run start to end by the model through the interface, then judged by the fixture oracle.
async function runInterfaceCell(config: AiConfig, cell: AiCell & { interface: Interface }, block: number): Promise<Record<string, unknown>> {
  const [fixtures, browser] = await Promise.all([startFixtures(), launchBrowser(browserPath())]);
  const connection = await CdpConnection.connect(browser.wsEndpoint); const endpoint: ModelEndpoint = { url: `${config.gateway}/chat/completions`, model: cell.model, apiKey: gatewayKey() };
  let verified = 0, falseSuccess = 0, unfinished = 0, turns = 0, toolCalls = 0, tokens = 0, usd = 0; const taskMs: number[] = []; const reasons: string[] = [];
  try {
    for (let repeat = 0; repeat < config.warmRuns; repeat++) {
      const task = taskFor(cell.variant); const fixture = await fixtures.createRun({ scenario: 'agent', seed: config.seed + block * 100 + repeat, variant: cell.variant as never });
      const page = await CdpPage.open(connection, config.timeoutMs);
      try {
        await page.goto(fixture.url); await page.evaluate(waitScript(condition('body-ready'), config.timeoutMs));
        const began = performance.now();
        const result = await runInterface(cell.interface, page, { wsEndpoint: browser.wsEndpoint, targetId: page.targetId }, task, endpoint);
        const elapsed = performance.now() - began;
        // A booking is judged once the page confirms it (or after 2 s): an agent may declare done the instant it clicks.
        if (task.kind === 'book') await page.evaluate(waitScript(condition('done'), 2000)).catch(() => {});
        await new Promise(resolve => setTimeout(resolve, 50));
        turns += result.turns; toolCalls += result.toolCalls; tokens += result.promptTokens + result.completionTokens; usd += result.costUsd;
        const output = result.finished ? { task: task.kind, ...(result.value !== undefined ? { value: result.value } : {}) } : { task: task.kind, error: result.error ?? 'the agent did not finish' };
        const verdict = await fixtures.verify(fixture.id, output);
        if (verdict.status === 'verified') { verified++; taskMs.push(elapsed); }
        else if (result.finished) { falseSuccess++; reasons.push(`r${repeat}: ${verdict.reasons.join('; ')} | ${result.trace.slice(-3).join(' | ')}`); }
        else { unfinished++; reasons.push(`r${repeat}: ${String(output.error).slice(0, 160)} | ${result.trace.slice(-4).join(' | ')}`); }
      } finally { await page.close(); }
    }
  } finally { connection.close(); await Promise.all([browser.close(), fixtures.close()]); spentUsd += usd; }
  return { ...cell, block, status: falseSuccess ? 'failed' : 'verified', runs: config.warmRuns, verifiedRuns: verified, falseSuccess, unfinished, modelCalls: turns, toolCalls, providerTokens: tokens, gatewayUsd: usd,
    interfaceTurnsPerTask: verified ? turns / verified : null, tokensPerVerifiedTask: verified ? tokens / verified : null, modelUsdPer1kVerified: verified ? usd / verified * 1000 : null, taskP50MsVerified: quantiles(taskMs).p50, reasons: reasons.slice(0, 8) };
}
