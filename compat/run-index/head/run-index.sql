PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE consent_walls (
        domain TEXT NOT NULL,
        run_id TEXT NOT NULL,
        seen_at TEXT NOT NULL,
        PRIMARY KEY (domain, run_id)
    ) STRICT;
INSERT INTO "consent_walls" VALUES('walled.example','finished-run','2026-01-05T09:00:00Z');
CREATE TABLE format (
    name TEXT NOT NULL PRIMARY KEY,
    version TEXT NOT NULL,
    lowest_minor INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT INTO "format" VALUES('run-index','0.1',0);
CREATE TABLE needs_you_dismissed (
        item TEXT NOT NULL PRIMARY KEY,
        dismissed_at TEXT NOT NULL
    ) STRICT;
INSERT INTO "needs_you_dismissed" VALUES('corpus_waiting:1','2026-01-05T09:00:00Z');
CREATE TABLE refetch_stops (
        domain TEXT NOT NULL,
        run_id TEXT NOT NULL,
        cause TEXT NOT NULL,
        project_file TEXT,
        stopped_at TEXT NOT NULL,
        PRIMARY KEY (domain, run_id)
    ) STRICT;
INSERT INTO "refetch_stops" VALUES('walled.example','finished-run','challenge',NULL,'2026-01-05T09:00:00Z');
INSERT INTO "refetch_stops" VALUES('walled.example','aborted-run','status_403','/corpus/project/goodfetch.toml','2026-01-05T09:00:00Z');
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
        ended_at TEXT,
        owner TEXT,
        presence TEXT NOT NULL,
        progress_message TEXT,
        stop_reason TEXT,
        replayed INTEGER NOT NULL DEFAULT 0
    ) STRICT;
INSERT INTO "runs" VALUES('finished-run','runs/finished-run','collie','collie','["schedule:nightly","team:shop"]','finished','verified','complete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z',NULL,'present','Read 3 of 3 pages',NULL,0);
INSERT INTO "runs" VALUES('aborted-run','runs/aborted-run','collie','collie','["manual"]','stopped','verified','incomplete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z',NULL,'present',NULL,'{"kind":"abort","by":"client"}',0);
INSERT INTO "runs" VALUES('live-run','runs/live-run','collie','collie','["daemon"]','running',NULL,NULL,'2026-01-05T09:00:00Z',NULL,'{"daemon":{"unix":"daemon.sock"}}','present',NULL,NULL,0);
COMMIT;
