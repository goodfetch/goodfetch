# HTTP incidents of October 8, 2026

Contract matrix of D-http-transport, executed in `local-dev` on committed sources. Retained run: `http-incidents-2026-10-08T17-25-00-258Z-206e01` (it replaces `http-incidents-2026-10-08T17-07-35-305Z-73011e`, same results for the first four backends). 7 backends, 7 variants, 3 blocks. The adapters are the lab's, as configured for the benchmarks: none of them retries a failed request.

| Backend | Normal | 429 + Retry-After | Single 503 | Truncated | Cut off | Redirect loop | Slow server |
| --- | --- | --- | --- | --- | --- | --- | --- |
| node-http (fetch) | ok | error, no retry | error, no retry | explicit error | explicit error | explicit error | explicit error |
| crawlee-http | ok | error, no retry | error, no retry | explicit error | explicit error | explicit error | complete data, deadline ignored |
| crawlee-http-pooled | ok | error, no retry | error, no retry | explicit error | explicit error | explicit error | complete data, deadline ignored |
| spider-http | ok | **false success** | **false success** | **false success** | **false success** | **false success** | **false success** |
| reqwest-http | ok | error, no retry | error, no retry | explicit error | explicit error | explicit error | explicit error |
| wreq-http | ok | error, no retry | error, no retry | explicit error | explicit error | explicit error | explicit error |
| impit-http | ok | error, no retry | error, no retry | explicit error | explicit error | explicit error | explicit error |

- Spider HTTP drops the page with the incident and returns the rest as a success: records are missing, without any error. It is the only backend that produces false successes.
- No backend honors `Retry-After` or retries after a 503: the retry policy remains to be built in the engine, and these two variants are its contract.
- reqwest, wreq and Impit behave like `fetch`: explicit errors, no retry. They are transport building blocks; retrying belongs to the engine.
- Crawlee does not apply the task deadline to its HTTP requests: it waits for the slow response and returns it complete.

A first run (`http-incidents-2026-10-08T17-03-09-936Z-726cb0`) gave the same results, with less precise reasons, on uncommitted sources.
