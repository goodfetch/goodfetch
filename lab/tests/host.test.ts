import { test } from 'node:test';
import assert from 'node:assert/strict';
import { assessHost, loadHostClasses, type HostSnapshot } from '../src/core/host.ts';

const quietMac: HostSnapshot = {
  os: 'darwin', arch: 'arm64', cpu: 'Apple M3 Max', logicalCpus: 14, memoryBytes: 36e9, loadAverage1m: 1.2, memoryFreePercent: 60,
  thermalWarning: false, onAC: true, lowPowerMode: false, foreignCpuPercent: 8, foreignTop: [], cpuGovernor: null, virtualized: false
};
// The host recorded during runtime-http-backends: load average 25 on 14 CPUs, ffmpeg and virtualization running.
const loadedMac: HostSnapshot = { ...quietMac, loadAverage1m: 25.14, memoryFreePercent: 34, foreignCpuPercent: 310, foreignTop: [{ command: 'ffmpeg', cpuPercent: 280 }] };
const classes = loadHostClasses();

test('a quiet, clean macOS host is granted local-controlled', () => {
  const assessment = assessHost({ snapshot: quietMac, requested: 'local-controlled', allowDowngrade: false, gitClean: true, classes });
  assert.equal(assessment.refused, false);
  assert.equal(assessment.classGranted, 'local-controlled');
});

test('a loaded host is refused for local-controlled, or explicitly downgraded when allowed', () => {
  const refused = assessHost({ snapshot: loadedMac, requested: 'local-controlled', allowDowngrade: false, gitClean: true, classes });
  assert.equal(refused.refused, true);
  assert.equal(refused.classGranted, null);
  assert.deepEqual(refused.gates.filter(gate => gate.class === 'local-controlled' && !gate.ok).map(gate => gate.id).sort(), ['foreign-cpu-percent', 'load-per-cpu-1m']);
  const downgraded = assessHost({ snapshot: loadedMac, requested: 'local-controlled', allowDowngrade: true, gitClean: true, classes });
  assert.equal(downgraded.refused, false);
  assert.equal(downgraded.classGranted, 'local-dev');
});

test('a macOS host can never be granted the reference class', () => {
  const assessment = assessHost({ snapshot: quietMac, requested: 'reference', allowDowngrade: true, gitClean: true, classes });
  assert.equal(assessment.classGranted, 'local-controlled');
  assert.ok(assessment.gates.some(gate => gate.class === 'reference' && gate.id === 'os' && !gate.ok));
});

test('uncommitted lab sources cap any host at local-dev', () => {
  const assessment = assessHost({ snapshot: quietMac, requested: 'local-controlled', allowDowngrade: true, gitClean: false, classes });
  assert.equal(assessment.classGranted, 'local-dev');
});

test('an unmeasurable gate fails instead of passing silently', () => {
  const assessment = assessHost({ snapshot: { ...quietMac, memoryFreePercent: null }, requested: 'local-controlled', allowDowngrade: false, gitClean: true, classes });
  assert.equal(assessment.refused, true);
  assert.ok(assessment.gates.some(gate => gate.id === 'memory-free-percent' && !gate.ok && gate.value === null));
});
