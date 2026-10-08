import {test} from 'node:test';
import assert from 'node:assert/strict';
import {runReaderExperiment} from '../../src/sqlite-readers.ts';
test('an old reader snapshot holds back checkpoints without hiding committed rows from new readers',async()=>{
  const run=await runReaderExperiment({engine:'rust-sync',writers:1,readerMode:'held',batchRows:100,batches:20,payloadBytes:512,seed:42});
  assert.equal(run.verdict.status,'verified');assert.equal(run.verdict.persistedRows,2000);assert.equal(run.oldSnapshotRows,0);
  assert.ok(Number(run.passiveCheckpoint.log)>Number(run.passiveCheckpoint.checkpointed));assert.equal(run.truncateCheckpoint.busy,0);assert.equal(run.truncateCheckpoint.log,0);assert.equal(run.truncateCheckpoint.checkpointed,0);
});
test('four independent writers can start on a fresh sink while an old reader snapshot remains open',async()=>{
  for(const engine of ['rust-sync','go-sync'] as const)for(let trial=0;trial<3;trial++){
    const run=await runReaderExperiment({engine,writers:4,readerMode:'held',batchRows:100,batches:20,payloadBytes:512,seed:42+trial});
    assert.equal(run.verdict.status,'verified');assert.equal(run.verdict.persistedRows,2000);
    assert.equal(run.oldSnapshotRows,0);assert.equal(run.walAfterTruncateBytes,0);
  }
});
