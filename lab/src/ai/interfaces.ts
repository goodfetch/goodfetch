// The interface arena (D-agent-interface): one task, one model, four ways to reach the same page operations.
// sdk: the operations are function tools of the model API. mcp: the same tools listed and called through an MCP server
// in its own process. cli: a single shell tool and an `automan` command to discover. code-mode: the model writes one
// program against a typed page object, run once in a sandbox, with one repair on error.
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import { runInNewContext } from 'node:vm';
import { join } from 'node:path';
import type { CdpPage } from '../../adapters/cdp-node/cdp.ts';
import type { ModelTask } from '../../fixtures/fake-llm.ts';
import { labRoot } from '../registry.ts';
import { PageTools, toolDefinitions } from './page-tools.ts';
import { observeScript, type ModelEndpoint } from './agent.ts';

export type Interface = 'sdk' | 'mcp' | 'cli' | 'code-mode';
export type InterfaceResult = { finished: boolean; value?: string; error?: string; turns: number; toolCalls: number; promptTokens: number; completionTokens: number; costUsd: number; trace: string[] };
type Message = { role: 'system' | 'user' | 'assistant' | 'tool'; content: string | null; tool_calls?: ToolCall[]; tool_call_id?: string };
type ToolCall = { id: string; type: 'function'; function: { name: string; arguments: string } };
type FunctionTool = { name: string; description: string; parameters: unknown };

const rules = 'Never click a control that deletes or cancels anything. For a "book" task, fill the name and the email, select the plan, tick the agreement checkbox and submit the booking. For a "read-price" task, read the price of the requested plan.';
const maxTurns = 14;

async function chat(endpoint: ModelEndpoint, messages: Message[], tools: FunctionTool[] | undefined, result: InterfaceResult) {
  const response = await fetch(endpoint.url, { method: 'POST', headers: { 'content-type': 'application/json', ...(endpoint.apiKey ? { authorization: `Bearer ${endpoint.apiKey}` } : {}) },
    body: JSON.stringify({ model: endpoint.model, temperature: 0, max_tokens: 1200, messages, ...(tools ? { tools: tools.map(t => ({ type: 'function', function: t })) } : {}) }) });
  if (!response.ok) throw new Error(`model HTTP ${response.status}: ${(await response.text()).slice(0, 200)}`);
  const body = await response.json() as { choices: { message: { content: string | null; tool_calls?: ToolCall[] } }[]; usage: { prompt_tokens: number; completion_tokens: number; cost?: number } };
  result.turns++; result.promptTokens += body.usage.prompt_tokens; result.completionTokens += body.usage.completion_tokens; result.costUsd += body.usage.cost ?? 0;
  return body.choices[0]!.message;
}

// The tool loop shared by sdk, mcp and cli: the model calls tools until it calls done, stops calling tools, or runs out of turns.
async function toolLoop(endpoint: ModelEndpoint, system: string, task: ModelTask, tools: FunctionTool[], call: (name: string, args: Record<string, unknown>) => Promise<string>, finished: () => boolean, result: InterfaceResult) {
  const messages: Message[] = [{ role: 'system', content: system }, { role: 'user', content: JSON.stringify({ task }) }];
  while (result.turns < maxTurns && !finished()) {
    const message = await chat(endpoint, messages, tools, result);
    messages.push({ role: 'assistant', content: message.content ?? null, ...(message.tool_calls?.length ? { tool_calls: message.tool_calls } : {}) });
    if (!message.tool_calls?.length) { result.trace.push(`stop: ${(message.content ?? '').slice(0, 120)}`); break; }
    for (const toolCall of message.tool_calls) {
      let output: string; result.toolCalls++;
      try { output = await call(toolCall.function.name, JSON.parse(toolCall.function.arguments || '{}')); } catch (error) { output = `error: ${error instanceof Error ? error.message : String(error)}`; }
      messages.push({ role: 'tool', tool_call_id: toolCall.id, content: output });
      result.trace.push(`${toolCall.function.name} ${toolCall.function.arguments} -> ${output.slice(0, 80)}`);
    }
  }
}

class McpClient {
  private child: ChildProcessWithoutNullStreams; private next = 1; private waiting = new Map<number, (message: { result?: unknown; error?: { message: string } }) => void>();
  constructor(wsEndpoint: string, targetId: string) {
    this.child = spawn(process.execPath, [join(labRoot, 'adapters/mcp-page/server.ts'), wsEndpoint, targetId], { stdio: ['pipe', 'pipe', 'pipe'] });
    createInterface({ input: this.child.stdout }).on('line', line => { const message = JSON.parse(line) as { id: number; result?: unknown; error?: { message: string } }; this.waiting.get(message.id)?.(message); this.waiting.delete(message.id); });
  }
  request<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    const id = this.next++;
    return new Promise((resolve, reject) => { this.waiting.set(id, message => message.error ? reject(new Error(message.error.message)) : resolve(message.result as T)); this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n'); });
  }
  notify(method: string) { this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method }) + '\n'); }
  close() { this.child.stdin.end(); this.child.kill(); }
}

const help = `automan: control the current web page.
  automan observe                 list interactive elements as JSON (ref, tag, type, label, text, plan)
  automan fill <ref> <value>      type a value into a text field
  automan select <ref> <value>    choose an option value in a select
  automan click <ref>             click a button or checkbox
  automan read <ref>              print the text of an element
  automan done [value]            finish the task (give the value for a reading task)`;
