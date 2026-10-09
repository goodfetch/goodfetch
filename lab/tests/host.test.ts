import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classifyProcesses, assessHost, loadHostClasses, type HostSnapshot } from '../src/core/host.ts';

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

test('the agent app that drives the run is reported apart from foreign CPU', () => {
  const rows = [
    { pid: 10, ppid: 1, pgid: 10, cpu: 20, command: '/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper' },
    { pid: 11, ppid: 1, pgid: 11, cpu: 7, command: 'Claude Helper (Renderer)' },
    { pid: 12, ppid: 1, pgid: 12, cpu: 3, command: '/Users/x/.local/bin/claude' },
    { pid: 13, ppid: 1, pgid: 13, cpu: 75, command: '/System/Library/Frameworks/CoreServices.framework/mds_stores' },
    { pid: 14, ppid: 1, pgid: 14, cpu: 40, command: 'node' }, { pid: 15, ppid: 14, pgid: 14, cpu: 90, command: 'chrome' }
  ];
  const split = classifyProcesses(rows, [14]);
  assert.equal(split.controllerCpuPercent, 30);
  assert.equal(split.foreignCpuPercent, 75);
  assert.deepEqual(split.foreignTop.map(row => row.command), ['mds_stores']);
});

test('a measured block is judged on foreign CPU, not on the load average its own run raises', async () => {
  const { blockNoisy } = await import('../src/core/run-context.ts');
  const gates = [{ id: 'load-per-cpu-1m', max: 0.35 }, { id: 'foreign-cpu-percent', max: 150 }];
  assert.equal(blockNoisy(gates, { foreignCpuPercent: 120 }), false, 'own load may push the load average past 0.35');
  assert.equal(blockNoisy(gates, { foreignCpuPercent: 160 }), true);
  assert.equal(blockNoisy(gates, { foreignCpuPercent: null }), true, 'an unmeasured block is not assumed quiet');
});
