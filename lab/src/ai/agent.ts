// The AI layer prototype for D-ai-layer: an agent that plans with a model on every run, and a compiled plan that is
// replayed without the model. A replayed step first checks the fingerprint of its target (tag, type, label, text,
// plan); any mismatch is drift, answered by one bounded repair through the model, never by acting on a changed target.
import type { CdpPage } from '../../adapters/cdp-node/cdp.ts';
import { waitScript, condition } from '../../adapters/browser-scripts/scripts.ts';
import type { ModelTask, Observed, Usage } from '../../fixtures/fake-llm.ts';

type Element = Observed & { selector: string };
type Fingerprint = Omit<Observed, 'ref'>;
type Step = { action: 'fill' | 'select' | 'click' | 'extract'; selector: string; value?: string; fingerprint: Fingerprint };
export type CompiledPlan = { steps: Step[] };
export type AgentMode = 'agent-each-run' | 'compiled-replay';
export type TaskResult = { value?: string; error?: string; modelCalls: number; usage: Usage[]; repairs: number };

const describe = `const describe=el=>({tag:el.tagName.toLowerCase(),type:el.type||'',label:(el.closest('label')?.textContent||el.getAttribute('aria-label')||'').trim().replace(/\\s+/g,' '),text:el.tagName==='BUTTON'?el.textContent.trim():'',plan:el.dataset.plan??null});`;
const observeScript = `(()=>{${describe}return Array.from(document.querySelectorAll('input,select,button,[data-plan]'),(el,ref)=>({ref,selector:el.id?'#'+CSS.escape(el.id):el.dataset.plan?'[data-plan="'+el.dataset.plan+'"]':el.name?el.tagName.toLowerCase()+'[name="'+el.name+'"]':null,...describe(el)})).filter(e=>e.selector)})()`;
const fingerprintScript = (selector: string) => `(()=>{${describe}const el=document.querySelector(${JSON.stringify(selector)});return el?describe(el):null})()`;
const same = (a: Fingerprint | null, b: Fingerprint) => !!a && a.tag === b.tag && a.type === b.type && a.label === b.label && a.text === b.text && a.plan === b.plan;

async function ask(modelUrl: string, task: ModelTask, observation: Observed[]) {
  const response = await fetch(modelUrl, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ model: 'fake-planner-2026-10-08', messages: [{ role: 'system', content: 'Plan the task with element references from the observation. Answer in JSON.' }, { role: 'user', content: JSON.stringify({ task, observation }) }] }) });
  if (!response.ok) throw new Error(`model HTTP ${response.status}`);
  const body = await response.json() as { choices: { message: { content: string } }[]; usage: { prompt_tokens: number; completion_tokens: number } };
  return { answer: JSON.parse(body.choices[0]!.message.content) as { steps?: { action: string; ref: number | null; value?: string }[]; extract?: number | null }, usage: { promptTokens: body.usage.prompt_tokens, completionTokens: body.usage.completion_tokens } };
}

// Observe, ask the model once, and compile its answer into selectors with the fingerprints seen now.
async function compile(page: CdpPage, modelUrl: string, task: ModelTask, result: TaskResult): Promise<CompiledPlan> {
  const elements = await page.evaluate<Element[]>(observeScript);
  const { answer, usage } = await ask(modelUrl, task, elements.map(({ selector: _selector, ...rest }) => rest)); result.modelCalls++; result.usage.push(usage);
  const at = (ref: number | null | undefined) => { const element = elements.find(e => e.ref === ref); if (!element) throw new Error('model answered with no usable element'); const { ref: _ref, selector, ...fingerprint } = element; return { selector, fingerprint }; };
  if (task.kind === 'read-price') return { steps: [{ action: 'extract', ...at(answer.extract) }] };
  return { steps: (answer.steps ?? []).map(step => ({ action: step.action as Step['action'], value: step.value, ...at(step.ref) })) };
}

async function execute(page: CdpPage, plan: CompiledPlan, checkFingerprints: boolean, timeoutMs: number) {
  let value: string | undefined;
  // The whole plan is checked against the page before the first effect: drift found midway would leave partial,
  // non-idempotent effects (a toggled checkbox) for the repair to replay.
  if (checkFingerprints) for (const step of plan.steps) if (!same(await page.evaluate<Fingerprint | null>(fingerprintScript(step.selector)), step.fingerprint)) return { drift: step.selector };
  for (const step of plan.steps) {
    if (step.action === 'fill') await page.fill(step.selector, step.value!);
    else if (step.action === 'select') await page.select(step.selector, step.value!);
    else if (step.action === 'click') await page.click(step.selector);
    else value = await page.evaluate<string>(`document.querySelector(${JSON.stringify(step.selector)}).textContent.trim()`);
  }
  if (plan.steps.at(-1)?.action === 'click') await page.evaluate(waitScript(condition('done'), timeoutMs));
  return { value };
}

export async function runTask(page: CdpPage, mode: AgentMode, task: ModelTask, store: Map<string, CompiledPlan>, modelUrl: string, timeoutMs: number): Promise<TaskResult> {
  const result: TaskResult = { modelCalls: 0, usage: [], repairs: 0 };
  try {
    if (mode === 'agent-each-run') { const outcome = await execute(page, await compile(page, modelUrl, task, result), false, timeoutMs); return { ...result, value: outcome.value }; }
    let plan = store.get(task.kind);
    if (!plan) { plan = await compile(page, modelUrl, task, result); store.set(task.kind, plan); }
    let outcome = await execute(page, plan, true, timeoutMs);
    if ('drift' in outcome) {
      // One bounded repair: recompile from a fresh observation and retry once; a second drift is an explicit failure.
      result.repairs++; plan = await compile(page, modelUrl, task, result); store.set(task.kind, plan);
      outcome = await execute(page, plan, true, timeoutMs);
      if ('drift' in outcome) throw new Error(`drift persists after repair at ${outcome.drift}`);
    }
    return { ...result, value: outcome.value };
  } catch (error) { return { ...result, error: error instanceof Error ? error.message : String(error) }; }
}