// Shell words with single or double quotes, enough for the commands above.
const words = (command: string) => [...command.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map(m => m[1] ?? m[2] ?? m[3]!);

export async function runInterface(kind: Interface, page: CdpPage, browser: { wsEndpoint: string; targetId: string }, task: ModelTask, endpoint: ModelEndpoint): Promise<InterfaceResult> {
  const result: InterfaceResult = { finished: false, turns: 0, toolCalls: 0, promptTokens: 0, completionTokens: 0, costUsd: 0, trace: [] };
  const tools = new PageTools(page); let mcp: McpClient | undefined; let mcpFinished = false; let mcpValue: string | undefined;
  try {
    if (kind === 'sdk') {
      await toolLoop(endpoint, `You operate a web page for a user through tools. Observe the page first, then act. ${rules} Call done when the task is complete.`, task, toolDefinitions.map(t => ({ ...t })), (name, args) => tools.call(name, args), () => tools.finished, result);
      result.finished = tools.finished; result.value = tools.value;
    } else if (kind === 'mcp') {
      mcp = new McpClient(browser.wsEndpoint, browser.targetId);
      await mcp.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'automan-lab', version: '0.1.0' } }); mcp.notify('notifications/initialized');
      const { tools: listed } = await mcp.request<{ tools: { name: string; description: string; inputSchema: unknown }[] }>('tools/list');
      await toolLoop(endpoint, `You operate a web page for a user through the tools of an MCP server. Observe the page first, then act. ${rules} Call done when the task is complete.`, task, listed.map(t => ({ name: t.name, description: t.description, parameters: t.inputSchema })), async (name, args) => {
        const called = await mcp!.request<{ content: { text: string }[]; isError: boolean; structuredContent?: { finished: boolean; value: string | null } }>('tools/call', { name, arguments: args });
        if (called.structuredContent?.finished) { mcpFinished = true; mcpValue = called.structuredContent.value ?? undefined; }
        return (called.isError ? 'error: ' : '') + called.content.map(c => c.text).join('\n');
      }, () => mcpFinished, result);
      result.finished = mcpFinished; result.value = mcpValue;
    } else if (kind === 'cli') {
      const shell = [{ name: 'shell', description: 'Run one shell command and return its output.', parameters: { type: 'object', properties: { command: { type: 'string' } }, required: ['command'], additionalProperties: false } }];
      // One command line may chain commands with ;, && or new lines, and echo, as in a shell; anything else is not found.
      const runOne = async (command: string): Promise<string> => {
        const [program, sub, ...rest] = words(command);
        if (program === 'echo') return [sub, ...rest].filter(Boolean).join(' ');
        if (program !== 'automan') return `${program ?? ''}: command not found`;
        if (!sub || sub === '--help' || sub === 'help') return help;
        if (sub === 'observe') return tools.call('observe', {});
        if (sub === 'fill' || sub === 'select') return tools.call(sub, { ref: Number(rest[0]), value: rest.slice(1).join(' ') });
        if (sub === 'click' || sub === 'read') return tools.call(sub, { ref: Number(rest[0]) });
        if (sub === 'done') return tools.call('done', rest.length ? { value: rest.join(' ') } : {});
        return `automan: unknown command ${sub}\n${help}`;
      };
      await toolLoop(endpoint, `You operate a web page for a user with a command-line tool named automan, through the shell tool. Run "automan --help" to learn its commands. ${rules} Run "automan done" when the task is complete.`, task, shell, async (_name, args) => {
        const outputs: string[] = [];
        for (const command of String(args.command ?? '').split(/\n|;|&&/).map(c => c.trim()).filter(Boolean)) { try { outputs.push(await runOne(command)); } catch (error) { outputs.push(`error: ${error instanceof Error ? error.message : String(error)}`); break; } }
        return outputs.join('\n');
      }, () => tools.finished, result);
      result.finished = tools.finished; result.value = tools.value;
    } else {
      // Code mode: the observation comes with the request; the model answers with a function body, run once.
      const elements = (await page.evaluate<({ selector: string } & Record<string, unknown>)[]>(observeScript)).map(({ selector: _selector, ...rest }) => rest);
      await tools.call('observe', {});
      const messages: Message[] = [{ role: 'system', content: `Write the body of an async JavaScript function that completes the task with the object page: await page.fill(ref, value), await page.select(ref, value), await page.click(ref), await page.read(ref) returns a string, await page.done(value). Element refs are given. ${rules} Answer with the code only.` },
        { role: 'user', content: JSON.stringify({ task, elements }) }];
      const api = { fill: (ref: number, value: string) => tools.call('fill', { ref, value }), select: (ref: number, value: string) => tools.call('select', { ref, value }), click: (ref: number) => tools.call('click', { ref }), read: (ref: number) => tools.call('read', { ref }), done: (value?: unknown) => tools.call('done', value === undefined ? {} : { value }) };
      for (let attempt = 0; attempt < 2 && !tools.finished; attempt++) {
        const message = await chat(endpoint, messages, undefined, result);
        const code = (message.content ?? '').replace(/^```[a-z]*\n?|```\s*$/gm, '').trim();
        messages.push({ role: 'assistant', content: message.content ?? '' });
        try { await runInNewContext(`(async () => {\n${code}\n})()`, { page: api }, { timeout: 2000 }); }
        catch (error) { messages.push({ role: 'user', content: `The code failed: ${error instanceof Error ? error.message : String(error)}. Answer with corrected code only.` }); }
      }
      result.toolCalls = tools.calls - 1; result.finished = tools.finished; result.value = tools.value;
    }
  } catch (error) { result.error = error instanceof Error ? error.message : String(error); }
  finally { mcp?.close(); }
  return result;
}
