PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE domain_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoff INTEGER,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT;
INSERT INTO "domain_limits" VALUES('shop.test',1767603614773,9474,1,1767603600000,1767603605299);
INSERT INTO "domain_limits" VALUES('hold.test',NULL,NULL,NULL,NULL,1767603605299);
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
INSERT INTO "permits" VALUES(1,'hold.test','holding',1,'0.0/runs/holding','28bd8653e56a5d40',1767603605299);
CREATE TABLE proxy_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT;
INSERT INTO "proxy_limits" VALUES('proxy http://proxy.test:3128 corpus',1,1767603610300,1767603610000);
CREATE TABLE returns (key TEXT NOT NULL PRIMARY KEY, generation INTEGER NOT NULL) STRICT;
INSERT INTO "returns" VALUES('shop.test',2);
INSERT INTO "returns" VALUES('proxy http://proxy.test:3128 corpus',1);
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
