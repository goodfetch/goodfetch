// In-process Node <-> Rust (napi-rs) microbenchmark.
const path = require('node:path');
const addon = require(path.join(__dirname, 'napi_bench.node'));

function makeHtml(len) {
  const unit = '<div class="item" data-id="42"><a href="/p?q=1&r=2">Item élève</a></div>\n';
  let s = '';
  while (Buffer.byteLength(s) < len) s += unit;
  return s.slice(0, len);
}

function timeSync(label, n, fn) {
  for (let i = 0; i < Math.min(n, 10000); i++) fn(i); // warmup
  const t0 = process.hrtime.bigint();
  for (let i = 0; i < n; i++) fn(i);
  const t1 = process.hrtime.bigint();
  console.log(JSON.stringify({ bench: label, n, mean_ns: Number(t1 - t0) / n }));
}

async function timeAsync(label, n, fn) {
  for (let i = 0; i < Math.min(n, 2000); i++) await fn(i);
  const samples = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const t0 = process.hrtime.bigint();
    await fn(i);
    samples[i] = Number(process.hrtime.bigint() - t0);
  }
  samples.sort();
  const mean = samples.reduce((a, b) => a + b, 0) / n;
  console.log(JSON.stringify({ bench: label, n, mean_ns: mean, p50_ns: samples[n >> 1], p99_ns: samples[Math.floor(n * 0.99)] }));
}

// A trivial async "page handler": parses nothing, returns a small JSON string.
async function handler(html) {
  return JSON.stringify({ n: html.length });
}

(async () => {
  console.log(JSON.stringify({ node: process.version, arch: process.arch }));
  timeSync('js_noop_baseline', 5_000_000, (i) => i + 1);
  timeSync('napi_sync_noop', 5_000_000, (i) => addon.noop(i));
  for (const size of [64, 100_000, 1_000_000]) {
    const s = makeHtml(size);
    const b = Buffer.from(s);
    const n = size >= 1_000_000 ? 2000 : size >= 100_000 ? 20000 : 1_000_000;
    timeSync(`napi_take_string_${size}`, n, () => addon.takeString(s));
    timeSync(`napi_take_buffer_${size}`, n, () => addon.takeBuffer(b));
  }
  await timeAsync('js_await_async_fn_baseline', 200_000, (i) => handler('x'));
  await timeAsync('napi_async_fn_tokio_roundtrip', 200_000, (i) => addon.noopAsync(i));
  for (const size of [64, 100_000, 1_000_000]) {
    const iters = size >= 1_000_000 ? 2000 : size >= 100_000 ? 10000 : 50000;
    await addon.benchHandler(handler, size, 500); // warmup
    const st = await addon.benchHandler(handler, size, iters);
    console.log(JSON.stringify({ bench: `rust_calls_js_handler_seq_${size}`, ...st }));
  }
  for (const size of [64, 100_000]) {
    const iters = size >= 100_000 ? 20000 : 200000;
    const rps = await addon.benchHandlerConcurrent(handler, size, iters, 16);
    console.log(JSON.stringify({ bench: `rust_calls_js_handler_c16_${size}`, calls_per_s: rps }));
  }
})();
