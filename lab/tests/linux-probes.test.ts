import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseProcStat, parseSmapsRollup, parseCgroupBytes, parseCgroupCpuStat } from '../src/core/metrics/linux.ts';

test('/proc/<pid>/stat CPU fields are read after the parenthesized command name', () => {
  // The command name may contain spaces and parentheses; utime and stime are fields 14 and 15 in clock ticks.
  const stat = '4242 (node (worker) x) S 1 4242 4242 0 -1 4194304 1200 0 0 0 250 75 0 0 20 0 11 0 9000 1234567 3000 18446744073709551615';
  assert.deepEqual(parseProcStat(stat, 100), { pid: 4242, ppid: 1, userMs: 2500, systemMs: 750 });
});

test('smaps_rollup gives proportional anonymous and swap memory in bytes', () => {
  const rollup = ['55d0f0a00000-7ffd5f9fe000 ---p 00000000 00:00 0 [rollup]', 'Rss:              102400 kB', 'Pss:               51200 kB', 'Pss_Anon:          40960 kB', 'Pss_File:          10240 kB', 'Pss_Shmem:             0 kB', 'SwapPss:            1024 kB'].join('\n');
  assert.deepEqual(parseSmapsRollup(rollup), { pssBytes: 51200 * 1024, pssAnonBytes: 40960 * 1024, swapPssBytes: 1024 * 1024 });
});

test('cgroup v2 counters are parsed exactly', () => {
  assert.equal(parseCgroupBytes('536870912\n'), 536870912);
  assert.deepEqual(parseCgroupCpuStat('usage_usec 1500000\nuser_usec 1000000\nsystem_usec 500000\nnr_periods 0\n'), { userMs: 1000, systemMs: 500 });
  assert.throws(() => parseCgroupBytes('max\n'), /not a byte count/);
});
