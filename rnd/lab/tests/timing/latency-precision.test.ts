// Timer-precision claims, run by the local CI in their own serial step: inside the parallel unit suite, a dozen test
// processes start at once and their startup burst delays the proxy's wake-ups by milliseconds, more than the 2 ms
// tolerance being checked. The parallel suite keeps the load-independent half (tests/network.test.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { calibrate } from '../../src/net/latency-proxy.ts';

test('a latency profile is calibrated within its tolerance', async () => {
  // 20 ms is the tightest case: its 10 % tolerance equals the 2 ms floor, so timer lateness on both directions shows.
  for (const rttMs of [0, 20, 40]) {
    const calibration = await calibrate(rttMs, 20);
    assert.equal(calibration.withinTolerance, true, `rtt ${rttMs}: added ${calibration.addedMedianMs} over a direct ${calibration.baselineMedianMs}`);
  }
});
