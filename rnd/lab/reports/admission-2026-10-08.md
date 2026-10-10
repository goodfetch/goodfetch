# Open admission and multi-domain scheduling (D-durable-runtime, D-scheduling), October 8, 2026

Prototype in the Rust runtime (`admission` mode of `automan-workflow-rust`), measured in `local-dev` on committed sources, on a Mac in use. Retained runs:
- `durable-admission-2026-10-08T19-43-14-860Z-b8ad5b`: without overload;
- `durable-scheduler-2026-10-08T19-43-56-509Z-dfabee`: one heavy domain in overload;
- `durable-admission-recovery-2026-10-08T19-50-26-865Z-f4c814`: crash under overload (heavy domain at 150 jobs/s), request timeout of 2 s.

Each run has 5 blocks. Within a block, both policies receive the same randomly drawn Poisson arrival schedule.

## The prototype

- **Admission.** A submission reserves a slot under the queue bound, then a writer thread journals it (SQLite in WAL, `synchronous=FULL`, shared lab build). This thread commits in one transaction everything that arrived during the previous commit. The acknowledgment is sent only after this commit. Queue full: `E_CAPACITY` refusal with a retry hint, and the job is never journaled.
- **Dispatch.** The dispatch of each job to the site is journaled before the request, in the same group commit.
- **Scheduling.** 8 workers, at most 2 simultaneous requests per domain. Two policies:
  - `fifo-global`: a global queue of 64; the dispatcher takes the oldest job whose domain has a free slot;
  - `fair-domain`: a queue of 16 per domain under the same global cap of 64; the dispatcher serves the domains in turn.
- **Load.** One heavy domain and three small domains at 15 jobs/s each, for 4 s; the site responds in 20 ms. The heavy domain sends 60 jobs/s without overload and 300 jobs/s in overload. With its limit of 2 requests, it can only process 100 jobs/s.

## The oracle

The harness and the multi-domain site observe the runtime from the outside:
- each submission receives a response;
- each acknowledged job is in the journal, completed, with the expected records, and requested from the site only once;
- each refusal carries `E_CAPACITY`, is never executed and is not journaled;
- the journal contains no unacknowledged job and passes `PRAGMA integrity_check`;
- the site records the peak of simultaneous requests per domain;
- the observed queue (acknowledged jobs that have not yet reached the site) stays under the bound. A tolerance equal to the number of workers covers the path between leaving the queue and reaching the site.

## Results

| Scenario | Policy | Verified cells | Submissions | Refusals | Small-domain acceptance (median) | Small-domain verified throughput (jobs/s, median) | p95 submission → completion, small domains (ms) | Acknowledgment p95 (ms) | Observed queue peak | Peak requests per domain |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| steady | fair-domain | 5/5 | 2086 | 0 | 1.00 | 47.8 | 26.5 | 1.83 | 9 | 2 |
| steady | fifo-global | 5/5 | 2086 | 0 | 1.00 | 47.8 | 27.6 | 1.82 | 7 | 2 |
| flood | fair-domain | 5/5 | 6851 | 4056 | 1.00 | 46.8 | 24.5 | 1.78 | 19 | 2 |
| flood | fifo-global | 5/5 | 6851 | 4426 | 0.36 | 16.8 | 23.6 | 1.60 | 65 | 2 |
| overload-crash | fair-domain | 5/5 | 3947 | 1882 | 1.00 | 42.8 | 34.6 | 1.61 | 33 | 2 |
| overload-crash | fifo-global | 5/5 | 3947 | 2351 | 0.39 | 17.5 | 24.7 | 1.13 | 65 | 2 |

- **No loss, under both policies.** All cells are verified. No domain exceeds 2 simultaneous requests, no refusal is executed, and the queue stays under its bound.
- **Under overload, the global queue fills up with jobs from the heavy domain.** The small domains then have nearly two out of three submissions refused, even though they remain very far from their capacity. The per-domain queue admits them all and refuses only the heavy domain's excess.
- **Admitted jobs wait little in both cases.** The dispatcher skips the jobs of a domain already at its limit, so the difference lies in admission, not in waiting.
- **The journaled acknowledgment stays under 2 ms at p95** (1.13 to 1.83 ms) under these conditions. The group commit absorbs simultaneous arrivals.
- **Verdict.** `pnpm lab verdict D-scheduling` compares the verified throughput of the small domains, paired by block, in the overload scenario. The result is rendered in [decisions.md](decisions.md).

## Recovery after a crash

The fault is carried by the first submission admitted after 2 s. The runtime journals it, then kills itself before responding: this is the case where a client loses the acknowledgment of a job that is nonetheless durable.

- The harness restarts the runtime on the same journal and resends, with the same identifier, each submission left without a response.
- At startup, the runtime recognizes all journaled jobs. An already-known submission receives a `duplicate` acknowledgment without a second entry, and unfinished jobs return to their queue.
- A job in flight at the time of the crash is redone once after recovery; no other job ever is.

| Policy | Jobs recovered (min–max) | Submissions resent | Duplicate acknowledgments | Jobs redone (min–max) |
| --- | --- | --- | --- | --- |
| fair-domain | 17–21 | 7 | 5 | 2–4 |
| fifo-global | 64–67 | 5 | 5 | 2–4 |

The 10 cells are verified.

**Defect found by the oracle.** The first version resumed the queue and dispatched immediately. The site then received a third simultaneous request on a domain limited to two: the dead process's request was still being served. The runtime now journals the dispatch of each job. On recovery, a slot whose request was in flight remains occupied until that request's deadline, i.e. the 2 s timeout; recovery costs the affected domains at most this timeout.

**Harness defects.**
- **Timer-based crash.** The first version killed the runtime at a fixed time. No submission was then in flight, so the idempotent resend was never exercised.
- **Asynchronous signal.** A signal that a multithreaded process sends to itself arrives asynchronously on macOS. The faulty task therefore sometimes responded before dying; it now blocks until the signal arrives.
- **Refused submission.** A submission refused for lack of space cannot carry the fault; the fault waits for the next one. The affected runs are classified `diagnostic`.

## Limitations

- Durability covers a process crash, like the rest of the runtime ([D-durable-runtime](../decisions/D-durable-runtime.json)), not a power cut.
- The sink faults in coordinator mode (`ledger.rs`) also kill the process with a self-sent signal; they do not block afterwards, which is enough in a single-threaded server but remains to be hardened.
- A job's domain is declared by the harness; the product will derive it from the URL host.
- The durations come from a `local-dev` host. The rerun in `local-controlled` is recorded in the ledger.
