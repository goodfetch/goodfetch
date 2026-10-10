# Access policy: robots.txt and challenge page, October 8, 2026

ADR-002 contracts testable on the existing backends, in `local-dev` on committed sources. Retained run: `access-policy-2026-10-08T17-27-03-497Z-f99eb0` (it replaces `access-policy-2026-10-08T17-15-34-616Z-cad714`, same results for the first four backends). 7 backends, 3 variants, 3 blocks. The campaign requests compliance with robots.txt: Crawlee receives `respectRobotsTxtFile`, Spider `respect_robots_txt`; node-http, reqwest, wreq and Impit have no equivalent option.

| Backend | Normal | Page disallowed by robots.txt | Challenge page served as 200 |
| --- | --- | --- | --- |
| node-http (fetch) | ok | **requests it anyway** | **false success** |
| crawlee-http | ok | respected | **false success** |
| crawlee-http-pooled | ok | respected, but the adapter stays blocked until the timeout | **false success** |
| spider-http | ok | respected | **false success** |
| reqwest-http | ok | **requests it anyway** | **false success** |
| wreq-http | ok | **requests it anyway** | **false success** |
| impit-http | ok | **requests it anyway** | **false success** |

- No backend recognizes a challenge page served as 200: all of them return the partial data as a success. Challenge detection remains to be built in the engine, and this variant is its contract.
- Pooled Crawlee does skip the disallowed page, but its task-completion tracking does not count the skipped request: the task never completes.

Content-Signal, TDMRep and Web Bot Auth assume an engine that declares a purpose or signs its requests; they will be tested with the identity profile prototype.
