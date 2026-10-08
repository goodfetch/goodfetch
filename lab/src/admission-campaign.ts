// Admission lane (D-durable-runtime, D-scheduling): open Poisson arrivals over several domains into the Rust runtime's admission
// mode. Each submission is acknowledged once journaled or rejected explicitly; the oracle checks that nothing is lost or run twice,
// that the queue bound and every domain's concurrency limit hold, and measures what light domains get beside a heavy one.
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { WorkerClient } from './worker-client.ts';
import { labRoot } from './registry.ts';
import { random, quantiles } from './statistics.ts';
import { startDomainSite, expectedRecords } from '../fixtures/domain-site.ts';

export type AdmissionPolicy = 'fifo-global' | 'fair-domain';
export type AdmissionConfig = { policies: AdmissionPolicy[]; scenario: string; domains: { name: string; ratePerSecond: number; light: boolean }[]; durationMs: number; serviceMs: number; workers: number; domainLimit: number; capacity: number; domainCapacity: number; batchMax: number; blocks: number; seed: number; timeoutMs: number; crashAtMs?: number };
export type AdmissionCell = { policy: AdmissionPolicy; scenario: string; fault: 'none' | 'crash' };
export const admissionCombinations = (config: AdmissionConfig): AdmissionCell[] => config.policies.map(policy => ({ policy, scenario: config.scenario, fault: config.crashAtMs ? 'crash' : 'none' }));
export const admissionBinary = () => join(labRoot, '.native/automan-workflow-rust');
type Submission = { jobId: string; domain: string; sentAt: number; respondedAt: number | null; admitted: boolean | null; code: string | null; attempts: number; duplicate?: boolean; error?: string };
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

// The arrival schedule depends only on the seed and the block, so both policies see the same arrivals in a block.
export function arrivals(config: AdmissionConfig, block: number) {
  const rng = random(config.seed + block * 7919); const schedule: { at: number; domain: string }[] = [];
  for (const domain of config.domains) for (let at = -Math.log(1 - rng()) / domain.ratePerSecond * 1000; at < config.durationMs; at += -Math.log(1 - rng()) / domain.ratePerSecond * 1000) schedule.push({ at, domain: domain.name });
  return schedule.sort((a, b) => a.at - b.at);
}

