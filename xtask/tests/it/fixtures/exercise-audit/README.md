# Exercise audit workspace

A workspace whose tests declare `operation:`, `error:`, `event:`, `command:`
and `exit:` Features (ADR 0072), with the exercise record each wrote while it
ran, as test support writes them under `just test`: a first line naming the
test, then the engine's JSON log lines and the command records. Every
declaring test reached what it declares, so
`xtask features audit --records records` passes; each test of the audit
removes or changes one record to show the failure it catches.

- The unit test that asserts `error:sample_refused` without `rpc` has no
  record, which the audit allows (ADR 0072). The unit test
  `echoes_in_process` reaches `rpc`, so each `error:` or `event:` it
  declared would be audited, as at every level.
- The integration test `refuses_a_text_the_stand_in_rejects` asserts
  `error:sample_refused` and `event:sample.echoed` itself and never reaches
  `rpc`: its record holds no dispatch record, which the audit allows. Unlike
  a unit test, it must still leave a record. The `dispatch-missing` cases
  add an `attached-engine` command record with exit code 0 to a record
  with no dispatch record: an engine served the test with its JSON log off.
- `crates/goodfetch-protocol/generated/catalog.json` holds the exit code of
  `exit:not_installed`.
- `junit/nextest.xml` lists the tests that ran: all but
  `installs_with_its_exit_code`.
