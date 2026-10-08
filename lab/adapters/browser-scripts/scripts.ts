// Page scripts shared by every browser driver. The Rust CDP driver embeds the same files, so both
// languages send byte-identical expressions and differ only in how they drive the protocol.
import { readFileSync } from 'node:fs';

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), 'utf8').trim();
const templates = { deep: read('deep.js'), wait: read('wait.js'), guard: read('guard.js'), point: read('point.js'), focus: read('focus.js'), select: read('select.js') };
const conditionTemplates = JSON.parse(read('conditions.json')) as Record<string, string>;
const fill = (template: string, values: Record<string, string>) => Object.entries(values).reduce((text, [key, value]) => text.split(`__${key}__`).join(value), template);

export const recordsScript = read('records.js');
export const nextScript = read('next.js');
export const snapshotScript = read('snapshot.js');
export function condition(name: string, values: Record<string, string> = {}) {
  const template = conditionTemplates[name];
  if (template === undefined) throw new Error(`unknown condition ${name}`);
  return fill(template, values);
}
export const waitScript = (conditionText: string, timeoutMs: number) => fill(templates.wait, { TIMEOUT: String(timeoutMs), CONDITION: conditionText });
export const guardScript = (selector: string, timeoutMs: number) => fill(templates.guard, { DEEP: templates.deep, TIMEOUT: String(timeoutMs), SELECTOR: JSON.stringify(selector) });
export const pointScript = (selector: string) => fill(templates.point, { DEEP: templates.deep, SELECTOR: JSON.stringify(selector) });
export const focusScript = (selector: string) => fill(templates.focus, { DEEP: templates.deep, SELECTOR: JSON.stringify(selector) });
export const selectScript = (selector: string, value: string) => fill(templates.select, { DEEP: templates.deep, SELECTOR: JSON.stringify(selector), VALUE: JSON.stringify(value) });
