PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE crawl_delays (
            key TEXT NOT NULL,
            location TEXT NOT NULL,
            delay INTEGER NOT NULL,
            PRIMARY KEY (key, location)
        ) STRICT;
INSERT INTO "crawl_delays" VALUES('shop.test','https://shop.test/robots.txt',2000);
CREATE TABLE domain_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoff INTEGER,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER,
            last_send INTEGER
        ) STRICT;
INSERT INTO "domain_limits" VALUES('shop.test',1767603622973,15588,1,1767603602000,1767603620000,1767603620000);
INSERT INTO "domain_limits" VALUES('hold.test',NULL,NULL,NULL,NULL,1767603607385,1767603607385);
CREATE TABLE domain_pacing (
            key TEXT NOT NULL PRIMARY KEY,
            latency INTEGER,
            theoretical_start INTEGER,
            interval INTEGER,
            theoretical_send INTEGER
        ) STRICT;
INSERT INTO "domain_pacing" VALUES('shop.test',120,1767603609385,2000,1767603622000);
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
INSERT INTO "permits" VALUES(1,'hold.test','holding',1,'0.0/runs/holding','374ad8b83fe02477',1767603607385);
CREATE TABLE proxy_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT;
INSERT INTO "proxy_limits" VALUES('proxy http://proxy.test:3128 corpus',1,1767603610300,1767603610000);
CREATE TABLE returns (key TEXT NOT NULL PRIMARY KEY, generation INTEGER NOT NULL) STRICT;
INSERT INTO "returns" VALUES('shop.test',2);
CREATE TABLE robots_txt (
            location TEXT NOT NULL PRIMARY KEY,
            fetched_at INTEGER NOT NULL,
            status INTEGER NOT NULL,
            body BLOB NOT NULL
        ) STRICT;
INSERT INTO "robots_txt" VALUES('https://shop.test/robots.txt',1767603600000,200,X'557365722D6167656E743A202A0A446973616C6C6F773A202F707269766174652F0A437261776C2D64656C61793A20320A');
INSERT INTO "robots_txt" VALUES('https://hold.test/robots.txt',1767603607385,404,X'6E6F7420666F756E64');
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
