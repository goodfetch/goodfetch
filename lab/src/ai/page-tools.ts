// The page surface every agent interface exposes (D-agent-interface): observe, fill, select, click, read and done, on one
// browser page. Function calling, the MCP server, the CLI and code mode all call these same operations.
import type { CdpPage } from '../../adapters/cdp-node/cdp.ts';
import { observeScript } from './agent.ts';

export const toolDefinitions = [
  { name: 'observe', description: 'List the interactive elements of the page (numeric ref, tag, type, label, text, plan) and the status messages the page shows.', parameters: { type: 'object', properties: {}, additionalProperties: false } },
  { name: 'fill', description: 'Type a value into the text field with this ref.', parameters: { type: 'object', properties: { ref: { type: 'integer' }, value: { type: 'string' } }, required: ['ref', 'value'], additionalProperties: false } },
  { name: 'select', description: 'Choose an option value in the select element with this ref.', parameters: { type: 'object', properties: { ref: { type: 'integer' }, value: { type: 'string' } }, required: ['ref', 'value'], additionalProperties: false } },
  { name: 'click', description: 'Click the element with this ref (buttons, checkboxes).', parameters: { type: 'object', properties: { ref: { type: 'integer' } }, required: ['ref'], additionalProperties: false } },
  { name: 'read', description: 'Return the text of the element with this ref.', parameters: { type: 'object', properties: { ref: { type: 'integer' } }, required: ['ref'], additionalProperties: false } },
  { name: 'done', description: 'Finish the task. For a reading task, give the value read.', parameters: { type: 'object', properties: { value: { type: 'string' } }, additionalProperties: false } }
] as const;

export class PageTools {
  finished = false; value: string | undefined; calls = 0;
  private refs = new Map<number, string>(); private page: CdpPage;
  constructor(page: CdpPage) { this.page = page; }
  private selector(ref: unknown) { const selector = this.refs.get(Number(ref)); if (!selector) throw new Error(`unknown ref ${String(ref)}: call observe first`); return selector; }
  // Every result is text, as a tool result is for the model.
  async call(name: string, args: Record<string, unknown>): Promise<string> {
    this.calls++;
    if (name === 'observe') {
      const elements = await this.page.evaluate<({ ref: number; selector: string } & Record<string, unknown>)[]>(observeScript);
      this.refs = new Map(elements.map(e => [e.ref, e.selector]));
      // Status messages (output, role=status, aria-live) let an agent see the outcome of what it did.
      const status = await this.page.evaluate<string[]>(`Array.from(document.querySelectorAll('output,[role=status],[aria-live]'),el=>el.textContent.trim()).filter(Boolean)`);
      return JSON.stringify({ elements: elements.map(({ selector: _selector, ...rest }) => rest), status });
    }
    if (name === 'fill') { await this.page.fill(this.selector(args.ref), String(args.value ?? '')); return 'ok'; }
    if (name === 'select') { await this.page.select(this.selector(args.ref), String(args.value ?? '')); return 'ok'; }
    if (name === 'click') { await this.page.click(this.selector(args.ref)); return 'ok'; }
    if (name === 'read') return await this.page.evaluate<string>(`document.querySelector(${JSON.stringify(this.selector(args.ref))}).textContent.trim()`);
    if (name === 'done') { this.finished = true; this.value = args.value === undefined ? undefined : String(args.value); return 'finished'; }
    throw new Error(`unknown tool ${name}`);
  }
}
