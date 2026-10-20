# Feature report workspace

A workspace for `xtask features report` (ADRs 0072, 0075 and 0094): one
Feature for each status and cause the report gives, the tests that declare
them, a Journey per due-milestone case, a quarantine list and the runners'
listings. Each test writes the results store it reads. Its inventory is
written by hand: the report reads it as committed.
