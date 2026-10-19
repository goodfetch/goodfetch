// A stand-in for `goodfetch-test-env` that misbehaves in one named way, so
// the package's tests show each check failing on a bad input. Run as
// `node fake-test-env.mjs <behaviour> [--test-name <name>]`.
const behaviour = process.argv[2];

const description = {
  folder: "/nonexistent/fake test é",
  home: "/nonexistent/fake test é/home",
  env: { GOODFETCH_HOME: "/nonexistent/fake test é/home" },
  seed: 1,
  start_time: "2026-01-05T09:00:00Z",
  tools: {},
};

switch (behaviour) {
  case "bad-description":
    // The seed is text, which the committed schema refuses.
    process.stdout.write(`${JSON.stringify({ ...description, seed: "1" })}\n`);
    process.stdin.resume();
    break;
  case "fails-to-start":
    process.stderr.write("fake-test-env: cannot prepare the test's folder\n");
    process.exitCode = 1;
    break;
  case "fails-teardown":
    process.stdout.write(`${JSON.stringify(description)}\n`);
    process.stdin.on("end", () => {
      process.exitCode = 1;
    });
    process.stdin.resume();
    break;
  case "stays-alive":
    process.stdout.write(`${JSON.stringify(description)}\n`);
    setInterval(() => {}, 1_000);
    break;
  default:
    process.stderr.write(`fake-test-env: unknown behaviour ${behaviour}\n`);
    process.exitCode = 2;
}
