# Security policy

This policy covers both Goodfetch Products, Collie and Foxhound, and every package, image and file this repository publishes (ADRs 0030 and 0067).

## Reporting a vulnerability

Report a vulnerability privately, through GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability**. Never open a public issue, discussion or pull request about it.

Include what you can: the affected Product and version, the operating system, the steps or input that show the flaw, and what an attacker gains.

## Acknowledgement

We acknowledge every report within three business days, and tell you whether we can reproduce it and what happens next.

## Coordinated disclosure

We fix the flaw and publish an advisory within 90 days of your report, coordinating the date with you. If a fix needs longer, we agree a new date with you before the 90 days end. We credit you in the advisory unless you ask us not to.

## What counts as a vulnerability

- **Anything that crosses a trust boundary** of the table in [`TESTING.md`](docs/testing/security.md#trust-boundaries): untrusted decoders, the Script sandbox, Plugin confinement, Chromium's sandbox, egress, the endpoints and the peer link, secrets, file outputs, the audit log, the reset gate, unsafe code, and the supply chain and workflows.
- **Any memory unsafety**, in Goodfetch's own code or in what it links.
- **A limit that never fires**: a configured limit on resources, time, cost, egress or outputs that the engine lets a run pass.

A flaw that crosses none of these, such as a site detecting the browser, is a bug: report it as an ordinary issue.
