import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveAvailability, measureMetric } from '../src/core/metrics/registry.ts';
import { loadRegistry } from '../src/core/decisions.ts';

const { metrics } = loadRegistry();

test('a metric without a probe on this platform is unavailable with a reason', () => {
  const availability = resolveAvailability(metrics, { platform: 'darwin', probes: { 'proc_pid_rusage-v4': true } });
  assert.equal(availability['footprint-peak-bytes']!.available, true);
  assert.equal(availability['pss-peak-bytes']!.available, false);
  assert.match(availability['pss-peak-bytes']!.reason!, /darwin/);
  assert.equal(availability['usd-per-1k-verified']!.available, false);
});

test('a platform probe that is not installed makes its metrics unavailable', () => {
  const availability = resolveAvailability(metrics, { platform: 'darwin', probes: { 'proc_pid_rusage-v4': false } });
  assert.equal(availability['cpu-ms-per-verified-job']!.available, false);
  assert.match(availability['cpu-ms-per-verified-job']!.reason!, /proc_pid_rusage-v4/);
});

test('an unavailable metric is recorded as null, never as zero', () => {
  const availability = resolveAvailability(metrics, { platform: 'darwin', probes: {} });
  assert.deepEqual(measureMetric(availability, metrics, 'pss-peak-bytes', 0, 'probe'), { value: null, unit: 'B', source: 'unavailable: no probe on darwin' });
  assert.equal(measureMetric(availability, metrics, 'task-ms', 12, 'harness').value, 12);
  assert.throws(() => measureMetric(availability, metrics, 'not-a-metric', 1, 'harness'), /unknown metric/);
});

test('on Linux, a probe whose producer is not wired stays unavailable even when the kernel interface exists', () => {
  const availability = resolveAvailability(metrics, { platform: 'linux', probes: { 'smaps_rollup': true, 'proc-stat': true, 'cgroup-v2-memory-peak': true } });
  assert.equal(availability['pss-peak-bytes']!.available, true);
  assert.equal(availability['cpu-ms-per-verified-job']!.available, true);
  assert.equal(availability['cgroup-memory-peak-bytes']!.available, false);
  assert.match(availability['cgroup-memory-peak-bytes']!.reason!, /per-run systemd scope/);
});
