// A minimal MCP server over stdio (JSON-RPC 2.0, protocol 2025-06-18) exposing the page tools of one browser page. It attaches
// to the page through the browser's DevTools endpoint given on the command line, as an Automan MCP server would.
import { createInterface } from 'node:readline';
import { CdpConnection, CdpPage } from '../cdp-node/cdp.ts';
import { PageTools, toolDefinitions } from '../../src/ai/page-tools.ts';

const [wsEndpoint, targetId] = process.argv.slice(2);
const connection = await CdpConnection.connect(wsEndpoint!);
const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true }) as { sessionId: string };
const tools = new PageTools(new CdpPage(connection, targetId!, sessionId, 10_000));
const reply = (id: unknown, body: Record<string, unknown>) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, ...body }) + '\n');

for await (const line of createInterface({ input: process.stdin })) {
  const message = JSON.parse(line) as { id?: unknown; method: string; params?: Record<string, unknown> };
  if (message.id === undefined) continue; // notifications need no answer
  if (message.method === 'initialize') reply(message.id, { result: { protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'automan-page', version: '0.1.0' } } });
  else if (message.method === 'tools/list') reply(message.id, { result: { tools: toolDefinitions.map(t => ({ name: t.name, description: t.description, inputSchema: t.parameters })) } });
  else if (message.method === 'tools/call') {
    const { name, arguments: args } = message.params as { name: string; arguments?: Record<string, unknown> };
    try { const text = await tools.call(name, args ?? {}); reply(message.id, { result: { content: [{ type: 'text', text }], isError: false, ...(name === 'done' ? { structuredContent: { finished: true, value: tools.value ?? null } } : {}) } }); }
    catch (error) { reply(message.id, { result: { content: [{ type: 'text', text: error instanceof Error ? error.message : String(error) }], isError: true } }); }
  } else reply(message.id, { error: { code: -32601, message: `unknown method ${message.method}` } });
}
connection.close();
