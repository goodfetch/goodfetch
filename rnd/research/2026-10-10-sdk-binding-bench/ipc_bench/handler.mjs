// Node handler host: newline-delimited JSON-RPC 2.0 on stdin/stdout.
async function handler(params) {
  return { n: params.html.length };
}

// Accumulate chunks without re-copying; only scan each new chunk for '\n'.
let parts = [];
process.stdin.on('data', (chunk) => {
  let start = 0;
  let idx;
  while ((idx = chunk.indexOf(10, start)) >= 0) {
    parts.push(chunk.subarray(start, idx));
    const line = parts.length === 1 ? parts[0] : Buffer.concat(parts);
    parts = [];
    start = idx + 1;
    const msg = JSON.parse(line.toString('utf8'));
    handler(msg.params).then((result) => {
      process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result }) + '\n');
    });
  }
  if (start < chunk.length) parts.push(chunk.subarray(start));
});
