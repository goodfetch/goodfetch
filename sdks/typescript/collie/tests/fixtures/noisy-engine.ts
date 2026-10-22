#!/usr/bin/env node
// An engine stand-in that writes 80,000 bytes of two-byte characters on its
// stderr, with no line end, and exits with code 3 before answering anything.
process.stderr.write("é".repeat(40_000));
process.exitCode = 3;
