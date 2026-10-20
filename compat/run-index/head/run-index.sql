PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-index','0.1');
CREATE TABLE runs (
        run_id TEXT NOT NULL PRIMARY KEY,
        location TEXT NOT NULL,
        kind TEXT NOT NULL,
        needs TEXT NOT NULL,
        tags TEXT NOT NULL,
        lifecycle TEXT NOT NULL,
        verdict TEXT,
        completeness TEXT,
        started_at TEXT NOT NULL,
        ended_at TEXT
    ) STRICT;
INSERT INTO "runs" VALUES('finished-run','runs/finished-run','collie','collie','["schedule:nightly","team:shop"]','finished','verified','complete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z');
INSERT INTO "runs" VALUES('aborted-run','runs/aborted-run','collie','collie','["manual"]','stopped','verified','incomplete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z');
COMMIT;
