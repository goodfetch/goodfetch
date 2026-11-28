PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('usage-ledger','0.1');
CREATE TABLE limit_needs_you (
        limit_id INTEGER NOT NULL,
        period_start TEXT NOT NULL,
        limit_amount INTEGER NOT NULL,
        reached_at TEXT NOT NULL,
        PRIMARY KEY (limit_id, period_start, limit_amount)
    ) STRICT;
INSERT INTO "limit_needs_you" VALUES(8,'',2,'2026-01-05T09:00:00Z');
CREATE TABLE limit_thresholds (
        limit_id INTEGER NOT NULL,
        period_start TEXT NOT NULL,
        threshold INTEGER NOT NULL,
        limit_amount INTEGER NOT NULL,
        slice_id INTEGER NOT NULL,
        crossed_at TEXT NOT NULL,
        sent_at TEXT,
        PRIMARY KEY (limit_id, period_start, threshold, limit_amount)
    ) STRICT;
INSERT INTO "limit_thresholds" VALUES(8,'',50,2,13,'2026-01-05T09:00:00Z','2026-01-05T09:00:00Z');
INSERT INTO "limit_thresholds" VALUES(8,'',80,2,14,'2026-01-05T09:00:00Z',NULL);
INSERT INTO "limit_thresholds" VALUES(8,'',100,2,14,'2026-01-05T09:00:00Z',NULL);
CREATE TABLE limits (
        limit_id INTEGER PRIMARY KEY,
        scope_kind TEXT NOT NULL,
        scope_id TEXT NOT NULL,
        usage_kind TEXT NOT NULL,
        currency TEXT,
        period TEXT,
        time_zone TEXT,
        amount INTEGER NOT NULL,
        must_understand INTEGER NOT NULL,
        set_by TEXT NOT NULL,
        set_at TEXT NOT NULL,
        managed_major TEXT
    ) STRICT;
INSERT INTO "limits" VALUES(1,'install','','requests',NULL,'month','UTC',100000,1,'person','2026-01-05T09:00:00Z',NULL);
INSERT INTO "limits" VALUES(2,'daemon','0.0','requests',NULL,'day','Europe/Paris',20000,1,'person','2026-01-05T09:00:00Z',NULL);
INSERT INTO "limits" VALUES(3,'connection','corpus-connection','requests',NULL,NULL,NULL,500,1,'client','2026-01-05T09:00:00Z',NULL);
INSERT INTO "limits" VALUES(4,'install','','paid_fetch','USD','month','America/New_York',25000000,1,'person','2026-01-05T09:00:00Z',NULL);
INSERT INTO "limits" VALUES(5,'install','','requests',NULL,'week','Asia/Tokyo',50000,1,'managed_policy','2026-01-05T09:00:00Z','0.0');
INSERT INTO "limits" VALUES(6,'run','live-run','requests',NULL,NULL,NULL,1000,1,'client','2026-01-05T09:00:00Z',NULL);
INSERT INTO "limits" VALUES(7,'run','live-run','paid_fetch','USD',NULL,NULL,2000000,1,'client','2026-01-05T09:00:00Z',NULL);
INSERT INTO "limits" VALUES(8,'connection','threshold-corpus','requests',NULL,NULL,NULL,2,1,'client','2026-01-05T09:00:00Z',NULL);
CREATE TABLE run_limits (
        run_id TEXT NOT NULL,
        usage_kind TEXT NOT NULL,
        currency TEXT,
        amount INTEGER NOT NULL
    ) STRICT;
INSERT INTO "run_limits" VALUES('live-run','requests',NULL,1000);
INSERT INTO "run_limits" VALUES('live-run','paid_fetch','USD',2000000);
CREATE TABLE slices (
        slice_id INTEGER PRIMARY KEY,
        limit_id INTEGER NOT NULL,
        period_start TEXT NOT NULL,
        run_id TEXT NOT NULL,
        run_epoch INTEGER NOT NULL,
        run_folder TEXT NOT NULL,
        size INTEGER NOT NULL,
        charged INTEGER,
        state TEXT NOT NULL,
        taken_at TEXT NOT NULL
    ) STRICT;
INSERT INTO "slices" VALUES(1,1,'2026-01-01T00:00:00Z','ended-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/ended-run',1000,40,'returned','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(2,2,'2026-01-04T23:00:00Z','ended-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/ended-run',200,40,'returned','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(3,5,'2026-01-04T15:00:00Z','ended-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/ended-run',500,40,'returned','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(4,1,'2026-01-01T00:00:00Z','dead-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/dead-run',1000,12,'reclaimed','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(5,2,'2026-01-04T23:00:00Z','dead-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/dead-run',200,12,'reclaimed','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(6,5,'2026-01-04T15:00:00Z','dead-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/dead-run',500,12,'reclaimed','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(7,1,'2026-01-01T00:00:00Z','unreadable-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/unreadable-run',1000,1000,'review','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(8,2,'2026-01-04T23:00:00Z','unreadable-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/unreadable-run',200,200,'review','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(9,5,'2026-01-04T15:00:00Z','unreadable-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/unreadable-run',500,500,'review','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(10,1,'2026-01-01T00:00:00Z','live-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/live-run',1000,NULL,'held','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(11,2,'2026-01-04T23:00:00Z','live-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/live-run',200,NULL,'held','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(12,5,'2026-01-04T15:00:00Z','live-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/live-run',500,NULL,'held','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(13,8,'','threshold-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/threshold-run',1,1,'returned','2026-01-05T09:00:00Z');
INSERT INTO "slices" VALUES(14,8,'','threshold-run',1,'/home/someone/.local/share/goodfetch/0.0/runs/threshold-run',1,1,'returned','2026-01-05T09:00:00Z');
CREATE INDEX limit_thresholds_by_slice ON limit_thresholds (slice_id, sent_at);
CREATE INDEX slices_by_limit ON slices (limit_id, period_start, state);
CREATE INDEX slices_by_state ON slices (state);
COMMIT;
