// AI-layer lane (D-ai-layer): sessions of warm runs on the unchanged page, then one run on a drifted page, for an agent
// that plans on every run and for a compiled plan replayed without the model. Model usage comes from the fake model's
// own accounting and is compared with what the agent reports.
import { startFixtures } from '../fixtures/server.ts';
import { startFakeModel, pricing, type ModelTask } from '../fixtures/fake-llm.ts';
import { launchBrowser } from './core/browser-host.ts';
import { browserPath } from './environment.ts';
import { CdpConnection, CdpPage } from '../adapters/cdp-node/cdp.ts';
import { waitScript, condition } from '../adapters/browser-scripts/scripts.ts';
import { runTask, type AgentMode, type CompiledPlan } from './ai/agent.ts';

export type AiConfig = { modes: AgentMode[]; variants: string[]; warmRuns: number; blocks: number; seed: number; timeoutMs: number };
export type AiCell = { mode: AgentMode; variant: string };
export const aiCombinations = (config: AiConfig): AiCell[] => config.modes.flatMap(mode => config.variants.map(variant => ({ mode, variant })));
const taskFor = (variant: string): ModelTask => variant === 'price-change' ? { kind: 'read-price', plan: 'pro' } : { kind: 'book', name: 'Ada Lovelace', email: 'ada@example.test', plan: 'pro' };

export async function runAiCell(config: AiConfig, cell: AiCell, block: number): Promise<Record<string, unknown>> {
  const [fixtures, model, browser] = await Promise.all([startFixtures(), startFakeModel(), launchBrowser(browserPath())]);
  const connection = await CdpConnection.connect(browser.wsEndpoint); const store = new Map<string, CompiledPlan>();
  const sequence = [...Array.from({ length: config.warmRuns }, () => 'baseline'), ...(cell.variant === 'baseline' ? [] : [cell.variant])];
  let verified = 0, falseSuccess = 0, failedExplicitly = 0, modelCalls = 0, repairs = 0, agentTokens = 0; const reasons: string[] = [];
  try {
    for (const variant of sequence) {
      const task = taskFor(cell.variant); const fixture = await fixtures.createRun({ scenario: 'agent', seed: config.seed + block, variant: variant as never });
      const page = await CdpPage.open(connection, config.timeoutMs);
      try {
        await page.goto(fixture.url); await page.evaluate(waitScript(condition('body-ready'), config.timeoutMs));
        const result = await runTask(page, cell.mode, task, store, model.url, config.timeoutMs);
        modelCalls += result.modelCalls; repairs += result.repairs; agentTokens += result.usage.reduce((sum, u) => sum + u.promptTokens + u.completionTokens, 0);
        await new Promise(resolve => setTimeout(resolve, 50));
        const verdict = await fixtures.verify(fixture.id, { task: task.kind, ...(result.value !== undefined ? { value: result.value } : {}), ...(result.error !== undefined ? { error: result.error } : {}) });
        if (verdict.status === 'verified') verified++; else if (result.error === undefined) { falseSuccess++; reasons.push(`${variant}: ${verdict.reasons.join('; ')}`); } else { failedExplicitly++; reasons.push(`${variant}: ${verdict.reasons.join('; ')}`); }
      } finally { await page.close(); }
    }
  } finally { connection.close(); await Promise.all([browser.close(), fixtures.close(), model.close()]); }
  const providerTokens = model.usage.reduce((sum, u) => sum + u.promptTokens + u.completionTokens, 0);
  const usd = model.usage.reduce((sum, u) => sum + u.promptTokens * pricing.inputUsdPerMillion / 1e6 + u.completionTokens * pricing.outputUsdPerMillion / 1e6, 0);
  return { ...cell, block, status: 'verified', runs: sequence.length, verifiedRuns: verified, falseSuccess, failedExplicitly, modelCalls, repairs, providerTokens, agentTokens, usageMismatch: providerTokens - agentTokens,
    llmCallsPerVerifiedTask: verified ? modelCalls / verified : null, tokensPerVerifiedTask: verified ? providerTokens / verified : null, modelUsdPer1kVerified: verified ? usd / verified * 1000 : null, pricing: pricing.model, reasons };
}
