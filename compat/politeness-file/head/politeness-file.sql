PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE domain_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT;
INSERT INTO "domain_limits" VALUES('shop.test',1767603602000,0,1,1767603600000,1767603602000);
INSERT INTO "domain_limits" VALUES('hold.test',NULL,0,NULL,NULL,1767603602000);
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('politeness-file','0.1');
CREATE TABLE permits (
        permit_id INTEGER NOT NULL PRIMARY KEY,
        key TEXT NOT NULL,
        run_id TEXT NOT NULL,
        epoch INTEGER NOT NULL,
        run_folder TEXT NOT NULL,
        engine TEXT NOT NULL,
        taken_at INTEGER NOT NULL
    ) STRICT;
INSERT INTO "permits" VALUES(1,'hold.test','holding',1,'0.0/runs/holding','903df1a0ade0b876',1767603602000);
CREATE TABLE returns (key TEXT NOT NULL PRIMARY KEY, generation INTEGER NOT NULL) STRICT;
INSERT INTO "returns" VALUES('shop.test',2);
CREATE TABLE waiters (
        key TEXT NOT NULL,
        run_id TEXT NOT NULL,
        endpoint TEXT NOT NULL,
        turn INTEGER NOT NULL,
        woken INTEGER NOT NULL,
        woken_at INTEGER NOT NULL,
        PRIMARY KEY (key, run_id)
    ) STRICT;
INSERT INTO "waiters" VALUES('hold.test','holding','wake.sock',1,0,0);
CREATE INDEX permits_by_key ON permits (key);
COMMIT;