export async function runAdmissionCell(config: AdmissionConfig, cell: AdmissionCell, block: number, directory: string): Promise<Record<string, unknown>> {
  const site = await startDomainSite(config.serviceMs); const cellDir = join(directory, `admission-${cell.policy}-b${block}`); await mkdir(cellDir, { recursive: true });
  const workers: WorkerClient[] = []; let worker: WorkerClient; let recovered = 0;
  const open = async () => {
    const client = new WorkerClient({ command: admissionBinary(), args: ['admission'], cwd: labRoot, timeoutMs: config.timeoutMs }); workers.push(client);
    await client.call('hello', {});
    const ready = await client.call('prepare', { path: join(cellDir, 'journal.db'), policy: cell.policy, capacity: config.capacity, domainCapacity: config.domainCapacity, workers: config.workers, domainLimit: config.domainLimit, timeoutMs: config.timeoutMs, batchMax: config.batchMax }) as { recovered: number };
    return { client, recovered: ready.recovered };
  };
  const submissions: Submission[] = []; const reasons: string[] = [];
  let restarting: Promise<void> | null = null; let crashedAt: number | null = null; let crashArmed = Boolean(config.crashAtMs); let start = 0;
  try {
    ({ client: worker } = await open());
    // A submission whose answer was lost in the crash is sent again with the same job id once the runtime is back.
    const submit = async (submission: Submission) => {
      for (;;) {
        if (restarting) await restarting;
        const used = worker; submission.attempts++;
        // The first submission after crashAtMs carries the fault: the runtime dies after journaling it, before answering.
        const fault = crashArmed && performance.now() - start >= config.crashAtMs! ? 'after-commit' : undefined; if (fault) crashArmed = false;
        try {
          const r = await used.call('run', { op: 'submit', job: { jobId: submission.jobId, domain: submission.domain, url: site.url(submission.domain, submission.jobId) }, ...(fault ? { fault } : {}) }) as { admitted: boolean; code?: string; duplicate?: boolean };
          // A refused submission is never journaled, so it cannot crash the runtime: the fault waits for the next one.
          if (fault && !r.admitted) crashArmed = true;
          submission.respondedAt = performance.now(); submission.admitted = r.admitted; submission.code = r.code ?? null; submission.duplicate = r.duplicate === true; delete submission.error; return;
        } catch (error) {
          submission.error = String(error);
          if (config.crashAtMs && crashedAt === null && used.isClosed) { crashedAt = performance.now(); restarting = (async () => { await used.close().catch(() => {}); ({ client: worker, recovered } = await open()); })().finally(() => { restarting = null; }); }
          if (crashedAt === null || submission.attempts > 3) return;
        }
      }
    };
    const schedule = arrivals(config, block); const pending: Promise<void>[] = []; start = performance.now();
    for (const [index, arrival] of schedule.entries()) {
      const wait = start + arrival.at - performance.now(); if (wait > 1) await sleep(wait);
      const submission: Submission = { jobId: `j${block}-${index}`, domain: arrival.domain, sentAt: performance.now(), respondedAt: null, admitted: null, code: null, attempts: 0 }; submissions.push(submission);
      pending.push(submit(submission));
    }
    await Promise.all(pending); if (restarting) await restarting;
    const runtime = await worker.call('run', { op: 'drain', timeoutMs: 60_000 }, 65_000) as { peakQueued: number; admitted: number; rejected: number; done: number; failed: number; duplicates: number };
    const snapshot = await worker.call('run', { op: 'snapshot' }) as { jobs: { jobId: string; domain: string; state: string; records: { id: string; price: number }[] }[]; integrity: { integrity_check: string }[] };
    await worker.call('close', {}).catch(() => {});

    // No loss: every submission answered, every acknowledged job journaled, run once and right, every rejection explicit and never run.
    const journal = new Map(snapshot.jobs.map(job => [job.jobId, job]));
    if (snapshot.integrity[0]?.integrity_check !== 'ok') reasons.push('journal integrity check failed');
    for (const s of submissions) {
      if (s.error || s.admitted === null) { reasons.push(`submission ${s.jobId} lost: ${s.error ?? 'no answer'}`); continue; }
      const fetched = site.fetches.get(s.jobId);
      if (!s.admitted) { if (s.code !== 'E_CAPACITY') reasons.push(`${s.jobId} rejected without the capacity code (${s.code})`); if (fetched) reasons.push(`rejected job ${s.jobId} was executed`); if (journal.has(s.jobId)) reasons.push(`rejected job ${s.jobId} is in the journal`); continue; }
      const job = journal.get(s.jobId); const expected = expectedRecords(s.jobId);
      if (!job) reasons.push(`acknowledged job ${s.jobId} missing from the journal`);
      else if (job.state !== 'done' || JSON.stringify(job.records) !== JSON.stringify(expected)) reasons.push(`job ${s.jobId} ${job.state} with wrong or missing records`);
      // A job running when the runtime died is run again after recovery; any other job runs exactly once.
      const inFlightAtCrash = crashedAt !== null && fetched !== undefined && fetched.startedAt < crashedAt;
      if (!fetched || fetched.count > (inFlightAtCrash ? 2 : 1)) reasons.push(`job ${s.jobId} fetched ${fetched?.count ?? 0} times`);
    }
    for (const id of journal.keys()) if (!submissions.some(s => s.jobId === id && s.admitted)) reasons.push(`journal holds ${id}, never acknowledged`);
    // Politeness and bounds, observed from outside: requests in flight per domain, and jobs acknowledged but not yet started.
    for (const [domain, peak] of site.peak) if (peak > config.domainLimit) reasons.push(`${domain}: ${peak} requests in flight, limit ${config.domainLimit}`);
    const events = submissions.filter(s => s.admitted).flatMap(s => [{ at: s.respondedAt!, d: 1, domain: s.domain }, { at: site.fetches.get(s.jobId)?.startedAt ?? Infinity, d: -1, domain: s.domain }]).sort((a, b) => a.at - b.at || a.d - b.d);
    let queued = 0, peakQueued = 0; const byDomain = new Map<string, number>(); let peakDomainQueued = 0;
    for (const e of events) { queued += e.d; peakQueued = Math.max(peakQueued, queued); const q = (byDomain.get(e.domain) ?? 0) + e.d; byDomain.set(e.domain, q); peakDomainQueued = Math.max(peakDomainQueued, q); }
    // A job leaves the queue when dispatched and reaches the site a moment later, so up to `workers` jobs can be in transit.
    if (peakQueued > config.capacity + config.workers) reasons.push(`${peakQueued} jobs waiting, bound ${config.capacity}`);
    if (config.crashAtMs && crashedAt === null) reasons.push('the planned crash did not happen');
    if (runtime.peakQueued > config.capacity) reasons.push(`runtime reports ${runtime.peakQueued} queued, bound ${config.capacity}`);
    if (cell.policy === 'fair-domain' && peakDomainQueued > config.domainCapacity + config.domainLimit) reasons.push(`${peakDomainQueued} jobs waiting for one domain, bound ${config.domainCapacity}`);

    const admitted = submissions.filter(s => s.admitted); const light = new Set(config.domains.filter(d => d.light).map(d => d.name));
    const lightOffered = submissions.filter(s => light.has(s.domain)); const lightAdmitted = lightOffered.filter(s => s.admitted);
    const verified = reasons.length ? 0 : admitted.length; const lastEnd = Math.max(...admitted.map(s => site.fetches.get(s.jobId)?.endedAt ?? 0)); const workSeconds = (lastEnd - start) / 1000;
    return { ...cell, block, status: reasons.length ? 'failed' : 'verified', offered: submissions.length, admitted: admitted.length, rejected: submissions.length - admitted.length, verifiedJobs: verified,
      ackP95Ms: quantiles(admitted.map(s => s.respondedAt! - s.sentAt)).p95, ackP50Ms: quantiles(admitted.map(s => s.respondedAt! - s.sentAt)).p50,
      lightAcceptance: lightOffered.length ? lightAdmitted.length / lightOffered.length : null, lightGoodput: reasons.length ? null : lightAdmitted.length / (config.durationMs / 1000),
      lightP95Ms: quantiles(lightAdmitted.map(s => (site.fetches.get(s.jobId)?.endedAt ?? NaN) - s.sentAt)).p95,
      heavyRejection: submissions.filter(s => !light.has(s.domain)).length ? submissions.filter(s => !light.has(s.domain) && !s.admitted).length / submissions.filter(s => !light.has(s.domain)).length : null,
      verifiedJobsPerSecond: verified && workSeconds > 0 ? verified / workSeconds : null, peakInflight: Object.fromEntries(site.peak), peakQueuedObserved: peakQueued, runtime, recovered, resubmitted: submissions.filter(s => s.attempts > 1).length, duplicateAcks: submissions.filter(s => s.duplicate).length, reexecuted: [...site.fetches.values()].filter(f => f.count > 1).length, reasons: reasons.slice(0, 12) };
  } finally { for (const client of workers) await client.close().catch(() => {}); await site.close(); }
}
